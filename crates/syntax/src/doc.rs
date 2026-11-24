//! Examples in doc comments, which are tests (`test.doc.1`): the lines of a `///` comment's
//! `# Example` (or `# Examples`) section, up to its next heading, each written as Moddil without
//! its `///`. Each is made into a test whose code is where the example is in the file: the file
//! with everything else blanked to spaces, `test "e" {` on the heading's line and `}` at the start
//! of the line after the example, so its ranges, its `// =>` lines and its module are the
//! file's.

use crate::{SyntaxKind, SyntaxNode, TextRange};

/// An example in a doc comment, made a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocExample {
    /// The name of the item it documents (empty if it has none).
    pub item: String,
    /// The test's range in the file: from the heading to the closing `}`.
    pub range: TextRange,
    /// The file's text with the test in place of the example and the rest blanked: parsed, it
    /// holds a `test` item at `range`.
    pub source: String,
}

/// What a doc comment's line says, after its `///` and the space after that.
fn content(line: &str) -> &str {
    let rest = line.strip_prefix("///").unwrap_or(line);
    rest.strip_prefix(' ').unwrap_or(rest)
}

/// The examples in the `///` comments of the items in `root`, a parse of `text`.
pub fn doc_examples(root: &SyntaxNode, text: &str) -> Vec<DocExample> {
    let mut out = Vec::new();
    for item in root.descendants() {
        // Its doc lines, in order: each attribute's `DOC_OUTER` token.
        let lines: Vec<(TextRange, String)> = item
            .children()
            .filter(|c| c.kind() == SyntaxKind::OUTER_ATTR)
            .flat_map(|a| a.child_tokens().collect::<Vec<_>>())
            .filter(|t| t.kind() == SyntaxKind::DOC_OUTER)
            .map(|t| (t.text_range(), t.text().to_string()))
            .collect();
        if lines.is_empty() {
            continue;
        }
        let name = item
            .child(SyntaxKind::NAME)
            .and_then(|n| {
                n.child_tokens()
                    .find(|t| t.kind() == SyntaxKind::IDENT)
                    .map(|t| t.text().to_string())
            })
            .unwrap_or_default();
        let mut k = 0;
        while k < lines.len() {
            let heading = content(&lines[k].1).trim();
            if heading != "# Example" && heading != "# Examples" {
                k += 1;
                continue;
            }
            let at = lines[k].0;
            let mut code = Vec::new();
            k += 1;
            while k < lines.len() && !content(&lines[k].1).trim_start().starts_with('#') {
                if !content(&lines[k].1).trim_start().starts_with("```") {
                    code.push(lines[k].0);
                }
                k += 1;
            }
            if let Some(e) = example(text, &name, at, &code) {
                out.push(e);
            }
        }
    }
    out
}

/// The test for the example whose heading is at `heading` and whose lines are at `code`, or
/// `None` if there is no room for it.
fn example(text: &str, item: &str, heading: TextRange, code: &[TextRange]) -> Option<DocExample> {
    const OPEN: &str = "test \"e\" {";
    let last = code.last()?;
    if (heading.len() as usize) < OPEN.len() {
        return None;
    }
    // The start of the line after the example's last.
    let close = text[last.end as usize..].find('\n')? + last.end as usize + 1;
    if close >= text.len() {
        return None;
    }
    // Every byte a space but the line breaks, so offsets and lines stay the file's.
    let mut source: Vec<u8> = text
        .bytes()
        .map(|b| if b == b'\n' || b == b'\r' { b } else { b' ' })
        .collect();
    let put = |source: &mut Vec<u8>, at: usize, s: &[u8]| {
        source[at..at + s.len()].copy_from_slice(s);
    };
    put(&mut source, heading.start as usize, OPEN.as_bytes());
    for (i, r) in code.iter().enumerate() {
        // The line without its `///`.
        let line = &text.as_bytes()[r.start as usize + 3..r.end as usize];
        put(&mut source, r.start as usize + 3, line);
        // A line whose value is observed (`// => v`) is a statement of its own, as an example
        // writes it with no `;`: one goes where the next line's `///` was.
        let observed = text[r.start as usize..r.end as usize].contains("// =>");
        if observed && let Some(next) = code.get(i + 1) {
            put(&mut source, next.start as usize, b";");
        }
    }
    put(&mut source, close, b"}");
    Some(DocExample {
        item: item.to_string(),
        range: TextRange::new(heading.start, close as u32 + 1),
        source: String::from_utf8(source).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_example_is_a_test_where_it_is() {
        let text = "/// Doubles.\n///\n/// # Example\n/// twice(2)  // => 4\nfn twice(x: int) -> int { x * 2 }\n";
        let parse = crate::parse(text);
        let found = doc_examples(&parse.syntax(), text);
        assert_eq!(found.len(), 1, "{found:?}");
        let e = &found[0];
        assert_eq!(e.item, "twice");
        assert_eq!(e.source.len(), text.len());
        let start = e.range.start as usize;
        assert!(e.source[start..].starts_with("test \"e\" {"));
        assert!(
            e.source.contains("    twice(2)  // => 4\n}"),
            "{:?}",
            e.source
        );
        assert!(crate::parse(&e.source).errors().is_empty());
    }
}
