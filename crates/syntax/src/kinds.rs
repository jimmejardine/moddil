//! `SyntaxKind`: the kind of every token and node in the syntax tree.
//!
//! Token kinds are generated from the grammar (`generated/kinds.rs`, written by
//! `moddil-grammar generate`); node kinds are listed here. Both live in one number space, so a
//! green tree element is a kind plus its children or text.

use std::fmt;

/// The kind of a token or a node.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SyntaxKind(pub(crate) u16);

include!("generated/kinds.rs");

macro_rules! nodes {
    ($($name:ident,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Node { $($name,)* }

        impl SyntaxKind {
            $(
                #[doc = concat!("The `", stringify!($name), "` node.")]
                pub const $name: SyntaxKind = SyntaxKind(TOKEN_COUNT + Node::$name as u16);
            )*
        }

        const NODE_NAMES: &[&str] = &[$(stringify!($name),)*];
    };
}

nodes! {
    SOURCE_FILE,
    // Attributes
    INNER_ATTR, OUTER_ATTR, ATTR, ATTR_INPUT, TOKEN_TREE,
    // Items
    NOTE, VIS, NAME, FN_ITEM, PARAM_LIST, PARAM, SELF_PARAM, NAMED_ONLY_MARKER, RET_TYPE,
    RESUME_CLAUSE, FROM_CLAUSE, EFFECT_CLAUSE, SIG_EFFECT, EFFECT, ERROR_TYPES, WHERE_CLAUSE,
    WHERE_PRED, CONTRACT, GENERIC_PARAMS, TYPE_PARAM, CONST_PARAM, STRUCT_ITEM, VIEW_ITEM,
    STRUCT_BODY, FIELD, INVARIANT, TUPLE_FIELDS, TUPLE_FIELD, ENUM_ITEM, VARIANT_LIST, VARIANT,
    TRAIT_ITEM, TRAIT_BODY, ASSOC_TYPE, IMPL_ITEM, IMPL_BODY, TYPE_ITEM, CONST_ITEM, USE_ITEM,
    USE_TREE, USE_LIST, MOD_ITEM, EXTERN_BLOCK, TEST_ITEM, TEST_INSTANCES, TEST_CAPABILITIES, PROPERTY_ITEM,
    BENCH_ITEM, GROUP_ITEM, UNIT_ITEM, UNIT_DEF, DIMENSION_ITEM,
    // Units
    UNIT_SUFFIX, UNIT_BIN, UNIT_POWER, UNIT_PAREN, EXPONENT,
    // Paths and types
    PATH, PATH_SEGMENT, QUALIFIED_PATH, GENERIC_ARGS, TYPE_ARG, CONST_ARG, ASSOC_ARG, PATH_TYPE,
    REF_TYPE, PTR_TYPE, ARRAY_TYPE, SLICE_TYPE, PAREN_TYPE, TUPLE_TYPE, REFINED_TYPE, RECORD_TYPE,
    RECORD_TYPE_FIELD, FN_TYPE, NEVER_TYPE, INFER_TYPE, IMPL_TYPE, DYN_TYPE, EFFECT_TYPE,
    TYPE_EFFECTS, BOUND, FN_SUGAR, TYPE_LIST,
    // Patterns
    OR_PAT, WILDCARD_PAT, REST_PAT, LITERAL_PAT, RANGE_PAT, IDENT_PAT, PATH_PAT, TUPLE_STRUCT_PAT,
    STRUCT_PAT, AT_PAT, REF_PAT, TUPLE_PAT, SLICE_PAT, RECORD_PAT, FIELD_PAT, TYPED_PAT,
    // Statements
    BLOCK, LET_STMT, LET_ELSE, EXPR_STMT, DEFER_STMT,
    // Expressions
    RETURN_EXPR, BREAK_EXPR, CONTINUE_EXPR, YIELD_EXPR, ESCAPE_EXPR, CLOSURE_EXPR,
    CLOSURE_PARAMS, CLOSURE_PARAM, ASSIGN_EXPR, RANGE_EXPR, BIN_EXPR, LET_COND, COMPARE_CHAIN,
    IN_EXPR, PREFIX_EXPR, REF_EXPR, KEY_PATH, PROPAGATE_EXPR, ERROR_CONTEXT, UNWRAP_EXPR,
    CALL_EXPR, ARG_LIST, ARG, METHOD_CALL_EXPR, FIELD_EXPR, INDEX_EXPR, LITERAL, PATH_EXPR,
    STRUCT_LIT, FIELD_INIT, SPREAD, PAREN_EXPR, TUPLE_EXPR, ARRAY_EXPR, RECORD_EXPR,
    RECORD_FIELD_INIT, CONST_BLOCK, IMPL_EXPR, FAIL_EXPR, IF_EXPR, MATCH_EXPR, MATCH_ARM_LIST,
    MATCH_ARM, MATCH_GUARD, LOOP_EXPR, WHILE_EXPR, FOR_EXPR, LABELLED_EXPR, TRY_EXPR, CATCH,
    WITH_EXPR, HANDLER, SELECT_EXPR, SELECT_ARM, UNSAFE_BLOCK, GEN_BLOCK,
    // Templates: `inline for`, a reflected field (`x.[f]`, `[f]: e`), and the type of types
    INLINE_FOR, INLINE_FOR_FIELDS, REFLECTED_FIELD_EXPR, REFLECTED_FIELD_INIT, TYPE_TYPE,
}

impl SyntaxKind {
    /// Whitespace and comments other than doc comments: kept in the tree, skipped by the parser.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::WHITESPACE | Self::LINE_COMMENT | Self::BLOCK_COMMENT
        )
    }

    /// A keyword.
    pub fn is_keyword(self) -> bool {
        KEYWORDS.iter().any(|&(_, k)| k == self)
    }

    /// A token kind, as opposed to a node kind.
    pub fn is_token(self) -> bool {
        self.0 < TOKEN_COUNT
    }

    /// The fixed text of a punctuation or keyword token, if it has one.
    pub fn text(self) -> Option<&'static str> {
        TOKEN_TEXT
            .get(usize::from(self.0))
            .copied()
            .filter(|t| !t.is_empty())
    }

    /// The kind's name: `IDENT`, `FN_KW`, `FN_ITEM`.
    pub fn name(self) -> &'static str {
        let i = usize::from(self.0);
        TOKEN_NAMES
            .get(i)
            .or_else(|| NODE_NAMES.get(i - usize::from(TOKEN_COUNT)))
            .copied()
            .unwrap_or("UNKNOWN")
    }

    /// How a diagnostic names the token: `'fn'`, `identifier`.
    pub fn describe(self) -> String {
        match self.text() {
            Some(t) => format!("`{t}`"),
            None => match self {
                Self::IDENT => "an identifier".to_string(),
                Self::EOF => "the end of the file".to_string(),
                Self::INTEGER | Self::FLOAT => "a number".to_string(),
                Self::LABEL => "a label".to_string(),
                _ => self.name().to_ascii_lowercase().replace('_', " "),
            },
        }
    }

    pub(crate) fn keyword(text: &str) -> Option<SyntaxKind> {
        KEYWORDS.iter().find(|&&(t, _)| t == text).map(|&(_, k)| k)
    }

    pub(crate) fn lexed_punct(text: &str) -> Option<(usize, SyntaxKind)> {
        LEXED_PUNCT
            .iter()
            .find(|&&(t, _)| text.starts_with(t))
            .map(|&(t, k)| (t.len(), k))
    }

    pub(crate) fn compounds() -> &'static [(&'static str, SyntaxKind)] {
        COMPOUNDS
    }
}

impl fmt::Debug for SyntaxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
