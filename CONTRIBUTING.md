# Contributing

Read [LANGUAGE.md](LANGUAGE.md) for behavior and [docs/internals.md](docs/internals.md)
for implementation entry points. Keep I/O and environment policy in hosts.

The workspace has two crates: `lonk` contains the language, standard library and
regression tests; `lonk-cli` reads files and exposes the command-line interface.
Root `examples/` contains runnable Lonk scripts. Editor support lives in `editors/`.

## Development

Use Rust 1.85 or newer, Cargo and a C linker, or enter `nix develop`. Editor work
also uses Tree-sitter CLI 0.26.11, a C compiler and Neovim. No Python is required.

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo clippy --workspace --lib --examples --bins --locked -- -D warnings
cargo doc --workspace --no-deps --locked
cargo run -p lonk --example embed
make -C editors/tree-sitter-lonk test
```

Lint production targets; tests use assertions and unwraps intentionally. Preserve
snapshot independence, whole-source checking, borrowed guarantees and demand
order. Include focused behavioral regressions when changing these contracts.
Cache changes need cold/warm equivalence, invalidation and failure-recovery checks.

After grammar changes, run `make -C editors/tree-sitter-lonk generate` and commit
`src/parser.c`, `src/grammar.json` and any changed `src/tree_sitter/parser.h`. The generator also
creates ignored intermediate files. See [editor setup](editors/README.md).

## Packaging

`cargo package -p lonk --locked` verifies the standalone library archive. Each
crate inherits the root license file; Cargo copies it into its source archive.
There are no maintained per-crate copies in the repository. The CLI's registry
package depends on a published matching `lonk` version; local installation uses
the workspace path dependency.

Contributions use [Apache-2.0](LICENSE). The retained Tree-sitter support header
has its own [upstream MIT notice](editors/tree-sitter-lonk/LICENSE.tree-sitter).
Cargo retrieves dependencies with their source licenses. It does not assemble a
notice bundle for a distributed linked executable: binary releases must include
the applicable dependency notices in the release artifact. There is no checked-in
dependency-license catalogue to keep synchronized.
