//! Edits of plain text files: `Furnish/Block` (lines between markers that name the app) and
//! `Furnish/Line` (one line). Each edit is a function from the current text to the desired text;
//! the resource is in its desired state when that function changes nothing.

use anyhow::{Result, bail};
use regex::Regex;

/// The lines of a text without their `\n`; a last line without one counts as a line.
fn split(text: &str) -> Vec<&str> {
    if text.is_empty() { vec![] } else { text.strip_suffix('\n').unwrap_or(text).split('\n').collect() }
}

/// `lines` as a text, or `text` itself if they are its lines (a missing last `\n` stays missing).
fn join(text: &str, lines: &[&str]) -> String {
    if lines == split(text).as_slice() {
        text.to_string()
    } else if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

/// Where a block or line goes the first time.
#[derive(Debug, Clone)]
pub enum Insert {
    Top,
    Bottom,
    /// Before the first line that matches.
    Before(Regex),
    /// After the last line that matches.
    After(Regex),
}

impl Insert {
    fn index(&self, lines: &[&str]) -> usize {
        match self {
            Insert::Top => 0,
            Insert::Bottom => lines.len(),
            Insert::Before(re) => lines.iter().position(|l| re.is_match(l)).unwrap_or(lines.len()),
            Insert::After(re) => lines.iter().rposition(|l| re.is_match(l)).map_or(lines.len(), |i| i + 1),
        }
    }
}

/// A block of lines between `<comment> BEGIN furnish <app>` and `<comment> END furnish <app>`.
pub struct Block<'a> {
    pub content: &'a str,
    pub comment: &'a str,
    pub app: &'a str,
    pub insert: Insert,
}

impl Block<'_> {
    fn markers(&self) -> (String, String) {
        (format!("{} BEGIN furnish {}", self.comment, self.app), format!("{} END furnish {}", self.comment, self.app))
    }

    /// The indexes of the marker lines, if the text has the block.
    fn find(&self, lines: &[&str]) -> Result<Option<(usize, usize)>> {
        let (begin, end) = self.markers();
        let at = |marker: &str| lines.iter().enumerate().filter(|(_, l)| l.trim() == marker).map(|(i, _)| i).collect::<Vec<_>>();
        match (at(&begin).as_slice(), at(&end).as_slice()) {
            ([], []) => Ok(None),
            ([b], [e]) if b < e => Ok(Some((*b, *e))),
            ([_], []) => bail!("the line '{begin}' has no '{end}' after it"),
            ([], [_]) => bail!("the line '{end}' has no '{begin}' before it"),
            _ => bail!("the markers '{begin}' and '{end}' must appear once each, in this order"),
        }
    }

    /// What is between the markers, `None` if there are no markers.
    pub fn current(&self, text: &str) -> Result<Option<String>> {
        let lines = split(text);
        Ok(self.find(&lines)?.map(|(b, e)| lines[b + 1..e].join("\n")))
    }

    /// The text with the block in place (`exist`) or without it.
    pub fn apply(&self, text: &str, exist: bool) -> Result<String> {
        let mut lines = split(text);
        let (begin, end) = self.markers();
        let mut block: Vec<&str> = vec![&begin];
        block.extend(split(self.content));
        block.push(&end);
        match (self.find(&lines)?, exist) {
            (Some((b, e)), true) => {
                lines.splice(b..=e, block);
            }
            (Some((b, e)), false) => {
                lines.drain(b..=e);
            }
            (None, true) => {
                let at = self.insert.index(&lines);
                lines.splice(at..at, block);
            }
            (None, false) => {}
        }
        Ok(join(text, &lines))
    }
}

/// One line: `line` exists, or with `pattern`, a line matching it exists and equals `line`.
/// With `before`/`after`, the line must also come before the first line matching `before` and
/// after the last line matching `after`. With `accept`, a line matching it in place counts as the
/// line, e.g. an ssh `Include` that names other files too.
pub struct Line {
    pub line: String,
    pub pattern: Option<Regex>,
    pub before: Option<Regex>,
    pub after: Option<Regex>,
    pub accept: Option<Regex>,
}

impl Line {
    /// Whether a line is the one this resource manages.
    fn ours(&self, l: &str) -> bool {
        match &self.pattern {
            Some(re) => re.is_match(l),
            None => l == self.line,
        }
    }

    /// The indexes where the line may be: after the last `after` match, before the first
    /// `before` match after that. The managed lines themselves don't count as matches.
    fn range(&self, lines: &[&str]) -> (usize, usize) {
        let anchor = |re: &Regex, l: &str| re.is_match(l) && !self.ours(l);
        let lo = self.after.as_ref().and_then(|re| lines.iter().rposition(|l| anchor(re, l))).map_or(0, |i| i + 1);
        let hi = self.before.as_ref().and_then(|re| lines[lo..].iter().position(|l| anchor(re, l))).map_or(lines.len(), |i| lo + i);
        (lo, hi)
    }

    /// Where a missing line goes: right after the `after` match, else right before the `before`
    /// match, else at the end.
    fn slot(&self, lines: &[&str]) -> usize {
        let (lo, hi) = self.range(lines);
        if self.after.is_some() { lo } else { hi }
    }

    /// The first managed line, and whether it is where `before`/`after` want it; without one,
    /// an accepted line in place.
    pub fn current<'t>(&self, text: &'t str) -> Option<(&'t str, bool)> {
        let lines = split(text);
        self.own(text).or_else(|| self.accepted(&lines).map(|i| (lines[i], true)))
    }

    /// The first managed line, and whether it is in place; an accepted line doesn't count, since
    /// removal leaves it.
    pub fn own<'t>(&self, text: &'t str) -> Option<(&'t str, bool)> {
        let lines = split(text);
        let (lo, hi) = self.range(&lines);
        let i = lines.iter().position(|l| self.ours(l))?;
        Some((lines[i], (lo..hi).contains(&i)))
    }

    /// An accepted line where the line may be.
    fn accepted(&self, lines: &[&str]) -> Option<usize> {
        let re = self.accept.as_ref()?;
        let (lo, hi) = self.range(lines);
        (lo..hi).find(|&i| re.is_match(lines[i]))
    }

    /// The text with the line in place (`exist`), or without any managed line.
    pub fn apply(&self, text: &str, exist: bool) -> String {
        let lines = split(text);
        let ours: Vec<usize> = (0..lines.len()).filter(|&i| self.ours(lines[i])).collect();
        fn others<'t>(lines: &[&'t str], ours: &[usize]) -> Vec<&'t str> {
            lines.iter().enumerate().filter(|(i, _)| !ours.contains(i)).map(|(_, l)| *l).collect()
        }
        if !exist {
            return join(text, &others(&lines, &ours));
        }
        let (lo, hi) = self.range(&lines);
        let placed = |i: &usize| (lo..hi).contains(i);
        if ours.is_empty() && self.accepted(&lines).is_some() {
            return text.to_string();
        }
        let result = match (&self.pattern, ours.first()) {
            // Without a pattern, any equal line in place will do.
            (None, _) if ours.iter().any(placed) => lines,
            // The first match in place becomes the line; other matches go.
            (Some(_), Some(first)) if placed(first) => {
                let mut kept = Vec::with_capacity(lines.len());
                for (i, l) in lines.iter().enumerate() {
                    if i == *first {
                        kept.push(self.line.as_str());
                    } else if !ours.contains(&i) {
                        kept.push(l);
                    }
                }
                kept
            }
            // Missing or out of place: insert it where it belongs.
            _ => {
                let mut kept = others(&lines, &ours);
                let at = self.slot(&kept);
                kept.insert(at, &self.line);
                kept
            }
        };
        join(text, &result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(s: &str) -> Option<Regex> {
        Some(Regex::new(s).unwrap())
    }

    fn block<'a>(content: &'a str, app: &'a str, insert: Insert) -> Block<'a> {
        Block { content, comment: "#", app, insert }
    }

    #[test]
    fn a_block_is_inserted_once_and_then_replaced_in_place() {
        let b = block("Include config.d/acme.conf", "acme/ssh", Insert::Top);
        let text = "Host *\n  User me\n";
        let once = b.apply(text, true).unwrap();
        assert_eq!(once, "# BEGIN furnish acme/ssh\nInclude config.d/acme.conf\n# END furnish acme/ssh\nHost *\n  User me\n");
        assert_eq!(b.apply(&once, true).unwrap(), once, "test and set agree: nothing changes the second time");
        // A comment above the block causes no drift, and the block isn't moved.
        let commented = format!("# my comment\n{once}");
        assert_eq!(b.apply(&commented, true).unwrap(), commented);
        let moved = "Host *\n# BEGIN furnish acme/ssh\nold\n# END furnish acme/ssh\n";
        assert_eq!(b.apply(moved, true).unwrap(), "Host *\n# BEGIN furnish acme/ssh\nInclude config.d/acme.conf\n# END furnish acme/ssh\n");
        assert_eq!(b.current(moved).unwrap().as_deref(), Some("old"));
    }

    #[test]
    fn a_block_is_removed_with_its_markers_and_two_apps_get_two_blocks() {
        let a = block("a = 1", "acme/a", Insert::Bottom);
        let b = block("b = 2", "acme/b", Insert::Bottom);
        let both = b.apply(&a.apply("x\n", true).unwrap(), true).unwrap();
        assert_eq!(both, "x\n# BEGIN furnish acme/a\na = 1\n# END furnish acme/a\n# BEGIN furnish acme/b\nb = 2\n# END furnish acme/b\n");
        assert_eq!(a.apply(&both, false).unwrap(), "x\n# BEGIN furnish acme/b\nb = 2\n# END furnish acme/b\n");
        assert_eq!(a.apply("x", false).unwrap(), "x", "nothing to remove leaves the text as it is");
    }

    #[test]
    fn a_block_goes_before_or_after_a_pattern_and_needs_both_markers() {
        let b = block("k", "c/a", Insert::Before(Regex::new("^Host").unwrap()));
        assert_eq!(b.apply("x\nHost a\nHost b\n", true).unwrap(), "x\n# BEGIN furnish c/a\nk\n# END furnish c/a\nHost a\nHost b\n");
        let b = block("k", "c/a", Insert::After(Regex::new("^Host").unwrap()));
        assert_eq!(b.apply("Host a\nHost b\nx\n", true).unwrap(), "Host a\nHost b\n# BEGIN furnish c/a\nk\n# END furnish c/a\nx\n");
        let b = Block { comment: "//", ..block("k", "c/a", Insert::Bottom) };
        assert_eq!(b.apply("", true).unwrap(), "// BEGIN furnish c/a\nk\n// END furnish c/a\n");
        assert!(b.apply("// BEGIN furnish c/a\nk\n", true).is_err());
    }

    #[test]
    fn the_ssh_include_goes_before_the_first_host_or_match() {
        let line = Line { line: "Include config.d/acme.conf".into(), pattern: None, before: re(r"^\s*(Host|Match)\s"), after: None, accept: None };
        let config = "# mine\nServerAliveInterval 30\nHost github.com\n  User git\nMatch all\n";
        let set = line.apply(config, true);
        assert_eq!(set, "# mine\nServerAliveInterval 30\nInclude config.d/acme.conf\nHost github.com\n  User git\nMatch all\n");
        assert_eq!(line.apply(&set, true), set);
        assert_eq!(line.current(&set), Some(("Include config.d/acme.conf", true)));
        // Moved below a Host line by hand: drift, and set moves it back.
        let moved = "Host github.com\n  User git\nInclude config.d/acme.conf\n";
        assert_eq!(line.current(moved), Some(("Include config.d/acme.conf", false)));
        assert_eq!(line.apply(moved, true), "Include config.d/acme.conf\nHost github.com\n  User git\n");
        // No Host line at all: the end of the file.
        assert_eq!(line.apply("", true), "Include config.d/acme.conf\n");
    }

    #[test]
    fn a_line_without_match_is_never_added_twice_and_removal_takes_every_copy() {
        let line = Line { line: "export A=1".into(), pattern: None, before: None, after: None, accept: None };
        let text = "x\nexport A=1\ny";
        assert_eq!(line.apply(text, true), text, "present: no duplicate, and the missing last newline stays missing");
        assert_eq!(line.apply("x\n", true), "x\nexport A=1\n");
        assert_eq!(line.apply("export A=1\nx\nexport A=1\n", false), "x\n");
    }

    #[test]
    fn a_line_with_match_replaces_the_first_match_and_removes_the_others() {
        let line = Line { line: "PasswordAuthentication no".into(), pattern: re(r"^#?\s*PasswordAuthentication\s"), before: None, after: None, accept: None };
        let text = "a\n#PasswordAuthentication yes\nb\nPasswordAuthentication yes\n";
        assert_eq!(line.current(text), Some(("#PasswordAuthentication yes", true)));
        let set = line.apply(text, true);
        assert_eq!(set, "a\nPasswordAuthentication no\nb\n");
        assert_eq!(line.apply(&set, true), set);
        assert_eq!(line.apply("a\n", true), "a\nPasswordAuthentication no\n");
        assert_eq!(line.apply(text, false), "a\nb\n");
    }

    #[test]
    fn an_accepted_line_counts_and_stays_on_removal() {
        // Home Manager writes every include into one line (#8).
        let line = Line {
            line: "Include config.d/acme.conf".into(),
            pattern: None,
            before: re(r"^\s*(Host|Match)\s"),
            after: None,
            accept: re(r"^\s*Include\s(.*\s)?config\.d/acme\.conf(\s|$)"),
        };
        let combined = "Include ~/.ssh/own.conf config.d/acme.conf\n\nHost *\n  ForwardAgent no\n";
        assert_eq!(line.current(combined), Some(("Include ~/.ssh/own.conf config.d/acme.conf", true)));
        assert_eq!(line.apply(combined, true), combined);
        assert_eq!(line.apply(combined, false), combined, "removal takes only the line itself");
        assert_eq!(line.own(combined), None, "so it counts as removed");
        // After a Host line, ssh doesn't read it: not accepted, the line goes before the Host.
        let late = "Host *\nInclude config.d/acme.conf other\n";
        assert_eq!(line.current(late), None);
        assert_eq!(line.apply(late, true), "Include config.d/acme.conf\nHost *\nInclude config.d/acme.conf other\n");
        assert_eq!(line.apply("Include config.d/acme.conf.bak\n", true), "Include config.d/acme.conf.bak\nInclude config.d/acme.conf\n");
    }

    #[test]
    fn after_is_a_condition_too() {
        let line = Line { line: "editor = vim".into(), pattern: re(r"^editor\s*="), before: re(r"^\["), after: re(r"^\[core\]"), accept: None };
        let text = "editor = nano\n[core]\nx = 1\n[user]\n";
        let set = line.apply(text, true);
        assert_eq!(set, "[core]\neditor = vim\nx = 1\n[user]\n");
        assert_eq!(line.apply(&set, true), set);
    }
}
