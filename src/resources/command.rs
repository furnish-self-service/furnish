//! `Furnish/Command`: for tools that must apply a setting themselves. `test` runs a command and
//! checks its exit code, its output (against a text or a file's content), or a value in its JSON
//! output; `set` runs only when the test fails, and `remove` (optional) on uninstall, or
//! `remove: keep` to leave the setting in place on uninstall. `present` (optional, the same shape
//! as `test`) tells an outdated setting from a missing one: the step then counts as there, so the
//! app shows an update instead of "not installed". A leading `~/` in the commands' arguments is
//! the home folder, as in the paths of the file resources.

use std::{borrow::Cow, fs, io::Write, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use serde_json::json;

use super::{Plan, common, diff, note::{Note, note}, parse, value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Props {
    content: Option<String>,
    source: Option<PathBuf>,
    #[serde(default, rename = "showDiff")]
    show_diff: bool,
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
    /// Expected output (stdout), compared as [`normalized`].
    output: Option<String>,
    /// A file whose content the output must equal, compared as [`normalized`].
    file: Option<String>,
    /// Lines of the output and of `output` or `file` to leave out of the comparison (a regex).
    ignore_lines: Option<String>,
    /// Compare occurrences of each line independently of their order.
    #[serde(default)]
    ignore_line_order: bool,
    /// A place in the JSON output that must exist, and equal `value` if given.
    json_path: Option<String>,
    value: Option<Value>,
}

/// A command that imports a file gets a fresh, private copy for the duration of that command.
enum Input {
    Content(String),
    Source(PathBuf),
}

impl Input {
    fn bytes(&self) -> Result<Cow<'_, [u8]>> {
        match self {
            Self::Content(text) => Ok(Cow::Borrowed(text.as_bytes())),
            Self::Source(path) => Ok(Cow::Owned(fs::read(path).with_context(|| format!("reading {} from the collection", path.display()))?)),
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Content(_) => "%s",
            Self::Source(path) => path.file_name().and_then(|n| n.to_str()).unwrap_or("%s"),
        }
    }

    fn temporary_file(&self) -> Result<(tempfile::TempDir, PathBuf)> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("furnish-input-");
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let dir = builder.tempdir()?;
        #[cfg(windows)]
        common::set_private(dir.path(), true)?;
        let path = dir.path().join(match self {
            Self::Content(_) => "input",
            Self::Source(_) => self.name(),
        });
        let mut file = fs::File::create_new(&path)?;
        common::set_private(&path, false)?;
        file.write_all(&self.bytes()?)?;
        Ok((dir, path))
    }
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
    if p.content.is_some() && p.source.is_some() {
        bail!("content and source: use one of them for the command's input file");
    }
    if p.show_diff && p.test.file.as_deref() != Some("%s") {
        bail!("showDiff needs test.file set to '%s' to compare command output with the supplied input");
    }
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
        if test.file.as_deref() == Some("%s") && p.content.is_none() && p.source.is_none() {
            bail!("{what}.file: '%s' needs content or source");
        }
        if test.output.is_some() && test.file.is_some() {
            bail!("{what}.output and {what}.file: use one of them");
        }
        if test.ignore_line_order && test.output.is_none() && test.file.is_none() {
            bail!("{what}.ignoreLineOrder needs {what}.output or {what}.file");
        }
        if let Some(pattern) = &test.ignore_lines {
            if test.output.is_none() && test.file.is_none() {
                bail!("{what}.ignoreLines needs {what}.output or {what}.file");
            }
            super::regex(pattern, &format!("{what}.ignoreLines"))?;
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

/// What a test found.
struct Outcome {
    ok: bool,
    /// What it expected and got: the check that failed, else the most telling one.
    note: Note,
    /// Why the result is assumed rather than found: the file to compare with can't be read as
    /// this account (see `super::unreadable`). It counts as passing then.
    unverified: Option<String>,
}

/// Run the test.
fn passes(test: &Test, input: Option<&Input>, projected: &Value, show_diff: bool) -> Result<Outcome> {
    let out = exec(&test.command, input)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let command = test.command.join(" ");
    let code = out.status.code();
    let expected_code = test.exit_code.unwrap_or(0);
    let code_note = note("test.exitCode", json!({ "command": command, "expected": expected_code, "got": code }));
    let mut result = Outcome { ok: code == Some(expected_code), note: code_note, unverified: None };
    let ignore = test.ignore_lines.as_deref().map(|p| super::regex(p, "ignoreLines")).transpose()?;
    // A tool may echo the private input or values from its imported copy.
    let shown = |text: &str| if input.is_some() { "•••".to_string() } else { text.to_string() };
    let normalize = |text: &str| normalized(text, ignore.as_ref(), test.ignore_line_order);
    let compared = match (&test.output, &test.file) {
        (Some(expected), _) => {
            let (got, want) = (normalize(&stdout), normalize(expected));
            let ok = got == want || (test.ignore_line_order && diff::unordered_lines(&got, &want, false).is_none());
            let n = if stdout.trim().is_empty() {
                note("test.noOutput", json!({ "command": command, "expected": shown(expected.trim()) }))
            } else {
                note("test.output", json!({ "command": command, "expected": shown(expected.trim()), "got": shown(&short(stdout.trim())) }))
            };
            Some((ok, n))
        }
        (None, Some(path)) => {
            let (file_text, hidden, shown_path) = if let Some(input) = input.filter(|_| path == "%s") {
                (Some(common::Text::decode(&input.bytes()?)?), !show_diff, input.name())
            } else {
                let mut file = common::Managed::open(path)?;
                if file.unreadable {
                    let why = format!("can't read {} as this account to compare the output of `{command}` with it", file.name());
                    return Ok(Outcome { ok: true, note: note("file.unreadable", json!({ "path": path })), unverified: Some(why) });
                }
                if !projected.is_null() {
                    file.bytes = if projected["exists"] == false {
                        None
                    } else if let Some(content) = projected["content"].as_str() {
                        Some(content.as_bytes().to_vec())
                    } else if let Some(source) = projected["source"].as_str() {
                        Some(std::fs::read(source).with_context(|| format!("reading {source} from the collection"))?)
                    } else {
                        Some(file.bytes.unwrap_or_default())
                    };
                }
                let text = file.bytes.as_ref().map(|_| file.text()).transpose()?;
                (text, input.is_some() || projected["private"] == true || common::owner_only(&file.real), path.as_str())
            };
            let params = json!({ "command": command, "path": shown_path });
            match file_text {
                None => Some((false, note("test.noFile", params))),
                Some(text) => {
                    let (got, want) = (normalize(&stdout), normalize(&text.text));
                    let delta = if got == want {
                        None
                    } else if test.ignore_line_order {
                        diff::unordered_lines(&got, &want, !hidden)
                    } else {
                        Some(diff::lines(&format!("{got}\n"), &format!("{want}\n")))
                    };
                    if let Some(delta) = delta {
                        let mut n = note("test.fileDiffers", params);
                        n.add_diff(Some(delta), hidden);
                        Some((false, n))
                    } else {
                        Some((true, note("test.file", params)))
                    }
                }
            }
        }
        (None, None) => None,
    };
    if let Some((ok, n)) = compared {
        // Output that differs tells more than an exit code nobody asked for.
        if result.ok || (test.exit_code.is_none() && !ok) {
            result = Outcome { ok: result.ok && ok, note: n, unverified: None };
        }
    }
    if let Some(path) = &test.json_path {
        let doc: Value = match serde_json::from_str(&stdout) {
            Ok(doc) => doc,
            Err(_) if !result.ok => return Ok(result),
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
        let n = note("test.json", json!({ "command": command, "path": path, "expected": shown(&expected), "got": shown(&got) }));
        if result.ok {
            result = Outcome { ok, note: n, unverified: None };
        }
    }
    Ok(result)
}

/// A text as the tests compare it: Windows line ends as `\n`, white space at the end of each
/// line and blank lines at the start and end left out, and the lines that `ignore` matches.
fn normalized(text: &str, ignore: Option<&regex::Regex>, unordered: bool) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| ignore.is_none_or(|re| !re.is_match(l))).collect();
    let joined = lines.join("\n");
    // Indentation on a nonblank line must compare the same wherever the tool moves it.
    if unordered { joined.trim_matches('\n') } else { joined.trim() }.to_string()
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
fn passes_check(test: &Test, input: Option<&Input>, projected: &Value) -> Result<bool> {
    match passes(test, input, projected, false) {
        Err(e) if e.chain().any(|e| e.is::<common::Missing>()) => Ok(false),
        result => Ok(result?.ok),
    }
}

/// Run a command as written in the app, a leading `~/` in its arguments expanded (see
/// [`common::expand_arg`]). Notes and messages show it as written.
fn exec(command: &[String], input: Option<&Input>) -> Result<std::process::Output> {
    let temporary = input.filter(|_| command.iter().skip(1).any(|arg| arg == "%s"))
        .map(Input::temporary_file).transpose()?;
    let args = command.iter().enumerate().map(|(index, arg)| match &temporary {
        Some((_, path)) if index > 0 && arg == "%s" => Ok(path.to_string_lossy().into_owned()),
        _ => common::expand_arg(arg),
    }).collect::<Result<Vec<_>>>()?;
    let result = common::run(&args);
    if let Some((dir, _)) = temporary {
        dir.close().context("removing the temporary command input")?;
    }
    result
}

fn run(command: &[String], input: Option<&Input>) -> Result<()> {
    let out = exec(command, input)?;
    if !out.status.success() {
        let output = if input.is_some() { String::new() } else { common::output_text(&out) };
        bail!("`{}` exited with code {}{}", command.join(" "), out.status.code().unwrap_or(-1), output);
    }
    Ok(())
}

pub fn evaluate(v: &Value) -> Result<Plan> {
    evaluate_with_files(v, &Value::Null)
}

pub(super) fn evaluate_with_files(v: &Value, files: &Value) -> Result<Plan> {
    let p = props(v)?;
    let input = p.content.map(Input::Content).or_else(|| p.source.map(Input::Source));
    let exist = p.exist.unwrap_or(true);
    let mut plan = Plan::new(v);
    let (passes, test_note) = match passes(&p.test, input.as_ref(), &files["test"], p.show_diff) {
        // Nothing to remove where the tool isn't installed.
        Err(e) if !exist && e.chain().any(|e| e.is::<common::Missing>()) => (false, None),
        result => {
            let outcome = result?;
            plan.unverified = outcome.unverified;
            (outcome.ok, Some(outcome.note))
        }
    };
    // Outdated: the test fails, but `present` finds the setting.
    let outdated = !passes && test_note.is_some() && match &p.present {
        Some(present) => passes_check(present, input.as_ref(), &files["present"])?,
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
            plan.change(steps, move || run(&p.set, input.as_ref()));
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
            plan.change(vec![note("run", json!({ "command": remove.join(" ") }))], move || run(&remove, input.as_ref()));
            plan.recheck = false;
        }
        (false, true) => bail!("this step has no remove command, so it can't undo what it set"),
        _ => plan.steps = test_note.into_iter().collect(),
    }
    Ok(plan)
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn the_command_gets_a_private_temporary_copy() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("my profile.p12");
        let bytes = [0, 0xff, 1, 2];
        fs::write(&source, bytes).unwrap();
        let input = Input::Source(source.clone());
        let (temporary, copy) = input.temporary_file().unwrap();
        assert!(common::is_private(temporary.path(), true).unwrap());
        assert!(common::is_private(&copy, false).unwrap());
        assert_eq!(copy.file_name(), source.file_name());
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        fs::write(&copy, [3]).unwrap();
        drop(temporary);
        assert!(!copy.exists());
        assert_eq!(fs::read(source).unwrap(), bytes);
    }
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

    #[test]
    fn command_input_is_scoped_to_each_invocation_and_diagnostics_hide_it() {
        use std::path::Path;

        let dir = tempfile::tempdir().unwrap();
        let receipt = dir.path().join("input-path");
        let imported = dir.path().join("tool copy");
        let secret = "private profile content";
        let props = json!({
            "content": secret,
            "test": { "command": ["cat", imported], "file": "%s" },
            "present": { "command": ["cat", "%s"], "file": "%s" },
            "set": ["sh", "-c", r#"echo "$1" > "$2"; cp "$1" "$3""#, "import", "%s", receipt, imported],
            "remove": ["sh", "-c", r#"test -f "$1" && rm "$2""#, "remove", "%s", imported]
        });
        let plan = evaluate(&props).unwrap();
        assert!(plan.steps.iter().any(|n| n.code == "test.present"));
        assert_eq!(plan.steps[0].params["hidden"], true);
        assert!(!format!("{:?}", plan.steps).contains(secret));
        (plan.apply.unwrap())().unwrap();
        assert!(!Path::new(fs::read_to_string(&receipt).unwrap().trim()).exists());
        assert!(evaluate(&props).unwrap().apply.is_none());

        let mut gone = props.clone();
        gone["_exist"] = false.into();
        (evaluate(&gone).unwrap().apply.unwrap())().unwrap();
        assert!(!imported.exists());

        let mut fail = props.clone();
        fail["set"] = json!(["sh", "-c", r#"echo "$1" > "$2"; cat "$1" >&2; exit 4"#, "import", "%s", receipt]);
        let err = (evaluate(&fail).unwrap().apply.unwrap())().unwrap_err().to_string();
        assert!(err.contains("exited with code 4") && !err.contains(secret));
        assert!(!Path::new(fs::read_to_string(receipt).unwrap().trim()).exists());
        fail["test"] = json!({"command":["cat", "%s"], "output":"different"});
        assert!(!format!("{:?}", evaluate(&fail).unwrap().steps).contains(secret));

        let literal = json!({"test":{"command":["echo", "%s"], "output":"%s"}, "set":["false"]});
        assert!(evaluate(&literal).unwrap().apply.is_none());
        assert!(check(&json!({"test":{"command":["cat"], "file":"%s"}, "set":["x"]})).is_err());
        let mut ambiguous = props;
        ambiguous["source"] = "vpn.ovpn".into();
        assert!(check(&ambiguous).is_err());
    }

    #[test]
    fn command_input_diff_shows_normalized_lines_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("vpn.ovpn");
        fs::write(&source, "# expected profile\nclient\nremote new.example\n").unwrap();
        let mut props = json!({
            "source": source,
            "test": {
                "command": ["printf", "client\n\nremote old.example\n"],
                "file": "%s",
                "ignoreLines": "^(#|$)"
            },
            "set": ["true"]
        });
        let comparison = |props: &Value| evaluate(props).unwrap().steps.remove(0).params;
        let hidden = comparison(&props);
        assert_eq!(hidden["hidden"], true);
        assert!(!hidden.contains_key("diff"));

        props["showDiff"] = true.into();
        let shown = comparison(&props);
        assert_eq!(shown["diff"]["hunks"][0]["lines"], json!([" client", "-remote old.example", "+remote new.example"]));
        assert!(!shown.contains_key("hidden"));

        props["test"]["file"] = json!(source);
        assert!(check(&props).is_err());
    }

    #[test]
    fn reordered_imports_match_and_the_preview_counts_only_unmatched_copies() {
        let mut p = json!({
            "content": "# profile\nclient\nremote example\nremote example\n<ca>\ncertificate\n</ca>\n",
            "test": {"command": ["printf", "<ca>\ncertificate\n</ca>\n\nremote example  \r\nclient\nremote example\n"],
                "file": "%s", "ignoreLines": "^(#|$)"}, "set": ["false"]
        });
        assert!(evaluate(&p).unwrap().apply.is_some());
        p["test"]["ignoreLineOrder"] = true.into();
        assert!(evaluate(&p).unwrap().apply.is_none());

        p["test"]["command"][1] = "client\nextra\n<ca>\ncertificate\n</ca>\nremote example\n".into();
        p["present"] = json!({"command": ["printf", "b\na"], "output": "a\nb", "ignoreLineOrder": true});
        let plan = evaluate(&p).unwrap();
        assert!(plan.apply.is_some());
        assert_eq!(plan.state["_exist"], true);
        let hidden = &plan.steps[0].params;
        assert_eq!(hidden["removed"], 1);
        assert_eq!(hidden["added"], 1);
        assert_eq!(hidden["hidden"], true);
        assert!(!hidden.contains_key("diff"));
        p["showDiff"] = true.into();
        let shown = evaluate(&p).unwrap();
        let hunks = shown.steps[0].params["diff"]["hunks"].as_array().unwrap();
        let lines: Vec<_> = hunks.iter().flat_map(|h| h["lines"].as_array().unwrap()).collect();
        assert_eq!(lines, vec!["-extra", "+remote example"]);
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
    fn the_output_can_be_compared_with_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("acme.ovpn");
        let copy = dir.path().join("imported.ovpn");
        let (profile_s, copy_s) = (profile.to_string_lossy().into_owned(), copy.to_string_lossy().into_owned());
        // The tool's copy drops the comment lines, adds blank lines, and ends lines with \r\n.
        std::fs::write(&profile, "# Acme VPN\nclient\nremote vpn.acme.example 1194  \n\n# keys\nproto udp\n").unwrap();
        let props = json!({
            "test": {"command": ["cat", copy_s], "file": profile_s, "ignoreLines": "^(#|$)"},
            "set": ["sh", "-c", format!("printf 'client\\r\\n\\r\\nremote vpn.acme.example 1194\\r\\nproto udp\\r\\n' > '{copy_s}'")],
            "remove": ["rm", "-f", copy_s],
        });
        // No copy yet: cat fails, and the note names the comparison.
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.state["_exist"], false);
        assert_eq!(plan.steps[0].code, "test.fileDiffers");
        (plan.apply.unwrap())().unwrap();
        let plan = evaluate(&props).unwrap();
        assert!(plan.apply.is_none(), "{:?}", plan.steps);
        assert_eq!(plan.steps[0].english(), format!("Test: the output of cat {copy_s} matches {profile_s}"));
        // The same output from a command that fails: the exit code is what failed.
        let mut failing = props.clone();
        failing["test"]["command"] = json!(["sh", "-c", format!("cat '{copy_s}'; exit 3")]);
        assert_eq!(evaluate(&failing).unwrap().steps[0].code, "test.exitCode");
        // A changed profile is drift, with the differing lines.
        std::fs::write(&profile, "client\nremote vpn2.acme.example 1194\nproto udp\n").unwrap();
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.steps[0].code, "test.fileDiffers");
        assert_eq!(plan.steps[0].params["diff"]["hunks"][0]["lines"], json!([" client", "-remote vpn.acme.example 1194", "+remote vpn2.acme.example 1194", " proto udp"]));
        assert_eq!((plan.steps[0].params["added"].as_u64(), plan.steps[0].params["removed"].as_u64()), (Some(1), Some(1)));
        // A file only its owner may read shows the counts only.
        common::set_private(&profile, false).unwrap();
        let hidden = evaluate(&props).unwrap();
        assert!(hidden.steps[0].params.get("diff").is_none());
        assert_eq!(hidden.steps[0].params["hidden"], true);
        // Without ignoreLines, the comment lines count.
        std::fs::write(&profile, "# Acme VPN\nclient\nremote vpn.acme.example 1194\nproto udp\n").unwrap();
        let mut strict = props.clone();
        strict["test"].as_object_mut().unwrap().remove("ignoreLines");
        assert!(evaluate(&strict).unwrap().apply.is_some());
        assert!(evaluate(&props).unwrap().apply.is_none());
        // No file to compare with.
        std::fs::remove_file(&profile).unwrap();
        let plan = evaluate(&props).unwrap();
        assert_eq!(plan.steps[0].code, "test.noFile");
        // Mistakes.
        let test = |t: Value| check(&json!({"test": t, "set": ["x"]}));
        assert!(test(json!({"command": ["x"], "file": "~/a", "output": "b"})).is_err());
        assert!(test(json!({"command": ["x"], "ignoreLines": "^#"})).is_err());
        assert!(test(json!({"command": ["x"], "ignoreLineOrder": true})).is_err());
        assert!(test(json!({"command": ["x"], "file": "~/a", "ignoreLines": "("})).is_err());
        assert!(test(json!({"command": ["x"], "output": "a", "ignoreLines": "^#"})).is_ok());
    }

    #[test]
    fn normalizing_ignores_line_ends_trailing_spaces_and_ignored_lines() {
        let re = regex::Regex::new("^(#|$)").unwrap();
        assert_eq!(normalized("\r\n a  \r\n\r\n# x\r\nb\r\n\r\n", Some(&re), false), "a\nb");
        assert_eq!(normalized("  a \n\n b\n", None, false), "a\n\n b");
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
