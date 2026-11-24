//! How C passes a struct by value (`ffi.types.2`), on the targets Moddil builds for: Windows
//! x64, SysV x86-64 (Linux and the BSDs) and AArch64 (Linux, and Apple's, which agrees for
//! structs). Native code and the MIR interpreter's trampolines both follow it, so that a call
//! passes a struct the way the C compiler on the other side expects.
//!
//! A struct is described by its [`CLayout`]: its size, its alignment, and its scalars by offset
//! (nested structs flattened). Its parts in registers are read from (and written to) a copy of
//! it padded to a multiple of 8 bytes, so a part may cover bytes past its end.

use cranelift_codegen::ir::types::{self, Type};

/// A scalar of a struct, as C lays it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    /// An integer, a `bool` or a pointer, of this many bytes.
    Int(u8),
    /// `float`.
    F32,
    /// `double`.
    F64,
}

/// A struct passed by value: its size, its alignment, and its scalars (offset and kind), in
/// order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CLayout {
    /// Its size in bytes.
    pub size: u32,
    /// Its alignment.
    pub align: u32,
    /// Its scalars, by offset, nested structs flattened.
    pub fields: Vec<(u32, Scalar)>,
}

/// What C makes of a value of a type `T`, as [`CLayout::flatten`] asks of it: a scalar, or a
/// struct's fields at their offsets.
pub enum Part<T> {
    /// One scalar.
    Scalar(Scalar),
    /// A struct: each field's offset and type.
    Fields(Vec<(u32, T)>),
}

impl CLayout {
    /// Its size rounded up to a multiple of 8: the room its padded copy takes.
    pub fn padded(&self) -> u32 {
        self.size.div_ceil(8) * 8
    }

    /// The layout of struct `ty`, of `size` bytes aligned to `align`: its scalars by offset,
    /// nested structs flattened. `part` says what each type is, as the caller lays types out.
    ///
    /// # Errors
    ///
    /// What `part` gives for a type it cannot say.
    pub fn flatten<T>(
        size: u32,
        align: u32,
        ty: &T,
        mut part: impl FnMut(&T) -> Result<Part<T>, String>,
    ) -> Result<CLayout, String> {
        fn walk<T>(
            ty: &T,
            at: u32,
            part: &mut impl FnMut(&T) -> Result<Part<T>, String>,
            out: &mut Vec<(u32, Scalar)>,
        ) -> Result<(), String> {
            match part(ty)? {
                Part::Scalar(s) => out.push((at, s)),
                Part::Fields(fs) => {
                    for (off, t) in &fs {
                        walk(t, at + off, part, out)?;
                    }
                }
            }
            Ok(())
        }
        let mut fields = Vec::new();
        walk(ty, 0, &mut part, &mut fields)?;
        Ok(CLayout {
            size,
            align,
            fields,
        })
    }
}

/// How a struct argument crosses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// In registers: each part a Cranelift type, read from the struct at an offset.
    Parts(Vec<(u32, Type)>),
    /// As a pointer to a copy the caller makes (Windows x64, and AArch64 above 16 bytes).
    Pointer,
    /// In the stack's argument area (SysV x86-64 above 16 bytes): Cranelift's
    /// `ArgumentPurpose::StructArgument`.
    Stack,
}

/// How a struct result crosses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ret {
    /// In registers, as [`Arg::Parts`].
    Parts(Vec<(u32, Type)>),
    /// Written by the callee where a hidden pointer argument says (Cranelift's
    /// `ArgumentPurpose::StructReturn`).
    Indirect,
}

/// The C calling conventions there are rules for here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Windows on x86-64.
    Win64,
    /// The System V ABI on x86-64: Linux, the BSDs.
    SysV,
    /// AArch64: Linux's, and Apple's.
    AArch64,
}

/// The target this was built for.
pub fn target() -> Target {
    if cfg!(target_arch = "aarch64") {
        Target::AArch64
    } else if cfg!(windows) {
        Target::Win64
    } else {
        Target::SysV
    }
}

/// A homogeneous float aggregate (AArch64): one to four floats of one kind, and nothing else.
fn hfa(l: &CLayout) -> Option<Type> {
    let first = l.fields.first()?.1;
    let t = match first {
        Scalar::F32 => types::F32,
        Scalar::F64 => types::F64,
        Scalar::Int(_) => return None,
    };
    (l.fields.len() <= 4 && l.fields.iter().all(|(_, s)| *s == first)).then_some(t)
}

/// The eightbytes of a struct of up to 16 bytes (SysV x86-64): each in an integer register if
/// any integer is in it, in a vector register if only floats are.
fn eightbytes(l: &CLayout) -> Vec<(u32, Type)> {
    (0..l.size.div_ceil(8))
        .map(|k| {
            let int = l
                .fields
                .iter()
                .any(|(o, s)| o / 8 == k && matches!(s, Scalar::Int(_)));
            (k * 8, if int { types::I64 } else { types::F64 })
        })
        .collect()
}

/// Up to 16 bytes in one or two integer registers.
fn words(l: &CLayout) -> Vec<(u32, Type)> {
    (0..l.size.div_ceil(8))
        .map(|k| (k * 8, types::I64))
        .collect()
}

/// How a struct of layout `l` is passed as an argument on target `t`.
pub fn arg(t: Target, l: &CLayout) -> Arg {
    match t {
        Target::Win64 => match l.size {
            1 | 2 | 4 | 8 => Arg::Parts(vec![(0, int(l.size))]),
            _ => Arg::Pointer,
        },
        Target::SysV if l.size > 16 => Arg::Stack,
        Target::SysV => Arg::Parts(eightbytes(l)),
        Target::AArch64 => match hfa(l) {
            Some(f) => Arg::Parts(l.fields.iter().map(|(o, _)| (*o, f)).collect()),
            None if l.size > 16 => Arg::Pointer,
            None => Arg::Parts(words(l)),
        },
    }
}

/// How a struct of layout `l` is given as a result on target `t`.
pub fn ret(t: Target, l: &CLayout) -> Ret {
    match t {
        Target::Win64 => match l.size {
            1 | 2 | 4 | 8 => Ret::Parts(vec![(0, int(l.size))]),
            _ => Ret::Indirect,
        },
        Target::SysV if l.size > 16 => Ret::Indirect,
        Target::SysV => Ret::Parts(eightbytes(l)),
        Target::AArch64 => match hfa(l) {
            Some(f) => Ret::Parts(l.fields.iter().map(|(o, _)| (*o, f)).collect()),
            None if l.size > 16 => Ret::Indirect,
            None => Ret::Parts(words(l)),
        },
    }
}

/// The integer type of `bytes` bytes.
fn int(bytes: u32) -> Type {
    match bytes {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => types::I64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(size: u32, fields: &[(u32, Scalar)]) -> CLayout {
        CLayout {
            size,
            align: 8,
            fields: fields.to_vec(),
        }
    }

    #[test]
    fn nested_structs_flatten_to_their_scalars() {
        // `struct In { a: u8, b: f32 }` at 4 of `struct Out { n: i32, i: In, d: f64 }`.
        enum T {
            Out,
            In,
            I32,
            U8,
            F32,
            F64,
        }
        let l = CLayout::flatten(24, 8, &T::Out, |t| {
            Ok(match t {
                T::Out => Part::Fields(vec![(0, T::I32), (4, T::In), (16, T::F64)]),
                T::In => Part::Fields(vec![(0, T::U8), (4, T::F32)]),
                T::I32 => Part::Scalar(Scalar::Int(4)),
                T::U8 => Part::Scalar(Scalar::Int(1)),
                T::F32 => Part::Scalar(Scalar::F32),
                T::F64 => Part::Scalar(Scalar::F64),
            })
        })
        .expect("a layout");
        assert_eq!(
            l.fields,
            vec![
                (0, Scalar::Int(4)),
                (4, Scalar::Int(1)),
                (8, Scalar::F32),
                (16, Scalar::F64)
            ]
        );
        assert_eq!((l.size, l.align), (24, 8));
    }

    #[test]
    fn each_target_classifies_as_its_c_compiler_does() {
        let pair = layout(8, &[(0, Scalar::Int(4)), (4, Scalar::Int(4))]);
        let floats = layout(8, &[(0, Scalar::F32), (4, Scalar::F32)]);
        let mixed = layout(16, &[(0, Scalar::F64), (8, Scalar::Int(4))]);
        let big = layout(
            24,
            &[
                (0, Scalar::Int(8)),
                (8, Scalar::Int(8)),
                (16, Scalar::Int(8)),
            ],
        );
        assert_eq!(arg(Target::Win64, &pair), Arg::Parts(vec![(0, types::I64)]));
        assert_eq!(arg(Target::Win64, &mixed), Arg::Pointer);
        assert_eq!(ret(Target::Win64, &mixed), Ret::Indirect);
        assert_eq!(
            arg(Target::SysV, &floats),
            Arg::Parts(vec![(0, types::F64)])
        );
        assert_eq!(
            arg(Target::SysV, &mixed),
            Arg::Parts(vec![(0, types::F64), (8, types::I64)])
        );
        assert_eq!(arg(Target::SysV, &big), Arg::Stack);
        assert_eq!(ret(Target::SysV, &big), Ret::Indirect);
        assert_eq!(
            arg(Target::AArch64, &floats),
            Arg::Parts(vec![(0, types::F32), (4, types::F32)])
        );
        assert_eq!(
            arg(Target::AArch64, &mixed),
            Arg::Parts(vec![(0, types::I64), (8, types::I64)])
        );
        assert_eq!(arg(Target::AArch64, &big), Arg::Pointer);
        assert_eq!(ret(Target::AArch64, &big), Ret::Indirect);
    }
}
