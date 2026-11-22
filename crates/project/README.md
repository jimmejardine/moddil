# moddil-project

Reads a Moddil program from disk: a single `.moddil` file, or a mod's directory with its manifest,
`moddil.toml` ([`specs/modules.md`: The manifest](../../specs/modules.md#the-manifest)), its module
tree and the mods it depends on by path. The result is plain data (files with their mod and module
path, and mods with their dependencies) that the compiler and the reference interpreter both take.

**Status:** slices S6 and S9d: the manifest's subset of TOML (`[mod]`, with `stub = true` for a
stub mod, `[lib]`, `[[exe]]`,
`[dependencies]` by path); the module tree from the file tree, leaving out directories with a
manifest of their own; dependency cycles (E0705) and manifest problems (E0704), with the
manifest's text so they can be shown in it.

## Does

- `load(path)` returns a `Project` or the problems found.
- Shows paths relative to the root mod's directory, with `/`.

## Does not

- Resolve names, or check anything about the code: that is `moddil-sema`'s.
- Fetch dependencies, versions or registries (later slices and milestones).
