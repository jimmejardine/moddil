// The external scanner for tree-sitter-moddil: the four tokens of reference/grammar/moddil.ebnf
// that are not regular languages. Hand-written; the rules are the comments above each %scanner
// token in the grammar. The enum's order must match `externals` in grammar.js.

#include "tree_sitter/parser.h"

#include <wctype.h>

enum TokenType {
  BLOCK_COMMENT,
  RAW_STRING,
  BLOCK_STRING,
  TAGGED_STRING,
};

void *tree_sitter_moddil_external_scanner_create(void) { return NULL; }
void tree_sitter_moddil_external_scanner_destroy(void *payload) { (void)payload; }
unsigned tree_sitter_moddil_external_scanner_serialize(void *payload, char *buffer) {
  (void)payload;
  (void)buffer;
  return 0;
}
void tree_sitter_moddil_external_scanner_deserialize(void *payload, const char *buffer,
                                                      unsigned length) {
  (void)payload;
  (void)buffer;
  (void)length;
}

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static bool is_ident_start(int32_t c) { return c == '_' || iswalpha(c) || c > 0x7f; }
static bool is_ident_char(int32_t c) { return c == '_' || iswalnum(c) || c > 0x7f; }

static bool finish(TSLexer *lexer, enum TokenType type) {
  lexer->mark_end(lexer);
  lexer->result_symbol = type;
  return true;
}

// After '/*': up to the matching '*/'; block comments nest.
static bool block_comment(TSLexer *lexer) {
  unsigned depth = 1;
  while (depth > 0) {
    if (lexer->eof(lexer)) return false;
    if (lexer->lookahead == '/') {
      advance(lexer);
      if (lexer->lookahead == '*') {
        advance(lexer);
        depth++;
      }
    } else if (lexer->lookahead == '*') {
      advance(lexer);
      if (lexer->lookahead == '/') {
        advance(lexer);
        depth--;
      }
    } else {
      advance(lexer);
    }
  }
  return finish(lexer, BLOCK_COMMENT);
}

// After the opening '"""': spaces or tabs and a newline, then lines up to one holding only spaces
// or tabs and '"""'.
static bool block_body(TSLexer *lexer, enum TokenType type) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') advance(lexer);
  if (lexer->lookahead == '\r') advance(lexer);
  if (lexer->lookahead != '\n') return false;
  advance(lexer);
  for (;;) {
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') advance(lexer);
    unsigned quotes = 0;
    while (lexer->lookahead == '"' && quotes < 3) {
      advance(lexer);
      quotes++;
    }
    if (quotes == 3) return finish(lexer, type);
    while (lexer->lookahead != '\n') {
      if (lexer->eof(lexer)) return false;
      advance(lexer);
    }
    advance(lexer);
  }
}

// After 'r' or 'br': n '#', '"', then anything up to '"' followed by n '#'. `r"""` is a raw block.
static bool raw_string(TSLexer *lexer, const bool *valid) {
  unsigned hashes = 0;
  while (lexer->lookahead == '#') {
    advance(lexer);
    hashes++;
  }
  if (lexer->lookahead != '"') return false;
  advance(lexer);
  if (hashes == 0 && lexer->lookahead == '"') {
    advance(lexer);
    if (lexer->lookahead == '"' && valid[BLOCK_STRING]) {
      advance(lexer);
      return block_body(lexer, BLOCK_STRING);
    }
    return valid[RAW_STRING] && finish(lexer, RAW_STRING);  // r""
  }
  for (;;) {
    if (lexer->eof(lexer)) return false;
    if (lexer->lookahead == '"') {
      advance(lexer);
      unsigned closing = 0;
      while (closing < hashes && lexer->lookahead == '#') {
        advance(lexer);
        closing++;
      }
      if (closing == hashes) return valid[RAW_STRING] && finish(lexer, RAW_STRING);
    } else {
      advance(lexer);
    }
  }
}

// Inside a hole: up to its matching '}', skipping string and char literals.
static bool hole(TSLexer *lexer) {
  unsigned depth = 1;
  while (depth > 0) {
    if (lexer->eof(lexer)) return false;
    switch (lexer->lookahead) {
      case '{':
        depth++;
        advance(lexer);
        break;
      case '}':
        depth--;
        advance(lexer);
        break;
      case '"':
        advance(lexer);
        while (lexer->lookahead != '"') {
          if (lexer->eof(lexer)) return false;
          if (lexer->lookahead == '\\') advance(lexer);
          advance(lexer);
        }
        advance(lexer);
        break;
      default:
        advance(lexer);
    }
  }
  return true;
}

// After a tag and its opening '"': a string whose holes nest, or a block string.
static bool tagged(TSLexer *lexer) {
  if (lexer->lookahead == '"') {
    advance(lexer);
    if (lexer->lookahead != '"') return finish(lexer, TAGGED_STRING);  // fmt""
    advance(lexer);
    return block_body(lexer, TAGGED_STRING);
  }
  for (;;) {
    if (lexer->eof(lexer)) return false;
    switch (lexer->lookahead) {
      case '"':
        advance(lexer);
        return finish(lexer, TAGGED_STRING);
      case '\\':
        advance(lexer);
        advance(lexer);
        break;
      case '{':
        advance(lexer);
        if (lexer->lookahead == '{') {
          advance(lexer);
        } else if (!hole(lexer)) {
          return false;
        }
        break;
      default:
        advance(lexer);
    }
  }
}

bool tree_sitter_moddil_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid) {
  (void)payload;
  while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);
  int32_t c = lexer->lookahead;

  if (c == '/' && valid[BLOCK_COMMENT]) {
    advance(lexer);
    if (lexer->lookahead != '*') return false;
    advance(lexer);
    return block_comment(lexer);
  }
  if (c == '"' && valid[BLOCK_STRING]) {
    for (int i = 0; i < 3; i++) {
      if (lexer->lookahead != '"') return false;  // a plain string: the regular lexer's
      advance(lexer);
    }
    return block_body(lexer, BLOCK_STRING);
  }
  if (!is_ident_start(c) || !(valid[RAW_STRING] || valid[BLOCK_STRING] || valid[TAGGED_STRING])) {
    return false;
  }
  // A word, then (with no space) a string: a raw string's prefix, or a tag.
  char word[3] = {0, 0, 0};
  unsigned length = 0;
  while (is_ident_char(lexer->lookahead)) {
    if (length < 2) word[length] = (char)lexer->lookahead;
    length++;
    advance(lexer);
  }
  bool raw = (length == 1 && word[0] == 'r') || (length == 2 && word[0] == 'b' && word[1] == 'r');
  if (raw) return raw_string(lexer, valid);
  if (length == 1 && word[0] == 'b') return false;  // b"…" and b'…': the regular lexer's
  if (lexer->lookahead != '"' || !valid[TAGGED_STRING]) return false;
  advance(lexer);
  return tagged(lexer);
}
