//! furnish's package and configuration resources: `Furnish/Package`, `Furnish/Directory`, `Furnish/File`, `Furnish/Block`,
//! `Furnish/Line`, `Furnish/Value` and `Furnish/Command`. They run inside the furnish program:
//! their manifests (written by `engine::init`) call it as `furnish dsc-resource <type> <get|test|set>`,
//! with the properties as JSON on stdin.
//!
//! Each configuration resource has one function that looks at the system and returns its state and the
//! change that set would make (see [`Plan`]). test reports the desired state when there is no
//! change, and set changes only then, so test and set can't disagree.

mod command;
pub(crate) mod common;
pub mod diff;
pub(crate) mod note;
pub mod text;
pub mod value;

use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use common::{Managed, Missing, Text, WriteOptions};
use note::{Note, note};

/// The first argument that makes furnish run a resource instead of a command.
pub const COMMAND: &str = "dsc-resource";

/// The resource types, their names on the command line, and their manifests.
pub const TYPES: [(&str, &str, &str); 7] = [
    ("Furnish/Package", "package", include_str!("../../resources/config/furnish-package.dsc.resource.json")),
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
            crate::cli_println!("{out}");
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
    if let [kind, op] = args && kind == "installer-download" {
        return crate::installer_download::command(op);
    }
    let (kind, op, what_if) = match args {
        [kind, op] => (kind, op.as_str(), false),
        [kind, op, flag] if flag == "--what-if" => (kind, op.as_str(), true),
        _ => bail!("usage: furnish {COMMAND} <type> <get|test|set> [--what-if]"),
    };
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let mut props: Value = serde_json::from_str(&input).context("the properties on stdin aren't JSON")?;
    if kind == "package" {
        return crate::package_names::run(&props, op, what_if);
    }
    let files = props.get_mut("_metadata").and_then(Value::as_object_mut).and_then(|m| m.remove("furnishComparisonFiles"));
    let plan = if kind == "command" && what_if && op == "set" {
        command::evaluate_with_files(&props, files.as_ref().unwrap_or(&Value::Null))?
    } else {
        evaluate(kind, &props)?
    };
    let action = if plan.apply.is_none() {
        crate::engine::Action::Nothing
    } else {
        match (plan.state.get("_exist"), plan.after.get("_exist")) {
            (Some(Value::Bool(false)), Some(Value::Bool(true))) => crate::engine::Action::Install,
            (Some(Value::Bool(true)), Some(Value::Bool(false))) => crate::engine::Action::Remove,
            _ => crate::engine::Action::Change,
        }
    };
    let mut out = match op {
        "get" => plan.state,
        "test" => {
            let mut state = plan.state;
            state.insert("_inDesiredState".into(), plan.apply.is_none().into());
            state
        }
        "set" if what_if => {
            let mut after = plan.after;
            // whatIf for the CLI and logs, furnish for the GUI (translated there).
            let english: Vec<String> = plan.steps.iter().map(Note::english).collect();
            let notes: Vec<Value> = plan.steps.iter().map(|n| json!({ "code": n.code, "params": n.params })).collect();
            // furnishChange: the properties shown may read the same before and after (a moved line,
            // a file's contents), so furnish learns from this that the step changes something.
            after.insert("_metadata".into(), json!({ "whatIf": english, "furnish": notes, "furnishChange": plan.apply.is_some() }));
            after
        }
        "set" if plan.unverified.is_some() => bail!("{}", plan.unverified.unwrap_or_default()),
        "set" => match plan.apply {
            None => plan.state,
            Some(apply) if !plan.recheck => {
                apply()?;
                plan.after
            }
            Some(apply) => {
                apply()?;
                let again = evaluate(kind, &props)?;
                if again.apply.is_some() {
                    bail!("still not in the desired state after the change ({})", plan.steps.iter().map(Note::english).collect::<Vec<_>>().join("; "));
                }
                again.state
            }
        },
        _ => bail!("unknown operation '{op}'"),
    };
    if op == "set" {
        out.entry("_metadata").or_insert_with(|| json!({}))["furnishAction"] = serde_json::to_value(action)?;
    }
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
    /// What set would do, one note per step, for previews.
    steps: Vec<Note>,
    /// Makes the change; `None` if the resource is in its desired state.
    apply: Option<Box<dyn FnOnce() -> Result<()>>>,
    /// Why the state is assumed rather than found (see [`unreadable`]); set then refuses.
    unverified: Option<String>,
    /// Test again after set and fail if something would still change (not after a removal
    /// command, whose test proves nothing, see command.rs).
    recheck: bool,
}

impl Plan {
    fn new(props: &Value) -> Self {
        let state = props.as_object().cloned().unwrap_or_default();
        Plan { after: state.clone(), state, steps: vec![], apply: None, unverified: None, recheck: true }
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

    fn change(&mut self, steps: Vec<Note>, apply: impl FnOnce() -> Result<()> + 'static) {
        self.steps = steps;
        self.apply = Some(Box::new(apply));
    }
}

/// The plan for a file this account can't read. furnish works out an app's status as the user,
/// also for apps the furnish service installed, and a file that only root may read (a VPN
/// profile with mode 600, say) can't be compared then. It exists, so the resource counts as set
/// up (removed: as still there); the service, which installs and removes such apps, reads it.
fn unreadable(mut plan: Plan, file: &Managed, exist: bool) -> Plan {
    plan.observe("_exist", Some(true.into()), Some(exist.into()));
    plan.steps = vec![note("file.unreadable", json!({ "path": file.shown }))];
    let why = format!("can't read {}: changing it needs the rights of whoever may read it (administrator)", file.name());
    if !exist {
        let refuse = why.clone();
        plan.apply = Some(Box::new(move || bail!("{refuse}")));
    }
    plan.unverified = Some(why);
    plan
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
    if kind.eq_ignore_ascii_case(crate::catalog::PACKAGE_TYPE) {
        return crate::package_names::validate(props);
    }
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

/// Steps that write the same file must agree on its mode: each enforces its own, so two modes would
/// undo each other on every install. A step without `mode` or `private` leaves the mode alone, so
/// one `private: true` covers a file that several steps edit.
pub fn check_modes(resources: &Value) -> Result<()> {
    let mut seen: Vec<(String, u32, String)> = vec![];
    for r in resources.as_array().into_iter().flatten() {
        let kind = r["type"].as_str().unwrap_or_default();
        if !["Furnish/File", "Furnish/Line", "Furnish/Block", "Furnish/Value"].iter().any(|t| t.eq_ignore_ascii_case(kind)) {
            continue;
        }
        let Some(props) = literal(&r["properties"]) else { continue };
        let (Some(path), name) = (props["path"].as_str(), r["name"].as_str().unwrap_or("?")) else { continue };
        let mode = match (&props["mode"], &props["private"]) {
            (Value::String(m), _) => common::parse_mode(m).with_context(|| format!("resource '{name}'"))?,
            (_, Value::Bool(true)) => 0o600,
            _ => continue,
        };
        match seen.iter().find(|(p, _, _)| p == path) {
            Some((_, other, first)) if *other != mode => bail!(
                "resources '{first}' and '{name}' give {path} different modes ({other:03o} and {mode:03o}), so each would undo the other: \
                 give it one mode, on one of them or the same on both"
            ),
            Some(_) => {}
            None => seen.push((path.to_string(), mode, name.to_string())),
        }
    }
    Ok(())
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
fn permissions(plan: &mut Plan, path: &Path, shown: &str, mode: Option<u32>, private: bool, folder: bool) -> Result<Vec<Note>> {
    let mut steps = Vec::new();
    if let Some(mode) = mode
        && let Some(actual) = common::mode(path)
    {
        plan.observe("mode", Some(format!("{actual:03o}").into()), Some(format!("{mode:03o}").into()));
        if actual != mode {
            steps.push(note("mode", json!({ "path": shown, "mode": format!("{mode:03o}") })));
        }
    }
    if private {
        let actual = common::is_private(path, folder)?;
        plan.observe("private", Some(actual.into()), Some(true.into()));
        if !actual {
            steps.push(note("private", json!({ "path": shown })));
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
            plan.change(vec![note("dir.remove", json!({ "path": shown }))], move || Ok(fs::remove_dir(&path)?));
        } else {
            plan.steps = vec![note("dir.keep", json!({ "path": shown }))];
        }
        return Ok(plan);
    }
    plan.observe("_exist", Some(found.into()), Some(true.into()));
    let steps = if found {
        permissions(&mut plan, &path, &shown, mode, private, true)?
    } else {
        plan.observe("mode", None, mode.map(|m| format!("{m:03o}").into()));
        vec![note("dir.create", json!({ "path": shown }))]
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

/// Whether a preview shows only how many lines change, not which: the file may hold secrets.
/// It is `private`, gets a `mode` without read access for group and others (e.g. 600), or has
/// such a mode or (Windows) an access list for the user only already.
fn hidden(file: &Managed, options: &WriteOptions) -> bool {
    options.private || options.mode.is_some_and(|m| m & 0o044 == 0) || (file.bytes.is_some() && common::owner_only(&file.real))
}

/// The preview lines for writing a file.
fn write_steps(file: &Managed, options: &WriteOptions, what: Vec<Note>) -> Vec<Note> {
    let mut steps = what;
    if options.backup && file.bytes.is_some() {
        steps.push(note("backup", json!({ "path": file.shown })));
    }
    if let Some(command) = &options.validate {
        steps.push(note("validate", json!({ "command": command.join(" ").replace("%s", "<new file>") })));
    }
    steps
}

fn file(props: &Value) -> Result<Plan> {
    let p: FileProps = parse(props)?;
    p.check()?;
    let mut plan = Plan::new(props);
    let file = Managed::open(&p.path)?;
    if file.unreadable {
        return Ok(unreadable(plan, &file, exist(p.exist)));
    }
    let mode = p.mode.as_deref().map(common::parse_mode).transpose()?;
    let options = write_options(mode, p.private, p.backup, &p.validate);
    let found = file.bytes.is_some();
    if !exist(p.exist) {
        plan.observe("_exist", Some(found.into()), Some(false.into()));
        if found {
            let mut delete = note("file.delete", json!({ "path": file.shown }));
            delete.add_diff(diff::bytes(file.bytes.as_deref(), b""), hidden(&file, &options));
            let steps = vec![delete];
            plan.change(steps, move || common::delete(&file, &options));
        }
        return Ok(plan);
    }
    plan.observe("_exist", Some(found.into()), Some(true.into()));
    let current = file.bytes.as_deref().and_then(|b| Text::decode(b).ok());
    // The new contents, `None` if they stay.
    // Whole files aren't shown as properties (they may be long or secret): a note says what changes.
    let mut what = None;
    let new: Option<Vec<u8>> = match (&p.content, &p.source) {
        (Some(content), _) => {
            let old = current.as_ref().map(|t| t.text.as_str()).filter(|_| found);
            if old != Some(content.as_str()) {
                what = Some(note::text_change(&file.shown, old, content, hidden(&file, &options)));
            }
            match &current {
                Some(t) if t.text == *content => None,
                Some(t) => Some(t.encode(content)),
                None => Some(content.clone().into_bytes()),
            }
        }
        (None, Some(source)) => {
            let bytes = fs::read(source).with_context(|| format!("reading {source} from the collection"))?;
            let same = file.bytes.as_deref() == Some(bytes.as_slice());
            if !same {
                let name = Path::new(source).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut copy = note("file.copy", json!({ "path": file.shown, "source": name }));
                copy.add_diff(diff::bytes(file.bytes.as_deref(), &bytes), hidden(&file, &options));
                what = Some(copy);
            }
            (!same).then_some(bytes)
        }
        (None, None) => {
            what = (!found).then(|| note("file.create", json!({ "path": file.shown, "lines": 0 })));
            (!found).then(Vec::new)
        }
    };
    let mut steps = if found { permissions(&mut plan, &file.real, &file.shown, mode, options.private, false)? } else { vec![] };
    if let Some(bytes) = new {
        let mut all = write_steps(&file, &options, what.into_iter().collect());
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

/// `mode` and `private` of a text-editing resource (Line, Block, Value): the write options and,
/// for an existing file, the notes for fixing its permissions. Part of the desired state, as for
/// `Furnish/File`; only when the step is to exist, since uninstalling never relaxes a file's
/// permissions (the file is the user's and stays).
fn text_permissions(plan: &mut Plan, file: &Managed, mode: &Option<String>, private: Option<bool>, exist: bool, backup: Option<bool>, validate: &Option<Vec<String>>) -> Result<(WriteOptions, Vec<Note>)> {
    if !exist {
        return Ok((write_options(None, None, backup, validate), vec![]));
    }
    let mode = mode.as_deref().map(common::parse_mode).transpose()?;
    let options = write_options(mode, private, backup, validate);
    let steps = match file.bytes {
        Some(_) => permissions(plan, &file.real, &file.shown, mode, options.private, false)?,
        None => {
            plan.observe("mode", None, mode.map(|m| format!("{m:03o}").into()));
            vec![]
        }
    };
    Ok((options, steps))
}

/// The change of a text-editing resource: write `new` if it differs from the file's text, and fix
/// the permissions that `perms` names (see [`text_permissions`]). The first note also gets the
/// diff (see [`Note::add_diff`]).
fn text_change(plan: &mut Plan, file: Managed, text: Text, new: String, options: WriteOptions, mut what: Vec<Note>, perms: Vec<Note>) {
    if new == text.text {
        if !perms.is_empty() {
            plan.change(perms, move || {
                if let Some(mode) = options.mode {
                    common::set_mode(&file.real, mode)?;
                }
                if options.private {
                    common::set_private(&file.real, false)?;
                }
                Ok(())
            });
        }
        return;
    }
    // A new file gets its permissions as it is written.
    if file.bytes.is_none() {
        if options.private {
            what.push(note("private", json!({ "path": file.shown })));
        } else if let Some(mode) = options.mode {
            what.push(note("mode", json!({ "path": file.shown, "mode": format!("{mode:03o}") })));
        }
    }
    if let Some(first) = what.first_mut() {
        first.add_diff(Some(diff::lines(&text.text, &new)), hidden(&file, &options));
    }
    what.extend(perms);
    let steps = write_steps(&file, &options, what);
    plan.change(steps, move || common::write(&file, &text.encode(&new), &options));
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
    mode: Option<String>,
    private: Option<bool>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl BlockProps {
    fn check(&self) -> Result<text::Insert> {
        check_validate(&self.validate)?;
        check_mode(&self.mode, self.private)?;
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
    if file.unreadable {
        return Ok(unreadable(plan, &file, exist(p.exist)));
    }
    let text = file.text()?;
    let block = text::Block { content: &p.content, comment: p.comment.as_deref().unwrap_or("#"), app, insert };
    let current = block.current(&text.text).with_context(|| file.shown.clone())?;
    let exist = exist(p.exist);
    let new = block.apply(&text.text, exist).with_context(|| file.shown.clone())?;
    plan.observe("_exist", Some(current.is_some().into()), Some(exist.into()));
    let code = match (&current, exist) {
        (None, _) => "block.add",
        (Some(_), true) => "block.update",
        (Some(_), false) => "block.remove",
    };
    let what = vec![note(code, json!({ "path": file.shown }))];
    let (options, perms) = text_permissions(&mut plan, &file, &p.mode, p.private, exist, p.backup, &p.validate)?;
    text_change(&mut plan, file, text, new, options, what, perms);
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
    accept: Option<String>,
    mode: Option<String>,
    private: Option<bool>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl LineProps {
    fn check(&self) -> Result<text::Line> {
        check_validate(&self.validate)?;
        check_mode(&self.mode, self.private)?;
        if self.line.contains('\n') {
            bail!("line: one line, without line breaks");
        }
        let pattern = self.pattern.as_deref().map(|p| regex(p, "match")).transpose()?;
        if let Some(re) = &pattern
            && !re.is_match(&self.line)
        {
            bail!("line '{}' doesn't match '{}': the next run wouldn't find it and would add it again", self.line, re.as_str());
        }
        let accept = self.accept.as_deref().map(|p| regex(p, "accept")).transpose()?;
        if let Some(re) = &accept {
            if pattern.is_some() {
                bail!("match and accept: use one of them (match replaces lines, accept leaves them as they are)");
            }
            if !re.is_match(&self.line) {
                bail!("line '{}' doesn't match accept '{}'", self.line, re.as_str());
            }
        }
        Ok(text::Line {
            line: self.line.clone(),
            pattern,
            before: self.before.as_deref().map(|p| regex(p, "before")).transpose()?,
            after: self.after.as_deref().map(|p| regex(p, "after")).transpose()?,
            accept,
        })
    }
}

/// Home Manager links a managed home file to <store>/<generation>-home-manager-files/<path>.
/// Check the link the activation script owns, before following it to the final store object:
/// an ordinary symlink to a Nix package is a different case.
#[cfg(unix)]
fn home_manager_link(path: &Path, store: &Path) -> bool {
    let (Ok(target), Ok(store)) = (fs::read_link(path), store.canonicalize()) else { return false };
    let Ok(relative) = target.strip_prefix(store) else { return false };
    let mut parts = relative.components();
    matches!(parts.next(), Some(std::path::Component::Normal(generation))
        if generation.to_str().is_some_and(|name| name.ends_with("-home-manager-files")))
        && matches!(parts.next(), Some(std::path::Component::Normal(_)))
        && parts.all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn line(props: &Value) -> Result<Plan> {
    let p: LineProps = parse(props)?;
    let line = p.check()?;
    let mut plan = Plan::new(props);
    let file = Managed::open(&p.path)?;
    if file.unreadable {
        return Ok(unreadable(plan, &file, exist(p.exist)));
    }
    let text = file.text()?;
    let exist = exist(p.exist);
    let current = if exist { line.current(&text.text) } else { line.own(&text.text) };
    #[cfg(unix)]
    if !exist && line.accept.is_some() && matches!(current, Some((_, true)))
        && home_manager_link(&common::expand(&p.path)?, Path::new("/nix/store"))
    {
        // Even an exact line belongs to Home Manager here; removing the option updates its
        // generation. A rebuild for boot leaves the active home file in place until activation.
        plan.observe("_exist", Some(true.into()), Some(true.into()));
        return Ok(plan);
    }
    let new = line.apply(&text.text, exist);
    plan.observe("_exist", Some(current.is_some().into()), Some(exist.into()));
    let path = file.shown.clone();
    let what = vec![match (current, exist) {
        (_, false) => note("line.remove", json!({ "path": path, "line": current.map_or(p.line.as_str(), |(l, _)| l) })),
        (None, true) => note("line.add", json!({ "path": path, "line": p.line })),
        (Some((l, _)), true) if l != p.line => note("line.replace", json!({ "path": path, "line": p.line, "old": l })),
        (Some(_), true) => note("line.move", json!({ "path": path, "line": p.line })),
    }];
    let (options, perms) = text_permissions(&mut plan, &file, &p.mode, p.private, exist, p.backup, &p.validate)?;
    text_change(&mut plan, file, text, new, options, what, perms);
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
    /// Remove the value on uninstall only if it is one of these; install leaves it as it is.
    #[serde(rename = "removeIf")]
    remove_if: Option<Vec<Value>>,
    mode: Option<String>,
    private: Option<bool>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
    backup: Option<bool>,
    validate: Option<Vec<String>>,
}

impl ValueProps {
    fn check(&self) -> Result<(value::Format, Vec<value::Seg>)> {
        check_validate(&self.validate)?;
        check_mode(&self.mode, self.private)?;
        let format = match self.format.or_else(|| value::Format::from_path(&self.path)) {
            Some(f) => f,
            None => bail!("format: json, jsonc, yaml or toml (the file name doesn't tell)"),
        };
        let merge = self.merge.unwrap_or_default();
        if let Some(values) = &self.remove_if {
            if self.value.is_some() || self.merge.is_some() {
                bail!("removeIf: a step with removeIf only removes, so it takes no value or merge");
            }
            if values.is_empty() {
                bail!("removeIf: the values that the app's uninstall removes, for example [acme-prod]");
            }
            return Ok((format, value::check(&self.location, None, merge)?));
        }
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
    if file.unreadable {
        return Ok(unreadable(plan, &file, exist));
    }
    let text = file.text()?;
    let doc = value::parse(format, &text.text).with_context(|| format!("{} can't be read, so furnish leaves it as it is", file.shown))?;
    // removeIf: install leaves the value alone, uninstall removes it if it is one of the app's.
    let found = value::current(&doc, &segs).with_context(|| format!("{}: {}", file.shown, p.location))?;
    let keep = match &p.remove_if {
        None => false,
        Some(_) if exist => true,
        Some(values) => found.is_some_and(|v| !values.contains(v)),
    };
    let (ops, expected) = if keep {
        (vec![], doc.clone())
    } else {
        value::plan(&doc, &segs, p.value.as_ref(), merge, exist).with_context(|| format!("{}: {}", file.shown, p.location))?
    };
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
    plan.observe("value", now.clone(), after);
    let (options, perms) = text_permissions(&mut plan, &file, &p.mode, p.private, exist, p.backup, &p.validate)?;
    if ops.is_empty() {
        // On uninstall, say why a value stays.
        if p.remove_if.is_some() && !exist && let Some(now) = now {
            let shown = match now {
                Value::String(s) => s,
                other => other.to_string(),
            };
            plan.steps = vec![note("value.keep", json!({ "path": file.shown, "location": p.location, "value": shown }))];
        }
        let same = text.text.clone();
        text_change(&mut plan, file, text, same, options, vec![], perms);
        return Ok(plan);
    }
    let new = value::edit(format, &text.text, &ops, &expected).with_context(|| file.shown.clone())?;
    let code = match (merge, exist) {
        (value::Merge::Member, true) => "value.add",
        (value::Merge::Member, false) => "value.removeMember",
        (_, true) => "value.set",
        (_, false) => "value.remove",
    };
    let what = vec![note(code, json!({ "path": file.shown, "location": p.location }))];
    text_change(&mut plan, file, text, new, options, what, perms);
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test, then set, then test again, through the same functions DSC's calls run.
    fn converge(kind: &str, props: Value) -> Vec<Note> {
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

    #[cfg(unix)]
    #[test]
    fn only_links_into_a_home_manager_generation_are_owned_by_it() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let generation = store.join("a-home-manager-files");
        fs::create_dir_all(&generation).unwrap();
        let ordinary = store.join("ordinary");
        fs::write(&ordinary, "Include config.d/acme.conf").unwrap();
        symlink(&ordinary, generation.join("config")).unwrap();
        let home_file = dir.path().join("config");
        symlink(generation.join("config"), &home_file).unwrap();
        assert!(home_manager_link(&home_file, &store));
        assert!(!home_manager_link(&ordinary, &store));
        fs::remove_file(&home_file).unwrap();
        symlink(&ordinary, &home_file).unwrap();
        assert!(!home_manager_link(&home_file, &store), "a plain store link still follows normal removal");
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
        assert_eq!(converge("file", props.clone()).iter().map(Note::english).collect::<Vec<_>>(), vec![format!("Create {p} (1 line)")]);
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
        let steps = converge("value", props);
        assert_eq!(steps[0].params["hidden"], true, "a file only its owner may read shows no lines");
        assert!(steps[0].params.get("diff").is_none());
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(fs::read_to_string(&p).unwrap().contains("git.example.org"));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_only_root_may_read_counts_as_set_up_and_set_refuses() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads everything
        }
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "acme-vpn.ovpn");
        fs::write(&p, "remote vpn.acme.example\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o000)).unwrap();
        for (kind, props) in [
            ("file", json!({"path": p, "content": "remote vpn.acme.example\n"})),
            ("line", json!({"path": p, "line": "remote vpn.acme.example"})),
            ("block", json!({"path": p, "content": "x", "app": "acme/vpn"})),
            ("value", json!({"path": p, "format": "yaml", "location": "$.a", "value": 1})),
        ] {
            let plan = evaluate(kind, &props).unwrap();
            assert!(plan.apply.is_none(), "{kind}: in the desired state as far as the user can tell");
            assert_eq!(plan.state["_exist"], true);
            assert_eq!(plan.steps[0].code, "file.unreadable");
            assert!(plan.unverified.is_some(), "{kind}: set refuses");
        }
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
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
    fn line_value_and_source_changes_carry_a_diff() {
        let dir = tempfile::tempdir().unwrap();
        let lines = |n: &Note| n.params["diff"]["hunks"][0]["lines"].clone();
        let p = path(&dir, "config");
        fs::write(&p, "Host a\n").unwrap();
        let steps = converge("line", json!({"path": p, "line": "Include x", "before": "^Host\\s"}));
        assert_eq!(lines(&steps[0]), json!(["+Include x", " Host a"]));
        let p = path(&dir, "settings.json");
        fs::write(&p, "{\n  // mine\n  \"a\": 1\n}\n").unwrap();
        let steps = converge("value", json!({"path": p, "format": "jsonc", "location": "$.a", "value": 2}));
        assert_eq!(lines(&steps[0]), json!([" {", "   // mine", "-  \"a\": 1", "+  \"a\": 2", " }"]));
        let source = path(&dir, "source.conf");
        fs::write(&source, "x\ny\n").unwrap();
        let p = path(&dir, "copy.conf");
        fs::write(&p, "x\n").unwrap();
        let steps = converge("file", json!({"path": p, "source": source}));
        assert_eq!(lines(&steps[0]), json!([" x", "+y"]));
        fs::write(&source, b"\x00\x01").unwrap();
        assert_eq!(converge("file", json!({"path": p, "source": source}))[0].params["binary"], true);
        let steps = converge("file", json!({"path": p, "content": "a\n", "mode": "640"}));
        assert!(steps[0].params.get("diff").is_some(), "group may read it: the lines are shown");
        let steps = converge("file", json!({"path": p, "content": "b\n", "mode": "600"}));
        assert_eq!(steps[0].params["hidden"], true);
    }

    #[test]
    fn current_context_is_removed_on_uninstall_only_when_it_names_the_apps_context() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, "config");
        let mine = "apiVersion: v1\nclusters:\n- name: mine\n  cluster: { server: https://mine.example }\n\
                    contexts:\n- name: mine\n  context: { cluster: mine, user: me }\ncurrent-context: mine\n\
                    users:\n- name: me\n  user: { token: secret }\n";
        let entry = |kind: &str, name: &str, body: Value| {
            let singular = kind.trim_end_matches('s');
            json!({"path": p, "format": "yaml", "location": format!("$.{kind}[?@.name=='{name}']"), "merge": "deep", "value": {"name": name, singular: body}})
        };
        // The kubeconfig app: cluster, user and context for production and staging, and current-context.
        let mut app = vec![];
        for name in ["acme-prod", "acme-stg"] {
            app.push(entry("clusters", name, json!({"server": format!("https://{name}.acme.example")})));
            app.push(entry("users", name, json!({"exec": {"command": "kubelogin"}})));
            app.push(entry("contexts", name, json!({"cluster": name, "user": name})));
        }
        app.push(json!({"path": p, "format": "yaml", "location": "$.current-context", "removeIf": ["acme-prod", "acme-stg"]}));
        let install = |app: &[Value]| app.iter().for_each(|props| drop(converge("value", props.clone())));
        let uninstall = |app: &[Value]| {
            app.iter().rev().flat_map(|props| {
                let mut gone = props.clone();
                gone["_exist"] = false.into();
                converge("value", gone)
            }).collect::<Vec<_>>()
        };
        let doc = || value::parse(value::Format::Yaml, &fs::read_to_string(&p).unwrap()).unwrap();

        // Install leaves current-context as it is: nothing to do, in the desired state, no change shown.
        fs::write(&p, mine.replace("current-context: mine", "current-context: acme-prod")).unwrap();
        let plan = evaluate("value", app.last().unwrap()).unwrap();
        assert!(plan.apply.is_none() && plan.steps.is_empty());
        assert_eq!(plan.state, plan.after, "the preview shows no change");
        install(&app);
        assert_eq!(doc()["current-context"], "acme-prod");
        assert_eq!(doc()["contexts"].as_array().unwrap().len(), 3);
        // Uninstall removes it, since it names the app's context; kubectl would fail on every command otherwise.
        let notes = uninstall(&app);
        assert!(notes.iter().any(|n| n.code == "value.remove" && n.params["location"] == "$.current-context"), "{notes:?}");
        assert_eq!(doc(), value::parse(value::Format::Yaml, &mine.replace("current-context: mine\n", "")).unwrap());

        // The user's own choice stays, and the preview says why.
        fs::write(&p, mine).unwrap();
        install(&app);
        let notes = uninstall(&app);
        assert!(notes.iter().any(|n| n.english() == format!("Keep $.current-context in {p}: it is mine, which this app doesn't remove")), "{notes:?}");
        assert_eq!(doc(), value::parse(value::Format::Yaml, mine).unwrap());
        // No current-context at all: nothing to do either way.
        fs::write(&p, "").unwrap();
        assert!(converge("value", app.last().unwrap().clone()).is_empty());
        assert!(!Path::new(&p).exists() || fs::read_to_string(&p).unwrap().is_empty());

        assert!(check("Furnish/Value", &json!({"path": "~/x.yaml", "location": "$.a", "removeIf": ["x"], "value": "x"})).is_err());
        assert!(check("Furnish/Value", &json!({"path": "~/x.yaml", "location": "$.a", "removeIf": []})).is_err());
        assert!(check("Furnish/Value", &json!({"path": "~/x.yaml", "location": "$.a", "removeIf": ["x"]})).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_kubeconfig_that_value_steps_create_is_private_and_stays_so_on_uninstall() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir, ".acme/kubeconfig");
        let mode = || fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        let step = |kind: &str, name: &str, body: Value| {
            let singular = kind.trim_end_matches('s');
            json!({"path": p, "format": "yaml", "location": format!("$.{kind}[?@.name=='{name}']"), "merge": "deep", "value": {"name": name, singular: body}})
        };
        // One private: true is enough; the steps without it don't reset the mode.
        let mut cluster = step("clusters", "acme-prod", json!({"server": "https://k8s.acme.example"}));
        cluster["private"] = true.into();
        let app = [
            cluster,
            step("users", "acme-prod", json!({"exec": {"command": "kubelogin"}})),
            step("contexts", "acme-prod", json!({"cluster": "acme-prod", "user": "acme-prod"})),
        ];
        let notes = converge("value", app[0].clone());
        assert!(notes.iter().any(|n| n.english() == format!("Make {p} accessible only to you")), "{notes:?}");
        assert_eq!(notes[0].params["hidden"], true, "a private file shows only the counts of its diff");
        assert_eq!(mode(), 0o600);
        for props in &app[1..] {
            converge("value", props.clone());
        }
        assert_eq!(mode(), 0o600);
        assert!(app.iter().all(|props| evaluate("value", props).unwrap().apply.is_none()), "the steps don't fight");
        assert!(check_modes(&json!(app.iter().map(|p| json!({"type": "Furnish/Value", "properties": p})).collect::<Vec<_>>())).is_ok());

        // Made group-readable by someone: an update, and installing makes it private again without touching the text.
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        let plan = evaluate("value", &app[0]).unwrap();
        assert_eq!((plan.state["private"].clone(), plan.state["_exist"].clone()), (json!(false), json!(true)), "an update, not missing");
        assert_eq!(converge("value", app[0].clone()).iter().map(|n| n.code).collect::<Vec<_>>(), vec!["private"]);
        assert_eq!((mode(), fs::read_to_string(&p).unwrap()), (0o600, text));

        // Uninstall removes the entries and keeps the file and its mode.
        for props in app.iter().rev() {
            let mut gone = props.clone();
            gone["_exist"] = false.into();
            converge("value", gone);
        }
        assert_eq!(mode(), 0o600);
        let mut gone = app[0].clone();
        gone["_exist"] = false.into();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(evaluate("value", &gone).unwrap().apply.is_none(), "uninstall leaves the mode alone");

        // Line and Block, and a mode instead of private.
        let ssh = path(&dir, "ssh/config");
        converge("line", json!({"path": ssh, "line": "Include acme.conf", "private": true}));
        assert_eq!(fs::metadata(&ssh).unwrap().permissions().mode() & 0o777, 0o600);
        let known = path(&dir, "known_hosts");
        let notes = converge("block", json!({"path": known, "content": "x", "app": "acme/ssh", "mode": "640"}));
        assert!(notes.iter().any(|n| n.code == "mode"), "{notes:?}");
        assert_eq!(fs::metadata(&known).unwrap().permissions().mode() & 0o777, 0o640);
        assert!(check("Furnish/Line", &json!({"path": ssh, "line": "x", "mode": "600", "private": true})).is_err());
    }

    #[test]
    fn steps_on_one_file_must_agree_on_its_mode() {
        let r = |name: &str, kind: &str, props: Value| json!({"name": name, "type": kind, "properties": props});
        let file = "~/.acme/kubeconfig";
        let ok = json!([
            r("a", "Furnish/Value", json!({"path": file, "private": true})),
            r("b", "Furnish/Value", json!({"path": file})),
            r("c", "Furnish/Line", json!({"path": file, "mode": "600"})),
            r("d", "Furnish/File", json!({"path": "~/other", "mode": "644"})),
        ]);
        assert!(check_modes(&ok).is_ok());
        let clash = json!([r("a", "Furnish/Value", json!({"path": file, "private": true})), r("b", "Furnish/Block", json!({"path": file, "mode": "644"}))]);
        let err = check_modes(&clash).unwrap_err().to_string();
        assert!(err.contains("'a' and 'b'") && err.contains("600 and 644"), "{err}");
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
