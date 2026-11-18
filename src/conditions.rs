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
        let kind = match value {
            Value::Bool(_) => DataType::Bool,
            Value::Number(_) => DataType::Int,
            Value::Array(_) => DataType::Array,
            Value::Object(_) => DataType::Object,
            _ => DataType::String,
        };
        context.parameters.insert(name.clone(), (value.clone(), kind));
    }
    context
}

fn parser() -> Result<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_dscexpression::LANGUAGE.into())?;
    Ok(parser)
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
}

impl Preview {
    pub(crate) fn new(system: &'static str, values: &Map<String, Value>) -> Self {
        let mut ctx = context(values);
        // Representative values used only to validate host-dependent expressions.
        let (id, like) = if system == "Linux" { ("debian", vec!["debian"]) } else { ("", vec![]) };
        ctx.variables.insert(HOST.into(), json!({"distribution": id, "distributionLike": like, "packageManagers": ["nix"]}));
        Self { system, context: ctx, unknown_parameters: Default::default() }
    }

    pub(crate) fn default(&mut self, name: &str, expr: &str) {
        match self.evaluate(expr) {
            Ok(Some(value)) => {
                if let Some(param) = self.context.parameters.get_mut(name) {
                    param.0 = value;
                }
            }
            _ => {
                self.unknown_parameters.insert(name.into());
            }
        }
    }

    pub(crate) fn condition(&self, expr: &str) -> Result<Option<bool>> {
        Ok(self.evaluate(expr)?.map(|v| v == Value::Bool(true)))
    }

    fn evaluate(&self, src: &str) -> Result<Option<Value>> {
        let tree = parser()?.parse(src, None).ok_or_else(|| anyhow::anyhow!("unable to parse DSC expression"))?;
        let root = tree.root_node();
        anyhow::ensure!(!root.has_error(), "invalid DSC expression {src}");
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

    fn inspect(&self, node: tree_sitter::Node, src: &str, unknown: &mut bool, replacements: &mut Vec<(std::ops::Range<usize>, String)>) {
        if let Some(function) = node.child_by_field_name("function") {
            let name = &src[function.child_by_field_name("name").unwrap().byte_range()];
            let accessor = node.child_by_field_name("accessor");
            let member = |i| {
                let field = accessor?.named_child(i)?;
                let text =
                    field.child_by_field_name("name").or_else(|| field.child_by_field_name("indexValue")?.child_by_field_name("string"))?;
                Some(&src[text.byte_range()])
            };
            let first_arg = function.child_by_field_name("args").and_then(|a| a.named_child(0));
            match name {
                "context" if first_arg.is_none() && member(0) == Some("os") && member(1) == Some("family") => {
                    let end = accessor.unwrap().named_child(1).unwrap().end_byte();
                    replacements.push((function.start_byte()..end, format!("string('{}')", self.system)));
                }
                "variables" if first_arg.is_some_and(|arg| arg.kind() == "string" && &src[arg.byte_range()] == HOST) => {
                    *unknown |= self.system == "Linux" || !matches!(member(0), Some("distribution" | "distributionLike"));
                    // Validate indexing into a host-dependent array using its representative
                    // element. Its actual length is known only when that DSC resource runs.
                    if matches!(member(0), Some("distributionLike" | "packageManagers"))
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
                "parameters"
                    if first_arg.is_some_and(|a| a.kind() == "string" && self.unknown_parameters.contains(&src[a.byte_range()])) =>
                {
                    *unknown = true
                }
                "parameters" if first_arg.is_some_and(|a| a.kind() == "expression") => *unknown |= !self.unknown_parameters.is_empty(),
                _ => {}
            }
        }
        for child in node.named_children(&mut node.walk()) {
            self.inspect(child, src, unknown, replacements);
        }
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
