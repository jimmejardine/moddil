//! What a `// =>` comment says a test's line gives (`item.test.value.1`, `item.test.value.2`):
//! its value, and, under the parallel runtime, whose tasks run in whatever order, the others it
//! may give: `// => v (in parallel, also w; x)`, `// => v (in parallel, in any order)` for a
//! tuple or array whose parts may come in another order, or both, `(in parallel, in any order,
//! also w; x)`. `moddil test` and the conformance suite read them alike.

/// A `// =>` comment's expectation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected<'a> {
    /// The value.
    pub value: &'a str,
    /// The other values it may have under the parallel runtime.
    pub also: Vec<&'a str>,
    /// Whether, under the parallel runtime, the parts of the value, or of one of the others, may
    /// come in any order.
    pub any_order: bool,
}

impl Expected<'_> {
    /// Whether `seen` is what it says: the value, or, under the parallel runtime if `parallel`,
    /// one of the others it may be.
    pub fn accepts(&self, seen: &str, parallel: bool) -> bool {
        let one = |v: &str| seen == v || self.any_order && parts(seen) == parts(v);
        seen == self.value || parallel && (one(self.value) || self.also.iter().any(|&a| one(a)))
    }
}

/// The expectation of line comment `comment`, if it is a `// =>` one; an error for one without
/// a value, or with `(in parallel, also …` and no `)`.
pub fn expected(comment: &str) -> Option<Result<Expected<'_>, String>> {
    let rest = comment.strip_prefix("// =>")?.trim();
    let mut e = Expected {
        value: rest,
        also: Vec::new(),
        any_order: false,
    };
    if let Some((v, how)) = rest.split_once(" (in parallel, ") {
        let Some(how) = how.strip_suffix(')') else {
            return Some(Err("`(in parallel, …)` without its `)`".to_string()));
        };
        e.value = v.trim();
        let also = match how.strip_prefix("in any order") {
            Some(rest) => {
                e.any_order = true;
                rest.strip_prefix(", ").unwrap_or(rest)
            }
            None => how,
        };
        if !also.is_empty() {
            let Some(also) = also.strip_prefix("also ") else {
                return Some(Err(
                    "`(in parallel, …)` is `in any order`, `also v; w`, or both".to_string(),
                ));
            };
            e.also = also.split("; ").map(str::trim).collect();
        }
    }
    if e.value.is_empty() {
        return Some(Err("`// =>` without a value".to_string()));
    }
    Some(Ok(e))
}

/// The parts of a value shown as Moddil source, the top-level elements of a tuple or an array,
/// sorted: what `(in parallel, in any order)` compares.
fn parts(v: &str) -> Vec<&str> {
    let inner = v
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .or_else(|| v.strip_prefix('[').and_then(|v| v.strip_suffix(']')))
        .unwrap_or(v);
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].trim());
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_what_they_may_be_in_parallel() {
        let e = expected("// => 3").unwrap().unwrap();
        assert!(e.accepts("3", true) && !e.accepts("4", true));
        let e = expected("// => -2 (in parallel, also -1; 0)")
            .unwrap()
            .unwrap();
        assert_eq!((e.value, e.also.clone()), ("-2", vec!["-1", "0"]));
        assert!(e.accepts("-1", true) && !e.accepts("-1", false));
        let e = expected("// => (Some(1), Some((2, 3))) (in parallel, in any order)")
            .unwrap()
            .unwrap();
        assert!(e.accepts("(Some((2, 3)), Some(1))", true));
        assert!(!e.accepts("(Some((2, 3)), Some(1))", false));
        assert!(!e.accepts("(Some(2), Some(1))", true));
        let e = expected("// => (1, 2) (in parallel, in any order, also (1, 3))")
            .unwrap()
            .unwrap();
        assert!(
            e.accepts("(3, 1)", true) && e.accepts("(2, 1)", true) && !e.accepts("(3, 2)", true)
        );
        assert!(
            expected("// => 1 (in parallel, sometimes 2)")
                .unwrap()
                .is_err()
        );
        assert!(expected("// =>").unwrap().is_err());
        assert!(expected("// => 1 (in parallel, also 2").unwrap().is_err());
        assert!(expected("// a comment").is_none());
    }
}
