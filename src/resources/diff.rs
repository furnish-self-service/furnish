//! Line diffs for previews: what a step changes in a text file, as hunks with 3 lines of context,
//! capped so a large file doesn't flood the preview. Resources put [`Diff::to_json`] into a note's
//! `diff` parameter; the GUI (Plan.svelte) and the CLI ([`plain`]) render it.
//!
//! JSON: `{"hunks": [{"old": 3, "new": 3, "lines": [" kept", "-removed", "+added"]}], "more": 0}`,
//! where `old`/`new` are the 1-based numbers of the hunk's first line in the old and new text, and
//! `more` counts the diff lines left out by the cap. A line `\-`, `\+` or `\ ` after a line says
//! that the old text, the new text or both end there without a line break, like git's
//! "\ No newline at end of file"; it isn't a line of the file.

use std::time::Duration;

use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};

/// Lines of context around each change.
const CONTEXT: usize = 3;
/// Diff lines (context included) a step shows at most.
pub const MAX_LINES: usize = 200;
/// Characters a line shows at most; longer ones end in `…`.
pub const MAX_WIDTH: usize = 200;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diff {
    /// Lines added and removed in the whole change (not only what is shown).
    pub added: usize,
    pub removed: usize,
    pub hunks: Vec<Hunk>,
    /// Diff lines left out because of [`MAX_LINES`].
    pub more: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    /// 1-based numbers of the first line in the old and the new text.
    pub old: usize,
    pub new: usize,
    /// Each line starts with ` ` (kept), `-` (removed) or `+` (added), or is a `\` marker.
    pub lines: Vec<String>,
}

/// Stands for "no line break at the end" as the last line of a text, so that a line differing
/// only in its line break compares equal and the missing break shows as a change of its own.
const NO_NEWLINE: &str = "\0furnish: no newline at end of file\n";

/// A text ending in a line break, followed by [`NO_NEWLINE`] if it had none.
fn ended(text: &str) -> String {
    match text {
        "" => String::new(),
        t if t.ends_with('\n') => t.to_string(),
        t => format!("{t}\n{NO_NEWLINE}"),
    }
}

/// The line diff from `old` to `new`.
pub fn lines(old: &str, new: &str) -> Diff {
    let (old, new) = (ended(old), ended(new));
    // A huge file with many changes falls back to a coarser (still correct) diff after a second.
    let diff = TextDiff::configure().timeout(Duration::from_secs(1)).diff_lines(&old, &new);
    let mut out = Diff::default();
    for change in diff.iter_all_changes().filter(|c| c.value() != NO_NEWLINE) {
        match change.tag() {
            ChangeTag::Insert => out.added += 1,
            ChangeTag::Delete => out.removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let mut shown = 0;
    for group in diff.grouped_ops(CONTEXT) {
        let Some(first) = group.first() else { continue };
        let mut hunk = Hunk { old: first.old_range().start + 1, new: first.new_range().start + 1, lines: vec![] };
        for op in &group {
            for change in diff.iter_changes(op) {
                if shown >= MAX_LINES {
                    out.more += 1;
                    continue;
                }
                let sign = match change.tag() {
                    ChangeTag::Equal => ' ',
                    ChangeTag::Delete => '-',
                    ChangeTag::Insert => '+',
                };
                let text = change.value();
                if text == NO_NEWLINE {
                    // The side that has the marker is the one without the line break.
                    hunk.lines.push(format!("\\{sign}"));
                    shown += 1;
                    continue;
                }
                hunk.lines.push(format!("{sign}{}", cut(text.strip_suffix('\n').unwrap_or(text))));
                shown += 1;
            }
        }
        if !hunk.lines.is_empty() {
            out.hunks.push(hunk);
        }
    }
    out
}

/// The diff of a file's bytes, `None` if either side isn't text (binary or another encoding).
/// `old`: `None` for a new file.
pub fn bytes(old: Option<&[u8]>, new: &[u8]) -> Option<Diff> {
    let old = match old {
        Some(b) => text(b)?,
        None => String::new(),
    };
    Some(lines(&old, &text(new)?))
}

/// Bytes as text if they are text: UTF-8 or UTF-16 with a BOM, without NUL characters.
pub fn text(bytes: &[u8]) -> Option<String> {
    super::common::Text::decode(bytes).ok().map(|t| t.text).filter(|t| !t.contains('\0'))
}

fn cut(line: &str) -> String {
    match line.char_indices().nth(MAX_WIDTH) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

impl Diff {
    pub fn to_json(&self) -> Value {
        let hunks: Vec<Value> = self.hunks.iter().map(|h| json!({ "old": h.old, "new": h.new, "lines": h.lines })).collect();
        json!({ "hunks": hunks, "more": self.more })
    }
}

/// A note's `diff` parameter as unified diff lines for a terminal, coloured with `color`.
pub fn plain(diff: &Value, color: bool) -> Vec<String> {
    let paint = |code: &str, s: String| if color { format!("\x1b[{code}m{s}\x1b[0m") } else { s };
    let mut out = Vec::new();
    for hunk in diff["hunks"].as_array().into_iter().flatten() {
        let lines: Vec<&str> = hunk["lines"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let count = |skip: char| lines.iter().filter(|l| !l.starts_with(skip) && !l.starts_with('\\')).count();
        // An empty side starts at the line before, as in `diff -u` (`-0,0` for a new file).
        let start = |key: &str, n: usize| hunk[key].as_u64().unwrap_or(1) - u64::from(n == 0);
        let (olds, news) = (count('+'), count('-'));
        out.push(paint("36", format!("@@ -{},{olds} +{},{news} @@", start("old", olds), start("new", news))));
        for line in lines {
            out.push(match line.chars().next() {
                Some('\\') => paint("2", match line {
                    "\\-" => "\\ No newline at end of file before the change",
                    "\\+" => "\\ No newline at end of file after the change",
                    _ => "\\ No newline at end of file",
                }.to_string()),
                Some('+') => paint("32", line.to_string()),
                Some('-') => paint("31", line.to_string()),
                _ => line.to_string(),
            });
        }
    }
    if let Some(more) = diff["more"].as_u64().filter(|&n| n > 0) {
        out.push(paint("2", format!("… {more} more {} not shown", if more == 1 { "line" } else { "lines" })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_has_context_and_line_numbers() {
        let old: String = (1..=10).map(|i| format!("{i}\n")).collect();
        let new = old.replace("5\n", "five\n");
        let d = lines(&old, &new);
        assert_eq!((d.added, d.removed, d.more), (1, 1, 0));
        assert_eq!(d.hunks, vec![Hunk { old: 2, new: 2, lines: vec![" 2", " 3", " 4", "-5", "+five", " 6", " 7", " 8"].into_iter().map(String::from).collect() }]);
        assert_eq!(plain(&d.to_json(), false)[0], "@@ -2,7 +2,7 @@");
    }

    #[test]
    fn changes_far_apart_are_separate_hunks() {
        let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
        let new = old.replace("\n2\n", "\ntwo\n").replace("\n28\n", "\n").replace("\n30\n", "\n30\n31\n");
        let d = lines(&old, &new);
        assert_eq!(d.hunks.len(), 2);
        assert_eq!((d.hunks[1].old, d.hunks[1].new), (25, 25));
        assert_eq!((d.added, d.removed), (2, 2));
    }

    #[test]
    fn a_new_file_is_all_added() {
        let d = lines("", "a\nb");
        assert_eq!(d.hunks[0].lines, vec!["+a", "+b", "\\+"], "the new text ends without a line break");
        assert_eq!((d.hunks[0].old, d.hunks[0].new, d.added), (1, 1, 2));
        assert_eq!(plain(&d.to_json(), false)[0], "@@ -0,0 +1,2 @@");
    }

    #[test]
    fn a_missing_last_line_break_is_a_marker_not_a_changed_line() {
        let marked = |old: &str, new: &str| {
            let d = lines(old, new);
            (d.added, d.removed, d.hunks.into_iter().flat_map(|h| h.lines).collect::<Vec<_>>())
        };
        // Only the line break at the end changes: no line is added or removed.
        assert_eq!(marked("a\nb", "a\nb\n"), (0, 0, vec![" a".into(), " b".into(), "\\-".into()]));
        assert_eq!(marked("a\nb\n", "a\nb"), (0, 0, vec![" a".into(), " b".into(), "\\+".into()]));
        // A line after the one without a break: that line is the change, the marker says why.
        assert_eq!(marked("x\na", "x\na\nb\n"), (1, 0, vec![" x".into(), " a".into(), "\\-".into(), "+b".into()]));
        // Both without a break: shown with the context, no change.
        assert_eq!(marked("a\nb", "a\nc"), (1, 1, vec![" a".into(), "-b".into(), "+c".into(), "\\ ".into()]));
        let d = lines("1\n2\n3\n4\n5", "1\n2\n3\n4\n5\n");
        assert_eq!((d.hunks[0].old, d.hunks[0].new), (3, 3), "3 lines of context before the end");
        let plain = plain(&d.to_json(), false);
        assert_eq!(plain, vec!["@@ -3,3 +3,3 @@", " 3", " 4", " 5", "\\ No newline at end of file before the change"]);
    }

    #[test]
    fn long_diffs_and_long_lines_are_capped() {
        let new: String = (0..500).map(|i| format!("{}{i}\n", "x".repeat(300))).collect();
        let d = lines("", &new);
        let shown: usize = d.hunks.iter().map(|h| h.lines.len()).sum();
        assert_eq!((shown, d.more, d.added), (MAX_LINES, 500 - MAX_LINES, 500));
        let line = &d.hunks[0].lines[0];
        assert_eq!(line.chars().count(), 1 + MAX_WIDTH + 1);
        assert!(line.ends_with('…'));
        assert_eq!(plain(&d.to_json(), false).last().unwrap(), "… 300 more lines not shown");
        assert!(plain(&d.to_json(), true)[1].starts_with("\x1b[32m+"));
    }

    #[test]
    fn binary_content_has_no_diff() {
        assert!(bytes(Some(b"a\n"), b"\x00\x01\x02").is_none());
        assert!(bytes(Some(b"\xff\xfe\x00"), b"a\n").is_none());
        assert_eq!(bytes(None, b"a\n").unwrap().added, 1);
        assert_eq!(bytes(Some(b"\xEF\xBB\xBFa\r\n"), b"a\nb\n").unwrap().hunks[0].lines, vec![" a", "+b"]);
    }
}
