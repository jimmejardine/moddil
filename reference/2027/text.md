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

**text.bytes.1** `s.as_bytes()` borrows the bytes of a `str` or a `String` as a `&[u8]`, its UTF-8:
`"é".as_bytes()` is `[195, 169]`. `str::from_utf8` (**text.utf8.1**) goes the other way.

**text.string.1** `String` is owned, growable text. `String::new()` is empty; `s.push_str(t)`
appends the text `t`, `s.push(c)` a character, `s.pop()` removes and returns the last character
(`None` if there is none), `s.clear()` empties it, and `s.as_str()` borrows its text as a `&str`.

**text.string.inline.1** `String` is the prelude's struct, written in Moddil over its UTF-8
bytes ([`prelude/text.moddil`](prelude/text.moddil)): up to 23 bytes are held inside the `String` itself,
and more in a heap buffer, so `s.capacity()`, how many bytes it holds before it grows, is never
below 23 (`String::new().capacity()` is 23). Past 23 bytes the text moves to the heap, and a full
heap buffer doubles; `String::with_capacity(n)` reserves `n` bytes, on the heap if `n` is above
23. Where its bytes are changes nothing else: its text, length and comparisons are the same.

**text.string.2** A `String` is moved, not copied ([Ownership](ownership.md)); `s.clone()` is a
new `String` with the same text, and changing one does not change the other.

**text.coerce.1** A `&String` is converted to a `&str` of its text where a `&str` is expected, and
a `&mut String` likewise.

**text.method.1** `str` has `len`, `is_empty`, `starts_with`, `ends_with`, `contains`, `trim`
(without the white space at its ends), and `to_uppercase` and `to_lowercase` (a new `String`, by
Unicode's rules, the same on every machine). `String` has the same methods, on its text.

**text.pos.1** Text is not indexed by integers: a position in it is a `StrPos`, the prelude's
view of a byte offset that is a character boundary of the text it was found in, which it
borrows, so the text cannot change while it is live (E0305). `s.start()` and `s.end()` are the ends,
`p.offset()` the byte offset, and `p.after(part)` the position just past `part`, which traps if
`part` is not at `p`. `&s[a..b]`, for a `str` or a `String` `s` and positions `a` and `b`, is the
text between them (`&s[a..]`, `&s[..b]` and `&s[..]` too; an `int` end is an error, E0201, and so
is `..=`). A position slices only the text it was found in, that text and not other text with
the same characters: where both are a local's (or the position is a local's and the text a
parameter's), other text is an error (E0316); where the position or the text came from a
parameter, or the text borrows nothing, it traps when it runs. It also traps if `b` comes before
`a`. An offset from outside the program becomes a position by
`s.pos_at(offset)`, and text by `s.slice_bytes(i..j)`, which fail with the prelude's
`NotABoundary { offset }` if an offset is not a boundary of `s` (or they are out of order).

**text.find.1** `s.find(part)` and `s.rfind(part)`, for a pattern `part` (**text.pattern.1**), give
the position where `part` first or last is, or `None`; `s.split_once(part)` the text before the
first `part` and the text after it, or `None`. `String` has them too, its positions borrowing it.

**text.find.2** `s.char_indices()`, for a `str` or `String`, is a view of type `CharIndices` that
gives each character with its position, a `(StrPos, char)`, in order.

**text.utf8.1** `str::from_utf8(bytes)`, for a `&[u8]`, is the bytes as text, failing with
`Utf8Error { valid_up_to }`, how many of the bytes from the start are UTF-8, if they are not all
(each character in its shortest encoding, and no surrogate). `String::from_utf8(bytes)`, for a
`Vec<u8>`, makes a `String` of them, failing with `FromUtf8Error { bytes, valid_up_to }`, which
gives the bytes back.

**text.pattern.1** `contains`, `starts_with` and `ends_with` take a **pattern**, a value of a type
that implements the prelude's `Pattern`: a `&str` or `&String`, whose text is looked for, or a
`char`. They are evaluated at compile time too, as in a refinement's predicate
(`self.chars().all(|c| ALPHABET.contains(c))`).

**text.rcstr.1** `RcStr` is the prelude's slice of shared text: a reference-counted `String`
and a byte range of it. `s.to_rcstr()` moves a `String` `s` into one, its whole text;
`r.as_str()` borrows its text; `r.slice(a, b)`, for positions `a` and `b` of `r.as_str()`, is
the slice between them, and `r.share()` (also `clone`) the same slice, each adding one to the
count and copying no text (it traps as `&s[a..b]` does). Unlike a position, an `RcStr` can be
kept in owned data. `r.len()`, `r.is_empty()` and `r.to_string()` are as for its text, and it is
compared, ordered, hashed, displayed and debugged as its text.

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
is an invalid literal (E0902), reported at the closing quotes. A raw block (`r"""`) takes off
indentation but has no escapes. A `fmt"""` block is a block in the same way, with holes
(**expr.fmt.1**), each on one line.

## Formatting

**text.fromstr.1** The prelude's trait `FromStr`, `fn from_str(s: &str) -> Option<Self>`, reads a
value from its text, `None` if the text is not one: an integer type's from its decimal digits
after an optional `-` or `+`, of a value in its type's range and in `int`'s; a `bool` from `true`
or `false`; a `char` from one character; and a `String` from the text as it is. A tagged literal's
typed fields are read with it (**comptime.tag.fields.3**). Floats are not read yet.

**core.text.1** The `core` mod's `text` module ([source](core/text.moddil)) has `Match`, where
something is in a text: its `start` and `end` bytes, given to `Match::new(start, end)`, with
`len()`, `is_empty()`, and `text(t)`, the text it is in `t`. Its fields are private; it is
`Copy`, `Eq` and `Debug` (`Match { start: 2, end: 5 }`). The `moddil/regex` mod's matches are
these.

**text.write.1** The prelude's trait `TextWrite` is where text is written:
`fn write_str(&mut self, s: &str)` writes `s`, and `fn write_char(&mut self, c: char)` writes one
character, through `write_str` unless the type gives its own. Writing to it cannot fail. `String`
implements it, appending; a `&mut String` is used as a `&mut dyn TextWrite` as any borrow is
(**type.dyn.coerce.1**).

**text.write.2** `TextWrite` also has `fn write_fmt(&mut self, text: String)`, which writes
`text` through `write_str`. Called with a `fmt"…"` literal, `w.write_fmt(fmt"…")` makes no
`String` of it: each run of its text is written by `w`'s `write_str` and each hole by its `fmt`
straight onto `w`, so what is written is the same and nothing is allocated for it. A hole with a
spec is written twice: once into the prelude's `Measure`, which counts its characters, then onto
`w` through its `Padded`, which writes the fill (each character a `write_char`), the zeros or the
`+`, and cuts it to a precision.

**text.display.1** The prelude's trait `Display` is how a value is written as text:
`fn fmt(&self, out: &mut dyn TextWrite)` writes it to `out`, and `x.to_string()` is a new `String`
of it.
`int`, `bool`, `char`, `str` and `String` implement it: an `int` in decimal with a `-` if it is
negative, a `bool` as `true` or `false`, and text as itself.

**expr.fmt.1** `fmt"…"` is a `String`. Its text is the literal's, with escapes as in a string
literal and `{{` and `}}` for `{` and `}`, and each **hole** `{e}` is an expression: in order, each
is evaluated, borrowed, and written by its `Display::fmt`. A hole whose value is a borrow writes
what it points to.

**expr.fmt.debug.1** A hole `{e:?}` writes `e`'s `Debug` in place of its `Display`. The
prelude's `Debug`, `fn fmt(&self, out: &mut dyn TextWrite)`, writes a value as Moddil source does: text
and characters in quotes and escaped, `[1, 2]`, `Some(x)`, `(a, b)`; `int`, `bool`, `char`, `str`,
`String`, `()`, slices, arrays, `Vec`, `Option`, `Result`, pairs, and the core mod's maps
(`{k: v}`) and sets implement it, and a borrow `&T` shows what it points to, so a view with
borrow fields derives it.

**expr.fmt.debug.2** A hole `{e:#?}` writes `e`'s `Debug` across lines, as Rust's `{:#?}` does:
the prelude's `debug_pretty` passes `Debug::fmt` a sink (`PrettyWrite`) that starts a new line,
four spaces further in, after each `{`, `[` or `(`, and after each `, `; and before each `}`, `]`
or `)` writes a `,` and a new line, back one level. An empty `()`, `[]` or `{}` stays as it is,
and quoted text and characters are written as they are, so every `Debug`, derived or written,
is written across lines this way: `Point {
    x: 1,
    y: 2,
}`.

**expr.fmt.2** The type of each hole, or what a borrow in it points to, implements `Display`
(E0402); for `{e:?}`, `Debug`. A hole holds one expression (E0902).

**expr.fmt.spec.1** A hole's **format spec**, after a `:` that is not part of a path's `::`, is
`[[fill]align][+][#][0][width][.precision][type]`, as Rust's, the type `?` (**expr.fmt.debug.1**,
with `#` **expr.fmt.debug.2**),
`x`, `X`, `b` or `o` (**expr.fmt.spec.4**), or `e` or `E` (**expr.fmt.spec.5**), and the width
and precision each a number or `name$` (**expr.fmt.spec.6**): `{n:>6}`, `{s:*^9}`, `{f:8.2}`,
`{x:>12?}`, `{n:#06x}`, `{f:w$.p$}`.
With a width, what the hole writes is padded to that many characters (not bytes) with the fill,
a space unless one is written: to its left for `>`, to its right for `<`, and on both sides for
`^`, the extra one on the right. Without an alignment a number (`int`, a sized integer, a float)
goes to the right and anything else to the left. Text longer than the width is written whole. A
spec that is not of that form, an alignment or `0` without a width, or `#` without a base, is an
invalid literal (E0902).

**expr.fmt.spec.2** `0` before the width pads a number with `0`s after its sign and a base's
prefix, whatever the alignment: `{n:06}` of `-42` is `-00042`, `{n:#06x}` of `42` is `0x002a`. On
anything but a number it is E0902.

**expr.fmt.spec.3** A precision is a float's digits after the point, rounded to the nearest
(`{f:.3}` of `3.14159` is `3.142`; the float's `fixed_text(digits)` gives the same text), or the
most characters of a `str` or `String` that `Display` writes (`{s:.2}` of `"abc"` is `ab`). On
any other type, and on text with `?`, it is E0902.

**expr.fmt.spec.4** `x`, `X`, `b` and `o` write an integer's digits in base 16 (with lower- or
upper-case letters), 2 or 8: its bits as its type holds them, so a negative one in two's
complement (`{n:x}` of `-1` as an `i8` is `ff`), and `#` puts the base's prefix before them
(`0x`, `0b`, `0o`). `+` writes a number's sign when it has no `-` (`{n:+}` of `5` is `+5`). The
prelude's `RadixText` writes the digits. A base on anything but an integer, or `+` on anything
but a number, is E0902.

**expr.fmt.spec.5** `e` and `E` write a float in exponent form, as Rust's: `{f:e}` of `1234.5`
is `1.2345e3`, with as many digits as it needs, or with a precision that many after the point
(`{f:.2e}` is `1.23e3`); `E` writes `E`. The float's `exp_text(digits, upper)` gives the same
text. On anything but a float it is E0902.

**expr.fmt.spec.6** A width or precision written `name$` is the value of `name`, an `int` in the
hole's scope (E0201 otherwise), evaluated where the hole is: `{f:w$.p$}`. A hole is an
expression, so Rust's `.*`, which takes the next argument, has no meaning here; `.name$` says
which value is meant.

## Patterns

**text.pat.1** A character literal is a pattern matching that character, and `'a'..='z'` (or
`..`, or no upper bound) one matching the characters whose code points are in the range. A
`match` on a `char` covers every code point from U+0000 to U+10FFFF.

**text.pat.2** A string literal is a pattern matching a `&str` with the same text (E0201 against
another type). No list of literals covers every string, so a `match` on text needs a `_` or a
binding (E0208).
