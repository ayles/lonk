# Editor support

`tree-sitter-lonk` provides syntax parsing and queries for `.lonk` files. It does
not type-check programs. The Rust checker remains authoritative.

## Build and validate

Use Tree-sitter CLI 0.26.11 and a C compiler, available through `nix develop`:

```sh
make -C editors/tree-sitter-lonk build test
```

`grammar.js` is the grammar source. `src/scanner.c` handles nested block comments
and newline separators; it is handwritten. `src/parser.c` and
`src/tree_sitter/parser.h` are generated and committed so parser consumers can
compile them without regenerating the grammar. `src/grammar.json` is also retained:
the Tree-sitter CLI reads it even when building an existing C parser. This follows
[Tree-sitter grammars](https://github.com/tree-sitter/tree-sitter-json).
The unused node-type JSON and support headers are ignored.

Regenerate with `make -C editors/tree-sitter-lonk generate`. The CLI's built-in
JavaScript runtime is used. `TREE_SITTER=/path/to/tree-sitter` overrides the tool.
The corpus checks syntax trees; the test target also parses all public examples
and standard-library source. It does not perform a differential comparison with
the Rust parser.

The header is upstream Tree-sitter code, so its [MIT notice](tree-sitter-lonk/LICENSE.tree-sitter)
is retained. Lonk's grammar, scanner and queries use the root Apache-2.0 license.

## Neovim

```sh
make -C editors/tree-sitter-lonk nvim
nvim --clean --cmd 'set runtimepath+=./editors/nvim' examples/collections.lonk
```

For regular use, add the absolute `editors/nvim` path to `runtimepath` in your
configuration. Re-run `make ... nvim` after parser/query changes. `NVIM_RUNTIME`
can select another runtime directory for the generated parser and query copies.

The integration uses Neovim's normal runtime layout: `ftdetect/lonk.lua` registers
the extension, `ftplugin/lonk.lua` starts highlighting and sets the comment syntax,
`parser/lonk.so` contains the compiled parser, and `queries/lonk/` holds queries.
The parser and copied queries are local build outputs. No global query overrides,
repository-path introspection, network installation or plugin manager is needed.
See [Neovim's parser discovery documentation](https://neovim.io/doc/user/treesitter/).

This also works alongside nvim-treesitter: both use Neovim's runtime parser/query
discovery. Lonk is not registered for `:TSInstall lonk` yet. Other editors can
compile `parser.c` and `scanner.c` with `src/` on the include path and use the
queries in `tree-sitter-lonk/queries/`.

## Known parser differences

- With a space before `<`, the Rust parser reads `f <int>(1)` as comparisons;
  Tree-sitter prefers a generic call. Such source fails type checking either way.
  Canonical formatting separates `a < b` and `f<int>(...)` unambiguously.
- `%{` in strings is reserved for interpolation and rejected by the checker;
  Tree-sitter currently treats it as string content. Use `%%{` or `\%` for literals.
- A newline separates record/type fields and match arms, including inside a block
  comment. Expressions continue across newlines when the next token continues
  them. Statements still need `;`; arrays need commas.
