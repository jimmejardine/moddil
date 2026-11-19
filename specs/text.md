# Text and strings

> **Status:** decided direction, details open. Listed in the [index](README.md). Syntax per
> [`syntax.md`](syntax.md); snippets are **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Encoding** | `str` and `String` are **always valid UTF-8**, as in Rust. Bytes are `[u8]`. Paths, environment variables and arguments are `Path` and `OsStr`, which round-trip losslessly ([Bytes, paths and OS strings](#bytes-paths-and-os-strings)). |
| **Types** | `&str` (borrow), `String` (owned, with **short strings stored inline**), `Rc<str>` (shared), `RcStr` (zero-copy slice of a shared buffer), `char` (a Unicode scalar value) ([String types](#string-types)). |
| **Indexing** | **No `s[i]`.** Searches return a `StrPos`, a byte position on a character boundary **tied by type to its string**, so slicing with positions never traps and a position cannot be used on the wrong string. Raw byte offsets go through a visibly fallible call ([Positions and slicing](#positions-and-slicing)). |
| **Literals** | Rust's literals, plus **dedented `"""` blocks**, which also work with tagged literals (`sql"""…"""`) ([Literals](#literals)). |
| **Formatting** | `fmt"…"` is checked at compile time; `Display` and `Debug` as in Rust. Output **never depends on the locale** ([Formatting](#formatting)). |
| **Localisation** | Locale-dependent formatting performs a **`Locale` effect**, so it is visible, handled per request, and deterministic in tests ([Localisation: the `Locale` effect](#localisation-the-locale-effect)). |
| **Comparison** | Byte-wise equality and ordering; normalisation and case folding are explicit ([Comparison, normalisation and case](#comparison-normalisation-and-case)). |

## Problem

Every reference example, and almost every program an agent writes, handles text. The common failures
are well known:

- **Slicing in the middle of a character**, which corrupts text (C, Java's UTF-16 surrogates) or
  traps at run time (Rust's `&s[0..n]`).
- **Mojibake**: text decoded with the wrong encoding, discovered far from where it entered.
- **Lossy paths**: file names that are not valid Unicode, silently mangled when converted.
- **Hidden locale dependence**: a CSV writer that emits `1,5` instead of `1.5` on a German machine, or
  `"I".to_lowercase()` behaving differently under a Turkish locale.
- **Injection** through string concatenation into SQL, HTML or shell commands.
- **Hidden cost**: implicit copies and allocations in string-heavy code.

## Prior art

| Language | Approach | Lesson |
|---|---|---|
| **Rust** | `str`/`String` always UTF-8; `OsStr`, `Path`, `CStr` for the boundaries; byte-offset slicing that panics off a boundary | The right types; the slicing trap is the weak spot |
| **Swift** | `String` of grapheme `Character`s; opaque `String.Index`; small-string optimisation | Correct for humans, but awkward and O(n) for counting; opaque indices are the right idea |
| **Go** | A string is bytes, usually UTF-8; `range` decodes runes | Simple, but invalid text surfaces late as U+FFFD |
| **Python 3** | Code-point strings; `surrogateescape` for undecodable OS data | Lossless OS round-trips, at the cost of strings that are not really Unicode |
| **Java, C#, JavaScript** | UTF-16 code units | A legacy encoding that makes surrogate pairs everyone's problem |
| **Zig** | `[]const u8`, with UTF-8 helpers | Explicit, but no type-level guarantee of validity |
| **ICU4X, Fluent, MessageFormat 2** | Modern, data-driven localisation; plural and gender rules; messages as data | What a localisation mod should build on |

## String types

| Type | What it is | Typical use |
|---|---|---|
| `str` | An unsized run of valid UTF-8 | Only behind a pointer |
| `&str` | A second-class borrow of a `str` ([`memory.md`](memory.md)) | Parameters and locals |
| `String` | Owned, growable UTF-8 | Building and storing text |
| `Rc<str>` | Shared, immutable text | Text held by many owners: keys, interned names |
| `RcStr` | A slice of a shared, reference-counted buffer | Zero-copy parsing of I/O buffers ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io)) |
| `char` | A Unicode scalar value (U+0000 to U+10FFFF, excluding surrogates) | Single characters |

Cost stays visible, as everywhere in Moddil ([`values.md`](values.md)):

| Conversion | Spelling | Cost |
|---|---|---|
| `&String` → `&str` | Automatic (unsizing) | Free |
| `&str` → `String` | `s.to_string()` | Allocates and copies |
| `String` → `Rc<str>` | `s.to_rc()` | Moves into a shared allocation |
| `Rc<str>` → another handle | `r.share()` | Count + 1 |
| `RcStr` sub-slice | `r.slice(range)` | Count + 1, no copy |
| String literal | `"abc"` is a `&str` with static storage | Free |

`String` has no implicit copy and `+` on strings does not exist; building text uses `fmt"…"`,
`push_str` or `join`.

### Short strings are stored inline

**Decision: `String` uses a small-string optimisation.** On 64-bit targets a `String` is 24 bytes,
and text of up to 23 bytes is stored inside it with no heap allocation (as `compact_str` does in Rust;
Swift and C++ standard libraries store 15).

- **Why:** most strings in services and agents' code are short (keys, identifiers, field names,
  small numbers formatted as text), and avoiding an allocation for each is a large, broad win.
- **It costs little here.** In Rust, SSO is awkward because code may hold a pointer into a `String`
  while the `String` moves. Moddil's borrows are second-class ([`memory.md`](memory.md)),
  so no borrow of the bytes outlives a move, and an inline string can move freely.
- **Cost stays visible:** growing past the inline capacity allocates, exactly as growing a heap
  string does; `String::with_capacity(n)` still reserves up front. `capacity()` is never less than
  the inline size.
- **Unaffected:** `&str`, `Rc<str>` and `RcStr` have no inline form; `CString` for FFI is always on
  the heap, so its pointer is stable while C holds it.
- On 32-bit targets (wasm32) the inline capacity is 11 bytes. The exact layout is not part of the
  ABI ([`interop.md`](interop.md)): FFI passes `&str` as pointer and length.

## Positions and slicing

**Decision: strings are not indexed by integers.**

- `s[i]` does not exist. `s.len()` is the length **in bytes**, as an `int`
  ([`numerics.md`: Integer types](numerics.md#integer-types)).
- **Searches return positions.** `find`, `rfind`, `char_indices`, `split_once` and similar return
  a `StrPos`: a byte offset that is guaranteed to lie on a character boundary of that string.
- **Slicing with positions never traps.** `s[a..b]` takes a range of `StrPos`; `s.start()` and
  `s.end()` give the ends.

  ```rust
  fn domain(email: &str) -> Option<&str> {
      let at = email.find('@')?;                 // StrPos
      Some(&email[at.after('@')..email.end()])   // always on a boundary
  }
  ```

- **Raw byte offsets are visible.** Offsets that come from outside (a protocol header, a file
  format) go through `s.slice_bytes(a..b)`, which fails with a typed error when an offset is out of
  range or not on a boundary. There is no trapping integer slice in everyday code.
- **Iteration** chooses its unit explicitly: `.bytes()`, `.chars()` (scalar values), or
  `.graphemes()` (user-perceived characters, from `moddil/text`, because the Unicode tables are
  large). Counting characters is `s.chars().count()`, visibly O(n).
- **A position belongs to its string, by type (decided).** A `StrPos` is a view
  ([`memory.md`: View types](memory.md#view-types)) derived from a borrow of
  the string it was found in, in the same way an accelerator buffer is branded with its device
  ([`accelerators.md`](accelerators.md)). The compiler therefore rejects:
  - using a position from one string to slice another;
  - mutating the string while any of its positions are live (the positions hold a shared borrow).

  ```rust
  let at = name.find(' ')?;
  let first = &name[name.start()..at];      // ✓
  let x = &other[at..other.end()];         // ✗ compile error: `at` is a position in `name`
  name.push('!');                          // ✗ compile error while `at` is live
  ```

  At run time a `StrPos` is just a byte offset: the guarantee costs nothing.
- **Storing positions.** Because a `StrPos` is a view, it cannot be kept in owned data. A parser that
  records token spans either keeps `RcStr` slices of an immutable buffer (zero-copy, and safe to
  store), or stores the plain offset (`at.offset()`, an `int`) and re-enters with
  `s.pos_at(offset)?`, which fails visibly if the offset is not a boundary of `s`.

## Bytes, paths and OS strings

**Decision: `String` is strict UTF-8; the operating system's strings have their own types.**

| Type | Holds | Notes |
|---|---|---|
| `[u8]`, `Vec<u8>`, `Bytes` | Arbitrary bytes | `Bytes` is the rc-slice form for zero-copy I/O |
| `OsStr`, `OsString` | Whatever the platform uses: bytes on Unix, potentially ill-formed UTF-16 on Windows (stored as WTF-8) | Lossless round trips |
| `Path`, `PathBuf` | A file-system path, built on `OsStr` | Joining, components and extensions are path operations, not string operations |
| `CStr`, `CString` | Nul-terminated bytes for C | Checked at the boundary ([`interop.md`](interop.md)) |

- **Conversion to `String` is explicit and fallible:** `String::from_utf8(bytes)?` and
  `os.to_str()?` fail with a typed error that keeps the original bytes. Lossy conversion is spelled
  out: `os.to_string_lossy()`.
- **Environment variables and command-line arguments** are `OsString`s and are reached through
  effects (`Env`, `Process`; [`effects-and-capabilities.md`: The effect catalogue](effects-and-capabilities.md#the-effect-catalogue)), never through
  ambient globals. How `main` receives them is part of [`lifecycle.md`](lifecycle.md).
- **Freestanding targets** have `str`, `[u8]` and `char`, but no `OsStr` or `Path`
  ([`freestanding.md`](freestanding.md)).

## Literals

- Rust's literals unchanged: `"…"` with escapes, raw `r#"…"#`, byte strings `b"…"`, `'c'` for
  `char`, `b'c'` for `u8`.
- **Dedented blocks (a Rust delta):** a `"""` block starts on the line after the opening quotes, and
  the indentation of the closing `"""` is removed from every line.

  ```rust
  let q = sql"""
      SELECT id, name
      FROM users
      WHERE id = {user_id}
      """;                                       // the four spaces before """ are stripped
  ```

  - The newline after the opening `"""` is dropped; the final newline before the closing `"""` is
    kept, so a block is a sequence of complete lines.
  - A line indented less than the closing quotes is a compile error, as is mixing tabs and spaces
    in the stripped indentation.
  - Raw blocks (`r"""…"""`) dedent but do not process escapes.
  - Any **tagged literal** may use a block ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)).
- Embedding a file's text is comptime `embed("path")` ([`comptime.md`](comptime.md)).

## Formatting

- **`fmt"…"`** is a tagged literal: parsed at compile time, with each `{…}` a **typed hole** checked
  against its argument's type. A bad format string is a compile error
  ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)).
- **`Display`** (for users) and **`Debug`** (for developers, derivable) are the formatting traits, as
  in Rust; format specifiers (`{x:>8}`, `{x:.3}`, `{x:?}`, `{x:#x}`) follow Rust's.
- **Locale-independent by decision.** `fmt"…"`, `Display`, `Debug`, number parsing and
  `to_lowercase`/`to_uppercase` (using Unicode's default, locale-free rules) give the same result on
  every machine. This makes them safe for files, protocols and logs, and part of the determinism
  guarantee ([`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).
- **No injection through formatting.** `sql"…"`, `html"…"` and `sh"…"` have their own tagged
  literals whose holes are parameters or escaped values, never string concatenation. `fmt"…"`
  produces plain text and cannot be passed where a `Sql` or `Html` value is expected.
- **Writing without allocating:** `fmt"…"` can write into any `Write` sink, including a fixed buffer
  on freestanding targets ([`freestanding.md`](freestanding.md)).

## Comparison, normalisation and case

- `==` and `Ord` on strings compare **bytes**, which for UTF-8 equals code-point order. Fast, total
  and deterministic; not linguistically correct.
- **Normalisation is explicit.** `"é"` written as one code point and as `e` + combining accent are
  different strings until normalised: `s.nfc()`, `s.nfd()` in `moddil/text`.
- **Case-insensitive comparison is explicit:** `a.eq_ignore_case(b)` uses Unicode default case
  folding, independent of locale.
- **Linguistic sorting** (collation) depends on the language and therefore needs the `Locale` effect
  ([below](#localisation-the-locale-effect)).
- Hashing follows [`values.md`: Hashing](values.md#hashing).

## Localisation: the `Locale` effect

**Decision: anything that depends on the user's language or region uses the `Locale` effect.**

```rust
fn order_summary(o: &Order) -> String with Locale {
    i18n::message("order-summary",                      // MessageFormat 2, plural-aware
        count: o.items.len(),
        total: i18n::money(o.total),                    // "1.234,50 €" or "€1,234.50"
        date:  i18n::date(o.placed, style: Medium))
}

// per request, from the Accept-Language header
with Locale = i18n::locale(req.header("Accept-Language")) {
    page.render(order_summary(&order))
}

// in tests: one fixed locale, so output is reproducible
with Locale = i18n::fixed("de-CH") { assert(order_summary(&o) == "…") }
```

- **Visible:** a function that formats for humans says so in its signature; a CSV writer without
  `Locale` cannot accidentally write `1,5`.
- **Scoped, not global:** each request, task or test installs its own handler
  ([`effects-and-capabilities.md`](effects-and-capabilities.md)); there is no process-wide locale to race on.
- **Deterministic:** simulation and replay install a fixed handler ([`determinism.md`](determinism.md)).
- **Implementation:** a standard `moddil/i18n` mod built on ICU4X-style data (numbers, dates, plural
  rules, collation) and MessageFormat 2 for messages. Locale data is selected at build time so
  binaries carry only the locales they need, which matters for WASM size.

## Interop boundaries

| Boundary | Representation | Cost |
|---|---|---|
| **C** | `CStr`/`CString`; interior nul bytes rejected when converting | A copy, or a check |
| **Rust** | `&str`/`String` map directly; both are UTF-8 | Zero-copy ([`interop.md`: Rust: the generated bridge](interop.md#rust-the-generated-bridge)) |
| **WASM components** | Component-model `string`, UTF-8 | Zero-copy where both sides are UTF-8 |
| **JavaScript and the DOM** | UTF-16 on the JS side | A transcoding copy at each crossing; batching and caching strategy belongs to [`ui.md`](ui.md) |

## Open questions

1. **Graphemes in core or in `moddil/text`?** Core keeps freestanding binaries small; a mod means
   `.graphemes()` needs a dependency.
2. **Regular expressions:** linear-time only (RE2, Rust's `regex`), with `re"…"` compiled at
   compile time? Backtracking features behind an explicit opt-in?
3. **Exact `"""` rules:** blank lines with less indentation, trailing whitespace, and line-ending
   normalisation (CRLF source files).
4. **Interning:** should the standard library offer an interner (`Symbol`) for compilers and
   protocol code, or leave it to mods?
