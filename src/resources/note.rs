//! What a set would do, one note per step: a code and its parameters, so the GUI can show it in
//! the user's language (`plan.note.<code>` in gui/src/lib/locales/), and an English sentence for
//! the CLI and logs (DSC's `_metadata.whatIf`).

use serde_json::{Map, Value, json};

use super::diff::{self, Diff};

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub code: &'static str,
    pub params: Map<String, Value>,
}

pub fn note(code: &'static str, params: Value) -> Note {
    Note { code, params: params.as_object().cloned().unwrap_or_default() }
}

impl Note {
    fn s(&self, key: &str) -> String {
        match self.params.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }

    /// The note as an English sentence.
    pub fn english(&self) -> String {
        let s = |k| self.s(k);
        match self.code {
            "zeroInstall.nameInUse" => format!("Zero Install launcher '{}' belongs to {}", s("name"), s("feed")),
            "package.explicit" => format!("The app selects the {} package manager", s("manager")),
            "package.mappedManager" => format!("The app provides a package name for the available {} manager", s("manager")),
            "package.native" => format!("{} is the first available system package manager", s("manager")),
            "package.explicitScript" => format!("The app selects the collection’s {} script manager", s("manager")),
            "package.onlyScript" => format!("{} is the only available collection script manager", s("manager")),
            "package.windows" => format!("Windows uses {} as its fallback package manager", s("manager")),
            "package.needsTool" => format!("Needs {} to preview the package changes", s("tool")),
            "package.noManager" => "No package manager is available".to_owned(),
            "package.distroName" => format!("Package name {} matches distribution {}", s("name"), s("selector")),
            "package.familyName" => format!("Package name {} matches {} in ID_LIKE", s("name"), s("selector")),
            "package.managerName" => format!("Package name {} comes from the mapping for {}", s("name"), s("selector")),
            "package.defaultName" => format!("{} is the default package name", s("name")),
            "dir.sync" => format!("Synchronize files in {} (added: {}, changed: {}, removed: {})", s("path"), s("filesAdded"), s("filesChanged"), s("filesRemoved")),
            "dir.create" => format!("Create the folder {}", s("path")),
            "dir.remove" => format!("Remove the empty folder {}", s("path")),
            "dir.keep" => format!("Keep {}: it isn't empty", s("path")),
            "mode" => format!("Set the mode of {} to {}", s("path"), s("mode")),
            "private" => format!("Make {} accessible only to you", s("path")),
            "file.requiresClosed" => format!("Close {} before changing this file", s("names")),
            "file.create" => format!("Create {} ({} {})", s("path"), s("lines"), if s("lines") == "1" { "line" } else { "lines" }),
            "file.copy" => format!("Copy {} to {}", s("source"), s("path")),
            "file.write" => format!("Change {} ({} lines added, {} removed)", s("path"), s("added"), s("removed")),
            "file.unreadable" => format!("Can't read {} as you; it exists, so it counts as set up", s("path")),
            "file.delete" => format!("Delete {}", s("path")),
            "backup" => format!("Keep a copy of the current {} first", s("path")),
            "validate" => format!("Check the new file first: {}", s("command")),
            "block.add" => format!("Add the block to {}", s("path")),
            "block.update" => format!("Update the block in {}", s("path")),
            "block.remove" => format!("Remove the block from {}", s("path")),
            "line.add" => format!("Add '{}' to {}", s("line"), s("path")),
            "line.replace" => format!("Replace '{}' with '{}' in {}", s("old"), s("line"), s("path")),
            "line.move" => format!("Move '{}' to its place in {}", s("line"), s("path")),
            "line.remove" => format!("Remove '{}' from {}", s("line"), s("path")),
            "value.set" => format!("Set {} in {}", s("location"), s("path")),
            "value.remove" => format!("Remove {} from {}", s("location"), s("path")),
            "value.add" => format!("Add the value to the list at {} in {}", s("location"), s("path")),
            "value.keep" => format!("Keep {} in {}: it is {}, which this app doesn't remove", s("location"), s("path"), s("value")),
            "value.removeMember" => format!("Remove the value from the list at {} in {}", s("location"), s("path")),
            "test.output" => format!("Test: {} printed {}, expected {}", s("command"), s("got"), s("expected")),
            "test.noOutput" => format!("Test: {} printed nothing, expected {}", s("command"), s("expected")),
            "test.present" => format!("{} finds the setting: it is there but outdated", s("command")),
            "test.exitCode" => format!("Test: {} exited with code {}, expected {}", s("command"), s("got"), s("expected")),
            "test.json" => format!("Test: {} has {} at {}, expected {}", s("command"), s("got"), s("path"), s("expected")),
            "test.file" => format!("Test: the output of {} matches {}", s("command"), s("path")),
            "test.fileDiffers" => format!("Test: the output of {} differs from {}", s("command"), s("path")),
            "test.noFile" => format!("Test: can't compare the output of {} with {}: there is no such file", s("command"), s("path")),
            "command.keep" => format!("Keep what {} set up: uninstalling leaves it in place", s("command")),
            // The GUI's existing format for commands.
            "run" => format!("Run: {}", s("command")),
            other => format!("{other} {}", Value::Object(self.params.clone())),
        }
    }
}

/// The note for writing a file whose text changes from `old` (`None`: no file) to `new`, with
/// the lines added and removed and a line diff unless `hidden`.
pub fn text_change(path: &str, old: Option<&str>, new: &str, hidden: bool) -> Note {
    let mut n = match old {
        None => note("file.create", json!({ "path": path, "lines": if new.is_empty() { 0 } else { new.lines().count() } })),
        Some(_) => note("file.write", json!({ "path": path })),
    };
    n.add_diff(Some(diff::lines(old.unwrap_or(""), new)), hidden);
    n
}

impl Note {
    /// Add what a step changes in a file: `added`, `removed` and `diff`; only the counts if
    /// `hidden` (the file may hold secrets), `binary` if the file isn't text (`None`).
    pub fn add_diff(&mut self, diff: Option<Diff>, hidden: bool) {
        let Some(diff) = diff else {
            self.params.insert("binary".into(), true.into());
            return;
        };
        self.params.insert("added".into(), diff.added.into());
        self.params.insert("removed".into(), diff.removed.into());
        if hidden {
            self.params.insert("hidden".into(), true.into());
        } else if !diff.hunks.is_empty() {
            self.params.insert("diff".into(), diff.to_json());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_shows_counts_and_a_diff_unless_hidden() {
        let n = text_change("~/.ssh/config", Some("a\nb\nc\n"), "a\nB\nc\nd\n", false);
        assert_eq!(n.code, "file.write");
        assert_eq!((n.params["added"].as_u64(), n.params["removed"].as_u64()), (Some(2), Some(1)));
        assert_eq!(n.params["diff"]["hunks"][0]["lines"], json!([" a", "-b", "+B", " c", "+d"]));
        assert_eq!(n.english(), "Change ~/.ssh/config (2 lines added, 1 removed)");
        let secret = text_change("~/vpn.ovpn", Some("key 1\n"), "key 2\n", true);
        assert!(secret.params.get("diff").is_none(), "a private file's lines aren't shown");
        assert_eq!((secret.params["added"].as_u64(), secret.params["hidden"].as_bool()), (Some(1), Some(true)));
        let created = text_change("~/x", None, "1\n2\n", false);
        assert_eq!(created.english(), "Create ~/x (2 lines)");
        assert_eq!(created.params["diff"]["hunks"][0]["lines"], json!(["+1", "+2"]));
        let mut binary = note("file.copy", json!({ "path": "~/x", "source": "x.bin" }));
        binary.add_diff(None, false);
        assert_eq!(binary.params["binary"], true);
    }
}
