//! `Furnish/Command`: for tools that must apply a setting themselves. `test` runs a command and
//! checks its exit code, its output, or a value in its JSON output; `set` runs only when the test
//! fails, and `remove` (optional) on uninstall, or `remove: keep` to leave the setting in place
//! on uninstall. `present` (optional, the same shape as `test`)
//! tells an outdated setting from a missing one: the step then counts as there, so the app shows
//! an update instead of "not installed". A leading `~/` in the commands' arguments is the home
//! folder, as in the paths of the file resources.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use serde_json::json;

use super::{Plan, common, note::{Note, note}, parse, value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Props {
    test: Test,
    present: Option<Test>,
    set: Vec<String>,
    remove: Option<Remove>,
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

/// What uninstalling does: run a command, or `keep` the setting.
#[derive(Deserialize)]
#[serde(untagged)]
enum Remove {
    Command(Vec<String>),
    Word(String),
}

impl Remove {
    /// The command, `None` for `keep`.
    fn command(&self) -> Option<&Vec<String>> {
        match self {
            Remove::Command(c) => Some(c),
            Remove::Word(_) => None,
        }
    }
}

fn props(v: &Value) -> Result<Props> {
    let p: Props = parse(v)?;
    if let Some(Remove::Word(word)) = &p.remove
        && word != "keep"
    {
        bail!("remove: a command, as a list of the program and its arguments, or 'keep' to leave the setting in place on uninstall");
    }
    let remove = p.remove.as_ref().and_then(Remove::command);
    for (what, command) in [("test.command", Some(&p.test.command)), ("set", Some(&p.set)), ("remove", remove)] {
        if command.is_some_and(Vec::is_empty) {
            bail!("{what}: a command, as a list of the program and its arguments");
        }
    }
    for (what, test) in [("test", Some(&p.test)), ("present", p.present.as_ref())] {
        let Some(test) = test else { continue };
        if test.command.is_empty() {
            bail!("{what}.command: a command, as a list of the program and its arguments");
        }
        if let Some(path) = &test.json_path {
            value::parse_location(path).with_context(|| format!("{what}.jsonPath"))?;
        } else if test.value.is_some() {
            bail!("{what}.value needs {what}.jsonPath");
        }
    }
    Ok(p)
}

pub fn check(v: &Value) -> Result<()> {
    props(v).map(drop)
}

/// Whether the test passes, and a note with what it expected and got (the check that failed,
/// else the most telling one).
fn passes(test: &Test) -> Result<(bool, Note)> {
    let out = exec(&test.command)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let command = test.command.join(" ");
    let code = out.status.code();
    let expected_code = test.exit_code.unwrap_or(0);
    let code_note = note("test.exitCode", json!({ "command": command, "expected": expected_code, "got": code }));
    let mut result = (code == Some(expected_code), code_note);
    if let Some(expected) = &test.output {
        let ok = stdout.trim() == expected.trim();
        let n = if stdout.trim().is_empty() {
            note("test.noOutput", json!({ "command": command, "expected": expected.trim() }))
        } else {
            note("test.output", json!({ "command": command, "expected": expected.trim(), "got": short(stdout.trim()) }))
        };
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
        // A filter may select several values (RFC 9535): the test passes if one of them fits.
        let found = value::query(&doc, &value::parse_location(path)?);
        let ok = match &test.value {
            Some(v) => found.contains(&v),
            None => !found.is_empty(),
        };
        let got = match found.as_slice() {
            [] => "—".to_string(),
            [one] => short(&one.to_string()),
            many => short(&Value::Array(many.iter().map(|v| (*v).clone()).collect()).to_string()),
        };
        let expected = test.value.as_ref().map_or("a value".to_string(), |v| short(&v.to_string()));
        let n = note("test.json", json!({ "command": command, "path": path, "expected": expected, "got": got }));
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

/// Whether a check passes; a program that isn't installed means it doesn't.
fn passes_check(test: &Test) -> Result<bool> {
    match passes(test) {
        Err(e) if e.chain().any(|e| e.is::<common::Missing>()) => Ok(false),
        result => Ok(result?.0),
    }
}

/// Run a command as written in the app, a leading `~/` in its arguments expanded (see
/// [`common::expand_arg`]). Notes and messages show it as written.
fn exec(command: &[String]) -> Result<std::process::Output> {
    let args = command.iter().map(|a| common::expand_arg(a)).collect::<Result<Vec<_>>>()?;
    common::run(&args)
}

fn run(command: &[String]) -> Result<()> {
    let out = exec(command)?;
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
    // Outdated: the test fails, but `present` finds the setting.
    let outdated = !passes && test_note.is_some() && match &p.present {
        Some(present) => passes_check(present)?,
        None => false,
    };
    let there = passes || outdated;
    plan.observe("_exist", Some(there.into()), Some(exist.into()));
    match (exist, there) {
        (true, _) if !passes => {
            let mut steps = test_note.into_iter().collect::<Vec<_>>();
            if outdated {
                let present = p.present.as_ref().map(|t| t.command.join(" ")).unwrap_or_default();
                steps.push(note("test.present", json!({ "command": present })));
            }
            steps.push(note("run", json!({ "command": p.set.join(" ") })));
            plan.change(steps, move || run(&p.set));
        }
        // A failing test means absent or outdated (an update waiting), so removal always runs
        // `remove`, which must therefore do nothing where there is nothing to remove. Its test
        // afterwards proves nothing either way. Where the tool isn't installed, there is nothing.
        // `remove: keep`: uninstalling leaves the setting as it is.
        (false, there) if p.remove.as_ref().is_some_and(|r| r.command().is_none()) => {
            plan.observe("_exist", Some(there.into()), Some(there.into()));
            if test_note.is_some() {
                plan.steps = vec![note("command.keep", json!({ "command": p.set.join(" ") }))];
            }
        }
        (false, _) if p.remove.is_some() && test_note.is_some() => {
            let remove = p.remove.and_then(|r| r.command().cloned()).unwrap_or_default();
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
    fn present_tells_an_outdated_setting_from_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("server").to_string_lossy().into_owned();
        let props = json!({
            "test": {"command": ["cat", marker], "output": "https://vault.example.org"},
            "present": {"command": ["test", "-s", marker]},
            "set": ["sh", "-c", format!("echo https://vault.example.org > '{marker}'")],
            "remove": ["rm", "-f", marker],
        });
        // Missing: not there.
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.state["_exist"], false);
        assert!(!plan.steps.iter().any(|n| n.code == "test.present"));
        // Outdated: there (the app shows an update), and set updates it.
        std::fs::write(&marker, "https://old.example.org\n").unwrap();
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.state["_exist"], true);
        assert!(plan.steps.iter().any(|n| n.code == "test.present"));
        (plan.apply.unwrap())().unwrap();
        let plan = evaluate(&props).unwrap();
        assert!(plan.apply.is_none());
        assert_eq!(plan.state["_exist"], true);
        // Without present, an outdated setting reads as missing, as before.
        std::fs::write(&marker, "https://old.example.org\n").unwrap();
        let mut without = props.clone();
        without.as_object_mut().unwrap().remove("present");
        assert_eq!(evaluate(&without).unwrap().state["_exist"], false);
        assert!(check(&json!({"test": {"command": ["x"]}, "present": {"command": []}, "set": ["x"]})).is_err());
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
    fn remove_keep_leaves_the_setting_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marketplace").to_string_lossy().into_owned();
        std::fs::write(&marker, "acme\n").unwrap();
        let props = json!({
            "test": {"command": ["cat", marker], "output": "acme"},
            "set": ["sh", "-c", format!("echo acme > '{marker}'")],
            "remove": "keep",
            "_exist": false,
        });
        let plan = evaluate(&props).unwrap();
        assert!(plan.apply.is_none(), "nothing runs");
        assert_eq!((&plan.state["_exist"], &plan.after["_exist"]), (&json!(true), &json!(true)));
        assert_eq!(plan.steps.iter().map(Note::english).collect::<Vec<_>>(), vec![format!("Keep what sh -c echo acme > '{marker}' set up: uninstalling leaves it in place")]);
        assert!(marker_exists(&marker));
        // Not there: nothing to keep, and no error.
        std::fs::remove_file(&marker).unwrap();
        let plan = evaluate(&props).unwrap();
        assert!(plan.apply.is_none());
        assert_eq!(plan.after["_exist"], false);
        assert!(check(&props).is_ok());
        assert!(check(&json!({"test": {"command": ["x"]}, "set": ["x"], "remove": "leave"})).is_err());
        assert!(check(&json!({"test": {"command": ["x"]}, "set": ["x"], "remove": []})).is_err());
    }

    #[test]
    fn a_leading_tilde_in_arguments_is_the_home_folder() {
        let home = dirs::home_dir().unwrap();
        let name = format!(".furnish-command-test-{}", std::process::id());
        let file = home.join(&name);
        let props = json!({
            "test": {"command": ["test", "-f", format!("~/{name}")]},
            "set": ["touch", format!("~/{name}")],
            "remove": ["rm", "-f", format!("~/{name}"), "~~/not-expanded"],
        });
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.steps.last().map(Note::english), Some(format!("Run: touch ~/{name}")), "the preview shows ~/");
        (plan.apply.unwrap())().unwrap();
        assert!(file.exists());
        assert!(evaluate(&props).unwrap().apply.is_none());
        let mut gone = props.clone();
        gone["_exist"] = false.into();
        (evaluate(&gone).unwrap().apply.unwrap())().unwrap();
        assert!(!file.exists());
        // Only at the start: `a~/b` is passed as it is.
        let literal = json!({"test": {"command": ["echo", "a~/b", "~"], "output": format!("a~/b {}", home.display())}, "set": ["false"]});
        assert!(evaluate(&literal).unwrap().apply.is_none());
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

    #[test]
    fn a_filter_selects_the_members_of_an_object() {
        // openvpn3 configs-list --json: the profiles keyed by their D-Bus path.
        let profiles = r#"{
          "/net/openvpn/v3/configuration/195ee2c8x8df": {"name": "Acme", "imported": "2026-10-28 08:49:40", "valid": true},
          "/net/openvpn/v3/configuration/2a7c11d0x41f": {"name": "Other", "imported": "2026-11-02 01:23:04", "valid": true}
        }"#;
        let props = |path: &str, value: Value| {
            json!({"test": {"command": ["echo", profiles], "jsonPath": path, "value": value}, "set": ["false"]})
        };
        assert!(evaluate(&props("$[?@.name=='Acme'].name", json!("Acme"))).unwrap().apply.is_none());
        let plan = evaluate(&props("$[?@.name=='Missing'].name", json!("Missing"))).unwrap();
        assert!(plan.apply.is_some());
        assert_eq!(plan.steps[0].params["got"], "—");
        // Several values: the test passes if one of them fits, and the note shows them all.
        assert!(evaluate(&props("$[?@.valid==true].name", json!("Other"))).unwrap().apply.is_none());
        let plan = evaluate(&props("$[?@.valid==true].name", json!("Third"))).unwrap();
        assert_eq!(plan.steps[0].params["got"], r#"["Acme","Other"]"#);
    }
}
