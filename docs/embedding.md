# Embedding Lonk

Add `lonk` as a Rust dependency. The library receives immutable source strings,
explicit module maps and optional storage implementations. It does not acquire
inputs or choose files, cache directories, transports or scheduling policies.

## Prepare and evaluate

```rust
use lonk::{Program, Syntax, Value};

let syntax = Syntax::parse("// lazy sibling\n{ answer = 42; bad = 1 / 0 }")?;
let canonical = syntax.format();
let program = Program::prepare(&canonical)?;
let mut evaluation = program.evaluator();
assert_eq!(evaluation.select(&["answer"])?, Value::Int(42));
Ok::<(), lonk::Diagnostic>(())
```

`Program::prepare` parses and checks the entire source, including uncalled bodies
and unselected fields. Only a prepared program can create an evaluator. Sessions
memoize successful demands and failures; separate sessions have independent
caches. `Program::type_at` inspects a selection without evaluating it. Selecting
unknown or optional fields requires the source to establish the necessary facts.

`Program::evaluator_with_cache` accepts an embedder's `CacheStore` and a ready
payload budget. It spills shared string/container data and reloads it within the
same session; the core still performs no filesystem access. Storage failures are
separate from memoized language diagnostics. Graphs, frames, thunk metadata and
active working sets remain resident, and these keys cannot survive a process
restart. See [the storage contract and limits](cache.md#session-spill).

`Compiler` adds bounded preparation reuse. `Compiler::with_store(namespace, policy, store)`
uses a host's `ArtifactStore` across compiler instances and processes. It persists
the checked prelude, independently checked modules, exact selected outputs and
computed scalar results. A root edit can reuse unchanged dependency modules;
changed dependency bodies with unchanged exported types do not recheck callers.
Unchanged scalar computations can also survive edits to sibling fields. Every
edited body still receives full checking. Generic renumbering, function-body
changes and unused argument changes can conservatively invalidate computations.

Use `Compiler::evaluate` or `evaluate_modules` to attach result caches;
`compiler.prepare(...).evaluate(...)` only reuses preparation. The host supplies
the cache namespace as well as storage; see [cache compatibility](cache.md#compatibility).
The [Rust example](../crates/lonk/examples/embed.rs) shows preparation, selection,
an artifact store and session storage without choosing a disk layout:

```sh
cargo run -p lonk --example embed
```

The example's stores hold bytes in RAM. Implement `ArtifactStore` and `CacheStore`
in the host for disk-backed storage. They have different retention contracts.

The compiler budget bounds estimated retained entries, not total process RAM.
The host owns total disk limits. Oversized records are skipped; corrupt records
and storage failures are explicit infrastructure errors. See the
[key schemas and limits](cache.md).

Assignments lower to immutable value versions. Copies, nested updates, function
parameters and closure captures retain their source snapshots. Fields and array
elements are demanded independently. Runtime objects and arrays use persistent
balanced trees: copying a container shares its root in O(1), and updating one
field or element copies O(log n) nodes while sharing untouched branches. Strings
share immutable storage. Array concatenation and slices share their interiors;
object spreads reuse a leading source and insert subsequent fields. Old versions
remain valid, including through borrowed writes. Selecting a record materializes its entire
subtree; select a leaf to leave siblings untouched. Whole-value data export rejects
function handles and type descriptors; these can still be fields in library results.

## Modules

`Program::prepare_modules(entry, &sources)` takes a map of `ModuleSource` values.
Each contains immutable source and explicit local binding names for dependency
identities. Core code does no file I/O and treats identities as opaque strings.
Dependency names use the ordinary lexical scope and can shadow prelude namespace
names without affecting built-in methods. Unused dependency bindings are allowed.
Module preparation currently has a dependency-depth limit of 128.
Files export their ordinary result, including types and functions. Imported
bindings are immutable; a new `let` makes a normal local snapshot. Private locals
remain private. Shared dependencies prepare once; unrelated sources are not visited.
All reachable bodies are still checked before evaluation. Cyclic inferred
interfaces are diagnosed; linking explicit cyclic interfaces remains unimplemented.
Use `Program::render_diagnostic` for runtime errors from imported or library code.

```rust
use lonk::{ModuleSource, Program};
use std::collections::BTreeMap;

let modules = BTreeMap::from([
    ("library".into(), ModuleSource::new("{ answer = 42 }")),
    ("main".into(), ModuleSource::new("lib.answer")
        .with_dependency("lib", "library")),
]);
let program = Program::prepare_modules("main", &modules)?;
println!("{}", program.evaluate(&[])?);
Ok::<(), Box<dyn std::error::Error>>(())
```

## Errors and resource ownership

`Diagnostic` describes a language error with a source span. For an evaluated
program, `Program::render_diagnostic` can resolve imported and standard-library
origins. `ModuleDiagnostic` includes its source module. `CompileError` and
`EvaluationError` distinguish language errors from host infrastructure failures.

The returned Rust `Value` owns fully materialized data. Retaining it retains its
memory, independently of evaluator/cache budgets. A prepared `Program` owns shared
source text and a checked graph. Dropping a compiler releases its retained entries,
but cannot release programs or values still held by the caller.

The library has depth and pending-demand limits; these are not execution-time,
process-memory or disk quotas. There is no cancellation API or whole-process
sandbox. Hosts that accept arbitrary programs need an appropriate execution and
resource boundary. See [language limits](../LANGUAGE.md#limits-and-planned-work).

Generate API documentation with `cargo doc -p lonk --no-deps`.
