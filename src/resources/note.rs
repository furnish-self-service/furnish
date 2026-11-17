//! What a set would do, one note per step: a code and its parameters, so the GUI can show it in
//! the user's language (`plan.note.<code>` in gui/src/lib/locales/), and an English sentence for
//! the CLI and logs (DSC's `_metadata.whatIf`).

use serde_json::{Map, Value, json};

/// Lines a diff shows at most, and characters per line.
const DIFF_LINES: usize = 12;
const DIFF_WIDTH: usize = 120;

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
            "dir.create" => format!("Create the folder {}", s("path")),
            "dir.remove" => format!("Remove the empty folder {}", s("path")),
            "dir.keep" => format!("Keep {}: it isn't empty", s("path")),
            "mode" => format!("Set the mode of {} to {}", s("path"), s("mode")),
            "private" => format!("Make {} accessible only to you", s("path")),
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
            "value.removeMember" => format!("Remove the value from the list at {} in {}", s("location"), s("path")),
            "test.output" => format!("Test: {} printed {}, expected {}", s("command"), s("got"), s("expected")),
            "test.noOutput" => format!("Test: {} printed nothing, expected {}", s("command"), s("expected")),
            "test.exitCode" => format!("Test: {} exited with code {}, expected {}", s("command"), s("got"), s("expected")),
            "test.json" => format!("Test: {} has {} at {}, expected {}", s("command"), s("got"), s("path"), s("expected")),
            // The GUI's existing format for commands.
            "run" => format!("Run: {}", s("command")),
            other => format!("{other} {}", Value::Object(self.params.clone())),
        }
    }
}

/// The notes for writing a file whose text changes from `old` (`None`: no file) to `new`, with
/// a short line diff unless `secret`.
pub fn text_change(path: &str, old: Option<&str>, new: &str, secret: bool) -> Note {
    let lines = |t: &str| if t.is_empty() { 0 } else { t.lines().count() };
    let Some(old) = old else {
        return note("file.create", json!({ "path": path, "lines": lines(new) }));
    };
    let (added, removed, diff) = diff(old, new);
    let mut params = json!({ "path": path, "added": added, "removed": removed });
    if !secret && !diff.is_empty() {
        params["diff"] = diff.into();
    }
    note("file.write", params)
}

/// Lines added and removed, and the first changed lines as `+ line` / `- line`.
fn diff(old: &str, new: &str) -> (usize, usize, Vec<String>) {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    // Common start and end, then the longest common subsequence of what is between (small files).
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    let mut out = Vec::new();
    let mut push = |sign: char, line: &str| {
        if out.len() < DIFF_LINES {
            let mut shown: String = line.chars().take(DIFF_WIDTH).collect();
            if line.chars().count() > DIFF_WIDTH {
                shown.push('…');
            }
            out.push(format!("{sign} {shown}"));
        }
    };
    if a.len() * b.len() > 1_000_000 {
        a.iter().for_each(|l| push('-', l));
        b.iter().for_each(|l| push('+', l));
        return (b.len(), a.len(), out);
    }
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut added, mut removed) = (0, 0, 0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            push('-', a[i]);
            removed += 1;
            i += 1;
        } else {
            push('+', b[j]);
            added += 1;
            j += 1;
        }
    }
    (added, removed, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_shows_counts_and_the_changed_lines_only() {
        let n = text_change("~/.ssh/config", Some("a\nb\nc\n"), "a\nB\nc\nd\n", false);
        assert_eq!(n.code, "file.write");
        assert_eq!((n.params["added"].as_u64(), n.params["removed"].as_u64()), (Some(2), Some(1)));
        assert_eq!(n.params["diff"], json!(["- b", "+ B", "+ d"]));
        assert_eq!(n.english(), "Change ~/.ssh/config (2 lines added, 1 removed)");
        let secret = text_change("~/vpn.ovpn", Some("key 1\n"), "key 2\n", true);
        assert!(secret.params.get("diff").is_none(), "a private file's lines aren't shown");
        let created = text_change("~/x", None, "1\n2\n", false);
        assert_eq!(created.english(), "Create ~/x (2 lines)");
    }

    #[test]
    fn a_long_diff_is_capped() {
        let new: String = (0..100).map(|i| format!("{}{i}\n", "x".repeat(200))).collect();
        let n = text_change("f", Some(""), &new, false);
        let diff = n.params["diff"].as_array().unwrap();
        assert_eq!(diff.len(), DIFF_LINES);
        assert!(diff[0].as_str().unwrap().chars().count() <= DIFF_WIDTH + 3);
        assert_eq!(n.params["added"], 100);
    }
}
