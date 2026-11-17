//! `Furnish/Command`: for tools that must apply a setting themselves. `test` runs a command and
//! checks its exit code, its output, or a value in its JSON output; `set` runs only when the test
//! fails, and `remove` (optional) when the test passes and the setting should go.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use serde_json::json;

use super::{Plan, common, note::{Note, note}, parse, value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Props {
    test: Test,
    set: Vec<String>,
    remove: Option<Vec<String>>,
    #[serde(rename = "_exist")]
    exist: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Test {
    command: Vec<String>,
    /// Expected exit code (default 0).
    exit_code: Option<i32>,
    /// Expected output (stdout), ignoring white space at the start and end.
    output: Option<String>,
    /// A place in the JSON output that must exist, and equal `value` if given.
    json_path: Option<String>,
    value: Option<Value>,
}

fn props(v: &Value) -> Result<Props> {
    let p: Props = parse(v)?;
    for (what, command) in [("test.command", Some(&p.test.command)), ("set", Some(&p.set)), ("remove", p.remove.as_ref())] {
        if command.is_some_and(Vec::is_empty) {
            bail!("{what}: a command, as a list of the program and its arguments");
        }
    }
    if let Some(path) = &p.test.json_path {
        value::parse_location(path).context("test.jsonPath")?;
    } else if p.test.value.is_some() {
        bail!("test.value needs test.jsonPath");
    }
    Ok(p)
}

pub fn check(v: &Value) -> Result<()> {
    props(v).map(drop)
}

/// Whether the test passes, and a note with what it expected and got (the check that failed,
/// else the most telling one).
fn passes(test: &Test) -> Result<(bool, Note)> {
    let out = common::run(&test.command)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let command = test.command.join(" ");
    let code = out.status.code();
    let expected_code = test.exit_code.unwrap_or(0);
    let code_note = note("test.exitCode", json!({ "command": command, "expected": expected_code, "got": code }));
    let mut result = (code == Some(expected_code), code_note);
    if let Some(expected) = &test.output {
        let ok = stdout.trim() == expected.trim();
        let got = if stdout.trim().is_empty() { "—".to_string() } else { short(stdout.trim()) };
        let n = note("test.output", json!({ "command": command, "expected": expected.trim(), "got": got }));
        // The output tells more than an exit code nobody asked for.
        if result.0 || test.exit_code.is_none() {
            result = (result.0 && ok, n);
        }
    }
    if let Some(path) = &test.json_path {
        let doc: Value = match serde_json::from_str(&stdout) {
            Ok(doc) => doc,
            Err(_) if !result.0 => return Ok(result),
            Err(_) => bail!("test: `{command}` didn't print JSON"),
        };
        let found = value::current(&doc, &value::parse_location(path)?).unwrap_or(None);
        let ok = match &test.value {
            Some(v) => found == Some(v),
            None => found.is_some(),
        };
        let show = |v: Option<&Value>| v.map_or("—".to_string(), |v| short(&v.to_string()));
        let expected = test.value.as_ref().map_or("a value".to_string(), |v| short(&v.to_string()));
        let n = note("test.json", json!({ "command": command, "path": path, "expected": expected, "got": show(found) }));
        if result.0 {
            result = (ok, n);
        }
    }
    Ok(result)
}

/// At most one line of 120 characters, for showing output.
fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut s: String = line.chars().take(120).collect();
    if line.chars().count() > 120 || text.lines().nth(1).is_some() {
        s.push('…');
    }
    s
}

fn run(command: &[String]) -> Result<()> {
    let out = common::run(command)?;
    if !out.status.success() {
        bail!("`{}` exited with code {}{}", command.join(" "), out.status.code().unwrap_or(-1), common::output_text(&out));
    }
    Ok(())
}

pub fn evaluate(v: &Value) -> Result<Plan> {
    let p = props(v)?;
    let exist = p.exist.unwrap_or(true);
    let mut plan = Plan::new(v);
    let (passes, test_note) = match passes(&p.test) {
        // Nothing to remove where the tool isn't installed.
        Err(e) if !exist && e.chain().any(|e| e.is::<common::Missing>()) => (false, None),
        result => {
            let (ok, n) = result?;
            (ok, Some(n))
        }
    };
    plan.observe("_exist", Some(passes.into()), Some(exist.into()));
    match (exist, passes) {
        (true, false) => {
            let mut steps = test_note.into_iter().collect::<Vec<_>>();
            steps.push(note("run", json!({ "command": p.set.join(" ") })));
            plan.change(steps, move || run(&p.set));
        }
        // A failing test means absent or outdated (an update waiting), so removal always runs
        // `remove`, which must therefore do nothing where there is nothing to remove. Its test
        // afterwards proves nothing either way. Where the tool isn't installed, there is nothing.
        (false, _) if p.remove.is_some() && test_note.is_some() => {
            let remove = p.remove.unwrap_or_default();
            plan.change(vec![note("run", json!({ "command": remove.join(" ") }))], move || run(&remove));
            plan.recheck = false;
        }
        (false, true) => bail!("this step has no remove command, so it can't undo what it set"),
        _ => plan.steps = test_note.into_iter().collect(),
    }
    Ok(plan)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_runs_only_when_the_test_fails() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("server").to_string_lossy().into_owned();
        let props = json!({
            "test": {"command": ["cat", marker], "output": "https://vault.example.org"},
            "set": ["sh", "-c", format!("echo https://vault.example.org > '{marker}'")],
            "remove": ["rm", "-f", marker],
        });
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.state["_exist"], false);
        (plan.apply.unwrap())().unwrap();
        assert!(evaluate(&props).unwrap().apply.is_none(), "test passes after set");
        let mut gone = props.clone();
        gone["_exist"] = false.into();
        (evaluate(&gone).unwrap().apply.unwrap())().unwrap();
        assert!(!marker_exists(&marker));
    }

    fn marker_exists(path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    #[test]
    fn removal_runs_remove_also_when_the_setting_is_outdated() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("server").to_string_lossy().into_owned();
        std::fs::write(&marker, "https://old.example.org\n").unwrap(); // present, but an update waits
        let props = json!({
            "test": {"command": ["cat", marker], "output": "https://vault.example.org"},
            "set": ["sh", "-c", format!("echo https://vault.example.org > '{marker}'")],
            "remove": ["rm", "-f", marker],
            "_exist": false,
        });
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.steps.iter().map(Note::english).collect::<Vec<_>>(), vec![format!("Run: rm -f {marker}")]);
        assert!(!plan.recheck, "a test that fails afterwards proves nothing");
        (plan.apply.unwrap())().unwrap();
        assert!(!marker_exists(&marker), "the outdated setting is left behind");
        // Nothing there: remove runs again and must not fail.
        (evaluate(&props).unwrap().apply.unwrap())().unwrap();
    }

    #[test]
    fn a_json_path_checks_json_output_and_a_missing_program_is_reported() {
        let props = json!({
            "test": {"command": ["echo", "[{\"id\": \"acme@marketplace\", \"enabled\": true}]"], "jsonPath": "$[?@.id=='acme@marketplace'].enabled", "value": true},
            "set": ["false"],
        });
        assert!(evaluate(&props).unwrap().apply.is_none());
        let missing = json!({"test": {"command": ["furnish-no-such-tool"]}, "set": ["x"]});
        let err = evaluate(&missing).err().unwrap();
        assert!(err.chain().any(|e| e.is::<common::Missing>()));
        assert!(check(&json!({"test": {"command": []}, "set": ["x"]})).is_err());
    }
}
