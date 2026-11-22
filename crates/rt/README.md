# moddil-rt

The runtime that native Moddil code calls into ([`specs/roadmap.md`: M1](../../specs/roadmap.md)).
Slice C0 has the functions JIT code calls when a check fails (`trap`) and when a test observes a
value (`observe`), and the float remainder Cranelift has no instruction for. Each is a plain Rust
function whose address `moddil-native` gives the JIT; what they record is kept per thread, and
the host takes it once a run ends.
