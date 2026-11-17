//! furnish's resources for configuration: `Furnish/Directory`, `Furnish/File`, `Furnish/Block`,
//! `Furnish/Line`, `Furnish/Value` and `Furnish/Command`. They run inside the furnish program:
//! their manifests (written by `engine::init`) call it as `furnish dsc-resource <type> <get|test|set>`,
//! with the properties as JSON on stdin.
//!
//! Each resource has one function that looks at the system and returns its state and the
//! change that set would make (see [`Plan`]). test reports the desired state when there is no
//! change, and set changes only then, so test and set can't disagree.

mod command;
mod common;
pub mod text;
pub mod value;

use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use common::{Managed, Missing, Text, WriteOptions};

/// The first argument that makes furnish run a resource instead of a command.
pub const COMMAND: &str = "dsc-resource";

/// The resource types, their names on the command line, and their manifests.
pub const TYPES: [(&str, &str, &str); 6] = [
    ("Furnish/Directory", "directory", include_str!("../../resources/config/furnish-directory.dsc.resource.json")),
    ("Furnish/File", "file", include_str!("../../resources/config/furnish-file.dsc.resource.json")),
    ("Furnish/Block", "block", include_str!("../../resources/config/furnish-block.dsc.resource.json")),
    ("Furnish/Line", "line", include_str!("../../resources/config/furnish-line.dsc.resource.json")),
    ("Furnish/Value", "value", include_str!("../../resources/config/furnish-value.dsc.resource.json")),
    ("Furnish/Command", "command", include_str!("../../resources/config/furnish-command.dsc.resource.json")),
];

/// The manifests, with `program` (this furnish) as their executable: (file name, contents).
pub fn manifests(program: &Path) -> Result<Vec<(String, String)>> {
    TYPES
        .iter()
        .map(|(_, name, manifest)| {
            let mut manifest: Value = serde_json::from_str(manifest)?;
            for op in ["get", "test", "set"] {
                manifest[op]["executable"] = program.to_string_lossy().into_owned().into();
            }
            Ok((format!("furnish-{name}.dsc.resource.json"), serde_json::to_string_pretty(&manifest)?))
        })
        .collect()
}

/// Run a resource if the program was started as one (`furnish dsc-resource …`): the exit code.
/// Called first thing by furnish's programs.
pub fn dispatch() -> Option<u8> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some(COMMAND) {
        return None;
    }
    Some(match run(&args[1..]) {
        Ok(out) => {
            println!("{out}");
            0
        }
        // Plain text and an exit code the manifests don't map, so DSC keeps the line in its
        // error; "furnish: needs <program>" makes furnish show the app as needing it.
        Err(err) => match err.chain().find_map(|e| e.downcast_ref::<Missing>()) {
            Some(missing) => {
                eprintln!("furnish: {missing}");
                11
            }
            None => {
                eprintln!("furnish: {err:#}");
                2
            }
        },
    })
}

fn run(args: &[String]) -> Result<String> {
    let (kind, op, what_if) = match args {
        [kind, op] => (kind, op.as_str(), false),
        [kind, op, flag] if flag == "--what-if" => (kind, op.as_str(), true),
        _ => bail!("usage: furnish {COMMAND} <type> <get|test|set> [--what-if]"),
    };
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let props: Value = serde_json::from_str(&input).context("the properties on stdin aren't JSON")?;
    let plan = evaluate(kind, &props)?;
    let mut out = match op {
        "get" => plan.state,
        "test" => {
            let mut state = plan.state;
            state.insert("_inDesiredState".into(), plan.apply.is_none().into());
            state
        }
        "set" if what_if => {
            let mut after = plan.after;
            after.insert("_metadata".into(), json!({ "whatIf": plan.steps }));
            after
        }
        "set" => match plan.apply {
            None => plan.state,
            Some(apply) => {
                apply()?;
                let again = evaluate(kind, &props)?;
                if again.apply.is_some() {
                    bail!("still not in the desired state after the change ({})", plan.steps.join("; "));
                }
                again.state
            }
        },
        _ => bail!("unknown operation '{op}'"),
    };
    out.retain(|k, _| !k.starts_with('_') || ["_exist", "_inDesiredState", "_metadata"].contains(&k.as_str()));
    Ok(Value::Object(out).to_string())
}

/// A resource's state now, after set, and the change in between.
pub struct Plan {
    /// The current state (DSC's get): the properties, with the values found in place of those
    /// the resource observes.
    state: Map<String, Value>,
    /// The state set would leave.
    after: Map<String, Value>,
    /// What set would do, one line per step, for previews.
    steps: Vec<String>,
    /// Makes the change; `None` if the resource is in its desired state.
    apply: Option<Box<dyn FnOnce() -> Result<()>>>,
}

impl Plan {
    fn new(props: &Value) -> Self {
        let state = props.as_object().cloned().unwrap_or_default();
        Plan { after: state.clone(), state, steps: vec![], apply: None }
    }

    /// Set an observed property: its value now and after set (`None`: absent).
    fn observe(&mut self, key: &str, now: Option<Value>, after: Option<Value>) {
        for (map, v) in [(&mut self.state, now), (&mut self.after, after)] {
            match v {
                Some(v) => map.insert(key.into(), v),
                None => map.remove(key),
            };
        }
    }

    fn change(&mut self, steps: Vec<String>, apply: impl FnOnce() -> Result<()> + 'static) {
        self.steps = steps;
        self.apply = Some(Box::new(apply));
    }
}

fn evaluate(kind: &str, props: &Value) -> Result<Plan> {
    match kind {
        "directory" => directory(props),
        "file" => file(props),
        "block" => block(props),
        "line" => line(props),
        "value" => value(props),
        "command" => command::evaluate(props),
        _ => bail!("unknown resource type '{kind}'"),
    }
}

/// Check an app's properties for a resource type of this module when furnish loads the app, so
/// `furnish check` and loading report mistakes before anything runs. Properties with DSC
/// expressions are left to DSC and the resource.
pub fn check(kind: &str, props: &Value) -> Result<()> {
    let Some((_, name, _)) = TYPES.iter().find(|(t, _, _)| t.eq_ignore_ascii_case(kind)) else { return Ok(()) };
    let Some(props) = literal(props) else { return Ok(()) };
    match *name {
        "directory" => parse::<DirectoryProps>(&props).and_then(|p| p.check()),
        "file" => parse::<FileProps>(&props).and_then(|p| p.check()),
        "block" => parse::<BlockProps>(&props).and_then(|p| p.check().map(drop)),
        "line" => parse::<LineProps>(&props).and_then(|p| p.check().map(drop)),
        "value" => parse::<ValueProps>(&props).and_then(|p| p.check().map(drop)),
        _ => command::check(&props),
    }
}

/// The properties as the resource will get them: `None` if a value is a DSC expression (`[…]`),
/// with `[[` (DSC's escape for a leading `[`) unescaped.
fn literal(v: &Value) -> Option<Value> {
    Some(match v {
        Value::String(s) if s.starts_with("[[") => Value::String(s[1..].to_string()),
        Value::String(s) if s.starts_with('[') => return None,
        Value::Array(a) => Value::Array(a.iter().map(literal).collect::<Option<_>>()?),
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| Some((k.clone(), literal(v)?))).collect::<Option<_>>()?),
        other => other.clone(),
    })
}

/// Deserialize properties, ignoring what DSC adds (keys starting with `_`, except `_exist`).
fn parse<T: DeserializeOwned>(props: &Value) -> Result<T> {
    let mut props = props.as_object().cloned().unwrap_or_default();
    props.retain(|k, _| !k.starts_with('_') || k == "_exist");
    Ok(serde_json::from_value(Value::Object(props))?)
}

fn regex(pattern: &str, what: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("{what} '{pattern}' isn't a valid regular expression"))
}

/// `validate` needs `%s` where the new file's path goes.
fn check_validate(validate: &Option<Vec<String>>) -> Result<()> {
    if let Some(command) = validate
        && !command.iter().any(|a| a.contains("%s"))
    {
        bail!("validate: the command needs '%s' where the path of the new file goes, e.g. [ssh, -G, -F, '%s', host]");
    }
    Ok(())
}

fn exist(e: Option<bool>) -> bool {
    e.unwrap_or(true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryProps {
    path: String,
    mode: Option<String>,
    private: Option<bool>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
}

impl DirectoryProps {
    fn check(&self) -> Result<()> {
        check_mode(&self.mode, self.private)
    }
}

fn check_mode(mode: &Option<String>, private: Option<bool>) -> Result<()> {
    if mode.is_some() && private == Some(true) {
        bail!("mode and private: use one of them");
    }
    mode.as_deref().map(common::parse_mode).transpose()?;
    Ok(())
}

/// Observe and fix `mode` or `private` of an existing file or folder.
fn permissions(plan: &mut Plan, path: &Path, shown: &str, mode: Option<u32>, private: bool, folder: bool) -> Result<Vec<String>> {
    let mut steps = Vec::new();
    if let Some(mode) = mode
        && let Some(actual) = common::mode(path)
    {
        plan.observe("mode", Some(format!("{actual:03o}").into()), Some(format!("{mode:03o}").into()));
        if actual != mode {
            steps.push(format!("Set the mode of {shown} to {mode:o}"));
        }
    }
    if private {
        let actual = common::is_private(path, folder)?;
        plan.observe("private", Some(actual.into()), Some(true.into()));
        if !actual {
            steps.push(format!("Make {shown} accessible only to you"));
        }
    }
    Ok(steps)
}

fn directory(props: &Value) -> Result<Plan> {
    let p: DirectoryProps = parse(props)?;
    p.check()?;
    let mut plan = Plan::new(props);
    let path = common::expand(&p.path)?;
    let mode = p.mode.as_deref().map(common::parse_mode).transpose()?;
    let private = p.private.unwrap_or(false);
    let found = path.is_dir();
    if !found && path.exists() {
        bail!("{} isn't a folder", p.path);
    }
    let shown = p.path.clone();
    if !exist(p.exist) {
        if !found {
            plan.observe("_exist", Some(false.into()), Some(false.into()));
            return Ok(plan);
        }
        let empty = fs::read_dir(&path)?.next().is_none();
        plan.observe("_exist", Some(true.into()), Some((!empty).into()));
        if empty {
            plan.change(vec![format!("Remove the empty folder {shown}")], move || Ok(fs::remove_dir(&path)?));
        } else {
            plan.steps = vec![format!("Keep {shown}: it isn't empty")];
        }
        return Ok(plan);
    }
    plan.observe("_exist", Some(found.into()), Some(true.into()));
    let steps = if found {
        permissions(&mut plan, &path, &shown, mode, private, true)?
    } else {
        plan.observe("mode", None, mode.map(|m| format!("{m:03o}").into()));
        vec![format!("Create the folder {shown}")]
    };
    if !steps.is_empty() {
        plan.change(steps, move || {
            fs::create_dir_all(&path).with_context(|| format!("creating {shown}"))?;
            if let Some(mode) = mode {
                common::set_mode(&path, mode)?;
            }
            if private {
                common::set_private(&path, true)?;
            }
            Ok(())
        });
    }
    Ok(plan)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileProps {
    path: String,
    content: Option<String>,
    source: Option<String>,
    mode: Option<String>,
    private: Option<bool>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl FileProps {
    fn check(&self) -> Result<()> {
        if self.content.is_some() && self.source.is_some() {
            bail!("content and source: use one of them");
        }
        check_mode(&self.mode, self.private)?;
        check_validate(&self.validate)
    }
}

fn write_options(mode: Option<u32>, private: Option<bool>, backup: Option<bool>, validate: &Option<Vec<String>>) -> WriteOptions {
    WriteOptions { mode, private: private.unwrap_or(false), backup: backup.unwrap_or(false), validate: validate.clone() }
}

/// The preview lines for writing a file.
fn write_steps(file: &Managed, options: &WriteOptions, what: String) -> Vec<String> {
    let mut steps = vec![what];
    if options.backup && file.bytes.is_some() {
        steps.push(format!("Keep a copy of the current {} first", file.shown));
    }
    if let Some(command) = &options.validate {
        steps.push(format!("Check the new file first: {}", command.join(" ").replace("%s", "<new file>")));
    }
    steps
}

fn file(props: &Value) -> Result<Plan> {
    let p: FileProps = parse(props)?;
    p.check()?;
    let mut plan = Plan::new(props);
    let file = Managed::open(&p.path)?;
    let mode = p.mode.as_deref().map(common::parse_mode).transpose()?;
    let options = write_options(mode, p.private, p.backup, &p.validate);
    let found = file.bytes.is_some();
    if !exist(p.exist) {
        plan.observe("_exist", Some(found.into()), Some(false.into()));
        if found {
            let steps = vec![format!("Delete {}", file.shown)];
            plan.change(steps, move || common::delete(&file, &options));
        }
        return Ok(plan);
    }
    plan.observe("_exist", Some(found.into()), Some(true.into()));
    let current = file.bytes.as_deref().and_then(|b| Text::decode(b).ok());
    // The new contents, `None` if they stay.
    let new: Option<Vec<u8>> = match (&p.content, &p.source) {
        (Some(content), _) => {
            plan.observe("content", current.as_ref().map(|t| t.text.clone().into()), Some(content.clone().into()));
            match &current {
                Some(t) if t.text == *content => None,
                Some(t) => Some(t.encode(content)),
                None => Some(content.clone().into_bytes()),
            }
        }
        (None, Some(source)) => {
            let bytes = fs::read(source).with_context(|| format!("reading {source} from the collection"))?;
            let same = file.bytes.as_deref() == Some(bytes.as_slice());
            plan.observe("source", same.then(|| source.clone().into()), Some(source.clone().into()));
            (!same).then_some(bytes)
        }
        (None, None) => (!found).then(Vec::new),
    };
    let mut steps = if found { permissions(&mut plan, &file.real, &file.shown, mode, options.private, false)? } else { vec![] };
    if let Some(bytes) = new {
        let verb = if found { "Write" } else { "Create" };
        let mut all = write_steps(&file, &options, format!("{verb} {}", file.shown));
        all.append(&mut steps);
        plan.change(all, move || common::write(&file, &bytes, &options));
    } else if !steps.is_empty() {
        plan.change(steps, move || {
            if let Some(mode) = options.mode {
                common::set_mode(&file.real, mode)?;
            }
            if options.private {
                common::set_private(&file.real, false)?;
            }
            Ok(())
        });
    }
    Ok(plan)
}

/// The change of a text-editing resource: write `new` if it differs from the file's text.
fn text_change(plan: &mut Plan, file: Managed, text: Text, new: String, options: WriteOptions, what: String) {
    if new != text.text {
        let steps = write_steps(&file, &options, what);
        plan.change(steps, move || common::write(&file, &text.encode(&new), &options));
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InsertProp {
    Place(String),
    Before { before: String },
    After { after: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockProps {
    path: String,
    content: String,
    comment: Option<String>,
    insert: Option<InsertProp>,
    /// `<collection>/<app>`, which furnish sets when it loads the app.
    app: Option<String>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl BlockProps {
    fn check(&self) -> Result<text::Insert> {
        check_validate(&self.validate)?;
        if self.comment.as_deref().is_some_and(|c| c.trim().is_empty() || c.contains('\n')) {
            bail!("comment: the characters that start a comment, e.g. '#' or '//'");
        }
        Ok(match &self.insert {
            None => text::Insert::Bottom,
            Some(InsertProp::Place(p)) if p == "top" => text::Insert::Top,
            Some(InsertProp::Place(p)) if p == "bottom" => text::Insert::Bottom,
            Some(InsertProp::Place(p)) => bail!("insert '{p}': top, bottom, before: <pattern> or after: <pattern>"),
            Some(InsertProp::Before { before }) => text::Insert::Before(regex(before, "insert before")?),
            Some(InsertProp::After { after }) => text::Insert::After(regex(after, "insert after")?),
        })
    }
}

fn block(props: &Value) -> Result<Plan> {
    let p: BlockProps = parse(props)?;
    let insert = p.check()?;
    let app = p.app.as_deref().context("app is missing: furnish sets it to <collection>/<app> when it loads the app")?;
    let mut plan = Plan::new(props);
    let file = Managed::open(&p.path)?;
    let text = file.text()?;
    let block = text::Block { content: &p.content, comment: p.comment.as_deref().unwrap_or("#"), app, insert };
    let current = block.current(&text.text).with_context(|| file.shown.clone())?;
    let exist = exist(p.exist);
    let new = block.apply(&text.text, exist).with_context(|| file.shown.clone())?;
    let content = p.content.trim_end_matches('\n').to_string();
    plan.observe("_exist", Some(current.is_some().into()), Some(exist.into()));
    plan.observe("content", current.clone().map(Value::from), exist.then(|| content.clone().into()));
    let what = match (&current, exist) {
        (None, _) => format!("Add the block to {}", file.shown),
        (Some(_), true) => format!("Update the block in {}", file.shown),
        (Some(_), false) => format!("Remove the block from {}", file.shown),
    };
    let options = write_options(None, None, p.backup, &p.validate);
    text_change(&mut plan, file, text, new, options, what);
    Ok(plan)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineProps {
    path: String,
    line: String,
    #[serde(rename = "match")]
    pattern: Option<String>,
    before: Option<String>,
    after: Option<String>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl LineProps {
    fn check(&self) -> Result<text::Line> {
        check_validate(&self.validate)?;
        if self.line.contains('\n') {
            bail!("line: one line, without line breaks");
        }
        let pattern = self.pattern.as_deref().map(|p| regex(p, "match")).transpose()?;
        if let Some(re) = &pattern
            && !re.is_match(&self.line)
        {
            bail!("line '{}' doesn't match '{}': the next run wouldn't find it and would add it again", self.line, re.as_str());
        }
        Ok(text::Line {
            line: self.line.clone(),
            pattern,
            before: self.before.as_deref().map(|p| regex(p, "before")).transpose()?,
            after: self.after.as_deref().map(|p| regex(p, "after")).transpose()?,
        })
    }
}

fn line(props: &Value) -> Result<Plan> {
    let p: LineProps = parse(props)?;
    let line = p.check()?;
    let mut plan = Plan::new(props);
    let file = Managed::open(&p.path)?;
    let text = file.text()?;
    let exist = exist(p.exist);
    let current = line.current(&text.text);
    let new = line.apply(&text.text, exist);
    plan.observe("_exist", Some(current.is_some().into()), Some(exist.into()));
    plan.observe("line", current.map(|(l, _)| l.into()), exist.then(|| p.line.clone().into()));
    let placed = current.is_some_and(|(_, placed)| placed);
    for (key, given) in [("before", &p.before), ("after", &p.after)] {
        plan.observe(key, given.clone().filter(|_| placed).map(Value::from), given.clone().filter(|_| exist).map(Value::from));
    }
    let what = match (current, exist) {
        (_, false) => format!("Remove '{}' from {}", current.map_or(p.line.as_str(), |(l, _)| l), file.shown),
        (None, true) => format!("Add '{}' to {}", p.line, file.shown),
        (Some((l, _)), true) if l != p.line => format!("Replace '{l}' with '{}' in {}", p.line, file.shown),
        (Some(_), true) => format!("Move '{}' to its place in {}", p.line, file.shown),
    };
    let options = write_options(None, None, p.backup, &p.validate);
    text_change(&mut plan, file, text, new, options, what);
    Ok(plan)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValueProps {
    path: String,
    format: Option<value::Format>,
    location: String,
    value: Option<Value>,
    merge: Option<value::Merge>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl ValueProps {
    fn check(&self) -> Result<(value::Format, Vec<value::Seg>)> {
        check_validate(&self.validate)?;
        let format = match self.format.or_else(|| value::Format::from_path(&self.path)) {
            Some(f) => f,
            None => bail!("format: json, jsonc, yaml or toml (the file name doesn't tell)"),
        };
        let merge = self.merge.unwrap_or_default();
        if self.value.is_none() && (exist(self.exist) || merge != value::Merge::Replace) {
            bail!("value is required");
        }
        Ok((format, value::check(&self.location, self.value.as_ref(), merge)?))
    }
}

fn value(props: &Value) -> Result<Plan> {
    let p: ValueProps = parse(props)?;
    let (format, segs) = p.check()?;
    let merge = p.merge.unwrap_or_default();
    let exist = exist(p.exist);
    let mut plan = Plan::new(props);
    let file = Managed::open(&p.path)?;
    let text = file.text()?;
    let doc = value::parse(format, &text.text).with_context(|| format!("{} can't be read, so furnish leaves it as it is", file.shown))?;
    let (ops, expected) = value::plan(&doc, &segs, p.value.as_ref(), merge, exist).with_context(|| format!("{}: {}", file.shown, p.location))?;
    let observed = |doc: &Value| -> Result<(bool, Option<Value>)> {
        let at = value::current(doc, &segs)?;
        Ok(match (merge, at, &p.value) {
            (value::Merge::Member, Some(Value::Array(a)), Some(v)) if a.contains(v) => (true, Some(v.clone())),
            (value::Merge::Member, _, _) => (false, None),
            (value::Merge::Deep, Some(at), Some(v)) => (true, Some(value::project(at, v))),
            (_, at, _) => (at.is_some(), at.cloned()),
        })
    };
    let (now_exists, now) = observed(&doc)?;
    let (after_exists, after) = observed(&expected)?;
    plan.observe("_exist", Some(now_exists.into()), Some(after_exists.into()));
    plan.observe("value", now, after);
    if ops.is_empty() {
        return Ok(plan);
    }
    let new = value::edit(format, &text.text, &ops, &expected).with_context(|| file.shown.clone())?;
    let what = match (merge, exist) {
        (value::Merge::Member, true) => format!("Add the value to the list at {} in {}", p.location, file.shown),
        (value::Merge::Member, false) => format!("Remove the value from the list at {} in {}", p.location, file.shown),
        (_, true) => format!("Set {} in {}", p.location, file.shown),
        (_, false) => format!("Remove {} from {}", p.location, file.shown),
    };
    let options = write_options(None, None, p.backup, &p.validate);
    text_change(&mut plan, file, text, new, options, what);
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test, then set, then test again, through the same functions DSC's calls run.
    fn converge(kind: &str, props: Value) -> Vec<String> {
        let plan = evaluate(kind, &props).unwrap();
        let steps = plan.steps.clone();
        if let Some(apply) = plan.apply {
            apply().unwrap();
        }
        let again = evaluate(kind, &props).unwrap();
        assert!(again.apply.is_none(), "test after set: still differs ({:?})", again.steps);
        steps
    }

    fn path(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn line_must_match_its_pattern_when_the_app_is_loaded() {
        let props = json!({"path": "~/.bashrc", "line": "export EDITOR=vim", "match": "^EDITOR="});
        let err = check("Furnish/Line", &props).unwrap_err().to_string();
        assert!(err.contains("doesn't match"), "{err}");
        assert!(check("furnish/line", &json!({"path": "~/.bashrc", "line": "EDITOR=vim", "match": "^EDITOR="})).is_ok());
        assert!(check("Furnish/Line", &json!({"path": "~/.bashrc", "line": "x", "pattern": "x"})).is_err(), "unknown property");
        // Expressions are left to DSC; [[ is a literal [.
        assert!(check("Furnish/Line", &json!({"path": "[parameters('p')]", "line": "x", "match": "y"})).is_ok());
        assert!(check("Furnish/Line", &json!({"path": "~/x", "line": "[[x]", "match": "^y"})).is_err());
        assert!(check("Furnish/File", &json!({"path": "~/x", "content": "a", "source": "b"})).is_err());
        assert!(check("Furnish/File", &json!({"path": "~/x", "validate": ["true"]})).is_err());
        assert!(check("Furnish/Value", &json!({"path": "~/x.ini", "location": "$.a", "value": 1})).is_err(), "unknown format");
    }

    #[test]
    fn file_with_content_converges_and_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "config.d/acme.conf");
        let props = json!({"path": p, "content": "Host git.acme.example\n"});
        assert_eq!(converge("file", props.clone()), vec![format!("Create {p}")]);
        assert_eq!(fs::read_to_string(&p).unwrap(), "Host git.acme.example\n");
        assert!(converge("file", props.clone()).is_empty());
        converge("file", json!({"path": p, "_exist": false}));
        assert!(!Path::new(&p).exists());
    }

    #[test]
    fn a_bom_is_kept_when_a_line_is_added() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "config");
        fs::write(&p, b"\xEF\xBB\xBFHost a\r\n").unwrap();
        converge("line", json!({"path": p, "line": "Include x", "before": "^Host\\s"}));
        assert_eq!(fs::read(&p).unwrap(), b"\xEF\xBB\xBFInclude x\r\nHost a\r\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_glab_config_keeps_mode_600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "config.yml");
        fs::write(&p, "hosts:\n  gitlab.com:\n    token: abc\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        let props = json!({"path": p, "location": "$.hosts['git.example.org'].git_protocol", "value": "ssh"});
        converge("value", props);
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(fs::read_to_string(&p).unwrap().contains("git.example.org"));
    }

    #[test]
    fn block_is_added_and_removed_through_the_resource() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "config");
        fs::write(&p, "Host *\n").unwrap();
        let props = json!({"path": p, "content": "Include config.d/acme.conf", "insert": "top", "app": "acme/ssh"});
        converge("block", props.clone());
        assert!(fs::read_to_string(&p).unwrap().starts_with("# BEGIN furnish acme/ssh\n"));
        let mut gone = props.clone();
        gone["_exist"] = false.into();
        converge("block", gone);
        assert_eq!(fs::read_to_string(&p).unwrap(), "Host *\n");
    }

    #[test]
    fn directory_private_and_removal_only_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "vpn");
        converge("directory", json!({"path": p, "private": true}));
        assert!(common::is_private(Path::new(&p), true).unwrap());
        fs::write(Path::new(&p).join("profile.ovpn"), "x").unwrap();
        let plan = evaluate("directory", &json!({"path": p, "_exist": false})).unwrap();
        assert!(plan.apply.is_none(), "a folder with files in it is kept");
        fs::remove_file(Path::new(&p).join("profile.ovpn")).unwrap();
        converge("directory", json!({"path": p, "_exist": false}));
        assert!(!Path::new(&p).exists());
    }

    #[test]
    fn value_in_a_file_that_doesnt_parse_is_an_error_and_the_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "settings.json");
        fs::write(&p, "{ broken").unwrap();
        assert!(evaluate("value", &json!({"path": p, "location": "$.a", "value": 1})).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "{ broken");
    }
}
