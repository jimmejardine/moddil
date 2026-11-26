//! Types.

use std::fmt;
use std::sync::Arc;

/// A type: the built-in scalars, tuples, borrows, structs and enums with their type arguments,
/// type parameters and associated types.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `int`: a signed 64-bit integer.
    Int,
    /// A sized integer or a float (`num.types.1`): `i8`, …, `u128`, `f32`, `f64`. `int` is
    /// [`Ty::Int`], also written `i64`.
    Num(Num),
    /// `bool`.
    Bool,
    /// `char`: a Unicode scalar value.
    Char,
    /// `str`: a run of valid UTF-8, with no size of its own, so only behind a borrow (`&str`).
    Str,
    /// `()`: the empty tuple.
    Unit,
    /// `!`: the type of expressions that never produce a value, such as `return`.
    Never,
    /// A tuple of one or more elements: `(int, bool)`, `(int,)`.
    Tuple(Arc<[Ty]>),
    /// An array, `[T; N]`: its element type and its length, which is a [`Ty::Len`], a const
    /// parameter, or a length being inferred.
    Array(Arc<Ty>, Arc<Ty>),
    /// A slice, `[T]`: a run of elements with no size of its own, so only behind a borrow.
    Slice(Arc<Ty>),
    /// `Buf<T>`: the built-in storage `Vec` is written over, a fixed capacity of which a prefix
    /// holds elements.
    Buf(Arc<Ty>),
    /// A length known at compile time, as the length of an array type or the argument of a const
    /// parameter.
    Len(i64),
    /// A struct or enum, by its index in the item tree, with its name and type arguments.
    Adt(u32, Arc<str>, Arc<[Ty]>),
    /// A refined type (`refine.1`), by its index among the `type` items, with its name. Its values
    /// are its base's that its predicate holds for.
    Refined(u32, Arc<str>),
    /// A borrow: `&T` (shared) or `&mut T` (exclusive).
    Ref(bool, Arc<Ty>),
    /// A raw pointer (`type.ptr.1`): `*const T` or `*mut T`. It carries no borrow, so nothing
    /// checks what it points to; `*p` is allowed only in an `unsafe` block.
    Ptr(bool, Arc<Ty>),
    /// A C function pointer (`type.fnptr.1`): `fn(A, B) -> R`, its parameters' types and its
    /// result's. Its values are C functions' addresses, and those of the C-ABI wrappers of the
    /// Moddil functions named where one is wanted.
    FnPtr(Arc<[Ty]>, Arc<Ty>),
    /// `dyn Trait`: a value of some type that implements the trait, known only through it. It
    /// has no size, so it is only ever behind a borrow (`&dyn Trait`).
    /// `dyn Trait`, and for a closure trait the associated types of `FnOnce` it fixes, by
    /// index (`Output`, `Fails`): `dyn Fn(int) -> int with fail(E)`.
    Dyn(TraitRef, Arc<[(u32, Ty)]>),
    /// A closure's own type: one per closure expression. It cannot be written; it implements the
    /// closure traits (`FnOnce` and, by its kind, `FnMut` and `Fn`).
    Closure(Arc<ClosureTy>),
    /// An anonymous impl's own type (`impl Time { … }` as an expression): it implements the trait,
    /// each method a closure.
    Anon(Arc<AnonTy>),
    /// `impl Trait` as a function's result type (`type.impl.ret.1`): known to the function's
    /// callers only through its bounds; the function's body fixes which type it is (its hidden
    /// type).
    Opaque(Arc<OpaqueTy>),
    /// A type parameter of the item being checked, by its index in the item's generics (for a
    /// trait's items, `Self` is parameter 0), with its name.
    Param(u32, Arc<str>),
    /// An associated type that cannot be resolved yet: `T::Output` for a type parameter `T`.
    Assoc(Arc<AssocTy>),
    /// An error set of two or more error types (structs and enums), in a canonical order: the
    /// error of a `Result` or of a `fail(…)` that can be any of them (`A | B`). Make one with
    /// [`Ty::errors`].
    Errors(Arc<[Ty]>),
    /// A type being inferred, by its index in the checker's table (or [`Ty::HOLE`] in HIR: a
    /// type left for inference, such as the arguments of `Pair::new`).
    Infer(u32),
    /// The type of an expression that already has an error; it matches everything, so one error
    /// is reported once.
    Error,
}

/// Where a `dyn` closure type keeps its control effects, as a `Len` of their bits, among its
/// fixed associated types: past `FnOnce`'s own (`Output`, `Fails`).
pub(crate) const DYN_CONTROL: u32 = 101;

/// A closure's type: where it is written, and what checking it found.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ClosureTy {
    /// The function or test whose body it is written in.
    pub owner: crate::Item,
    /// Its expression in that body, by index.
    pub expr: u32,
    /// The type arguments of the owner's type parameters, which its types may name.
    pub args: Arc<[Ty]>,
    /// Its parameters' types.
    pub params: Arc<[Ty]>,
    /// Its result type.
    pub ret: Ty,
    /// The errors it can fail with, as an error set (`!` if none).
    pub fails: Ty,
    /// Which closure traits it implements.
    pub kind: ClosureKind,
    /// The types of what it captures, in order: a borrow of each place it borrows, and each value
    /// it moves in, then the handlers it uses.
    pub captures: Arc<[Ty]>,
}

/// An anonymous impl's type: where it is written, its trait, and its methods' closures.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AnonTy {
    /// The function or test whose body it is written in.
    pub owner: crate::Item,
    /// Its expression in that body, by index.
    pub expr: u32,
    /// The trait it implements.
    pub trait_ref: TraitRef,
    /// Its methods: each the trait's method it defines, and the closure type of its body.
    pub methods: Arc<[(u32, Ty)]>,
}

/// The sized integer and float types (`num.types.1`). `int` (`i64`) is [`Ty::Int`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Num {
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i128`.
    I128,
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`.
    U64,
    /// `u128`.
    U128,
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// `isize`: a signed integer as wide as a pointer (`num.types.2`).
    Isize,
    /// `usize`: an unsigned integer as wide as a pointer (`num.types.2`).
    Usize,
}

impl Num {
    /// Every one, in order.
    pub const ALL: [Num; 13] = [
        Num::I8,
        Num::I16,
        Num::I32,
        Num::I128,
        Num::U8,
        Num::U16,
        Num::U32,
        Num::U64,
        Num::U128,
        Num::F32,
        Num::F64,
        Num::Isize,
        Num::Usize,
    ];

    /// How it is written.
    pub fn name(self) -> &'static str {
        match self {
            Num::I8 => "i8",
            Num::I16 => "i16",
            Num::I32 => "i32",
            Num::I128 => "i128",
            Num::U8 => "u8",
            Num::U16 => "u16",
            Num::U32 => "u32",
            Num::U64 => "u64",
            Num::U128 => "u128",
            Num::F32 => "f32",
            Num::F64 => "f64",
            Num::Isize => "isize",
            Num::Usize => "usize",
        }
    }

    /// The type a name stands for (`i64` is `int`, so not here).
    pub fn from_name(name: &str) -> Option<Num> {
        Num::ALL.into_iter().find(|n| n.name() == name)
    }

    /// Whether it is `f32` or `f64`.
    pub fn is_float(self) -> bool {
        matches!(self, Num::F32 | Num::F64)
    }

    /// Whether it is a signed integer.
    pub fn is_signed(self) -> bool {
        matches!(self, Num::I8 | Num::I16 | Num::I32 | Num::I128 | Num::Isize)
    }

    /// Whether it is `usize` or `isize`, as wide as a pointer on the target (`num.types.2`).
    pub fn is_pointer_sized(self) -> bool {
        matches!(self, Num::Isize | Num::Usize)
    }

    /// Its width in bits: a pointer's is 64 on every target of this edition.
    pub fn bits(self) -> u32 {
        match self {
            Num::I8 | Num::U8 => 8,
            Num::I16 | Num::U16 => 16,
            Num::I32 | Num::U32 | Num::F32 => 32,
            Num::U64 | Num::F64 | Num::Isize | Num::Usize => 64,
            Num::I128 | Num::U128 => 128,
        }
    }

    /// The least and greatest values of an integer type, as far as an `i128` reaches.
    pub fn int_range(self) -> (i128, i128) {
        match self {
            Num::I8 => (i8::MIN.into(), i8::MAX.into()),
            Num::I16 => (i16::MIN.into(), i16::MAX.into()),
            Num::I32 => (i32::MIN.into(), i32::MAX.into()),
            Num::I128 => (i128::MIN, i128::MAX),
            Num::U8 => (0, u8::MAX.into()),
            Num::U16 => (0, u16::MAX.into()),
            Num::U32 => (0, u32::MAX.into()),
            Num::U64 | Num::Usize => (0, u64::MAX.into()),
            Num::Isize => (i64::MIN.into(), i64::MAX.into()),
            Num::U128 | Num::F32 | Num::F64 => (0, i128::MAX),
        }
    }
}

/// An opaque result type: the function whose result it is, the type arguments of its call
/// (for its type parameters), and how it is written (`impl Handler`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct OpaqueTy {
    /// The function.
    pub func: u32,
    /// The type arguments for the function's type parameters.
    pub args: Arc<[Ty]>,
    /// How it is written.
    pub name: Arc<str>,
}

/// Which closure traits a closure implements: every one does `FnOnce`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClosureKind {
    /// `Fn`, `FnMut` and `FnOnce`: it only reads what it captures.
    Fn,
    /// `FnMut` and `FnOnce`: it changes what it captures.
    FnMut,
    /// Only `FnOnce`: it moves out what it captures.
    FnOnce,
}

impl ClosureTy {
    /// The closure type with `f` applied to each type in it.
    #[must_use]
    pub fn map(&self, f: &mut impl FnMut(&Ty) -> Ty) -> ClosureTy {
        ClosureTy {
            owner: self.owner,
            expr: self.expr,
            args: self.args.iter().map(&mut *f).collect(),
            params: self.params.iter().map(&mut *f).collect(),
            ret: f(&self.ret),
            fails: f(&self.fails),
            kind: self.kind,
            captures: self.captures.iter().map(&mut *f).collect(),
        }
    }
}

/// An associated type of a trait, for some type: `<T as Add>::Output`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AssocTy {
    /// The type the trait is implemented for.
    pub self_ty: Ty,
    /// The trait, with its arguments.
    pub trait_ref: TraitRef,
    /// The associated type's index among the trait's.
    pub index: u32,
    /// Its name.
    pub name: Arc<str>,
}

/// A trait with its type arguments: `PartialEq`, `From<int>`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TraitRef {
    /// The trait, by its index in the item tree.
    pub trait_: u32,
    /// Its name.
    pub name: Arc<str>,
    /// Its type arguments (not including `Self`).
    pub args: Arc<[Ty]>,
}

impl TraitRef {
    /// `self` with type parameters replaced by `args`.
    #[must_use]
    pub fn subst(&self, args: &[Ty]) -> TraitRef {
        TraitRef {
            trait_: self.trait_,
            name: self.name.clone(),
            args: self.args.iter().map(|t| t.subst(args)).collect(),
        }
    }

    /// `self` with `f` applied to each argument.
    #[must_use]
    pub fn map(&self, f: &mut impl FnMut(&Ty) -> Ty) -> TraitRef {
        TraitRef {
            trait_: self.trait_,
            name: self.name.clone(),
            args: self.args.iter().map(&mut *f).collect(),
        }
    }
}

impl Ty {
    /// The placeholder for "infer this type" in HIR.
    pub const HOLE: u32 = u32::MAX;

    /// Whether a value of type `self` can be used where `expected` is wanted, for types without
    /// inference variables.
    pub fn fits(&self, expected: &Ty) -> bool {
        match (self, expected) {
            // `&mut T` can be used where `&T` is wanted, and `*mut T` where `*const T` is.
            (Ty::Ref(true, a), Ty::Ref(false, b)) | (Ty::Ptr(true, a), Ty::Ptr(false, b)) => {
                a.fits(b) && b.fits(a)
            }
            _ => {
                self == expected || matches!(self, Ty::Never | Ty::Error) || *expected == Ty::Error
            }
        }
    }

    /// Whether the type is or contains an error, so that errors about it would be noise.
    pub fn has_error(&self) -> bool {
        self.any(&|t| matches!(t, Ty::Error))
    }

    /// Whether the type is or contains an inference variable.
    pub fn has_infer(&self) -> bool {
        self.any(&|t| matches!(t, Ty::Infer(_)))
    }

    /// Whether the type is or contains a type parameter or an unresolved associated type.
    pub fn has_param(&self) -> bool {
        self.any(&|t| matches!(t, Ty::Param(..) | Ty::Assoc(_)))
    }

    /// Whether `f` holds for the type or a type inside it.
    pub fn any(&self, f: &impl Fn(&Ty) -> bool) -> bool {
        if f(self) {
            return true;
        }
        match self {
            Ty::Tuple(ts) | Ty::Adt(_, _, ts) | Ty::Errors(ts) => ts.iter().any(|t| t.any(f)),
            Ty::Ref(_, t) | Ty::Ptr(_, t) | Ty::Slice(t) | Ty::Buf(t) => t.any(f),
            Ty::FnPtr(ps, r) => ps.iter().any(|t| t.any(f)) || r.any(f),
            Ty::Array(t, n) => t.any(f) || n.any(f),
            Ty::Dyn(tr, fixed) => {
                tr.args.iter().any(|t| t.any(f)) || fixed.iter().any(|(_, t)| t.any(f))
            }
            Ty::Anon(a) => {
                a.trait_ref.args.iter().any(|t| t.any(f)) || a.methods.iter().any(|(_, t)| t.any(f))
            }
            Ty::Closure(c) => {
                c.args.iter().any(|t| t.any(f))
                    || c.params.iter().any(|t| t.any(f))
                    || c.ret.any(f)
                    || c.fails.any(f)
                    || c.captures.iter().any(|t| t.any(f))
            }
            Ty::Assoc(a) => a.self_ty.any(f) || a.trait_ref.args.iter().any(|t| t.any(f)),
            Ty::Opaque(o) => o.args.iter().any(|t| t.any(f)),
            _ => false,
        }
    }

    /// The type with `f` applied to each type directly inside it.
    #[must_use]
    pub fn map(&self, f: &mut impl FnMut(&Ty) -> Ty) -> Ty {
        match self {
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(&mut *f).collect()),
            Ty::Errors(ts) => Ty::errors(ts.iter().map(&mut *f).collect()),
            Ty::Adt(a, n, ts) => Ty::Adt(*a, n.clone(), ts.iter().map(&mut *f).collect()),
            Ty::Ref(m, t) => Ty::Ref(*m, Arc::new(f(t))),
            Ty::Ptr(m, t) => Ty::Ptr(*m, Arc::new(f(t))),
            Ty::FnPtr(ps, r) => Ty::FnPtr(ps.iter().map(&mut *f).collect(), Arc::new(f(r))),
            Ty::Array(t, n) => Ty::Array(Arc::new(f(t)), Arc::new(f(n))),
            Ty::Slice(t) => Ty::Slice(Arc::new(f(t))),
            Ty::Buf(t) => Ty::Buf(Arc::new(f(t))),
            Ty::Dyn(tr, fixed) => {
                Ty::Dyn(tr.map(f), fixed.iter().map(|(i, t)| (*i, f(t))).collect())
            }
            Ty::Closure(c) => Ty::Closure(Arc::new(c.map(f))),
            Ty::Opaque(o) => Ty::Opaque(Arc::new(OpaqueTy {
                func: o.func,
                args: o.args.iter().map(&mut *f).collect(),
                name: o.name.clone(),
            })),
            Ty::Anon(a) => Ty::Anon(Arc::new(AnonTy {
                owner: a.owner,
                expr: a.expr,
                trait_ref: a.trait_ref.map(f),
                methods: a.methods.iter().map(|(m, t)| (*m, f(t))).collect(),
            })),
            Ty::Assoc(a) => Ty::Assoc(Arc::new(AssocTy {
                self_ty: f(&a.self_ty),
                trait_ref: a.trait_ref.map(f),
                index: a.index,
                name: a.name.clone(),
            })),
            t => t.clone(),
        }
    }

    /// The type with each type parameter `i` replaced by `args[i]` (parameters past the end of
    /// `args` stay as they are).
    #[must_use]
    pub fn subst(&self, args: &[Ty]) -> Ty {
        match self {
            Ty::Param(i, _) => args
                .get(*i as usize)
                .cloned()
                .unwrap_or_else(|| self.clone()),
            t => t.map(&mut |x| x.subst(args)),
        }
    }

    /// The tuple type of `elements` (`()` when there are none).
    pub fn tuple(elements: Vec<Ty>) -> Ty {
        if elements.is_empty() {
            Ty::Unit
        } else {
            Ty::Tuple(elements.into())
        }
    }

    /// The control effects a stored closure's call has (`dyn Fn() with suspend`).
    pub fn dyn_control(&self) -> u8 {
        match self {
            Ty::Dyn(_, fixed) => fixed
                .iter()
                .find_map(|(i, t)| match t {
                    Ty::Len(c) if *i == DYN_CONTROL => u8::try_from(*c).ok(),
                    _ => None,
                })
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// `dyn tr`, fixing no associated types.
    pub fn dyn_of(tr: TraitRef) -> Ty {
        Ty::Dyn(tr, Arc::new([]))
    }

    /// The error set of `members` (error types, or error sets, which are flattened): `!` when
    /// there are none, the type itself when there is one.
    pub fn errors(members: Vec<Ty>) -> Ty {
        let mut flat: Vec<Ty> = Vec::new();
        for m in members {
            for t in m.members() {
                if !flat.contains(&t) {
                    flat.push(t);
                }
            }
        }
        flat.sort_by_key(|t| format!("{t:?}"));
        match flat.len() {
            0 => Ty::Never,
            1 => flat.pop().expect("one"),
            _ => Ty::Errors(flat.into()),
        }
    }

    /// The error types an error set is made of: its members, none for `!`, or the type itself.
    pub fn members(&self) -> Vec<Ty> {
        match self {
            Ty::Errors(ts) => ts.to_vec(),
            Ty::Never => Vec::new(),
            t => vec![t.clone()],
        }
    }

    /// A borrow of `t`.
    pub fn reference(mutable: bool, t: Ty) -> Ty {
        Ty::Ref(mutable, Arc::new(t))
    }

    /// The type a borrow points to, or the type itself.
    pub fn strip_ref(&self) -> &Ty {
        match self {
            Ty::Ref(_, t) => t,
            t => t,
        }
    }

    /// Whether the type is or contains a borrow anywhere, including in type arguments. A closure
    /// borrows what it captures, so it counts as one.
    pub fn has_ref_inside(&self) -> bool {
        self.any(&|t| matches!(t, Ty::Ref(..) | Ty::Closure(_) | Ty::Anon(_)))
    }

    /// Whether the type is or contains a borrow (a closure counts as one).
    pub fn has_ref(&self) -> bool {
        match self {
            Ty::Ref(..) | Ty::Closure(_) | Ty::Anon(_) => true,
            Ty::Tuple(ts) => ts.iter().any(Ty::has_ref),
            Ty::Array(t, _) => t.has_ref(),
            _ => false,
        }
    }

    /// An array type of `n` elements of type `t`.
    pub fn array(t: Ty, n: Ty) -> Ty {
        Ty::Array(Arc::new(t), Arc::new(n))
    }

    /// The slice type of elements of type `t`.
    pub fn slice(t: Ty) -> Ty {
        Ty::Slice(Arc::new(t))
    }

    /// The struct or enum's index in the item tree, if it is one.
    pub fn adt(&self) -> Option<u32> {
        match self {
            Ty::Adt(i, ..) => Some(*i),
            _ => None,
        }
    }

    /// The type arguments of a struct or enum (none for other types).
    pub fn args(&self) -> &[Ty] {
        match self {
            Ty::Adt(_, _, args) => args,
            _ => &[],
        }
    }
}

fn list(f: &mut fmt::Formatter<'_>, ts: &[Ty]) -> fmt::Result {
    for (i, t) in ts.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{t:?}")?;
    }
    Ok(())
}

impl fmt::Debug for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int => f.write_str("int"),
            Ty::Num(n) => f.write_str(n.name()),
            Ty::Bool => f.write_str("bool"),
            Ty::Char => f.write_str("char"),
            Ty::Str => f.write_str("str"),
            Ty::Unit => f.write_str("()"),
            Ty::Never => f.write_str("!"),
            Ty::Error => f.write_str("{error}"),
            Ty::Infer(_) => f.write_str("_"),
            Ty::Param(_, name) | Ty::Refined(_, name) => f.write_str(name),
            Ty::Adt(_, name, args) => {
                f.write_str(name)?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    list(f, args)?;
                    f.write_str(">")?;
                }
                Ok(())
            }
            Ty::Assoc(a) => match &a.self_ty {
                Ty::Param(..) => write!(f, "{:?}::{}", a.self_ty, a.name),
                t => write!(f, "<{t:?} as {:?}>::{}", a.trait_ref, a.name),
            },
            Ty::Errors(ts) => {
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{t:?}")?;
                }
                Ok(())
            }
            Ty::Dyn(tr, _) => write!(f, "dyn {tr:?}"),
            Ty::Anon(a) => write!(f, "{{impl {:?}}}", a.trait_ref),
            Ty::Opaque(o) => f.write_str(&o.name),
            Ty::Closure(c) => {
                f.write_str("{closure(")?;
                list(f, &c.params)?;
                write!(f, ") -> {:?}}}", c.ret)
            }
            Ty::Array(t, n) => write!(f, "[{t:?}; {n:?}]"),
            Ty::Slice(t) => write!(f, "[{t:?}]"),
            Ty::Buf(t) => write!(f, "Buf<{t:?}>"),
            Ty::Len(n) => write!(f, "{n}"),
            Ty::Ref(true, t) => write!(f, "&mut {t:?}"),
            Ty::Ref(false, t) => write!(f, "&{t:?}"),
            Ty::Ptr(true, t) => write!(f, "*mut {t:?}"),
            Ty::Ptr(false, t) => write!(f, "*const {t:?}"),
            Ty::FnPtr(ps, r) => {
                f.write_str("fn(")?;
                list(f, ps)?;
                f.write_str(")")?;
                if **r != Ty::Unit {
                    write!(f, " -> {r:?}")?;
                }
                Ok(())
            }
            Ty::Tuple(ts) => {
                f.write_str("(")?;
                list(f, ts)?;
                if ts.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl fmt::Debug for TraitRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if !self.args.is_empty() {
            f.write_str("<")?;
            list(f, &self.args)?;
            f.write_str(">")?;
        }
        Ok(())
    }
}

impl fmt::Display for TraitRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
