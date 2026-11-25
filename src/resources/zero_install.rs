//! Zero Install owns the feed's trust, implementation cache and launcher. Query its own
//! registration when determining presence, including apps added outside Furnish.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Plan, common, exist, note, parse};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct App {
    name: String,
    feed: String,
    #[serde(rename = "_exist")]
    exists: Option<bool>,
}

impl App {
    fn checked(props: &Value) -> Result<Self> {
        let app: Self = parse(props)?;
        ensure!(app.name.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && app.name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && !app.name.eq_ignore_ascii_case("0install"), "invalid Zero Install launcher name");
        let uri: ureq::http::Uri = app.feed.parse().context("invalid Zero Install feed URL")?;
        ensure!(uri.scheme_str() == Some("https") && uri.authority().is_some_and(|a| !a.host().is_empty() && !a.as_str().contains('@'))
            && !app.feed.contains(['#', '\\']) && !app.feed.chars().any(char::is_control), "Zero Install needs an HTTPS feed URL without credentials or fragment");
        Ok(app)
    }
}

pub(super) fn check(props: &Value) -> Result<()> {
    App::checked(props).map(drop)
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
fn installed(app: &App) -> Result<bool> {
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
        if root.join("apps").join(&app.name).try_exists()? {
            let root_uri = execute(&argv(&["show", "--root-uri", &app.name]))?;
            let selected = root_uri.trim();
            ensure!(selected == app.feed, "Zero Install launcher '{}' belongs to {selected}, expected {}", app.name, app.feed);
            return Ok(true);
        }
    }
    // A missing prerequisite must surface as "Needs 0install", including during preview.
    execute(&argv(&["--version"]))?;
    Ok(false)
}

#[cfg(windows)]
fn installed(app: &App) -> Result<bool> {
    let text = execute(&argv(&["list-apps", "--batch", "--xml", &app.feed]))?;
    let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).context("reading Zero Install application list")?;
    let root = doc.root_element();
    ensure!(root.tag_name().name() == "app-list", "unexpected Zero Install application list");
    // list-apps filters an absolute URI by the exact interface, including its URI normalization.
    Ok(root.children().any(|node| node.has_tag_name("app")))
}

pub(super) fn evaluate(props: &Value) -> Result<Plan> {
    let app = App::checked(props)?;
    let want = exist(app.exists);
    let present = installed(&app)?;
    let mut plan = Plan::new(props);
    plan.observe("_exist", Some(present.into()), Some(want.into()));
    if want != present {
        #[cfg(unix)]
        let commands = if want { vec![argv(&["add", "--console", &app.name, &app.feed])] } else { vec![argv(&["destroy", &app.name])] };
        #[cfg(windows)]
        let commands = if want {
            // Desktop integration can download in the background. Finish downloading first so
            // an unsuccessful install cannot leave an apparently installed registration.
            vec![argv(&["download", "--batch", &app.feed]), argv(&["integrate", "--batch", "--no-download", "--add-standard", &app.feed])]
        } else {
            vec![argv(&["remove", "--batch", &app.feed])]
        };
        let steps = commands.iter().map(|args| note("run", json!({ "command": args.join(" ") }))).collect();
        plan.change(steps, move || {
            for args in commands { execute(&args)?; }
            Ok(())
        });
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_and_launcher_are_validated_before_invoking_zero_install() {
        let valid = json!({"name":"acme-tool", "feed":"https://software.example/acme.xml"});
        assert!(check(&valid).is_ok());
        for feed in ["http://example/acme.xml", "file:///etc/acme.xml", "https://user:pass@example/acme.xml", "https://example/acme.xml#other"] {
            let mut invalid = valid.clone();
            invalid["feed"] = json!(feed);
            assert!(check(&invalid).is_err(), "{feed}");
        }
        for name in ["", "--help", "0install", "../other", "C:\\other"] {
            let mut invalid = valid.clone();
            invalid["name"] = json!(name);
            assert!(check(&invalid).is_err(), "{name}");
        }
    }
}
