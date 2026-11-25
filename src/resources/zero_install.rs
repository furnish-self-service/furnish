//! Zero Install owns the feed's trust, implementation cache and launcher. Query its own
//! registration when determining presence, including apps added outside Furnish.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{Plan, common, note};

struct App<'a> {
    name: &'a str,
    feed: &'a str,
}

pub(crate) fn valid_launcher(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !name.eq_ignore_ascii_case("0install")
}

pub(crate) fn validate_feed(feed: &str) -> Result<()> {
    let uri: ureq::http::Uri = feed.parse().context("invalid Zero Install feed URL")?;
    ensure!(uri.scheme_str() == Some("https") && uri.authority().is_some_and(|a| !a.host().is_empty() && !a.as_str().contains('@'))
        && !feed.contains(['#', '\\']) && !feed.chars().any(char::is_control), "Zero Install needs an HTTPS feed URL without credentials or fragment");
    Ok(())
}

fn argv(args: &[&str]) -> Vec<String> {
    std::iter::once("0install").chain(args.iter().copied()).map(str::to_owned).collect()
}

fn execute(args: &[String]) -> Result<String> {
    let output = common::run(args)?;
    ensure!(output.status.success(), "{} exited with {}{}", args.join(" "), output.status, common::output_text(&output));
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(unix)]
fn registered_feed(app: &App<'_>) -> Result<Option<String>> {
    use std::{env, path::PathBuf};
    // The Unix CLI has no list-apps command. A missing name passed to "show" is
    // interpreted as a selections filename. Look for its registered app directory first.
    let roots = if let Some(base) = env::var_os("ZEROINSTALL_PORTABLE_BASE") {
        vec![PathBuf::from(base).join("config")]
    } else {
        let home = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|p| p.join(".config"))).context("cannot locate Zero Install configuration")?;
        let dirs = env::var_os("XDG_CONFIG_DIRS").filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
        std::iter::once(home).chain(env::split_paths(&dirs)).map(|p| p.join("0install.net")).collect()
    };
    for root in roots {
        if root.join("apps").join(app.name).try_exists()? {
            let root_uri = execute(&argv(&["show", "--root-uri", app.name]))?;
            return Ok(Some(root_uri.trim().to_owned()));
        }
    }
    // A missing prerequisite must surface as "Needs 0install", including during preview.
    execute(&argv(&["--version"]))?;
    Ok(None)
}

#[cfg(windows)]
fn registered_feed(app: &App<'_>) -> Result<Option<String>> {
    let text = execute(&argv(&["list-apps", "--batch", "--xml", app.feed]))?;
    let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).context("reading Zero Install application list")?;
    let root = doc.root_element();
    ensure!(root.tag_name().name() == "app-list", "unexpected Zero Install application list");
    // list-apps filters an absolute URI by the exact interface, including its URI normalization.
    Ok(root.children().any(|node| node.has_tag_name("app")).then(|| app.feed.to_owned()))
}

pub(crate) fn run(props: &Value, name: &str, op: &str, what_if: bool) -> Result<Value> {
    let feed = props["feed"].as_str().context("0install requires a feed URL")?;
    validate_feed(feed)?;
    ensure!(props.get("targetVersion").is_none(), "0install manages versions through its feed and cannot pin a targetVersion");
    let want = props.get("_exist").map(|v| v.as_bool().context("_exist must be a boolean")).transpose()?.unwrap_or(true);
    let app = App { name, feed };
    let owner = registered_feed(&app);
    if let Some(command) = props["command"].as_str() {
        ensure!(super::valid_program_name(command), "command must be a program name without a directory");
        let missing = owner.as_ref().is_err_and(|e| e.downcast_ref::<common::Missing>().is_some());
        if (missing || owner.as_ref().is_ok_and(|feed| feed.as_deref() != Some(app.feed)))
            && let Some(path) = common::on_command_path(command) {
            return super::externally_owned("0install", name, path, op, want, what_if);
        }
    }
    let plan = evaluate(&app, want, owner?);
    super::run_plan(props, op, what_if, plan, || Ok(evaluate(&app, want, registered_feed(&app)?)))
}

fn evaluate(app: &App<'_>, want: bool, owner: Option<String>) -> Plan {
    let present = owner.as_deref() == Some(app.feed);
    let mut plan = Plan::new(&json!({"name":app.name,"packageManager":"0install","feed":app.feed}));
    plan.observe("_exist", Some(present.into()), Some(want.into()));
    if want && let Some(other) = owner.filter(|_| !present) {
        plan.observe("_exist", Some(false.into()), Some(false.into()));
        let conflict = note("zeroInstall.nameInUse", json!({ "name": app.name, "feed": other }));
        let reason = conflict.english();
        plan.change(vec![conflict], move || anyhow::bail!("{reason}"));
        return plan;
    }
    if want != present {
        #[cfg(unix)]
        let commands = if want { vec![argv(&["add", "--console", app.name, app.feed])] } else { vec![argv(&["destroy", app.name])] };
        #[cfg(windows)]
        let commands = if want {
            // Desktop integration can download in the background. Finish downloading first so
            // an unsuccessful install cannot leave an apparently installed registration.
            vec![argv(&["download", "--batch", app.feed]), argv(&["integrate", "--batch", "--no-download", "--add-standard", app.feed])]
        } else {
            vec![argv(&["remove", "--batch", app.feed])]
        };
        let steps = commands.iter().map(|args| note("run", json!({ "command": args.join(" ") }))).collect();
        plan.change(steps, move || {
            for args in commands { execute(&args)?; }
            Ok(())
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_and_launcher_are_validated_before_invoking_zero_install() {
        for feed in ["http://example/acme.xml", "file:///etc/acme.xml", "https://user:pass@example/acme.xml", "https://example/acme.xml#other"] {
            assert!(validate_feed(feed).is_err(), "{feed}");
        }
        for name in ["", "--help", "0install", "../other", "C:\\other"] {
            assert!(!valid_launcher(name), "{name}");
        }
    }
}
