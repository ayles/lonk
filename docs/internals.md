# Implementation guide

Read [LANGUAGE.md](../LANGUAGE.md) for observable behavior and
[embedding.md](embedding.md) for the host API. This guide follows the path from
source to output; [cache.md](cache.md) explains identities, reuse and storage.

The workspace contains `crates/lonk` (language and embedding API) and
`crates/lonk-cli` (file-reading command-line host). The language crate's `src/`
contains implementation modules; `std/` contains checked Lonk collection helpers;
`tests/` contains integration and behavioral regression tests. The source modules
share private graph, type and evaluator representations. They remain in one crate
until a useful independent API boundary justifies a split.

## From source to a value

The embedder supplies immutable source strings and, for modules, an explicit map
from import binding names to module names. The core never opens a file, resolves
a path, accesses a network or knows about package management.

The handwritten lexer and parser produce syntax with comments and source spans.
The formatter prints that syntax canonically. Parsing determines what the source
says; it does not execute it. Operator parsing uses precedence rules. Borrowed
token text avoids allocating a second copy of each identifier while parsing.

The checker resolves names, enforces types and usefulness, and visits every body,
including lazy fields and functions that will never be called. Its environments
and record type maps share immutable storage and copy when changed. Importing a
module exposes its checked type, independently of its implementation.

**Lowering** means translating the checked source into simpler operations for the
evaluator. For example, `v.x = 2` produces a new immutable version of `v`; later
reads use that version and earlier snapshots retain the old one. Borrowed writes
are represented as outputs with checked writeback. Conditions choose between
versions. The result is a graph of operations, not machine code. Source positions
stay attached for diagnostics.

Each module is checked into its own graph with import placeholders. A linker
replaces placeholders with references to dependency graphs and renames graph,
scope and generic IDs. The evaluator still receives one ordinary graph. A
reachability pass removes graph nodes that cannot affect any exported result,
after all source has been checked. Recursive function cycles are preserved.

Evaluation follows demands. Selecting `packages.editor.name` constructs only the
necessary containers and computes that field. Exporting a whole object demands
all its data fields, in lexical key order. A thunk is a suspended computation;
once demanded, it holds either its result or its language error. An explicit work
stack handles calls without recursively consuming the Rust stack.

Arrays and objects use persistent balanced trees. Copies share a root; updates
copy the changed path and share untouched branches. Strings share immutable
storage. Completed thunks release their pending work. This is copy-on-write
storage for by-value semantics, not mutable aliases visible to the language.

Useful code entry points, in the same order:

| Concern | Entry point |
|---|---|
| Public embedding API | [`lib.rs`](../crates/lonk/src/lib.rs) |
| Syntax and canonical printing | [`syntax.rs`](../crates/lonk/src/syntax.rs), [`format.rs`](../crates/lonk/src/format.rs) |
| Checking and lowering | [`check.rs`](../crates/lonk/src/check.rs), its focused submodules |
| Independent modules and linking | [`check/units.rs`](../crates/lonk/src/check/units.rs), [`compiler/modules.rs`](../crates/lonk/src/compiler/modules.rs) |
| Demand evaluation | [`eval.rs`](../crates/lonk/src/eval.rs) |
| Shared container storage | [`eval/shared.rs`](../crates/lonk/src/eval/shared.rs) |
| Preparation and persisted results | [`compiler.rs`](../crates/lonk/src/compiler.rs) |
| Computation identities | [`eval/identity.rs`](../crates/lonk/src/eval/identity.rs), [`eval/incremental.rs`](../crates/lonk/src/eval/incremental.rs) |

## Invariants

- A value is a snapshot. Mutating a local cannot change an earlier copy or an
  earlier closure capture.
- A binding has a fixed contract. Checks can refine knowledge within their scope;
  an explicit typed copy can forget fields. Writable borrows preserve the caller's
  guarantees, including unknown generic remainder fields.
- Laziness never exempts source from static checking. Cache hits reuse a result
  already checked under a compatible host namespace and relevant inputs.
- Constructing a cache key never demands a lazy value. An unused division by zero
  remains unused with caching enabled.
- Missing reusable artifacts cause recomputation. Storage failures and corrupt
  artifacts are infrastructure errors, separate from language errors.
- Object observations use lexical UTF-8 key order. Demand order is observable
  through errors and is covered by regression tests.
- The core has no filesystem or package-manager dependency. Hosts control input
  acquisition, storage location, admission, eviction and scheduling.

These are contracts supported by implementation and tests, not a proof of the
entire language. Resource limits are listed in [LANGUAGE.md](../LANGUAGE.md#limits-and-planned-work).

## Tests

Focused integration suites exercise individual features. `acceptance.rs` and
`*_regressions.rs` use complete programs to check combinations of snapshots,
typing, laziness, defaults, borrows, returns and formatting. They are executable
tests, not review reports. Division-by-zero fields deliberately make accidental
eager evaluation observable. Formatter round trips compare behavior as well as
syntax. Cache suites cover restart reuse, source edits, invalidation, corrupt
records, interrupted writes and infrastructure failure recovery.

`tests/fixtures/modules` contains a multi-module configuration regression and
small accepted/rejected syntax probes. These are test inputs, not user tutorials.
`tests/support` contains host adapters used to exercise storage and module loading.

Optimizations must preserve output values, diagnostic positions and demand order,
including which unused failures remain unobserved. See [CONTRIBUTING.md](../CONTRIBUTING.md)
for the normal checks.
