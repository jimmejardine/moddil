//! A text form of MIR, in the style of Rust's MIR dumps, for snapshot tests and debugging.

use std::fmt;

use crate::{
    AggregateKind, BinOp, Body, Const, Operand, Place, Projection, Rvalue, Statement, Terminator,
    UnOp,
};

impl fmt::Debug for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = format!("{:?}", self.local);
        for p in &self.projection {
            s = match p {
                Projection::Field(n) => format!("{s}.{n}"),
                Projection::Downcast(v) => format!("({s} as #{v})"),
                Projection::Deref => format!("(*{s})"),
                Projection::Index(i) => format!("{s}[{i:?}]"),
            };
        }
        f.write_str(&s)
    }
}

impl fmt::Debug for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Copy(l) => write!(f, "{l:?}"),
            Operand::Move(l) => write!(f, "move {l:?}"),
            Operand::Const(c) => write!(f, "{c:?}"),
        }
    }
}

impl fmt::Debug for Const {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Const::Int(v) => write!(f, "const {v}"),
            Const::Bool(v) => write!(f, "const {v}"),
            Const::Unit => write!(f, "const ()"),
            Const::Char(c) => write!(f, "const {c:?}"),
            Const::Str(i) => write!(f, "const str{i}"),
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Use(a) => write!(f, "{a:?}"),
            Rvalue::Unsize(a, t) => write!(f, "Unsize({a:?}, {t:?})"),
            Rvalue::Unary(UnOp::Neg, a) => write!(f, "Neg({a:?})"),
            Rvalue::Unary(UnOp::Not, a) => write!(f, "Not({a:?})"),
            Rvalue::Binary(op, a, b) => write!(f, "{op:?}({a:?}, {b:?})"),
            Rvalue::Overflows(op, a, b) => write!(f, "{op:?}Overflows({a:?}, {b:?})"),
            Rvalue::NegOverflows(a) => write!(f, "NegOverflows({a:?})"),
            Rvalue::Aggregate(kind, ops) => {
                let ops: Vec<String> = ops.iter().map(|o| format!("{o:?}")).collect();
                match kind {
                    AggregateKind::Tuple if ops.len() == 1 => write!(f, "({},)", ops[0]),
                    AggregateKind::Tuple => write!(f, "({})", ops.join(", ")),
                    AggregateKind::Array => write!(f, "[{}]", ops.join(", ")),
                    AggregateKind::Adt { path, .. } => write!(f, "{path}({})", ops.join(", ")),
                    AggregateKind::Closure(_) => write!(f, "{{closure}}({})", ops.join(", ")),
                }
            }
            Rvalue::Discriminant(p) => write!(f, "discriminant({p:?})"),
            Rvalue::ErrorIs(p, a) => write!(f, "error_is({p:?}, #{a})"),
            Rvalue::Ref(true, p) => write!(f, "&mut {p:?}"),
            Rvalue::Ref(false, p) => write!(f, "&{p:?}"),
            Rvalue::Repeat(a, n) => write!(f, "[{a:?}; {n:?}]"),
            Rvalue::Length(n) => write!(f, "length({n:?})"),
            Rvalue::InBounds(i, p) => write!(f, "in_bounds({i:?}, {p:?})"),
            Rvalue::StaticRef(n) => write!(f, "&static#{n}"),
        }
    }
}

impl BinOp {
    /// The operator's symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
        }
    }
}

/// ```text
/// fn add(_1: int, _2: int) -> int {
///     let _3: bool;
///
///     bb0: {
///         _3 = AddOverflows(_1, _2);
///         assert(!_3, overflow) @ 30..35;
///         _0 = Add(_1, _2);
///         return;
///     }
/// }
/// ```
impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = |i: usize| match &self.locals[i].name {
            Some(n) => format!(" // {n}"),
            None => String::new(),
        };
        let args: Vec<String> = (1..=self.arg_count)
            .map(|i| format!("_{i}: {:?}", self.locals[i].ty))
            .collect();
        writeln!(
            f,
            "fn {}({}) -> {:?} {{",
            self.name,
            args.join(", "),
            self.locals[0].ty
        )?;
        for i in 1..=self.arg_count {
            if let Some(n) = &self.locals[i].name {
                writeln!(f, "    // _{i}: {n}")?;
            }
        }
        for i in self.arg_count + 1..self.locals.len() - self.cleanup_locals {
            writeln!(f, "    let _{i}: {:?};{}", self.locals[i].ty, name(i))?;
        }
        // Cleanup blocks, which only unwinding reaches, are not shown.
        for (i, block) in self.blocks.iter().enumerate() {
            if block.cleanup {
                continue;
            }
            writeln!(f)?;
            writeln!(f, "    bb{i}: {{")?;
            for s in &block.statements {
                match s {
                    Statement::Assign(l, r) => writeln!(f, "        {l:?} = {r};")?,
                    Statement::Assert {
                        cond,
                        expected,
                        trap,
                        range,
                        parts,
                    } => {
                        let not = if *expected { "" } else { "!" };
                        let shown: Vec<String> = parts
                            .iter()
                            .map(|p| format!("{:?} if {:?}", p.value, p.evaluated))
                            .collect();
                        let shown = if shown.is_empty() {
                            String::new()
                        } else {
                            format!(" showing [{}]", shown.join(", "))
                        };
                        writeln!(
                            f,
                            "        assert({not}{cond:?}, {trap}) @ {range:?}{shown};"
                        )?;
                    }
                    Statement::Read(p) => writeln!(f, "        read({p:?});")?,
                    Statement::StorageDead(l) => writeln!(f, "        StorageDead({l:?});")?,
                    Statement::Observe { range, value } => {
                        writeln!(f, "        observe({value:?}) @ {range:?};")?;
                    }
                }
            }
            match &block.terminator {
                Terminator::Resume => writeln!(f, "        resume;")?,
                Terminator::Goto(t) => writeln!(f, "        goto -> {t:?};")?,
                Terminator::SwitchBool { cond, then, else_ } => writeln!(
                    f,
                    "        switch({cond:?}) -> [true: {then:?}, false: {else_:?}];"
                )?,
                Terminator::Call {
                    callee,
                    name,
                    args,
                    dest,
                    target,
                    on_error,
                    range,
                } => {
                    let args: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
                    let targs = callee.args();
                    let targs = if targs.is_empty() {
                        String::new()
                    } else {
                        let ts: Vec<String> = targs.iter().map(|t| format!("{t:?}")).collect();
                        format!("::<{}>", ts.join(", "))
                    };
                    let error = match on_error {
                        Some((p, b)) => format!(", error {p:?} -> {b:?}"),
                        None => String::new(),
                    };
                    writeln!(
                        f,
                        "        {dest:?} = {name}{targs}({}) -> {target:?}{error} @ {range:?};",
                        args.join(", ")
                    )?;
                }
                Terminator::Fail(e) => writeln!(f, "        fail({e:?});")?,
                Terminator::Trap(class, reason) => {
                    writeln!(f, "        trap({class}, {reason:?});")?;
                }
                Terminator::Drop {
                    place,
                    target,
                    glue_only,
                } => {
                    let what = if *glue_only { "drop_fields" } else { "drop" };
                    writeln!(f, "        {what}({place:?}) -> {target:?};")?;
                }
                Terminator::Return => writeln!(f, "        return;")?,
                Terminator::Unreachable => writeln!(f, "        unreachable;")?,
            }
            writeln!(f, "    }}")?;
        }
        writeln!(f, "}}")
    }
}
