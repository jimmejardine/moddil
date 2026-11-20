//! A text form of MIR, in the style of Rust's MIR dumps, for snapshot tests and debugging.

use std::fmt;

use crate::{BinOp, Body, Const, Operand, Rvalue, Statement, Terminator, UnOp};

impl fmt::Debug for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Copy(l) => write!(f, "{l:?}"),
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
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Use(a) => write!(f, "{a:?}"),
            Rvalue::Unary(UnOp::Neg, a) => write!(f, "Neg({a:?})"),
            Rvalue::Unary(UnOp::Not, a) => write!(f, "Not({a:?})"),
            Rvalue::Binary(op, a, b) => write!(f, "{op:?}({a:?}, {b:?})"),
            Rvalue::Overflows(op, a, b) => write!(f, "{op:?}Overflows({a:?}, {b:?})"),
            Rvalue::NegOverflows(a) => write!(f, "NegOverflows({a:?})"),
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
        for i in self.arg_count + 1..self.locals.len() {
            writeln!(f, "    let _{i}: {:?};{}", self.locals[i].ty, name(i))?;
        }
        for (i, block) in self.blocks.iter().enumerate() {
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
                    } => {
                        let not = if *expected { "" } else { "!" };
                        writeln!(f, "        assert({not}{cond:?}, {trap}) @ {range:?};")?;
                    }
                    Statement::Observe { range, value } => {
                        writeln!(f, "        observe({value:?}) @ {range:?};")?;
                    }
                }
            }
            match &block.terminator {
                Terminator::Goto(t) => writeln!(f, "        goto -> {t:?};")?,
                Terminator::SwitchBool { cond, then, else_ } => writeln!(
                    f,
                    "        switch({cond:?}) -> [true: {then:?}, false: {else_:?}];"
                )?,
                Terminator::Call {
                    func,
                    args,
                    dest,
                    target,
                    range,
                } => {
                    let args: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
                    writeln!(
                        f,
                        "        {dest:?} = fn#{func}({}) -> {target:?} @ {range:?};",
                        args.join(", ")
                    )?;
                }
                Terminator::Return => writeln!(f, "        return;")?,
                Terminator::Unreachable => writeln!(f, "        unreachable;")?,
            }
            writeln!(f, "    }}")?;
        }
        writeln!(f, "}}")
    }
}
