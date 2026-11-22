# Text

`char`, `str` and `String` are built-in types; their methods and `Display` are the prelude's
([The prelude](prelude.md)). Text is always valid UTF-8.

## Characters

**text.char.1** `char` is a Unicode scalar value: U+0000 to U+10FFFF, surrogates excluded. A
character literal `'c'` holds one character, written as itself or as an escape
(**text.lit.escape.1**). `c.code()` is its code point, and `char::from_code(n)` is the character
with code point `n`, or `None` if there is none. `char` is `Copy`, and characters compare by code
point.

## Strings

**text.str.1** `str` is a run of valid UTF-8 text. A string literal has type `&str`, a borrow of its
text. `s.len()` is its length in bytes, so `"café".len()` is `5`.

**text.str.2** `str` has no size of its own: it is named only behind a borrow (`&str`,
`&mut str`) or as the type an impl is for (E0901).

**text.string.1** `String` is owned, growable text. `String::new()` is empty; `s.push_str(t)`
appends the text `t`, `s.push(c)` a character, `s.pop()` removes and returns the last character
(`None` if there is none), `s.clear()` empties it, and `s.as_str()` borrows its text as a `&str`.

**text.string.2** A `String` is moved, not copied ([Ownership](ownership.md)); `s.clone()` is a
new `String` with the same text, and changing one does not change the other.

**text.coerce.1** A `&String` is converted to a `&str` of its text where a `&str` is expected, and
a `&mut String` likewise.

**text.method.1** `str` has `len`, `is_empty`, `starts_with`, `ends_with`, `contains`, `trim`
(without the white space at its ends), and `to_uppercase` and `to_lowercase` (a new `String`, by
Unicode's rules, the same on every machine). `String` has the same methods, on its text.

**text.pattern.1** `contains`, `starts_with` and `ends_with` take a **pattern**, a value of a type
that implements the prelude's `Pattern`: a `&str` or `&String`, whose text is looked for, or a
`char`. They are evaluated at compile time too, as in a refinement's predicate
(`self.chars().all(|c| ALPHABET.contains(c))`).

**text.cmp.1** `==`, `!=`, `<`, `<=`, `>` and `>=` on two `&str`s compare their text, byte by byte,
which orders by code point. Two `String`s compare the same way, through `PartialEq` and `Ord`
([Comparisons](expressions/comparison.md)). No normalisation or case folding is implied.

## Literals

**text.lit.escape.1** A string or character literal may hold the escapes `\n`, `\r`, `\t`, `\0`,
`\\`, `\'`, `\"`, `\s` (a space), `\xHH` (an ASCII character, up to `\x7F`) and `\u{H…}` (any
character, with up to six hexadecimal digits and `_` between them). A `\` at the end of a line
leaves out the line break and the white space that starts the next line. Any other escape is a
syntax error (E0001).

**text.lit.raw.1** A raw literal, `r"…"` or `r#"…"#` with any number of `#`s, holds its text as
written: it has no escapes, and ends at the `"` followed by as many `#`s as it started with.

**text.lit.block.1** A block, `"""` to `"""`, starts on the line after its opening quotes; the
closing quotes are on a line of their own, and their indentation is taken off every line. A line
of only white space is an empty line, white space at the end of a line is left out (`\s` keeps a
space), and each line ends with a line break, `\n` whatever the source file uses. A line indented
less than the closing quotes, or indentation that mixes tabs and spaces with the closing line's,
is a syntax error (E0001). A raw block (`r"""`) takes off indentation but has no escapes.

## Formatting

**text.display.1** The prelude's trait `Display` is how a value is written as text:
`fn fmt(&self, out: &mut String)` appends it to `out`, and `x.to_string()` is a new `String` of it.
`int`, `bool`, `char`, `str` and `String` implement it: an `int` in decimal with a `-` if it is
negative, a `bool` as `true` or `false`, and text as itself.

**expr.fmt.1** `fmt"…"` is a `String`. Its text is the literal's, with escapes as in a string
literal and `{{` and `}}` for `{` and `}`, and each **hole** `{e}` is an expression: in order, each
is evaluated, borrowed, and written by its `Display::fmt`. A hole whose value is a borrow writes
what it points to.

**expr.fmt.debug.1** A hole `{e:?}` writes `e`'s `Debug` in place of its `Display`. The
prelude's `Debug`, `fn fmt(&self, out: &mut String)`, writes a value as Moddil source does: text
and characters in quotes and escaped, `[1, 2]`, `Some(x)`, `(a, b)`; `int`, `bool`, `char`, `str`,
`String`, `()`, slices, arrays, `Vec`, `Option`, `Result`, pairs, and the core mod's maps
(`{k: v}`) and sets implement it. Other format specs wait.

**expr.fmt.2** The type of each hole, or what a borrow in it points to, implements `Display`
(E0402); for `{e:?}`, `Debug`. A hole holds one expression (E0902).

## Patterns

**text.pat.1** A character literal is a pattern matching that character, and `'a'..='z'` (or
`..`, or no upper bound) one matching the characters whose code points are in the range. A
`match` on a `char` covers every code point from U+0000 to U+10FFFF.

**text.pat.2** A string literal is a pattern matching a `&str` with the same text (E0201 against
another type). No list of literals covers every string, so a `match` on text needs a `_` or a
binding (E0208).
