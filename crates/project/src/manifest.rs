//! The manifest, `moddil.toml` (`modules.md`: The manifest): a subset of TOML.
//!
//! ```toml
//! [mod]
//! name = "github.com/acme/matcher"
//! edition = "2027"
//!
//! [lib]
//! root = "src/lib.moddil"
//!
//! [[exe]]
//! name = "matcher"
//! main = "src/main.moddil"
//!
//! [dependencies]
//! shapes = { path = "../shapes" }
//! ```
//!
//! The subset: `#` comments, `[table]` and `[[array]]` headers, and `key = value` lines whose
//! value is a string in double quotes, `true` or `false`, or an inline table of strings on one
//! line. Anything else is a problem at its line.

use std::ops::Range;

/// A value: a string, a boolean, or an inline table of strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Str(String),
    Bool(bool),
    Table(Vec<(String, String)>),
}

/// A `key = value` line: the key, the value, and the byte range of the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pair {
    pub(crate) key: String,
    pub(crate) value: Value,
    pub(crate) range: Range<usize>,
}

/// A table: its header (`mod`, or `exe` for each `[[exe]]`), where the header is, and its pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Table {
    pub(crate) name: String,
    pub(crate) array: bool,
    pub(crate) range: Range<usize>,
    pub(crate) pairs: Vec<Pair>,
}

/// A manifest's tables in order, after the pairs before any header (in a table named "").
pub(crate) fn parse(text: &str) -> Result<Vec<Table>, (Range<usize>, String)> {
    let mut tables = vec![Table {
        name: String::new(),
        array: false,
        range: 0..0,
        pairs: Vec::new(),
    }];
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let line = strip_comment(raw).trim();
        let range = start..start + raw.trim_end().len();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix("[[") {
            let name = inner
                .strip_suffix("]]")
                .ok_or((range.clone(), "a table header ends with `]]`".to_string()))?;
            tables.push(Table {
                name: name.trim().to_string(),
                array: true,
                range,
                pairs: Vec::new(),
            });
        } else if let Some(inner) = line.strip_prefix('[') {
            let name = inner
                .strip_suffix(']')
                .ok_or((range.clone(), "a table header ends with `]`".to_string()))?;
            let name = name.trim().to_string();
            if tables.iter().any(|t| t.name == name && !t.array) {
                return Err((range, format!("the table `[{name}]` is written twice")));
            }
            tables.push(Table {
                name,
                array: false,
                range,
                pairs: Vec::new(),
            });
        } else {
            let (key, value) = line
                .split_once('=')
                .ok_or((range.clone(), "expected `key = value`".to_string()))?;
            let key =
                unquote_key(key.trim()).ok_or((range.clone(), "a key is a name".to_string()))?;
            let value = parse_value(value.trim()).map_err(|m| (range.clone(), m))?;
            let table = tables.last_mut().expect("a table");
            if table.pairs.iter().any(|p| p.key == key) {
                return Err((range, format!("`{key}` is written twice in this table")));
            }
            table.pairs.push(Pair { key, value, range });
        }
    }
    Ok(tables)
}

/// The line without a `#` comment (outside a string).
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

/// A key, bare (`name`) or quoted (`"github.com/acme"`).
fn unquote_key(k: &str) -> Option<String> {
    if let Some(s) = k.strip_prefix('"') {
        return s.strip_suffix('"').map(String::from);
    }
    (!k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then(|| k.to_string())
}

fn parse_string(v: &str) -> Result<String, String> {
    let inner = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("expected a string in double quotes, found `{v}`"))?;
    if inner.contains('"') || inner.contains('\\') {
        return Err("a string here has no quotes or escapes inside it".to_string());
    }
    Ok(inner.to_string())
}

fn parse_value(v: &str) -> Result<Value, String> {
    if let Some(inner) = v.strip_prefix('{') {
        let inner = inner
            .strip_suffix('}')
            .ok_or("an inline table ends with `}`")?;
        let mut pairs = Vec::new();
        for part in inner.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=').ok_or("expected `key = value`")?;
            let k = unquote_key(k.trim()).ok_or("a key is a name")?;
            pairs.push((k, parse_string(v.trim())?));
        }
        return Ok(Value::Table(pairs));
    }
    match v {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    parse_string(v).map(Value::Str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_subset() {
        let t = parse(
            "# a mod\n[mod]\nname = \"a\" # its name\n\n[[exe]]\nmain = \"src/m.moddil\"\n\
             [dependencies]\nb = { path = \"../b\" }\n",
        )
        .expect("valid");
        assert_eq!(t.len(), 4);
        assert_eq!(t[1].name, "mod");
        assert_eq!(t[1].pairs[0].value, Value::Str("a".into()));
        assert!(t[2].array);
        assert_eq!(
            t[3].pairs[0].value,
            Value::Table(vec![("path".into(), "../b".into())])
        );
        assert!(parse("[mod\n").is_err());
        assert!(parse("name = 3\n").is_err());
        assert!(parse("[mod]\n[mod]\n").is_err());
    }
}
