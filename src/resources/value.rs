//! `Furnish/Value`: one value in a JSON, JSONC, YAML or TOML file, at a location named by a
//! singular JSONPath (RFC 9535 normalized path, member shorthand, and a keyed filter
//! `[?@.name=='x']`) or a JSON Pointer.
//!
//! The edit is planned on the parsed document as a list of operations at concrete places, then
//! carried out by a format-preserving editor (comments, key order and layout stay). The edited
//! text is parsed again and must equal the planned document, so an editor mistake fails the step
//! instead of changing other parts of the file.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Jsonc,
    Yaml,
    Toml,
}

impl Format {
    /// The format a file name suggests.
    pub fn from_path(path: &str) -> Option<Format> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "json" => Format::Json,
            "jsonc" => Format::Jsonc,
            "yaml" | "yml" => Format::Yaml,
            "toml" => Format::Toml,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Merge {
    /// The value at the location is `value`.
    #[default]
    Replace,
    /// `value`'s keys are set, recursively; other keys stay.
    Deep,
    /// The location is a list that has `value` among its elements.
    Member,
}

/// One part of a location.
#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
    /// The element of a list whose member `.0` equals `.1`.
    Filter(String, Value),
}

/// A concrete place in a document.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Key(String),
    Index(usize),
}

/// Parse a singular JSONPath (`$…`) or a JSON Pointer (`/…`, or empty for the whole document).
pub fn parse_location(location: &str) -> Result<Vec<Seg>> {
    if location.is_empty() {
        return Ok(vec![]);
    }
    if let Some(pointer) = location.strip_prefix('/') {
        return pointer
            .split('/')
            .map(|part| {
                if part.match_indices('~').any(|(i, _)| !matches!(part.as_bytes().get(i + 1), Some(b'0' | b'1'))) {
                    bail!("location '{location}': in a JSON Pointer, ~ is written ~0 and / is written ~1");
                }
                Ok(Seg::Key(part.replace("~1", "/").replace("~0", "~")))
            })
            .collect();
    }
    let bad = |why: &str| anyhow!("location '{location}': {why}. Use a JSONPath like $.a['b.c'][0] or $.list[?@.name=='x'], or a JSON Pointer like /a/b");
    let mut p = Parser { s: location.as_bytes(), i: 0 };
    if !p.eat(b'$') {
        return Err(bad("starts with $ (JSONPath) or / (JSON Pointer)"));
    }
    let mut segs = Vec::new();
    while p.i < p.s.len() {
        if p.eat(b'.') {
            segs.push(Seg::Key(p.name().ok_or_else(|| bad("a name after ."))?));
        } else if p.eat(b'[') {
            p.spaces();
            let seg = if p.peek() == Some(b'?') {
                p.i += 1;
                p.spaces();
                let parens = p.eat(b'(');
                p.spaces();
                if !p.eat(b'@') {
                    return Err(bad("a filter compares a member of @, like [?@.name=='x']"));
                }
                let key = if p.eat(b'.') {
                    p.name()
                } else if p.eat(b'[') {
                    p.spaces();
                    let k = p.string();
                    p.spaces();
                    k.filter(|_| p.eat(b']'))
                } else {
                    None
                }
                .ok_or_else(|| bad("a filter names a member of @, like @.name"))?;
                p.spaces();
                if !(p.eat(b'=') && p.eat(b'=')) {
                    return Err(bad("a filter compares with ==, like [?@.name=='x']"));
                }
                p.spaces();
                let value = p.literal().ok_or_else(|| bad("a filter compares with a string, number, true, false or null"))?;
                p.spaces();
                if parens && !p.eat(b')') {
                    return Err(bad("a ) closes the filter"));
                }
                Seg::Filter(key, value)
            } else if let Some(name) = p.string() {
                Seg::Key(name)
            } else if let Some(index) = p.index() {
                Seg::Index(index)
            } else {
                return Err(bad("[...] holds a quoted name, an index or a filter, and names one place (no *, slices or unions)"));
            };
            p.spaces();
            if !p.eat(b']') {
                return Err(bad("a ] is missing"));
            }
            segs.push(seg);
        } else {
            return Err(bad("unexpected text after a part"));
        }
    }
    Ok(segs)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        let yes = self.peek() == Some(c);
        if yes {
            self.i += 1;
        }
        yes
    }
    fn spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    /// A member name in shorthand: letters, digits, `_` and anything outside ASCII, not starting with a digit.
    fn name(&mut self) -> Option<String> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 || (c == b'-' && self.i > start) {
                self.i += 1;
            } else {
                break;
            }
        }
        let name = std::str::from_utf8(&self.s[start..self.i]).ok()?;
        (!name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit())).then(|| name.to_string())
    }
    fn index(&mut self) -> Option<usize> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
    /// A string in single or double quotes, with JSON's escapes.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek().filter(|c| *c == b'\'' || *c == b'"')?;
        let start = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            let rest = std::str::from_utf8(&self.s[self.i..]).ok()?;
            let c = rest.chars().next()?;
            self.i += c.len_utf8();
            match c {
                '\\' => {
                    let e = self.peek()? as char;
                    self.i += 1;
                    out.push(match e {
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
                        }
                        other => other,
                    });
                }
                c if c as u32 == quote as u32 => return Some(out),
                c => out.push(c),
            }
            if self.i >= self.s.len() {
                self.i = start;
                return None;
            }
        }
    }
    fn literal(&mut self) -> Option<Value> {
        if let Some(s) = self.string() {
            return Some(Value::String(s));
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'.')) {
            self.i += 1;
        }
        serde_json::from_slice(&self.s[start..self.i]).ok()
    }
}

/// The location's segments must name one place, and a keyed item's value must carry its key
/// (or the next run wouldn't find it and would add it again).
pub fn check(location: &str, value: Option<&Value>, merge: Merge) -> Result<Vec<Seg>> {
    let segs = parse_location(location)?;
    if let (Some(Seg::Filter(key, expected)), Some(value), Merge::Replace | Merge::Deep) = (segs.last(), value, merge)
        && value.get(key) != Some(expected)
    {
        bail!("location '{location}' selects the element whose {key} is {expected}, so value must have {key}: {expected}");
    }
    Ok(segs)
}

/// An edit at a concrete place.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Put the value there (adding the last key, or replacing what is there).
    Set(Vec<Step>, Value),
    Remove(Vec<Step>),
    /// Add the value at the end of the list there.
    Append(Vec<Step>, Value),
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

fn get<'v>(doc: &'v Value, path: &[Step]) -> Option<&'v Value> {
    path.iter().try_fold(doc, |v, s| match s {
        Step::Key(k) => v.get(k),
        Step::Index(i) => v.get(i),
    })
}

fn get_mut<'v>(doc: &'v mut Value, path: &[Step]) -> Option<&'v mut Value> {
    path.iter().try_fold(doc, |v, s| match s {
        Step::Key(k) => v.get_mut(k),
        Step::Index(i) => v.get_mut(i),
    })
}

/// Where the location is in `doc`: `Ok(path)` if it exists, else `Err((path, n))` with the path
/// of the deepest existing place and the number of segments it covers.
type Located = std::result::Result<Vec<Step>, (Vec<Step>, usize)>;

fn locate(doc: &Value, segs: &[Seg]) -> Result<Located> {
    let mut path = Vec::new();
    let mut node = doc;
    for (n, seg) in segs.iter().enumerate() {
        let next = match (seg, node) {
            (_, Value::Null) => None,
            (Seg::Key(k), Value::Object(m)) => m.get(k).map(|v| (Step::Key(k.clone()), v)),
            (Seg::Key(k), Value::Array(a)) if k.parse::<usize>().is_ok() => {
                let i = k.parse::<usize>().unwrap_or_default();
                a.get(i).map(|v| (Step::Index(i), v))
            }
            (Seg::Index(i), Value::Array(a)) => a.get(*i).map(|v| (Step::Index(*i), v)),
            (Seg::Filter(key, expected), Value::Array(a)) => {
                let found: Vec<usize> = (0..a.len()).filter(|&i| a[i].get(key) == Some(expected)).collect();
                if found.len() > 1 {
                    bail!("{} elements have {key} {expected}; the location must name one", found.len());
                }
                found.first().map(|&i| (Step::Index(i), &a[i]))
            }
            (Seg::Key(k), other) => bail!("can't look up '{k}' in {}", kind(other)),
            (_, other) => bail!("{} is not a list", describe(&path, other)),
        };
        match next {
            Some((step, v)) => {
                path.push(step);
                node = v;
            }
            None => return Ok(Err((path, n))),
        }
    }
    Ok(Ok(path))
}

fn describe(path: &[Step], v: &Value) -> String {
    let place = if path.is_empty() { "the document".to_string() } else { format!("{} at {}", kind(v), pointer(path)) };
    if path.is_empty() { format!("{place} ({})", kind(v)) } else { place }
}

/// A path as a JSON Pointer, for messages.
pub fn pointer(path: &[Step]) -> String {
    path.iter()
        .map(|s| match s {
            Step::Key(k) => format!("/{}", k.replace('~', "~0").replace('/', "~1")),
            Step::Index(i) => format!("/{i}"),
        })
        .collect()
}

/// The structure that the segments describe, with `leaf` at the end; for missing places.
fn build(segs: &[Seg], leaf: Value) -> Result<Value> {
    let Some((first, rest)) = segs.split_first() else { return Ok(leaf) };
    Ok(match first {
        Seg::Key(k) => Value::Object(Map::from_iter([(k.clone(), build(rest, leaf)?)])),
        Seg::Filter(key, expected) => Value::Array(vec![element(key, expected, rest, leaf)?]),
        Seg::Index(i) => bail!("there is no element {i} to set"),
    })
}

/// A new list element for a keyed filter: `{key: expected}` with the rest of the location set in it.
fn element(key: &str, expected: &Value, rest: &[Seg], leaf: Value) -> Result<Value> {
    if rest.is_empty() {
        return Ok(leaf);
    }
    let mut item = build(rest, leaf)?;
    match item.as_object_mut() {
        Some(m) => {
            m.insert(key.to_string(), expected.clone());
        }
        None => bail!("a list element selected by {key} must be an object"),
    }
    Ok(item)
}

/// The operation that puts `leaf` at the location, creating what is missing on the way.
fn plan_set(doc: &Value, segs: &[Seg], leaf: Value) -> Result<Op> {
    Ok(match locate(doc, segs)? {
        Ok(path) => Op::Set(path, leaf),
        Err((found, n)) => match (&segs[n], get(doc, &found)) {
            (_, Some(Value::Null)) => Op::Set(found, build(&segs[n..], leaf)?),
            (Seg::Key(k), Some(Value::Object(_))) => {
                let mut path = found;
                path.push(Step::Key(k.clone()));
                Op::Set(path, build(&segs[n + 1..], leaf)?)
            }
            (Seg::Filter(key, expected), _) => Op::Append(found, element(key, expected, &segs[n + 1..], leaf)?),
            (Seg::Key(k), _) => bail!("there is no element {k} to set"),
            (Seg::Index(i), _) => bail!("there is no element {i} to set"),
        },
    })
}

pub fn apply(doc: &mut Value, op: &Op) -> Result<()> {
    let missing = || anyhow!("internal error: no place for the edit");
    match op {
        Op::Set(path, v) => match path.split_last() {
            None => *doc = v.clone(),
            Some((last, parent)) => match (get_mut(doc, parent).ok_or_else(missing)?, last) {
                (Value::Object(m), Step::Key(k)) => {
                    m.insert(k.clone(), v.clone());
                }
                (Value::Array(a), Step::Index(i)) if *i < a.len() => a[*i] = v.clone(),
                _ => return Err(missing()),
            },
        },
        Op::Remove(path) => {
            let (last, parent) = path.split_last().ok_or_else(|| anyhow!("can't remove the whole document"))?;
            match (get_mut(doc, parent).ok_or_else(missing)?, last) {
                (Value::Object(m), Step::Key(k)) => {
                    m.remove(k);
                }
                (Value::Array(a), Step::Index(i)) if *i < a.len() => {
                    a.remove(*i);
                }
                _ => return Err(missing()),
            }
        }
        Op::Append(path, v) => get_mut(doc, path).and_then(Value::as_array_mut).ok_or_else(missing)?.push(v.clone()),
    }
    Ok(())
}

/// The leaves of an object, by their keys: what a deep merge sets.
fn leaves(value: &Value) -> Vec<(Vec<Seg>, Value)> {
    match value {
        Value::Object(m) if !m.is_empty() => m
            .iter()
            .flat_map(|(k, v)| {
                leaves(v).into_iter().map(move |(mut rest, leaf)| {
                    rest.insert(0, Seg::Key(k.clone()));
                    (rest, leaf)
                })
            })
            .collect(),
        other => vec![(vec![], other.clone())],
    }
}

/// The edits that bring `doc` to the desired state, and the document after them. No edits: the
/// resource is in its desired state. Only the location (and what `value` names in it) counts.
pub fn plan(doc: &Value, segs: &[Seg], value: Option<&Value>, merge: Merge, exist: bool) -> Result<(Vec<Op>, Value)> {
    let mut work = doc.clone();
    let mut ops = Vec::new();
    let mut push = |work: &mut Value, op: Op| -> Result<()> {
        apply(work, &op)?;
        ops.push(op);
        Ok(())
    };
    let need = || value.ok_or_else(|| anyhow!("value is required"));
    match (merge, exist) {
        (Merge::Replace, true) => {
            let value = need()?;
            if current(doc, segs)? != Some(value) {
                let op = plan_set(&work, segs, value.clone())?;
                push(&mut work, op)?;
            }
        }
        (Merge::Deep, true) => {
            for (rest, leaf) in leaves(need()?) {
                let full: Vec<Seg> = segs.iter().cloned().chain(rest).collect();
                if current(&work, &full)? != Some(&leaf) {
                    let op = plan_set(&work, &full, leaf)?;
                    push(&mut work, op)?;
                }
            }
        }
        (Merge::Member, true) => {
            let value = need()?;
            match locate(doc, segs)? {
                Ok(path) => match get(doc, &path) {
                    Some(Value::Array(a)) if a.contains(value) => {}
                    Some(Value::Array(_)) => push(&mut work, Op::Append(path, value.clone()))?,
                    Some(Value::Null) => push(&mut work, Op::Set(path, Value::Array(vec![value.clone()])))?,
                    Some(other) => bail!("{} is not a list", describe(&path, other)),
                    None => unreachable!(),
                },
                Err(_) => {
                    let op = plan_set(&work, segs, Value::Array(vec![value.clone()]))?;
                    push(&mut work, op)?;
                }
            }
        }
        (Merge::Replace, false) => {
            if let Ok(path) = locate(doc, segs)? {
                push(&mut work, Op::Remove(path))?;
            }
        }
        // A filtered element is the app's as a whole, also with a deep merge: removing only the keys of value
        // would leave an element without the key the filter finds it by.
        (Merge::Deep, false) if matches!(segs.last(), Some(Seg::Filter(..))) => {
            if let Ok(path) = locate(doc, segs)? {
                push(&mut work, Op::Remove(path))?;
            }
        }
        (Merge::Deep, false) => {
            for (rest, _) in leaves(need()?) {
                let full: Vec<Seg> = segs.iter().cloned().chain(rest).collect();
                if let Ok(path) = locate(&work, &full)? {
                    push(&mut work, Op::Remove(path))?;
                }
            }
        }
        (Merge::Member, false) => {
            let value = need()?;
            if let Ok(path) = locate(doc, segs)?
                && let Some(Value::Array(a)) = get(doc, &path)
            {
                for i in (0..a.len()).rev().filter(|&i| a[i] == *value) {
                    let mut at = path.clone();
                    at.push(Step::Index(i));
                    push(&mut work, Op::Remove(at))?;
                }
            }
        }
    }
    Ok((ops, work))
}

/// The value at the location, if it exists.
pub fn current<'v>(doc: &'v Value, segs: &[Seg]) -> Result<Option<&'v Value>> {
    Ok(locate(doc, segs)?.ok().and_then(|path| get(doc, &path)))
}

/// `actual` limited to the keys `desired` has (recursively), for showing a deep merge's state.
pub fn project(actual: &Value, desired: &Value) -> Value {
    match (actual, desired) {
        (Value::Object(a), Value::Object(d)) if !d.is_empty() => {
            Value::Object(d.iter().filter_map(|(k, dv)| a.get(k).map(|av| (k.clone(), project(av, dv)))).collect())
        }
        _ => actual.clone(),
    }
}

/// Parse a file's text; an empty text is an empty object.
pub fn parse(format: Format, text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value = match format {
        Format::Json => serde_json::from_str(text).context("not valid JSON (for JSON with comments, use format: jsonc)")?,
        Format::Jsonc => jsonc_parser::parse_to_serde_value::<Option<Value>>(text, &Default::default())
            .map_err(|e| anyhow!("not valid JSONC: {e}"))?
            .unwrap_or_else(|| Value::Object(Map::new())),
        Format::Yaml => {
            let v: Value = serde_yaml::from_str(text).context("not valid YAML (or more than one document)")?;
            if v.is_null() { Value::Object(Map::new()) } else { v }
        }
        Format::Toml => toml_to_json(text.parse::<toml_edit::DocumentMut>().context("not valid TOML")?.as_item()),
    };
    Ok(value)
}

/// Carry out the edits on the file's text, keeping everything else as it is. Fails if the
/// result wouldn't parse as `expected`.
pub fn edit(format: Format, text: &str, ops: &[Op], expected: &Value) -> Result<String> {
    let new = match format {
        Format::Json | Format::Jsonc => json_edit(text, ops)?,
        // A whole new document can't keep anything of the old one.
        Format::Yaml if ops.iter().any(|op| matches!(op, Op::Set(p, _) if p.is_empty())) => serde_yaml::to_string(expected)?,
        Format::Yaml => yaml_edit(text, ops)?,
        Format::Toml => toml_edit_text(text, ops)?,
    };
    match parse(format, &new) {
        Ok(v) if v == *expected => Ok(new),
        _ => bail!("furnish can't make this change without changing other parts of the file, so it left the file as it is"),
    }
}

fn json_input(v: &Value) -> jsonc_parser::cst::CstInputValue {
    use jsonc_parser::cst::CstInputValue as I;
    match v {
        Value::Null => I::Null,
        Value::Bool(b) => I::Bool(*b),
        Value::Number(n) => I::Number(n.to_string()),
        Value::String(s) => I::String(s.clone()),
        Value::Array(a) => I::Array(a.iter().map(json_input).collect()),
        Value::Object(m) => I::Object(m.iter().map(|(k, v)| (k.clone(), json_input(v))).collect()),
    }
}

fn json_edit(text: &str, ops: &[Op]) -> Result<String> {
    use jsonc_parser::cst::{CstNode, CstRootNode};
    let root = CstRootNode::parse(text, &Default::default()).map_err(|e| anyhow!("{e}"))?;
    if root.value().is_none() {
        root.object_value_or_set();
    }
    let missing = || anyhow!("internal error: no place for the edit");
    let nav = |path: &[Step]| -> Result<CstNode> {
        path.iter().try_fold(root.value().ok_or_else(missing)?, |node, step| match step {
            Step::Key(k) => node.as_object().and_then(|o| o.get(k)).and_then(|p| p.value()).ok_or_else(missing),
            Step::Index(i) => node.as_array().and_then(|a| a.elements().get(*i).cloned()).ok_or_else(missing),
        })
    };
    for op in ops {
        match op {
            Op::Set(path, v) => match path.split_last() {
                None => root.set_value(json_input(v)),
                Some((Step::Key(k), parent)) => {
                    let object = nav(parent)?.as_object().ok_or_else(missing)?;
                    match object.get(k) {
                        Some(prop) => prop.set_value(json_input(v)),
                        None => {
                            object.append(k, json_input(v));
                        }
                    }
                }
                Some((Step::Index(i), parent)) => {
                    let array = nav(parent)?.as_array().ok_or_else(missing)?;
                    array.insert(*i, json_input(v));
                    array.elements().get(i + 1).cloned().ok_or_else(missing)?.remove();
                }
            },
            Op::Remove(path) => match path.split_last().ok_or_else(missing)? {
                (Step::Key(k), parent) => nav(parent)?.as_object().and_then(|o| o.get(k)).ok_or_else(missing)?.remove(),
                (Step::Index(_), _) => nav(path)?.remove(),
            },
            Op::Append(path, v) => {
                nav(path)?.as_array().ok_or_else(missing)?.append(json_input(v));
            }
        }
    }
    Ok(root.to_string())
}

fn yaml_value(v: &Value) -> yaml_edit::YamlValue {
    use yaml_edit::{ScalarValue, YamlValue as Y};
    match v {
        Value::Null => Y::Scalar(ScalarValue::null()),
        Value::Bool(b) => Y::Scalar(ScalarValue::from(*b)),
        Value::Number(n) if n.is_f64() => Y::Scalar(ScalarValue::from(n.as_f64().unwrap_or_default())),
        Value::Number(n) => Y::Scalar(ScalarValue::plain(n.to_string())),
        Value::String(s) => Y::Scalar(ScalarValue::string(s.clone())),
        Value::Array(a) => Y::Sequence(a.iter().map(yaml_value).collect()),
        Value::Object(m) => Y::Mapping(m.iter().map(|(k, v)| (k.clone(), yaml_value(v))).collect()),
    }
}

fn yaml_edit(text: &str, ops: &[Op]) -> Result<String> {
    use std::str::FromStr;
    use yaml_edit::{Document, YamlNode};
    let doc = if text.trim().is_empty() { Document::new_mapping() } else { Document::from_str(text).map_err(|e| anyhow!("{e}"))? };
    let missing = || anyhow!("internal error: no place for the edit");
    let root = || -> Result<YamlNode> {
        doc.as_mapping().map(YamlNode::Mapping).or_else(|| doc.as_sequence().map(YamlNode::Sequence)).ok_or_else(missing)
    };
    let nav = |path: &[Step]| -> Result<YamlNode> {
        path.iter().try_fold(root()?, |node, step| match step {
            Step::Key(k) => node.as_mapping().and_then(|m| m.get(k.as_str())).ok_or_else(missing),
            Step::Index(i) => node.as_sequence().and_then(|s| s.get(*i)).ok_or_else(missing),
        })
    };
    for op in ops {
        match op {
            Op::Set(path, v) => match path.split_last() {
                None => unreachable!("handled above"),
                Some((Step::Key(k), parent)) => nav(parent)?.as_mapping().ok_or_else(missing)?.set(k.as_str(), yaml_value(v)),
                Some((Step::Index(i), parent)) => {
                    if !nav(parent)?.as_sequence().ok_or_else(missing)?.set(*i, yaml_value(v)) {
                        return Err(missing());
                    }
                }
            },
            Op::Remove(path) => match path.split_last().ok_or_else(missing)? {
                (Step::Key(k), parent) => {
                    nav(parent)?.as_mapping().ok_or_else(missing)?.remove(k.as_str()).ok_or_else(missing)?;
                }
                (Step::Index(i), parent) => {
                    nav(parent)?.as_sequence().ok_or_else(missing)?.remove(*i).ok_or_else(missing)?;
                }
            },
            Op::Append(path, v) => nav(path)?.as_sequence().ok_or_else(missing)?.push(yaml_value(v)),
        }
    }
    Ok(doc.to_string())
}

fn toml_to_json(item: &toml_edit::Item) -> Value {
    use toml_edit::Item;
    match item {
        Item::None => Value::Null,
        Item::Value(v) => toml_value_to_json(v),
        Item::Table(t) => Value::Object(t.iter().map(|(k, v)| (k.to_string(), toml_to_json(v))).collect()),
        Item::ArrayOfTables(a) => Value::Array(a.iter().map(|t| Value::Object(t.iter().map(|(k, v)| (k.to_string(), toml_to_json(v))).collect())).collect()),
    }
}

fn toml_value_to_json(v: &toml_edit::Value) -> Value {
    use toml_edit::Value as T;
    match v {
        T::String(s) => Value::String(s.value().clone()),
        T::Integer(i) => Value::from(*i.value()),
        T::Float(f) => serde_json::Number::from_f64(*f.value()).map_or(Value::Null, Value::Number),
        T::Boolean(b) => Value::Bool(*b.value()),
        T::Datetime(d) => Value::String(d.value().to_string()),
        T::Array(a) => Value::Array(a.iter().map(toml_value_to_json).collect()),
        T::InlineTable(t) => Value::Object(t.iter().map(|(k, v)| (k.to_string(), toml_value_to_json(v))).collect()),
    }
}

fn toml_value(v: &Value) -> Result<toml_edit::Value> {
    Ok(match v {
        Value::Null => bail!("TOML has no null"),
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.into(),
            (None, Some(f)) if n.is_f64() => f.into(),
            _ => bail!("{n} is too large for TOML"),
        },
        Value::String(s) => s.as_str().into(),
        Value::Array(a) => toml_edit::Value::Array(a.iter().map(toml_value).collect::<Result<_>>()?),
        Value::Object(m) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, v) in m {
                t.insert(k, toml_value(v)?);
            }
            toml_edit::Value::InlineTable(t)
        }
    })
}

/// Objects as tables and lists of objects as arrays of tables, as people write TOML by hand.
fn toml_item(v: &Value) -> Result<toml_edit::Item> {
    use toml_edit::Item;
    Ok(match v {
        Value::Object(m) => {
            let mut t = toml_edit::Table::new();
            for (k, v) in m {
                t.insert(k, toml_item(v)?);
            }
            Item::Table(t)
        }
        Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_object) => {
            let mut tables = toml_edit::ArrayOfTables::new();
            for v in a {
                tables.push(toml_table(v)?);
            }
            Item::ArrayOfTables(tables)
        }
        other => Item::Value(toml_value(other)?),
    })
}

fn toml_table(v: &Value) -> Result<toml_edit::Table> {
    match toml_item(v)? {
        toml_edit::Item::Table(t) => Ok(t),
        _ => bail!("an element of an array of tables must be an object"),
    }
}

fn toml_edit_text(text: &str, ops: &[Op]) -> Result<String> {
    use toml_edit::{Item, Value as T};
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    let missing = || anyhow!("internal error: no place for the edit");
    fn nav<'a>(item: &'a mut Item, path: &[Step]) -> Option<&'a mut Item> {
        path.iter().try_fold(item, |item, step| match step {
            Step::Key(k) if item.get(k.as_str()).is_some() => item.get_mut(k.as_str()),
            Step::Key(_) => None,
            Step::Index(i) => item.get_mut(*i),
        })
    }
    for op in ops {
        let root = doc.as_item_mut();
        match op {
            Op::Set(path, v) => match path.split_last() {
                None => *root = toml_item(v)?,
                Some((Step::Key(k), parent)) => match nav(root, parent).ok_or_else(missing)? {
                    Item::Table(t) => {
                        let mut new = toml_item(v)?;
                        // A value that replaces a value keeps its comment.
                        if let (Some(Item::Value(old)), Item::Value(n)) = (t.get(k), &mut new) {
                            *n.decor_mut() = old.decor().clone();
                        }
                        t.insert(k, new);
                    }
                    Item::Value(T::InlineTable(t)) => {
                        t.insert(k, toml_value(v)?);
                    }
                    _ => return Err(missing()),
                },
                Some((Step::Index(i), parent)) => match nav(root, parent).ok_or_else(missing)? {
                    Item::ArrayOfTables(a) => *a.get_mut(*i).ok_or_else(missing)? = toml_table(v)?,
                    Item::Value(T::Array(a)) => {
                        a.replace(*i, toml_value(v)?);
                    }
                    _ => return Err(missing()),
                },
            },
            Op::Remove(path) => match path.split_last().ok_or_else(missing)? {
                (Step::Key(k), parent) => {
                    nav(root, parent).and_then(Item::as_table_like_mut).ok_or_else(missing)?.remove(k).ok_or_else(missing)?;
                }
                (Step::Index(i), parent) => match nav(root, parent).ok_or_else(missing)? {
                    Item::ArrayOfTables(a) => {
                        a.remove(*i);
                    }
                    Item::Value(T::Array(a)) => {
                        a.remove(*i);
                    }
                    _ => return Err(missing()),
                },
            },
            Op::Append(path, v) => match nav(root, path).ok_or_else(missing)? {
                Item::ArrayOfTables(a) => a.push(toml_table(v)?),
                Item::Value(T::Array(a)) => a.push(toml_value(v)?),
                _ => return Err(missing()),
            },
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The file's text after setting `value` at `location`, and whether that changed anything.
    fn change(format: Format, text: &str, location: &str, value: Value, merge: Merge, exist: bool) -> (String, bool) {
        let segs = check(location, Some(&value), merge).unwrap();
        let doc = parse(format, text).unwrap();
        let (ops, expected) = plan(&doc, &segs, Some(&value), merge, exist).unwrap();
        if ops.is_empty() {
            return (text.to_string(), false);
        }
        let new = edit(format, text, &ops, &expected).unwrap();
        // test and set agree: the result needs no further change.
        let (again, _) = plan(&parse(format, &new).unwrap(), &segs, Some(&value), merge, exist).unwrap();
        assert!(again.is_empty(), "still differs after the change: {again:?}\n{new}");
        (new, true)
    }

    #[test]
    fn locations_parse_as_jsonpath_or_pointer() {
        use Seg::*;
        assert_eq!(parse_location("$.enabledPlugins['acme@marketplace']").unwrap(), vec![Key("enabledPlugins".into()), Key("acme@marketplace".into())]);
        assert_eq!(parse_location("$['hosts'][\"git.example.org\"].git_protocol").unwrap(), vec![Key("hosts".into()), Key("git.example.org".into()), Key("git_protocol".into())]);
        assert_eq!(parse_location("$.contexts[?@.name=='org-prod']").unwrap(), vec![Key("contexts".into()), Filter("name".into(), json!("org-prod"))]);
        assert_eq!(parse_location("$.a[?(@['id'] == 3)][0]").unwrap(), vec![Key("a".into()), Filter("id".into(), json!(3)), Index(0)]);
        assert_eq!(parse_location("/a~1b/~0c/0").unwrap(), vec![Key("a/b".into()), Key("~c".into()), Key("0".into())]);
        assert_eq!(parse_location("$").unwrap(), vec![]);
        for bad in ["a.b", "$.a[*]", "$..a", "$.a[0:2]", "$.a['b','c']", "$.a[?@.x>1]", "$.a['b'"] {
            assert!(parse_location(bad).is_err(), "{bad}");
        }
        assert!(check("$.contexts[?@.name=='org-prod']", Some(&json!({"name": "other"})), Merge::Replace).is_err());
    }

    #[test]
    fn the_claude_settings_key_is_set_and_the_rest_stays() {
        let settings = "{\n  \"model\": \"opus\",\n  \"enabledPlugins\": {\n    \"other@x\": true\n  }\n}\n";
        let (new, changed) = change(Format::Json, settings, "$.enabledPlugins['acme@marketplace']", json!(true), Merge::Replace, true);
        assert!(changed);
        assert_eq!(new, "{\n  \"model\": \"opus\",\n  \"enabledPlugins\": {\n    \"other@x\": true,\n    \"acme@marketplace\": true\n  }\n}\n");
        // Missing objects on the way are created, also in an empty file.
        let (new, _) = change(Format::Json, "", "$.enabledPlugins['acme@marketplace']", json!(true), Merge::Replace, true);
        assert_eq!(parse(Format::Json, &new).unwrap(), json!({"enabledPlugins": {"acme@marketplace": true}}));
        // A value changed by hand is drift; removal takes only that key.
        let changed_by_hand = new.replace("true", "false");
        assert!(change(Format::Json, &changed_by_hand, "$.enabledPlugins['acme@marketplace']", json!(true), Merge::Replace, true).1);
        let (removed, _) = change(Format::Json, settings, "$.model", json!(null), Merge::Replace, false);
        assert_eq!(parse(Format::Json, &removed).unwrap(), json!({"enabledPlugins": {"other@x": true}}));
    }

    #[test]
    fn a_deep_merged_list_element_keeps_extra_keys_and_is_removed_whole() {
        // kubectl writes defaults into the user (env, provideClusterInfo), and people set a context's namespace.
        let kube = "contexts:\n- context:\n    cluster: org-prod\n    namespace: team\n    user: org\n  name: org-prod\n\
                    - context: {cluster: mine, user: me}\n  name: mine\n\
                    users:\n- name: org\n  user:\n    exec:\n      command: kubelogin\n      env: null\n      provideClusterInfo: false\n";
        let context = json!({"name": "org-prod", "context": {"cluster": "org-prod", "user": "org"}});
        let user = json!({"name": "org", "user": {"exec": {"command": "kubelogin"}}});
        let at_context = "$.contexts[?@.name=='org-prod']";
        let at_user = "$.users[?@.name=='org']";
        assert!(!change(Format::Yaml, kube, at_context, context.clone(), Merge::Deep, true).1, "the namespace is the user's");
        assert!(!change(Format::Yaml, kube, at_user, user.clone(), Merge::Deep, true).1, "kubectl's defaults are no drift");
        // The filter names the element as the app's: removal takes all of it, not only the keys of value, which
        // would leave an element without its name that the next install doesn't find and adds again.
        let (without_context, _) = change(Format::Yaml, kube, at_context, context, Merge::Deep, false);
        let (removed, _) = change(Format::Yaml, &without_context, at_user, user, Merge::Deep, false);
        assert_eq!(
            parse(Format::Yaml, &removed).unwrap(),
            json!({"contexts": [{"context": {"cluster": "mine", "user": "me"}, "name": "mine"}], "users": []})
        );
    }

    #[test]
    fn jsonc_keeps_its_comments() {
        let text = "// editor settings\n{\n  \"a\": 1, // the answer\n  /* more */\n  \"b\": [1, 2],\n}\n";
        let (new, _) = change(Format::Jsonc, text, "$.c", json!("x"), Merge::Replace, true);
        assert!(new.contains("// editor settings") && new.contains("// the answer") && new.contains("/* more */"), "{new}");
        assert_eq!(parse(Format::Jsonc, &new).unwrap(), json!({"a": 1, "b": [1, 2], "c": "x"}));
        let (new, _) = change(Format::Jsonc, text, "/b", json!(3), Merge::Member, true);
        assert_eq!(parse(Format::Jsonc, &new).unwrap()["b"], json!([1, 2, 3]));
        assert!(!change(Format::Jsonc, &new, "/b", json!(3), Merge::Member, true).1, "no duplicate member");
        let (new, _) = change(Format::Jsonc, &new, "/b", json!(1), Merge::Member, false);
        assert_eq!(parse(Format::Jsonc, &new).unwrap()["b"], json!([2, 3]));
    }

    #[test]
    fn the_kubeconfig_merge_keeps_current_context_and_other_entries() {
        let kubeconfig = "\
apiVersion: v1
kind: Config
# my clusters
clusters:
- name: home
  cluster:
    server: https://home.example:6443
contexts:
- name: home
  context:
    cluster: home
    user: me
current-context: home
users:
- name: me
  user:
    token: secret
";
        let cluster = json!({"name": "org-prod", "cluster": {"server": "https://k8s.example.org"}});
        let (new, _) = change(Format::Yaml, kubeconfig, "$.clusters[?@.name=='org-prod']", cluster.clone(), Merge::Replace, true);
        let context = json!({"name": "org-prod", "context": {"cluster": "org-prod", "user": "org-prod"}});
        let (new, _) = change(Format::Yaml, &new, "$.contexts[?@.name=='org-prod']", context.clone(), Merge::Replace, true);
        let user = json!({"name": "org-prod", "user": {"exec": {"command": "kubelogin"}}});
        let (new, _) = change(Format::Yaml, &new, "$.users[?@.name=='org-prod']", user.clone(), Merge::Replace, true);
        let doc = parse(Format::Yaml, &new).unwrap();
        assert_eq!(doc["current-context"], "home");
        assert_eq!(doc["clusters"].as_array().unwrap().len(), 2);
        assert_eq!(doc["clusters"][1], cluster);
        assert_eq!(doc["contexts"][1], context);
        assert_eq!(doc["users"][0]["user"]["token"], "secret");
        assert!(new.contains("# my clusters"), "{new}");
        // A changed element is replaced in place; removal takes only that element.
        let edited = new.replace("https://k8s.example.org", "https://old.example.org");
        let (fixed, changed) = change(Format::Yaml, &edited, "$.clusters[?@.name=='org-prod']", cluster.clone(), Merge::Replace, true);
        assert!(changed);
        assert_eq!(parse(Format::Yaml, &fixed).unwrap(), doc);
        let (removed, _) = change(Format::Yaml, &new, "$.clusters[?@.name=='org-prod']", cluster, Merge::Replace, false);
        assert_eq!(parse(Format::Yaml, &removed).unwrap()["clusters"], json!([{"name": "home", "cluster": {"server": "https://home.example:6443"}}]));
    }

    #[test]
    fn a_glab_host_is_added_and_the_others_stay() {
        let config = "\
# What protocol to use when performing git operations.
git_protocol: https
hosts:
  gitlab.com:
    api_protocol: https
    token: abc
";
        let (new, _) = change(Format::Yaml, config, "$.hosts['git.example.org'].git_protocol", json!("ssh"), Merge::Replace, true);
        let doc = parse(Format::Yaml, &new).unwrap();
        assert_eq!(doc["hosts"]["git.example.org"]["git_protocol"], "ssh");
        assert_eq!(doc["hosts"]["gitlab.com"]["token"], "abc");
        assert_eq!(doc["git_protocol"], "https");
        assert!(new.starts_with("# What protocol"), "{new}");
    }

    #[test]
    fn deep_merge_sets_only_the_named_keys() {
        let text = "[server]\nurl = \"https://old\" # the server\nport = 1\n\n[other]\nx = 1\n";
        let (new, _) = change(Format::Toml, text, "$.server", json!({"url": "https://new", "tls": {"verify": true}}), Merge::Deep, true);
        let doc = parse(Format::Toml, &new).unwrap();
        assert_eq!(doc, json!({"server": {"url": "https://new", "port": 1, "tls": {"verify": true}}, "other": {"x": 1}}));
        assert!(new.contains("# the server"), "{new}");
        let (removed, _) = change(Format::Toml, &new, "$.server", json!({"url": "https://new", "tls": {"verify": true}}), Merge::Deep, false);
        assert_eq!(parse(Format::Toml, &removed).unwrap(), json!({"server": {"port": 1, "tls": {}}, "other": {"x": 1}}));
    }

    #[test]
    fn a_member_is_added_to_an_allow_list_once() {
        let text = "{\"permissions\": {\"allow\": [\"Bash(ls)\"]}}";
        let (new, _) = change(Format::Json, text, "$.permissions.allow", json!("Bash(git status)"), Merge::Member, true);
        assert_eq!(parse(Format::Json, &new).unwrap()["permissions"]["allow"], json!(["Bash(ls)", "Bash(git status)"]));
        let (new, _) = change(Format::Toml, "", "$.permissions.allow", json!("x"), Merge::Member, true);
        assert_eq!(parse(Format::Toml, &new).unwrap(), json!({"permissions": {"allow": ["x"]}}));
    }

    #[test]
    fn a_file_that_doesnt_parse_is_an_error() {
        assert!(parse(Format::Json, "{\"a\": 1,}").is_err());
        assert!(parse(Format::Json, "// c\n{}").is_err());
        assert!(parse(Format::Yaml, "a: [1").is_err());
        assert!(parse(Format::Toml, "a = ").is_err());
        let doc = json!({"a": "text"});
        assert!(plan(&doc, &parse_location("$.a.b").unwrap(), Some(&json!(1)), Merge::Replace, true).is_err(), "a string isn't overwritten by an object");
    }
}
