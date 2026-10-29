// External scanner: nested block comments and the line-break entry separator.
//
// syntax.rs::separator accepts `;`, `,`, the closing brace, or any line break between
// the previous token and the next one (comments included) inside record literals,
// record types and match arms. Expressions themselves ignore line breaks, so the
// separator is only produced when the next token cannot continue an expression.
#include "tree_sitter/parser.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

enum TokenType { BLOCK_COMMENT, NEWLINE, ERROR_SENTINEL };

typedef struct {
  // A line break was seen before or inside a comment that the scanner returned last.
  bool pending_newline;
  // Changes on every emitted separator so the serialized state differs across the
  // zero-width token; tree-sitter never has to reuse an identical state at one position.
  uint8_t emitted;
} Scanner;

void *tree_sitter_lonk_external_scanner_create(void) {
  Scanner *scanner = calloc(1, sizeof(Scanner));
  return scanner;
}
void tree_sitter_lonk_external_scanner_destroy(void *payload) { free(payload); }
unsigned tree_sitter_lonk_external_scanner_serialize(void *payload, char *buffer) {
  Scanner *scanner = payload;
  buffer[0] = scanner->pending_newline;
  buffer[1] = (char)scanner->emitted;
  return 2;
}
void tree_sitter_lonk_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  Scanner *scanner = payload;
  scanner->pending_newline = length > 0 && buffer[0];
  scanner->emitted = length > 1 ? (uint8_t)buffer[1] : 0;
}

static bool is_word(int32_t c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_';
}

// Rust char::is_whitespace and the grammar's whitespace include Unicode spaces.
// Only a literal LF counts as a field separator, matching syntax.rs::separator.
static bool is_space(int32_t c) {
  return (c >= 0x09 && c <= 0x0d) || c == 0x20 || c == 0x85 || c == 0xa0 ||
         c == 0x1680 || (c >= 0x2000 && c <= 0x200a) || c == 0x2028 ||
         c == 0x2029 || c == 0x202f || c == 0x205f || c == 0x3000;
}

static bool scan_block_comment(TSLexer *lexer, bool *saw_newline) {
  // Called with the lookahead on the `*` of `/*`.
  lexer->advance(lexer, false);
  uint32_t depth = 1;
  while (!lexer->eof(lexer)) {
    int32_t c = lexer->lookahead;
    if (c == '\n') *saw_newline = true;
    if (c == '/') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '*') { depth++; lexer->advance(lexer, false); }
    } else if (c == '*') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '/') {
        lexer->advance(lexer, false);
        if (--depth == 0) { lexer->mark_end(lexer); return true; }
      }
    } else {
      lexer->advance(lexer, false);
    }
  }
  return false;
}

bool tree_sitter_lonk_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  Scanner *scanner = payload;
  bool in_recovery = valid_symbols[ERROR_SENTINEL];
  bool want_newline = valid_symbols[NEWLINE] && !in_recovery;
  bool saw_newline = scanner->pending_newline;
  scanner->pending_newline = false;
  if (!valid_symbols[BLOCK_COMMENT] && !want_newline) return false;

  while (is_space(lexer->lookahead)) {
    if (lexer->lookahead == '\n') saw_newline = true;
    lexer->advance(lexer, true);
  }
  lexer->mark_end(lexer);
  if (lexer->eof(lexer)) return false;
  int32_t c = lexer->lookahead;

  if (c == '/') {
    lexer->advance(lexer, false);
    if (lexer->lookahead == '*') {
      if (!valid_symbols[BLOCK_COMMENT]) return false;
      if (!scan_block_comment(lexer, &saw_newline)) return false;
      scanner->pending_newline = saw_newline;
      lexer->result_symbol = BLOCK_COMMENT;
      return true;
    }
    if (lexer->lookahead == '/') scanner->pending_newline = saw_newline;  // line comment: lexed by the grammar
    return false;  // `//` comment or the `/` operator continuing an expression
  }
  if (!want_newline || !saw_newline) return false;

  // Tokens that continue the current expression or close the entry list.
  switch (c) {
    case '}': case ')': case ']': case ';': case ',': case ':':
    case '(': case '[': case '+': case '-': case '*': case '%': case '=': case '!':
    case '<': case '>': case '&': case '|': case '?':
      return false;
    case '.': {
      lexer->advance(lexer, false);
      if (lexer->lookahead != '.') return false;  // member access
      lexer->advance(lexer, false);
      if (lexer->lookahead != '.') return false;
      break;  // `...` starts a spread or row entry
    }
    case 'i': {
      lexer->advance(lexer, false);
      if (lexer->lookahead == 'n') {
        lexer->advance(lexer, false);
        if (!is_word(lexer->lookahead)) return false;  // the `in` operator
      }
      break;
    }
    default:
      break;
  }
  scanner->emitted++;
  lexer->result_symbol = NEWLINE;
  return true;
}
