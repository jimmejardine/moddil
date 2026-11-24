# moddil-show

Values shown as the interpreters write them (`Point { x: 1, y: 2 }`, `Some(3)`, `"text"`), from
their bytes and a description of their types: `Types`, a table of `Node`s, each a type's layout
as showing needs it, its parts by index (so a type can hold itself through a pointer).

**Status:** native code describes types from its layouts (`moddil-native`'s `show`) to show a
run's values; a built program holds the description of `main`'s result (`Types::encode`), which
`moddil-rt-main` decodes and shows it with.

## Rules

- **No compiler types here.** It depends on `moddil-rt` alone (for numbers' text), so the
  runtime library can show values without the item tree or the layouts.
