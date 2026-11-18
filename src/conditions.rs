//! Host facts shared by DSC documents and the DSC expressions in collection metadata.
use anyhow::Result;
use dsc_lib::{
    configure::{
        config_doc::DataType,
        context::{Context, ProcessMode},
    },
    parser::Statement,
};
use serde_json::{Map, Value, json};

pub(crate) const HOST: &str = "furnish.host";

fn host(doc: &Value) -> Result<Value> {
    let distro = crate::platform::distribution();
    let mut managers: Vec<String> = crate::platform::package_managers().into_iter().map(str::to_owned).collect();
    managers.extend(crate::script_managers::available(doc)?.into_keys());
    Ok(json!({"distribution": distro.id, "distributionLike": distro.like, "packageManagers": managers}))
}

fn context(values: &Map<String, Value>) -> Context {
    let mut context = Context::new();
    for (name, value) in values {
        context.parameters.insert(name.clone(), (value.clone(), data_type(value)));
    }
    context
}

fn data_type(value: &Value) -> DataType {
    match value {
        Value::Bool(_) => DataType::Bool,
        Value::Number(_) => DataType::Int,
        Value::Array(_) => DataType::Array,
        Value::Object(_) => DataType::Object,
        _ => DataType::String,
    }
}

fn parser() -> Result<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_dscexpression::LANGUAGE.into())?;
    Ok(parser)
}

fn member<'a>(accessor: Option<tree_sitter::Node>, index: u32, src: &'a str) -> Option<&'a str> {
    let field = accessor?.named_child(index)?;
    let text = field.child_by_field_name("name").or_else(|| field.child_by_field_name("indexValue")?.child_by_field_name("string"))?;
    Some(&src[text.byte_range()])
}

fn family_read(function: tree_sitter::Node, src: &str) -> Option<std::ops::Range<usize>> {
    let accessor = function.parent()?.child_by_field_name("accessor");
    if function.child_by_field_name("args").is_some() || member(accessor, 0, src) != Some("os") || member(accessor, 1, src) != Some("family") {
        return None;
    }
    Some(function.start_byte()..accessor?.named_child(1)?.end_byte())
}

fn uses_host(node: tree_sitter::Node, src: &str) -> bool {
    if let Some(name) = node.child_by_field_name("name")
        && node.kind() == "function"
        && &src[name.byte_range()] == "variables"
        && let Some(arg) = node.child_by_field_name("args").and_then(|a| a.named_child(0))
        && (arg.kind() == "expression" || (arg.kind() == "string" && &src[arg.byte_range()] == HOST))
    {
        return true;
    }
    node.named_children(&mut node.walk()).any(|child| uses_host(child, src))
}

/// DSC functions whose result depends on the invoking host, a live resource or the time.
fn machine_function(name: &str) -> bool {
    matches!(name, "envvar" | "systemRoot" | "tryWhich" | "utcNow" | "secret" | "reference" | "path" | "restartRequired" | "stateChanged" | "stdout")
}

fn expression_uses_host(src: &str, parser: &mut Option<tree_sitter::Parser>) -> Result<bool> {
    if !src.starts_with('[') || src.starts_with("[[") {
        return Ok(false);
    }
    if parser.is_none() {
        *parser = Some(self::parser()?);
    }
    Ok(parser.as_mut().unwrap().parse(src, None).is_some_and(|tree| uses_host(tree.root_node(), src)))
}

/// Package managers can change between steps. Take fresh facts when an expression
/// needs them, avoiding directory and PATH probes for documents with no host reads.
pub(crate) fn supply_host(doc: &Value, context: &mut Context) -> Result<()> {
    fn needs_host(value: &Value, parser: &mut Option<tree_sitter::Parser>) -> Result<bool> {
        match value {
            Value::String(s) => expression_uses_host(s, parser),
            Value::Object(o) => o.values().try_fold(false, |found, v| Ok(found || needs_host(v, parser)?)),
            Value::Array(a) => a.iter().try_fold(false, |found, v| Ok(found || needs_host(v, parser)?)),
            _ => Ok(false),
        }
    }
    if needs_host(doc, &mut None)? {
        context.variables.insert(HOST.into(), host(doc)?);
    }
    Ok(())
}

fn execute(doc: &Value, expr: &str, mut context: Context) -> Result<Value> {
    if expression_uses_host(expr, &mut None)? {
        context.variables.insert(HOST.into(), host(doc)?);
    }
    Ok(Statement::new()?.parse_and_execute(expr, &context)?)
}

pub fn holds(expr: &str, values: &Map<String, Value>) -> Result<bool> {
    holds_for(&Value::Null, expr, values)
}

pub(crate) fn holds_for(doc: &Value, expr: &str, values: &Map<String, Value>) -> Result<bool> {
    Ok(execute(doc, expr, context(values))? == Value::Bool(true))
}

/// Resolve author-supplied DSC parameter defaults before Furnish uses them in its
/// own metadata and resource filtering. DSC reports invalid defaults on validation.
pub(crate) fn parameter_default(doc: &Value, value: &Value) -> Value {
    let Some(expr) = value.as_str().filter(|s| s.starts_with('[')) else { return value.clone() };
    let mut ctx = Context::new();
    ctx.process_mode = ProcessMode::ParametersDefault;
    execute(doc, expr, ctx).unwrap_or_else(|_| value.clone())
}

/// An OS-family preview has no concrete package managers or Linux distribution.
/// Use the same grammar as DSC to identify reads of those facts (including within
/// a larger expression). DSC still parses, type-checks and evaluates the calls.
pub(crate) struct Preview {
    system: &'static str,
    context: Context,
    unknown_parameters: std::collections::BTreeSet<String>,
    unknown_variables: std::collections::BTreeSet<String>,
}

impl Preview {
    pub(crate) fn new(system: &'static str, values: &Map<String, Value>) -> Self {
        let mut ctx = context(values);
        // Representative values used only to validate host-dependent expressions.
        let (id, like) = if system == "Linux" { ("debian", vec!["debian"]) } else { ("", vec![]) };
        ctx.variables.insert(HOST.into(), json!({"distribution": id, "distributionLike": like, "packageManagers": ["nix"]}));
        Self { system, context: ctx, unknown_parameters: Default::default(), unknown_variables: Default::default() }
    }

    /// Prepare a document whose DSC defaults and variables no longer consult the checker host.
    /// Unknown values stay unset; known values still go through DSC type and constraint validation.
    pub(crate) fn document(&mut self, source: &Value, shared: &std::collections::BTreeMap<String, crate::options::Declaration>) -> Result<Value> {
        fn literal(value: Value) -> Value {
            match value {
                Value::String(s) if s.starts_with('[') => format!("[{s}").into(),
                v => v,
            }
        }
        let mut doc = source.clone();
        for (name, param) in doc.get_mut("parameters").and_then(Value::as_object_mut).into_iter().flatten() {
            let value = if let Some(value) = shared.get(name).and_then(|d| d.default.as_ref()) {
                Some(value.clone())
            } else if let Some(value) = param.get("defaultValue") {
                self.context.process_mode = ProcessMode::ParametersDefault;
                let result = self.value(value);
                self.context.process_mode = ProcessMode::Normal;
                result?
            } else if let Some((sample, _)) = self.context.parameters.get(name) {
                Some(sample.clone())
            } else {
                continue;
            };
            if let Some(value) = value {
                self.context.parameters.insert(name.clone(), (value.clone(), data_type(&value)));
                param["defaultValue"] = literal(value);
            } else {
                self.unknown_parameters.insert(name.clone());
                if let Some(fields) = param.as_object_mut() {
                    fields.remove("defaultValue");
                }
            }
        }
        if let Some(variables) = doc.get_mut("variables").and_then(Value::as_object_mut) {
            let mut known = Map::new();
            for (name, raw) in variables.iter() {
                if let Some(value) = self.value(raw)? {
                    self.context.variables.insert(name.clone(), value.clone());
                    known.insert(name.clone(), literal(value));
                } else {
                    self.unknown_variables.insert(name.clone());
                }
            }
            *variables = known;
        }
        // Furnish evaluates metadata and step filters with parameters and its own host facts.
        self.context.variables.retain(|name, _| name == HOST);
        self.unknown_variables.clear();
        Ok(doc)
    }

    fn value(&self, value: &Value) -> Result<Option<Value>> {
        match value.as_str().filter(|s| s.starts_with('[')) {
            Some(expr) => self.evaluate(expr),
            None => Ok(Some(value.clone())),
        }
    }

    pub(crate) fn condition(&self, expr: &str) -> Result<Option<bool>> {
        Ok(self.evaluate(expr)?.map(|v| v == Value::Bool(true)))
    }

    fn evaluate(&self, src: &str) -> Result<Option<Value>> {
        let tree = parser()?.parse(src, None).ok_or_else(|| anyhow::anyhow!("unable to parse DSC expression"))?;
        let root = tree.root_node();
        anyhow::ensure!(!root.has_error(), "invalid DSC expression {src}");
        if self.needs_unsupplied_fact(root, src) {
            validate_calls(root, src)?;
            return Ok(None);
        }
        let mut replacements = Vec::new();
        let mut unknown = false;
        self.inspect(root, src, &mut unknown, &mut replacements);
        let mut code = src.to_string();
        for (range, value) in replacements.into_iter().rev() {
            code.replace_range(range, &value);
        }
        let value = Statement::new()?.parse_and_execute(&code, &self.context)?;
        Ok((!unknown).then_some(value))
    }

    fn needs_unsupplied_fact(&self, node: tree_sitter::Node, src: &str) -> bool {
        if node.kind() == "function" && let Some(name) = node.child_by_field_name("name") {
            let arg = node.child_by_field_name("args").and_then(|args| args.named_child(0));
            let reads = |unknown: &std::collections::BTreeSet<String>| arg.is_some_and(|a|
                !unknown.is_empty() && (a.kind() == "expression" || unknown.contains(&src[a.byte_range()])));
            match &src[name.byte_range()] {
                name if machine_function(name) => return true,
                "context" if family_read(node, src).is_none() => return true,
                "parameters" if reads(&self.unknown_parameters) => return true,
                "variables" if reads(&self.unknown_variables) => return true,
                _ => {}
            }
        }
        node.named_children(&mut node.walk()).any(|child| self.needs_unsupplied_fact(child, src))
    }

    fn inspect(&self, node: tree_sitter::Node, src: &str, unknown: &mut bool, replacements: &mut Vec<(std::ops::Range<usize>, String)>) {
        if let Some(function) = node.child_by_field_name("function") {
            let name = &src[function.child_by_field_name("name").unwrap().byte_range()];
            let accessor = node.child_by_field_name("accessor");
            let first_arg = function.child_by_field_name("args").and_then(|a| a.named_child(0));
            match name {
                "context" => {
                    if let Some(range) = family_read(function, src) {
                        replacements.push((range, format!("string('{}')", self.system)));
                    } else {
                        *unknown = true;
                    }
                }
                "variables" if first_arg.is_some_and(|arg| arg.kind() == "string" && &src[arg.byte_range()] == HOST) => {
                    *unknown |= self.system == "Linux" || !matches!(member(accessor, 0, src), Some("distribution" | "distributionLike"));
                    // Validate indexing into a host-dependent array using its representative
                    // element. Its actual length is known only when that DSC resource runs.
                    if matches!(member(accessor, 0, src), Some("distributionLike" | "packageManagers"))
                        && let Some(index) = accessor.and_then(|a| a.named_child(1)).and_then(|i| i.child_by_field_name("indexValue"))
                        && index.kind() == "number"
                        && !src[index.byte_range()].starts_with('-')
                    {
                        *unknown = true;
                        // Substitute the element, retaining the native DSC type for the rest
                        // of this expression. An absent member is diagnosed by DSC normally.
                        replacements.push((function.start_byte()..index.parent().unwrap().end_byte(), "string('example')".into()));
                    }
                }
                "variables" if first_arg.is_some_and(|a| a.kind() == "expression") => *unknown = true,
                _ => {}
            }
        }
        for child in node.named_children(&mut node.walk()) {
            self.inspect(child, src, unknown, replacements);
        }
    }
}

/// Check names, arity and literal argument types even when a machine-dependent call cannot execute.
fn validate_calls(node: tree_sitter::Node, src: &str) -> Result<()> {
    use dsc_lib::functions::{FunctionArgKind, FunctionDefinition, FunctionDispatcher};
    use std::{collections::HashMap, sync::LazyLock};
    static FUNCTIONS: LazyLock<HashMap<String, FunctionDefinition>> = LazyLock::new(|| {
        FunctionDispatcher::new().list().into_iter().map(|f| (f.name.clone(), f)).collect()
    });
    if node.kind() == "function" && let Some(name) = node.child_by_field_name("name") {
        let name = &src[name.byte_range()];
        if !name.contains('.') {
            let f = FUNCTIONS.get(name).ok_or_else(|| anyhow::anyhow!("unknown DSC function {name}"))?;
            let args = node.child_by_field_name("args");
            let count = args.map_or(0, |args| args.named_child_count());
            anyhow::ensure!((f.min_args..=f.max_args).contains(&count), "{name}() received {count} arguments");
            if let Some(args) = args {
                for (i, arg) in args.named_children(&mut args.walk()).enumerate() {
                    let literal = match arg.kind() {
                        "string" => FunctionArgKind::String,
                        "number" => FunctionArgKind::Number,
                        "boolean" => FunctionArgKind::Boolean,
                        _ => continue,
                    };
                    if let Some(allowed) = f.accepted_arg_ordered_types.get(i).or(f.remaining_arg_accepted_types.as_ref()) {
                        anyhow::ensure!(allowed.contains(&literal), "{name}() argument {} does not accept {literal}", i + 1);
                    }
                }
            }
        }
    }
    for child in node.named_children(&mut node.walk()) {
        validate_calls(child, src)?;
    }
    Ok(())
}

/// Evaluate concrete target expressions with DSC's parameter, variable and property semantics.
/// Substitute only DSC calls, never text in string literals. Facts outside the profile must
/// produce a diagnostic instead of silently reading the machine on which CI runs.
pub(crate) struct TargetContext {
    system: &'static str,
    grammar: tree_sitter::Parser,
    statement: Statement,
    context: Context,
}

impl TargetContext {
    pub(crate) fn new(system: &'static str, host: Value, doc: &Value, input: Map<String, Value>) -> Result<Self> {
        use dsc_lib::{configure::Configurator, progress::ProgressFormat};
        let mut target = Self { system, grammar: parser()?, statement: Statement::new()?, context: Context::new() };
        let mut config = json!({"$schema": doc["$schema"], "resources": [], "directives": {"resourceDiscovery": "duringDeployment"}});
        for key in ["parameters", "variables", "functions"] {
            if let Some(value) = doc.get(key) {
                config[key] = value.clone();
            }
        }
        for (name, parameter) in config.get_mut("parameters").and_then(Value::as_object_mut).into_iter().flatten() {
            if !input.contains_key(name) && let Some(value) = parameter.get_mut("defaultValue") {
                target.rewrite(value)?;
            }
        }
        for key in ["variables", "functions"] {
            if let Some(value) = config.get_mut(key) {
                target.rewrite(value)?;
            }
        }
        let mut configurator = Configurator::new(&config.to_string(), ProgressFormat::None)?;
        configurator.context.variables.insert(HOST.to_string(), host);
        configurator.set_context((!input.is_empty()).then(|| json!({"parameters":input})).as_ref())?;
        target.context = configurator.context;
        Ok(target)
    }

    fn expression(&mut self, src: &str) -> Result<String> {
        if !src.starts_with('[') || src.starts_with("[[") {
            return Ok(src.to_owned());
        }
        let tree = self.grammar.parse(src, None).ok_or_else(|| anyhow::anyhow!("unable to parse DSC expression"))?;
        let root = tree.root_node();
        anyhow::ensure!(!root.has_error(), "invalid DSC expression {src}");
        let mut calls = Vec::new();
        fn visit(node: tree_sitter::Node, src: &str, calls: &mut Vec<std::ops::Range<usize>>) -> Result<()> {
            if node.kind() == "function" && let Some(name) = node.child_by_field_name("name") {
                match &src[name.byte_range()] {
                    "context" if node.child_by_field_name("args").is_none() => {
                        let range = family_read(node, src).ok_or_else(|| anyhow::anyhow!("only context().os.family is defined by the target profile"))?;
                        calls.push(range);
                    }
                    name if machine_function(name) => anyhow::bail!("{name}() requires facts outside the target profile"),
                    _ => {}
                }
            }
            for child in node.named_children(&mut node.walk()) {
                visit(child, src, calls)?;
            }
            Ok(())
        }
        visit(root, src, &mut calls)?;
        let mut rewritten = src.to_owned();
        for range in calls.into_iter().rev() {
            rewritten.replace_range(range, &format!("string('{}')", self.system));
        }
        Ok(rewritten)
    }

    fn rewrite(&mut self, value: &mut Value) -> Result<()> {
        match value {
            Value::String(s) => *s = self.expression(s)?,
            Value::Object(o) => o.values_mut().try_for_each(|v| self.rewrite(v))?,
            Value::Array(a) => a.iter_mut().try_for_each(|v| self.rewrite(v))?,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn condition(&mut self, expr: &str) -> Result<bool> {
        let expr = self.expression(expr)?;
        Ok(self.statement.parse_and_execute(&expr, &self.context)? == Value::Bool(true))
    }

    pub(crate) fn properties(&mut self, properties: &Value) -> Result<Value> {
        let mut properties = properties.clone();
        self.rewrite(&mut properties)?;
        Ok(serde_json::to_value(dsc_lib::configure::invoke_property_expressions(
            &mut self.statement, &self.context, properties.as_object(),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_previews_keep_runtime_facts_conditional_and_validate_dsc() {
        let linux = Preview::new("Linux", &Map::new());
        let windows = Preview::new("Windows", &Map::new());
        let distro = "[equals(variables('furnish.host').distribution, 'debian')]";
        assert_eq!(linux.condition(distro).unwrap(), None);
        assert_eq!(windows.condition(distro).unwrap(), Some(false));
        assert_eq!(windows.condition("[equals(context()['os'].family, 'Windows')]").unwrap(), Some(true));
        assert_eq!(linux.condition("[equals('context().os.family', 'Linux')]").unwrap(), Some(false));
        assert_eq!(linux.condition("[equals(variables('furnish.host')['distributionLike'][43], 'fedora')]").unwrap(), None);
        assert_eq!(windows.condition("[contains(variables('furnish.host').packageManagers, 'winget')]").unwrap(), None);
        assert!(linux.condition("[and('not a boolean', false)]").is_err());
        assert!(linux.condition("[equals(variables('furnish.host').distributionTypo, 'debian')]").is_err());
    }
}
