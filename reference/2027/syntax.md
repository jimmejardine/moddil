# Syntax

**syntax.grammar.1** The syntax of a source file is defined by the grammar
[`reference/grammar/moddil.ebnf`](../grammar/README.md). A file the grammar does not accept is
rejected with E0001.

**syntax.text.1** A source file is UTF-8 text. Identifiers follow Unicode UAX #31 and must be in
NFC; bidirectional control characters are rejected outside escapes in literals (E0001).

**syntax.comment.1** Comments (`// …` to the end of the line, and `/* … */`, which nest) and
whitespace separate tokens and have no other meaning.

**item.doc.1** A **doc comment** documents an item: `/// …` the item that follows, and `//! …`
the module or block it is in. Doc comments go where attributes do. For now the compiler reads
nothing in them: they mean nothing to the program, and `moddil doc` and doc tests come later.
