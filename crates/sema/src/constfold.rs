//! Const expressions in types (`generic.const.3`): an array's length or a const argument
//! written as an `int` const item's name, or as arithmetic on literals and such names, folded
//! from the syntax when the program is collected, before any body is checked.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

/// What a name in a const expression stands for: a const item's value, if it is one whose
/// value folds, `None` if it is not.
pub(crate) type Names<'a> = dyn FnMut(&[String]) -> Option<Result<i64, String>> + 'a;

/// The value of const expression `e`, or why it has none: a literal, a const item's name,
/// `-e`, `(e)`, `{ e }`, and `+ - * / % << >> & | ^` on them, without overflow.
pub(crate) fn fold(e: &ast::Expr, names: &mut Names<'_>) -> Result<i64, String> {
    let node = e.syntax();
    match e {
        ast::Expr::Literal(_) => {
            let tok = node
                .child_tokens()
                .find(|t| t.kind() == K::INTEGER)
                .ok_or("only an integer is a length")?;
            ast::int_value(tok.text())
                .and_then(|v| i64::try_from(v).ok())
                .ok_or_else(|| "this does not fit in `int`".to_string())
        }
        ast::Expr::Paren(_) => fold(&only(node)?, names),
        ast::Expr::Block(_) => fold_block(node, names),
        ast::Expr::Path(_) => {
            let segs: Vec<String> = node
                .descendants()
                .into_iter()
                .filter(|n| n.kind() == K::PATH_SEGMENT)
                .filter_map(|s| s.token(K::IDENT).map(|t| t.text().to_string()))
                .collect();
            names(&segs)
                .unwrap_or_else(|| Err(format!("`{}` is not an `int` const", segs.join("::"))))
        }
        ast::Expr::Prefix(_) if ast::operator(node).is_some_and(|t| t.kind() == K::MINUS) => {
            fold(&only(node)?, names)?
                .checked_neg()
                .ok_or_else(overflow)
        }
        ast::Expr::Bin(_) => {
            let parts = e.sub_exprs();
            let [l, r] = parts.as_slice() else {
                return Err("an operation of two values".to_string());
            };
            let op = ast::operator(node).map(|t| t.kind()).ok_or("an operator")?;
            let (a, b) = (fold(l, names)?, fold(r, names)?);
            let v = match op {
                K::PLUS => a.checked_add(b),
                K::MINUS => a.checked_sub(b),
                K::STAR => a.checked_mul(b),
                K::SLASH => a.checked_div(b),
                K::PERCENT => a.checked_rem(b),
                K::LT_LT => u32::try_from(b).ok().and_then(|b| a.checked_shl(b)),
                K::GT_GT => u32::try_from(b).ok().and_then(|b| a.checked_shr(b)),
                K::AMP => Some(a & b),
                K::PIPE => Some(a | b),
                K::CARET => Some(a ^ b),
                _ => return Err("only `+ - * / % << >> & | ^` fold in a type".to_string()),
            };
            v.ok_or_else(overflow)
        }
        _ => Err("only literals, `int` consts and arithmetic fold in a type".to_string()),
    }
}

/// `{ e }`, a block whose only part is its value.
pub(crate) fn fold_block(block: &SyntaxNode, names: &mut Names<'_>) -> Result<i64, String> {
    let b = if block.kind() == K::BLOCK {
        block.clone()
    } else {
        block.child(K::BLOCK).unwrap_or_else(|| block.clone())
    };
    let stmts = ast::Block::cast(b.clone()).map_or(0, |b| b.stmts().len());
    match b.children().filter_map(ast::Expr::cast).last() {
        Some(e) if stmts == 0 => fold(&e, names),
        _ => Err("a block whose only part is its value".to_string()),
    }
}

fn overflow() -> String {
    "this overflows `int`, or divides by zero".to_string()
}

fn only(node: &SyntaxNode) -> Result<ast::Expr, String> {
    node.children()
        .find_map(ast::Expr::cast)
        .ok_or_else(|| "an expression".to_string())
}
