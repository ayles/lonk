<img src="assets/lonk.svg" alt="Lonk: a sheep with curled horns and a dry sideways glance" align="right" width="128">

# Lonk

*Have you ever touched a sheep? It's like grass, but better.*

Lonk is a small, pure, typed language for producing structured data. You construct
values with familiar imperative code; every copy and closure keeps its own
snapshot. Evaluation is lazy, while all source receives static checking.

```text
let original = { version = "1.0" };
let updated = original;
updated.version = "2.0";
{ before = original.version; after = updated.version }
```

Result: `{"after":"2.0","before":"1.0"}`.

Lonk is experimental. Language and API contracts may change before its first
release. The library performs no filesystem, network, clock or process operations;
embedders supply inputs, modules and storage.

## Try it

Use Rust 1.85 or newer and a C linker, or run `nix develop`.

```sh
cargo run -p lonk-cli -- eval examples/showcase.lonk
cargo run -p lonk-cli -- check examples/showcase.lonk snapshots.captured_label
cargo run -p lonk-cli -- fmt examples/showcase.lonk
```

Install with `cargo install --path crates/lonk-cli --locked`. The command is
`lonk`; source files use `.lonk`. `eval` prints JSON, `check` reports a type without
evaluation, and `fmt` prints canonical source. `fmt --check` checks formatting
without modifying the file. Use `lonk --help` for the complete CLI.

## Examples

Every file below is a runnable script: `lonk eval examples/NAME.lonk`.

| Script | Demonstrates |
| --- | --- |
| [showcase](examples/showcase.lonk) | Service data, snapshots, typed projection and lazy fields |
| [snapshots](examples/snapshots.lonk) | Delayed computations keep their captured values |
| [matching](examples/matching.lonk) | Structural checks and type refinement |
| [generics](examples/generics.lonk) | Generic functions, aliases and captured rows |
| [aliases](examples/aliases.lonk) | Scoped aliases, reborrows and independent captures |
| [defaults](examples/defaults.lonk) | Lazy defaults and callable contracts |
| [variants](examples/variants.lonk) | Literal unions, optional values and exhaustive matching |
| [methods](examples/methods.lonk) | Receiver calls, unbound methods and extension fields |
| [callable contracts](examples/callable-contracts.lonk) | Generic callbacks and receiver contracts |
| [collection API](examples/collection-api.lonk) | Returns, indexing, slicing and collection helpers |
| [collections](examples/collections.lonk) | Recursion and a dictionary implemented in Lonk |

## Read further

- [LANGUAGE.md](LANGUAGE.md): language rules, syntax, built-ins and current limits.
- [Embedding](docs/embedding.md): Rust API, modules, diagnostics and ownership.
- [Caching](docs/cache.md): keys, reuse after edits, spill/reload and host obligations.
- [Implementation](docs/internals.md): parser, checker, lowering, evaluator and invariants.
- [Editor setup](editors/README.md): Tree-sitter and Neovim.
- [Contributing](CONTRIBUTING.md): workspace layout and validation commands.

The [Rust embedding example](crates/lonk/examples/embed.rs) runs with
`cargo run -p lonk --example embed`. Persistent storage is an embedding API; the
CLI does not yet expose a disk cache.

Licensed under [Apache-2.0](LICENSE), including the mascot.
