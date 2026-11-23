//! A control-flow graph as WASM's structured control flow, after Norman Ramsey's "Beyond
//! Relooper" (ICFP 2022): from the dominator tree, a `loop` for each loop header, a `block` for
//! each merge node (one reached by more than one forward edge) ending just before it, and each
//! branch either the code it goes to, written in place, or a `br` to the `loop` or `block` that
//! leads there. MIR built from structured source is reducible, which this needs.

/// How a node's code ends, as the emitter reports it.
pub(crate) enum Exit {
    /// It goes on at this node.
    Goto(usize),
    /// It goes on at the first node if the condition it has left on the stack is true, else at
    /// the second.
    Branch(usize, usize),
    /// It has left the function (returned, or trapped).
    Done,
}

/// What writes the code: each node's own, and the structure around it.
pub(crate) trait Emit {
    /// Node `n`'s code, and how it ends.
    fn node(&mut self, n: usize) -> Result<Exit, String>;
    /// What goes along edge `k` (0 for the first successor, 1 for the second) out of node
    /// `from`, before control goes on: a block's arguments.
    fn edge(&mut self, from: usize, k: usize) -> Result<(), String>;
    fn block(&mut self);
    fn loop_(&mut self);
    /// An `if` on the condition on the stack.
    fn if_(&mut self);
    fn else_(&mut self);
    fn end(&mut self);
    /// A branch to the label `depth` out.
    fn br(&mut self, depth: u32);
}

/// What an enclosing label is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    IfThenElse,
    LoopHeadedBy(usize),
    BlockFollowedBy(usize),
}

/// The graph of nodes `0..succs.len()`, entered at 0, its successors given, analysed.
struct Graph<'a> {
    succs: &'a [Vec<usize>],
    /// Each reachable node's number in reverse postorder (`usize::MAX` if unreachable).
    rpo: Vec<usize>,
    /// Each reachable node's immediate dominator (the entry's is itself).
    idom: Vec<usize>,
    /// Each node's children in the dominator tree.
    children: Vec<Vec<usize>>,
    loop_header: Vec<bool>,
    merge: Vec<bool>,
}

impl<'a> Graph<'a> {
    fn new(succs: &'a [Vec<usize>]) -> Result<Self, String> {
        let n = succs.len();
        // Reverse postorder, from a depth-first search.
        let mut post = Vec::new();
        let mut seen = vec![false; n];
        let mut stack = vec![(0usize, 0usize)];
        seen[0] = true;
        while let Some((x, k)) = stack.pop() {
            if k < succs[x].len() {
                stack.push((x, k + 1));
                let y = succs[x][k];
                if !seen[y] {
                    seen[y] = true;
                    stack.push((y, 0));
                }
            } else {
                post.push(x);
            }
        }
        let mut rpo = vec![usize::MAX; n];
        let order: Vec<usize> = post.iter().rev().copied().collect();
        for (i, &x) in order.iter().enumerate() {
            rpo[x] = i;
        }
        let mut preds = vec![Vec::new(); n];
        for &x in &order {
            for &y in &succs[x] {
                preds[y].push(x);
            }
        }
        // Dominators (Cooper, Harvey and Kennedy's iteration).
        let mut idom = vec![usize::MAX; n];
        idom[0] = 0;
        let intersect = |idom: &[usize], mut a: usize, mut b: usize| {
            while a != b {
                while rpo[a] > rpo[b] {
                    a = idom[a];
                }
                while rpo[b] > rpo[a] {
                    b = idom[b];
                }
            }
            a
        };
        let mut changed = true;
        while changed {
            changed = false;
            for &x in order.iter().skip(1) {
                let mut new = usize::MAX;
                for &p in &preds[x] {
                    if idom[p] == usize::MAX {
                        continue;
                    }
                    new = if new == usize::MAX {
                        p
                    } else {
                        intersect(&idom, p, new)
                    };
                }
                if new != idom[x] {
                    idom[x] = new;
                    changed = true;
                }
            }
        }
        let mut children = vec![Vec::new(); n];
        for &x in order.iter().skip(1) {
            children[idom[x]].push(x);
        }
        let dominates = |a: usize, mut b: usize| loop {
            if a == b {
                return true;
            }
            if b == 0 {
                return false;
            }
            b = idom[b];
        };
        let mut loop_header = vec![false; n];
        let mut merge = vec![false; n];
        for &x in &order {
            let forward = preds[x].iter().filter(|&&p| rpo[p] < rpo[x]).count();
            merge[x] = forward >= 2;
            for &p in &preds[x] {
                if rpo[p] >= rpo[x] {
                    // A back edge goes to a node that dominates where it comes from.
                    if !dominates(x, p) {
                        return Err("control flow that is not reducible".to_string());
                    }
                    loop_header[x] = true;
                }
            }
        }
        Ok(Graph {
            succs,
            rpo,
            idom,
            children,
            loop_header,
            merge,
        })
    }
}

/// Writes the structured code of the graph whose node `n` goes on at `succs[n]`.
pub(crate) fn structure(succs: &[Vec<usize>], e: &mut dyn Emit) -> Result<(), String> {
    let g = Graph::new(succs)?;
    let _ = (&g.succs, &g.idom);
    let mut ctx = Vec::new();
    do_tree(&g, 0, &mut ctx, e)
}

fn do_tree(g: &Graph<'_>, x: usize, ctx: &mut Vec<Frame>, e: &mut dyn Emit) -> Result<(), String> {
    // Its children that are merge nodes, the latest (in reverse postorder) first: the outermost
    // block ends just before it.
    let mut merges: Vec<usize> = g.children[x]
        .iter()
        .copied()
        .filter(|&c| g.merge[c])
        .collect();
    merges.sort_by_key(|&c| std::cmp::Reverse(g.rpo[c]));
    if g.loop_header[x] {
        e.loop_();
        ctx.push(Frame::LoopHeadedBy(x));
        node_within(g, x, &merges, ctx, e)?;
        ctx.pop();
        e.end();
        Ok(())
    } else {
        node_within(g, x, &merges, ctx, e)
    }
}

fn node_within(
    g: &Graph<'_>,
    x: usize,
    merges: &[usize],
    ctx: &mut Vec<Frame>,
    e: &mut dyn Emit,
) -> Result<(), String> {
    if let Some((&y, rest)) = merges.split_first() {
        e.block();
        ctx.push(Frame::BlockFollowedBy(y));
        node_within(g, x, rest, ctx, e)?;
        ctx.pop();
        e.end();
        return do_tree(g, y, ctx, e);
    }
    match e.node(x)? {
        Exit::Goto(t) => do_branch(g, x, 0, t, ctx, e),
        Exit::Branch(t, f) => {
            e.if_();
            ctx.push(Frame::IfThenElse);
            do_branch(g, x, 0, t, ctx, e)?;
            e.else_();
            do_branch(g, x, 1, f, ctx, e)?;
            ctx.pop();
            e.end();
            Ok(())
        }
        Exit::Done => Ok(()),
    }
}

fn do_branch(
    g: &Graph<'_>,
    src: usize,
    k: usize,
    t: usize,
    ctx: &mut Vec<Frame>,
    e: &mut dyn Emit,
) -> Result<(), String> {
    e.edge(src, k)?;
    let backward = g.rpo[t] <= g.rpo[src];
    if backward || g.merge[t] {
        let want = if backward {
            Frame::LoopHeadedBy(t)
        } else {
            Frame::BlockFollowedBy(t)
        };
        let depth = ctx
            .iter()
            .rev()
            .position(|f| *f == want)
            .ok_or("a branch with no label to go to")?;
        e.br(depth as u32);
        Ok(())
    } else {
        do_tree(g, t, ctx, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An emitter that writes the structure as text, each node's own code as `nN`, and whose
    /// nodes end as `ends` says.
    struct Text {
        ends: Vec<Exit2>,
        out: Vec<String>,
    }

    #[derive(Clone, Copy)]
    enum Exit2 {
        Goto(usize),
        Branch(usize, usize),
        Done,
    }

    impl Emit for Text {
        fn node(&mut self, n: usize) -> Result<Exit, String> {
            self.out.push(format!("n{n}"));
            Ok(match self.ends[n] {
                Goto(t) => Exit::Goto(t),
                Branch(t, f) => Exit::Branch(t, f),
                Done => Exit::Done,
            })
        }
        fn edge(&mut self, _: usize, _: usize) -> Result<(), String> {
            Ok(())
        }
        fn block(&mut self) {
            self.out.push("block".into());
        }
        fn loop_(&mut self) {
            self.out.push("loop".into());
        }
        fn if_(&mut self) {
            self.out.push("if".into());
        }
        fn else_(&mut self) {
            self.out.push("else".into());
        }
        fn end(&mut self) {
            self.out.push("end".into());
        }
        fn br(&mut self, depth: u32) {
            self.out.push(format!("br{depth}"));
        }
    }

    fn shape(ends: &[Exit2]) -> String {
        let succs: Vec<Vec<usize>> = ends
            .iter()
            .map(|e| match *e {
                Goto(t) => vec![t],
                Branch(t, f) => vec![t, f],
                Done => vec![],
            })
            .collect();
        let mut t = Text {
            ends: ends.to_vec(),
            out: Vec::new(),
        };
        structure(&succs, &mut t).expect("reducible");
        t.out.join(" ")
    }

    use Exit2::{Branch, Done, Goto};

    #[test]
    fn a_straight_line_is_written_in_order() {
        assert_eq!(shape(&[Goto(1), Goto(2), Done]), "n0 n1 n2");
    }

    #[test]
    fn a_diamond_ends_in_a_block_before_its_join() {
        assert_eq!(
            shape(&[Branch(1, 2), Goto(3), Goto(3), Done]),
            "block n0 if n1 br1 else n2 br1 end end n3"
        );
    }

    #[test]
    fn a_loop_branches_back_to_its_header() {
        // 0 -> 1 (header) -> 2 (body) -> 1, or 1 -> 3 (exit).
        assert_eq!(
            shape(&[Goto(1), Branch(2, 3), Goto(1), Done]),
            "n0 loop n1 if n2 br1 else n3 end end"
        );
    }

    #[test]
    fn nested_loops_and_an_early_exit() {
        // 0 -> 1 (outer header) -> 2 (inner header) -> 3 -> 2 | 4; 4 -> 1 | 5 (exit); 3 can also
        // leave early to 5.
        let s = shape(&[
            Goto(1),
            Goto(2),
            Branch(3, 4),
            Branch(2, 5),
            Branch(1, 5),
            Done,
        ]);
        // The exit, 5, is a merge node dominated by 2, so its block is inside the inner loop:
        // branches back go to the loops (3 out to 2's, 4 out to 1's), and to 5 two out.
        assert_eq!(
            s,
            concat!(
                "n0 loop n1 loop block n2 if n3 if br3 else br2 end else n4 if br4 else br2 ",
                "end end end n5 end end"
            )
        );
    }

    #[test]
    fn irreducible_flow_is_refused() {
        // 0 -> 1 | 2; 1 -> 2; 2 -> 1: two ways into the cycle.
        let succs = vec![vec![1, 2], vec![2], vec![1]];
        let mut t = Text {
            ends: vec![Branch(1, 2), Goto(2), Goto(1)],
            out: Vec::new(),
        };
        assert!(structure(&succs, &mut t).is_err());
    }
}
