//! Structured control flow from a body's blocks: `if` and `loop` rebuilt from the MIR's
//! control-flow graph, so the WGSL has the shape GPU compilers optimise well.
//!
//! - **Loops** are natural loops: a back edge (found by depth-first search) to a header, the
//!   blocks that reach it without passing the header, and at most one exit block. A path out of
//!   the loop that runs straight to a `return` (a `return` inside it) is not an exit: it is
//!   written where it is taken, and the function's shared return block, when reached that way, is
//!   a fresh `return`.
//! - **A trap** (or code that cannot run) ends its path without reaching the function's end, so
//!   it does not keep the branches around it from merging.
//! - **Branches** run to their merge, the branching block's immediate post-dominator, when it is
//!   inside the innermost loop; otherwise each branch ends by itself (`continue`, `break` or
//!   `return`).
//! - A body that does not fit (a jump out of two loops, a loop with two exits, a block reached
//!   twice) gives `None`, and the caller keeps the loop-over-switch dispatcher, which is always
//!   correct: WGSL has no labelled `break`.

use std::collections::HashMap;

use naga::{Block, Expression, Handle, Statement as S};

/// Where a block goes after its statements.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Exit {
    /// On to a block.
    Goto(usize),
    /// To the first block if the condition holds, else the second.
    Branch(Handle<Expression>, usize, usize),
    /// Out of the function, with its value.
    Return(Option<Handle<Expression>>),
    /// Out of the function by a trap, or code that cannot run (as `Return`, on the GPU).
    Abort(Option<Handle<Expression>>),
}

impl Exit {
    fn succs(self) -> Vec<usize> {
        match self {
            Exit::Goto(t) => vec![t],
            Exit::Branch(_, t, e) => vec![t, e],
            Exit::Return(_) | Exit::Abort(_) => Vec::new(),
        }
    }
}

/// The statements of blocks `stmts` (block 0 first), each followed by its exit, as structured
/// statements; `None` if the graph's shape does not fit. `fresh_return` gives the statements of
/// another `return` of the function's value, for a `return` written where it is taken.
pub(crate) fn structure(
    stmts: &[Vec<S>],
    exits: &[Exit],
    fresh_return: &mut dyn FnMut() -> Vec<S>,
) -> Option<Vec<S>> {
    if exits.is_empty() {
        return Some(Vec::new());
    }
    // A block that only returns the function's value: its statements emit that value, at most.
    let pure_return: Vec<bool> = exits
        .iter()
        .zip(stmts)
        .map(|(e, s)| {
            matches!(e, Exit::Return(_))
                && s.len() <= 1
                && s.iter().all(|st| matches!(st, S::Emit(_)))
        })
        .collect();
    // A block with no statements that only goes on is seen through: as where it goes.
    let forward: Vec<usize> = (0..exits.len())
        .map(|b| {
            let mut t = b;
            for _ in 0..exits.len() {
                match exits[t] {
                    Exit::Goto(next) if stmts[t].is_empty() && next != t => t = next,
                    _ => break,
                }
            }
            t
        })
        .collect();
    let exits: Vec<Exit> = exits
        .iter()
        .map(|e| match *e {
            Exit::Goto(t) => Exit::Goto(forward[t]),
            Exit::Branch(c, t, f) => Exit::Branch(c, forward[t], forward[f]),
            e => e,
        })
        .collect();
    let exits = &exits[..];
    let graph = Graph::new(exits)?;
    let mut e = Emitter {
        stmts,
        exits,
        graph: &graph,
        done: vec![false; exits.len()],
        pure_return,
        fresh_return,
    };
    e.seq(forward[0], None, &mut Vec::new(), false)
}

/// Whether block `b` runs straight to a `return` (or a trap): by `goto`s alone.
fn terminates(exits: &[Exit], mut b: usize) -> bool {
    for _ in 0..exits.len() {
        match exits[b] {
            Exit::Goto(t) => b = t,
            Exit::Return(_) | Exit::Abort(_) => return true,
            Exit::Branch(..) => return false,
        }
    }
    false
}

/// A loop: its blocks, and the block it leaves to (none if it never ends by itself).
struct Loop {
    body: Vec<bool>,
    exit: Option<usize>,
}

struct Graph {
    /// The loops, by their headers.
    loops: HashMap<usize, Loop>,
    /// Each block's immediate post-dominator (`None`: only the function's end).
    ipdom: Vec<Option<usize>>,
}

impl Graph {
    fn new(exits: &[Exit]) -> Option<Graph> {
        let n = exits.len();
        let succ: Vec<Vec<usize>> = exits.iter().map(|e| e.succs()).collect();
        let mut preds = vec![Vec::new(); n];
        for (b, ss) in succ.iter().enumerate() {
            for &s in ss {
                preds.get_mut(s)?.push(b);
            }
        }
        // Back edges, by a depth-first search from the entry.
        let mut state = vec![0u8; n]; // 0 unseen, 1 on the stack, 2 done
        let mut back: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        state[0] = 1;
        while let Some(&mut (b, ref mut i)) = stack.last_mut() {
            if let Some(&s) = succ[b].get(*i) {
                *i += 1;
                match state[s] {
                    0 => {
                        state[s] = 1;
                        stack.push((s, 0));
                    }
                    1 => back.push((b, s)),
                    _ => {}
                }
            } else {
                state[b] = 2;
                stack.pop();
            }
        }
        // Each header's natural loop.
        let mut loops: HashMap<usize, Loop> = HashMap::new();
        for (latch, header) in back {
            let l = loops.entry(header).or_insert_with(|| Loop {
                body: vec![false; n],
                exit: None,
            });
            l.body[header] = true;
            let mut work = vec![latch];
            while let Some(x) = work.pop() {
                if !l.body[x] {
                    l.body[x] = true;
                    work.extend(preds[x].iter().copied());
                }
            }
        }
        for l in loops.values_mut() {
            let mut outs: Vec<usize> = Vec::new();
            for (b, inside) in l.body.iter().enumerate() {
                if *inside {
                    for &s in &succ[b] {
                        if !l.body[s] && !outs.contains(&s) {
                            outs.push(s);
                        }
                    }
                }
            }
            // Paths that run straight to a `return` are written where they are taken; the exit
            // is the one that does not, or, if every one does, the one most jumps go to (so the
            // others are each written once).
            let open: Vec<usize> = outs
                .iter()
                .copied()
                .filter(|&s| !terminates(exits, s))
                .collect();
            if open.len() > 1 {
                return None;
            }
            let jumps_to = |s: usize| preds[s].iter().filter(|&&p| l.body[p]).count();
            l.exit = open
                .first()
                .copied()
                .or_else(|| outs.iter().copied().max_by_key(|&s| (jumps_to(s), s)));
        }
        // Post-dominators, with the function's end as block `n`.
        let all: Vec<bool> = vec![true; n + 1];
        let mut pdom: Vec<Vec<bool>> = vec![all; n + 1];
        pdom[n] = (0..=n).map(|i| i == n).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for b in (0..n).rev() {
                // A trap never reaches the end: it leaves the post-dominators of what leads to it
                // to the other paths.
                let ends: Vec<usize> = match exits[b] {
                    Exit::Abort(_) => Vec::new(),
                    Exit::Return(_) => vec![n],
                    _ => succ[b].clone(),
                };
                let mut next: Vec<bool> = vec![true; n + 1];
                for s in ends {
                    for (i, v) in next.iter_mut().enumerate() {
                        *v = *v && pdom[s][i];
                    }
                }
                next[b] = true;
                if next != pdom[b] {
                    pdom[b] = next;
                    changed = true;
                }
            }
        }
        // The nearest strict post-dominator has the most post-dominators of its own.
        let count = |d: usize| pdom[d].iter().filter(|v| **v).count();
        let ipdom = (0..n)
            .map(|b| {
                (0..=n)
                    .filter(|&d| d != b && pdom[b][d])
                    .max_by_key(|&d| count(d))
                    .filter(|&d| d != n)
            })
            .collect();
        Some(Graph { loops, ipdom })
    }
}

/// What a jump becomes: the statements that end the sequence, or the block to go on with.
enum Jump {
    Done(Vec<S>),
    Next(usize),
}

struct Emitter<'e> {
    stmts: &'e [Vec<S>],
    exits: &'e [Exit],
    graph: &'e Graph,
    /// The blocks emitted so far: none is emitted twice.
    done: Vec<bool>,
    /// The blocks that only return the function's value.
    pure_return: Vec<bool>,
    fresh_return: &'e mut dyn FnMut() -> Vec<S>,
}

impl Emitter<'_> {
    /// The statements from block `b` until block `stop` (exclusive) or the end of every path,
    /// inside the loops of `ctx` (each a header and its exit, innermost last). `at_header`: `b`
    /// is the header of the innermost loop, being emitted as its body.
    fn seq(
        &mut self,
        mut b: usize,
        stop: Option<usize>,
        ctx: &mut Vec<(usize, Option<usize>)>,
        mut at_header: bool,
    ) -> Option<Vec<S>> {
        let mut out = Vec::new();
        loop {
            if Some(b) == stop {
                return Some(out);
            }
            if !at_header && let Some(l) = self.graph.loops.get(&b) {
                let exit = l.exit;
                ctx.push((b, exit));
                let body = self.seq(b, None, ctx, true)?;
                ctx.pop();
                out.push(S::Loop {
                    body: Block::from_vec(body),
                    continuing: Block::new(),
                    break_if: None,
                });
                match exit {
                    Some(x) => {
                        b = x;
                        continue;
                    }
                    None => return Some(out),
                }
            }
            at_header = false;
            if std::mem::replace(self.done.get_mut(b)?, true) {
                return None;
            }
            out.extend(self.stmts[b].iter().cloned());
            match self.exits[b] {
                Exit::Return(value) | Exit::Abort(value) => {
                    out.push(S::Return { value });
                    return Some(out);
                }
                Exit::Goto(t) => match self.jump(t, stop, ctx)? {
                    Jump::Done(s) => {
                        out.extend(s);
                        return Some(out);
                    }
                    Jump::Next(t) => b = t,
                },
                Exit::Branch(condition, t, e) => {
                    let merge = self.graph.ipdom[b].filter(|&m| self.in_region(m, ctx));
                    let until = merge.or(stop);
                    let accept = self.branch(t, until, ctx)?;
                    let reject = self.branch(e, until, ctx)?;
                    out.push(S::If {
                        condition,
                        accept: Block::from_vec(accept),
                        reject: Block::from_vec(reject),
                    });
                    match merge {
                        Some(m) => b = m,
                        None => return Some(out),
                    }
                }
            }
        }
    }

    /// One branch's statements, from block `t` until `stop`.
    fn branch(
        &mut self,
        t: usize,
        stop: Option<usize>,
        ctx: &mut Vec<(usize, Option<usize>)>,
    ) -> Option<Vec<S>> {
        match self.jump(t, stop, ctx)? {
            Jump::Done(s) => Some(s),
            Jump::Next(t) => self.seq(t, stop, ctx, false),
        }
    }

    /// A jump to block `t`: on to it, or `continue`, `break`, or nothing (at `stop`). A jump to
    /// an outer loop's header or exit does not fit.
    fn jump(
        &mut self,
        t: usize,
        stop: Option<usize>,
        ctx: &[(usize, Option<usize>)],
    ) -> Option<Jump> {
        if Some(t) == stop {
            return Some(Jump::Done(Vec::new()));
        }
        // The function's shared return block: a `return` of its own, here.
        if self.pure_return[t] {
            return Some(Jump::Done((self.fresh_return)()));
        }
        if let Some(&(h, x)) = ctx.last() {
            if t == h {
                return Some(Jump::Done(vec![S::Continue]));
            }
            if Some(t) == x {
                return Some(Jump::Done(vec![S::Break]));
            }
        }
        let n = ctx.len().saturating_sub(1);
        if ctx[..n].iter().any(|&(h, x)| t == h || Some(t) == x) {
            return None;
        }
        Some(Jump::Next(t))
    }

    /// Whether a branch may merge at block `m`: inside the innermost loop, short of its header.
    fn in_region(&self, m: usize, ctx: &[(usize, Option<usize>)]) -> bool {
        match ctx.last() {
            Some(&(h, _)) => m != h && self.graph.loops[&h].body[m],
            None => true,
        }
    }
}
