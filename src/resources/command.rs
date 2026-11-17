//! `Furnish/Command`: for tools that must apply a setting themselves. `test` runs a command and
//! checks its exit code, its output, or a value in its JSON output; `set` runs only when the test
//! fails, and `remove` (optional) when the test passes and the setting should go.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use super::{Plan, common, parse, value};

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

/// Whether the test passes.
fn passes(test: &Test) -> Result<bool> {
    let out = common::run(&test.command)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut ok = out.status.code() == Some(test.exit_code.unwrap_or(0));
    if let Some(expected) = &test.output {
        ok &= stdout.trim() == expected.trim();
    }
    if let Some(path) = &test.json_path {
        let doc: Value = match serde_json::from_str(&stdout) {
            Ok(doc) => doc,
            Err(_) if !ok => return Ok(false),
            Err(_) => bail!("test: `{}` didn't print JSON", test.command.join(" ")),
        };
        let found = value::current(&doc, &value::parse_location(path)?).unwrap_or(None);
        ok &= match &test.value {
            Some(v) => found == Some(v),
            None => found.is_some(),
        };
    }
    Ok(ok)
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
    let passes = match passes(&p.test) {
        // Nothing to remove where the tool isn't installed.
        Err(e) if !exist && e.chain().any(|e| e.is::<common::Missing>()) => false,
        result => result?,
    };
    plan.observe("_exist", Some(passes.into()), Some(exist.into()));
    match (exist, passes) {
        (true, false) => {
            let steps = vec![format!("Run {}", p.set.join(" "))];
            plan.change(steps, move || run(&p.set));
        }
        (false, true) => match p.remove {
            Some(remove) => {
                let steps = vec![format!("Run {}", remove.join(" "))];
                plan.change(steps, move || run(&remove));
            }
            None => bail!("this step has no remove command, so it can't undo what it set"),
        },
        _ => {}
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
            "remove": ["rm", marker],
        });
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.state["_exist"], false);
        (plan.apply.unwrap())().unwrap();
        assert!(evaluate(&props).unwrap().apply.is_none(), "test passes after set");
        let mut gone = props.clone();
        gone["_exist"] = false.into();
        (evaluate(&gone).unwrap().apply.unwrap())().unwrap();
        assert!(evaluate(&gone).unwrap().apply.is_none());
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
