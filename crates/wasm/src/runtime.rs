//! The runtime module, `moddil-rt-wasm` built for wasm32 by the build script: its bytes, the
//! types of the functions it exports, and where its memory ends (`__heap_base`), above which the
//! program's begins.

use std::collections::HashMap;
use std::sync::OnceLock;

use wasm_encoder::ValType;
use wasmparser::{CompositeInnerType, ExternalKind, Operator, Payload, TypeRef};

/// The runtime module's bytes.
pub(crate) static BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/runtime.wasm"));

/// What the program needs to know of the runtime module.
pub(crate) struct Runtime {
    /// Each function it exports: its parameters' and results' types.
    pub(crate) funcs: HashMap<String, (Vec<ValType>, Vec<ValType>)>,
    /// Where its static memory ends.
    pub(crate) heap_base: u32,
}

/// The runtime module, read once.
pub(crate) fn runtime() -> Result<&'static Runtime, String> {
    static R: OnceLock<Result<Runtime, String>> = OnceLock::new();
    R.get_or_init(|| read(BYTES)).as_ref().map_err(Clone::clone)
}

fn val(t: wasmparser::ValType) -> Result<ValType, String> {
    Ok(match t {
        wasmparser::ValType::I32 => ValType::I32,
        wasmparser::ValType::I64 => ValType::I64,
        wasmparser::ValType::F32 => ValType::F32,
        wasmparser::ValType::F64 => ValType::F64,
        t => return Err(format!("a runtime function taking a `{t}`")),
    })
}

fn read(bytes: &[u8]) -> Result<Runtime, String> {
    let e = |e: wasmparser::BinaryReaderError| format!("the runtime module: {e}");
    let mut types = Vec::new();
    // Each function's type, imported ones first.
    let mut funcs = Vec::new();
    let mut globals = Vec::new();
    let mut exports = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        match payload.map_err(e)? {
            Payload::TypeSection(r) => {
                for rec in r {
                    for sub in rec.map_err(e)?.into_types() {
                        let CompositeInnerType::Func(f) = &sub.composite_type.inner else {
                            types.push(None);
                            continue;
                        };
                        let ps = f
                            .params()
                            .iter()
                            .map(|t| val(*t))
                            .collect::<Result<_, _>>()?;
                        let rs = f
                            .results()
                            .iter()
                            .map(|t| val(*t))
                            .collect::<Result<_, _>>()?;
                        types.push(Some((ps, rs)));
                    }
                }
            }
            Payload::ImportSection(r) => {
                for i in r {
                    if let TypeRef::Func(t) = i.map_err(e)?.ty {
                        funcs.push(t);
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    funcs.push(t.map_err(e)?);
                }
            }
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g.map_err(e)?;
                    let mut ops = g.init_expr.get_operators_reader();
                    globals.push(match ops.read().map_err(e)? {
                        Operator::I32Const { value } => Some(value as u32),
                        _ => None,
                    });
                }
            }
            Payload::ExportSection(r) => {
                for x in r {
                    let x = x.map_err(e)?;
                    exports.push((x.name.to_string(), x.kind, x.index));
                }
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    let mut heap_base = None;
    for (name, kind, index) in exports {
        match kind {
            ExternalKind::Func => {
                let t = funcs
                    .get(index as usize)
                    .and_then(|t| types.get(*t as usize))
                    .cloned()
                    .flatten()
                    .ok_or_else(|| format!("the runtime's `{name}`"))?;
                out.insert(name, t);
            }
            ExternalKind::Global if name == "__heap_base" => {
                heap_base = globals.get(index as usize).copied().flatten();
            }
            _ => {}
        }
    }
    Ok(Runtime {
        funcs: out,
        heap_base: heap_base.ok_or("the runtime's `__heap_base`")?,
    })
}
