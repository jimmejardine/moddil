//! The lossless syntax tree.
//!
//! **Green** nodes are immutable and position-free: a kind, a text length and children, shared
//! through `Arc`, with identical tokens interned. **Red** nodes (`SyntaxNode`, `SyntaxToken`) are
//! cursors over a green tree that know their parent and their offset in the text. The text of the
//! root is exactly the source text.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::{SyntaxKind, TextRange};

/// An immutable node: its kind, its children and its text length.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GreenNode(Arc<GreenNodeData>);

#[derive(PartialEq, Eq, Hash)]
struct GreenNodeData {
    kind: SyntaxKind,
    text_len: u32,
    children: Vec<GreenElement>,
}

/// An immutable token: its kind and its text.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GreenToken(Arc<GreenTokenData>);

#[derive(PartialEq, Eq, Hash)]
struct GreenTokenData {
    kind: SyntaxKind,
    text: Box<str>,
}

/// A green node or token.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum GreenElement {
    /// A node.
    Node(GreenNode),
    /// A token.
    Token(GreenToken),
}

impl GreenNode {
    /// The node's kind.
    pub fn kind(&self) -> SyntaxKind {
        self.0.kind
    }

    /// The length of the node's text in bytes.
    pub fn text_len(&self) -> u32 {
        self.0.text_len
    }

    /// The node's children, tokens included.
    pub fn children(&self) -> &[GreenElement] {
        &self.0.children
    }
}

impl GreenToken {
    /// The token's kind.
    pub fn kind(&self) -> SyntaxKind {
        self.0.kind
    }

    /// The token's text.
    pub fn text(&self) -> &str {
        &self.0.text
    }
}

impl GreenElement {
    fn text_len(&self) -> u32 {
        match self {
            GreenElement::Node(n) => n.text_len(),
            GreenElement::Token(t) => t.text().len() as u32,
        }
    }
}

/// Builds a green tree bottom-up: `start_node`, `token`, `finish_node`.
#[derive(Default)]
pub struct GreenBuilder {
    stack: Vec<(SyntaxKind, Vec<GreenElement>)>,
    tokens: HashMap<(SyntaxKind, Box<str>), GreenToken>,
}

impl GreenBuilder {
    /// Opens a node; the tokens and nodes until the matching `finish_node` are its children.
    pub fn start_node(&mut self, kind: SyntaxKind) {
        self.stack.push((kind, Vec::new()));
    }

    /// Adds a token to the open node.
    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let key = (kind, Box::<str>::from(text));
        let token = self
            .tokens
            .entry(key)
            .or_insert_with(|| {
                GreenToken(Arc::new(GreenTokenData {
                    kind,
                    text: text.into(),
                }))
            })
            .clone();
        self.stack
            .last_mut()
            .expect("a token must be inside a node")
            .1
            .push(GreenElement::Token(token));
    }

    /// Closes the open node.
    pub fn finish_node(&mut self) {
        let (kind, children) = self.stack.pop().expect("an open node");
        let text_len = children.iter().map(GreenElement::text_len).sum();
        let node = GreenNode(Arc::new(GreenNodeData {
            kind,
            text_len,
            children,
        }));
        match self.stack.last_mut() {
            Some(parent) => parent.1.push(GreenElement::Node(node)),
            None => self
                .stack
                .push((SyntaxKind::EOF, vec![GreenElement::Node(node)])),
        }
    }

    /// The finished tree: exactly one node must have been built at the top level.
    pub fn finish(mut self) -> GreenNode {
        assert_eq!(self.stack.len(), 1, "unbalanced start_node and finish_node");
        let (kind, mut children) = self.stack.pop().expect("a root");
        assert!(
            kind == SyntaxKind::EOF && children.len() == 1,
            "one root node"
        );
        match children.pop() {
            Some(GreenElement::Node(n)) => n,
            _ => unreachable!("the root is a node"),
        }
    }
}

/// A node in a syntax tree, with its position and parent.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SyntaxNode(Arc<NodeData>);

#[derive(PartialEq, Eq, Hash)]
struct NodeData {
    green: GreenNode,
    parent: Option<SyntaxNode>,
    offset: u32,
}

/// A token in a syntax tree, with its position and parent.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SyntaxToken {
    green: GreenToken,
    parent: SyntaxNode,
    offset: u32,
}

/// A node or a token.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxElement {
    /// A node.
    Node(SyntaxNode),
    /// A token.
    Token(SyntaxToken),
}

impl SyntaxNode {
    /// The root of the tree `green`.
    pub fn new_root(green: GreenNode) -> SyntaxNode {
        SyntaxNode(Arc::new(NodeData {
            green,
            parent: None,
            offset: 0,
        }))
    }

    /// The node's kind.
    pub fn kind(&self) -> SyntaxKind {
        self.0.green.kind()
    }

    /// The green node underneath.
    pub fn green(&self) -> &GreenNode {
        &self.0.green
    }

    /// Where the node's text is in the source.
    pub fn text_range(&self) -> TextRange {
        TextRange::new(self.0.offset, self.0.offset + self.0.green.text_len())
    }

    /// The node's text, trivia included.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity(self.0.green.text_len() as usize);
        fn walk(n: &GreenNode, s: &mut String) {
            for c in n.children() {
                match c {
                    GreenElement::Node(n) => walk(n, s),
                    GreenElement::Token(t) => s.push_str(t.text()),
                }
            }
        }
        walk(&self.0.green, &mut s);
        s
    }

    /// The parent node.
    pub fn parent(&self) -> Option<SyntaxNode> {
        self.0.parent.clone()
    }

    /// The node and its ancestors, innermost first.
    pub fn ancestors(&self) -> impl Iterator<Item = SyntaxNode> + use<> {
        std::iter::successors(Some(self.clone()), SyntaxNode::parent)
    }

    /// Children, tokens included, in order.
    pub fn children_with_tokens(&self) -> impl Iterator<Item = SyntaxElement> + '_ {
        let mut offset = self.0.offset;
        self.0.green.children().iter().map(move |c| {
            let at = offset;
            offset += c.text_len();
            match c {
                GreenElement::Node(n) => SyntaxElement::Node(SyntaxNode(Arc::new(NodeData {
                    green: n.clone(),
                    parent: Some(self.clone()),
                    offset: at,
                }))),
                GreenElement::Token(t) => SyntaxElement::Token(SyntaxToken {
                    green: t.clone(),
                    parent: self.clone(),
                    offset: at,
                }),
            }
        })
    }

    /// Child nodes, in order.
    pub fn children(&self) -> impl Iterator<Item = SyntaxNode> + '_ {
        self.children_with_tokens().filter_map(|e| match e {
            SyntaxElement::Node(n) => Some(n),
            SyntaxElement::Token(_) => None,
        })
    }

    /// Child tokens, in order, trivia included.
    pub fn child_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        self.children_with_tokens().filter_map(|e| match e {
            SyntaxElement::Token(t) => Some(t),
            SyntaxElement::Node(_) => None,
        })
    }

    /// The node and every node inside it, in preorder.
    pub fn descendants(&self) -> Vec<SyntaxNode> {
        let mut out = vec![self.clone()];
        let mut i = 0;
        while i < out.len() {
            let children: Vec<SyntaxNode> = out[i].children().collect();
            out.splice(i + 1..i + 1, children);
            i += 1;
        }
        out
    }

    /// Every token inside the node, in order, trivia included.
    pub fn tokens(&self) -> Vec<SyntaxToken> {
        let mut out = Vec::new();
        fn walk(n: &SyntaxNode, out: &mut Vec<SyntaxToken>) {
            for c in n.children_with_tokens() {
                match c {
                    SyntaxElement::Node(n) => walk(&n, out),
                    SyntaxElement::Token(t) => out.push(t),
                }
            }
        }
        walk(self, &mut out);
        out
    }

    /// The first child node of `kind`.
    pub fn child(&self, kind: SyntaxKind) -> Option<SyntaxNode> {
        self.children().find(|c| c.kind() == kind)
    }

    /// The first child token of `kind`.
    pub fn token(&self, kind: SyntaxKind) -> Option<SyntaxToken> {
        self.child_tokens().find(|t| t.kind() == kind)
    }
}

impl SyntaxToken {
    /// The token's kind.
    pub fn kind(&self) -> SyntaxKind {
        self.green.kind()
    }

    /// The token's text.
    pub fn text(&self) -> &str {
        self.green.text()
    }

    /// Where the token is in the source.
    pub fn text_range(&self) -> TextRange {
        TextRange::new(self.offset, self.offset + self.green.text().len() as u32)
    }

    /// The node containing the token.
    pub fn parent(&self) -> SyntaxNode {
        self.parent.clone()
    }
}

impl SyntaxElement {
    /// The element's kind.
    pub fn kind(&self) -> SyntaxKind {
        match self {
            SyntaxElement::Node(n) => n.kind(),
            SyntaxElement::Token(t) => t.kind(),
        }
    }
}

impl fmt::Display for SyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// The tree, one element per line: `FN_ITEM 0..12`, tokens with their text.
impl fmt::Debug for SyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn walk(n: &SyntaxNode, depth: usize, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            writeln!(
                f,
                "{:indent$}{:?} {:?}",
                "",
                n.kind(),
                n.text_range(),
                indent = depth * 2
            )?;
            for c in n.children_with_tokens() {
                match c {
                    SyntaxElement::Node(n) => walk(&n, depth + 1, f)?,
                    SyntaxElement::Token(t) => {
                        writeln!(f, "{:indent$}{t:?}", "", indent = depth * 2 + 2)?;
                    }
                }
            }
            Ok(())
        }
        walk(self, 0, f)
    }
}

impl fmt::Debug for SyntaxToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} {:?} {:?}",
            self.kind(),
            self.text_range(),
            self.text()
        )
    }
}
