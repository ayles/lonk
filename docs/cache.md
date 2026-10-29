# Caching and storage

There are three different kinds of cache. They should not be conflated.

| Cache | Lifetime | What its keys identify |
|---|---|---|
| Evaluator memo table | One evaluation session | A graph node in a lexical call frame |
| `CacheStore` payload spill | One evaluation session | Immutable container node IDs or ready-string thunk IDs |
| `ArtifactStore` | Across compilers, processes and source edits | Content-derived preparation or result identities |

The first two belong to `Program::evaluator()` and
`Program::evaluator_with_cache()`. Restart reuse is exposed through
`Compiler::with_store(namespace, policy, store)`, `evaluate()` and `evaluate_modules()`. Calling
`prepare()` and then `Program::evaluate()` reuses preparation but creates a plain
evaluator; it does not implicitly attach the compiler's persistent result store.

## Compatibility

The host supplies an opaque namespace when attaching an `ArtifactStore`. It must
identify the Lonk implementation and every host policy that can affect cached
results or interpretation of stored bytes. The core does not discover build IDs,
read source files, inspect the executable or choose cache directories.

For a local statically linked host, a native binary build ID is a conservative
choice. The [buildid crate](https://docs.rs/buildid/latest/buildid/) reads platform
identities. Wrappers and statically linked dependencies are part of that binary.
External configuration and dynamically loaded implementations need their own
identity contributions. Without a reliable namespace, use `Compiler::new` for
memory-only reuse. Empty namespaces are rejected.

Lonk supplies a versioned format discriminator. A stable released version may
eventually serve as the Lonk compatibility contribution; it does not identify
unreleased edits that retain the same package version. Compatibility across
versions or different hosts is not currently promised.

### Key encoding

`artifact::key` is a 256-bit BLAKE3 digest of:

```text
"lonk-artifact-v2" || one-byte kind || length(part_0) || part_0 || ...
```

Lengths are unsigned 64-bit little-endian integers, separating `["ab", "c"]`
from `["a", "bc"]`. Logical keys below are local to one compiler. At the storage
boundary they become `key(15, [host_namespace, logical_key])`; the encoded envelope
also contains that namespaced key. Both preparation and scalar-result records
pass through this boundary. Collision resistance is a cryptographic assumption.

### Preparation and selected-output keys

| Kind | Parts | Retained result |
|---|---|---|
| 0 | Exact source bytes | Parsed syntax or parse error; memory only |
| 1 | Exact single-source bytes | Whole preparation identity; memory only |
| 4 | Module name, exact source, sorted binding/target pairs | Component of the whole-input identity |
| 2 | Entry module name, kind-4 identities of all supplied modules in name order | Whole preparation identity; memory only |
| 3 | Kind-1 or kind-2 identity, each selected path segment | Exported data or exact-input diagnostic; persisted |
| 5 | No parts | Checked standard-library graph; persisted |
| 6 | Module name, exact source, standard-library interface fingerprint, then sorted binding/target/interface-fingerprint triples | Independently checked module graph or diagnostic; persisted |

An interface fingerprint serializes the exported type with generic IDs renamed
consistently from zero, then hashes it. It contains all exported fields, callable
contracts and generic row constraints. It does **not** contain a dependency's
implementation. Generic names are retained, so some harmless renamings can miss.

The kind-3 shortcut comes first. An exact repeated selection can return without
parsing, checking, linking or evaluating. After any source edit that shortcut
misses. The compiler then loads unchanged kind-6 units, checks changed units,
links them and evaluates with the finer computation cache below. No whole linked
graph is persisted, avoiding repeated copies of dependencies after each root edit.
Failed preparations and selections are also memoized in compiler memory, so an
exact failing input can hit without rechecking.

### Computation keys

A scalar result can be reused even when its containing file's bytes changed.
The identity describes the computation and its suspended inputs:

| Kind | Identity |
|---|---|
| 7 | Graph component digest plus member position |
| 8 | The fixed top-level frame |
| 9 | Node identity plus lexical frame identity |
| 10 | Source thunk identity plus projection type and fields to drop |
| 11 | Source thunk identity, update path and replacement thunk identity |
| 12 | A scalar value created directly by a native primitive |
| 13 | Scope-definition identity, captured parent frame, exact type substitutions and ordered argument thunk identities |
| 14 | A thunk identity; names the persisted scalar result |

The graph digest includes the operation, its checked type and ordered dependency
edges. Source spans, source-file indices and graph arena offsets are excluded.
Lexical scope is represented by an edge to its defining function or loop;
arbitrary owner numbers are excluded. Generic IDs remain in types and substitution
maps consistently; renumbering those IDs can conservatively cause misses.

Cycles are handled as strongly connected components: a group of graph nodes that
can reach one another. A component digest contains each member's operation and
tagged references to members inside the component or hashes of dependencies
outside it. Members retain their relative graph order. This handles recursion
without recursively hashing forever or using addresses as identities.

Function frames include the callee's captured environment, argument **thunk
identities**, and instantiated types. Loop frames include the current element and
prior state. These are identities of work, not forced argument values. Unavailable
identities disable reuse for dependent work; they are never approximated.

Current persisted demand candidates are function calls, function results, computed
record fields/array elements and the root expression. Successful `int`, `bool` and
`str` results are stored. Literal loads, aliases, parameters and cheap container
construction are excluded. Containers and closures are rebuilt lazily; their
computed scalar children can hit independently. Whole exported containers also
benefit from the kind-3 exact-output shortcut.
Each cacheable suspended demand consults compiler memory first and the store
second, once. Computation hits are counted per demanded thunk.

The lexical-owner edge conservatively joins a function with its body. Editing
inside that function invalidates its component and its call frames, even when
one branch would be unaffected. Top-level sibling computations can still survive.
Changing an unused argument's expression also changes the frame key. Detecting
that changed work produced the same observed value—often called *early cutoff*—
is not implemented. This cache tracks structural dependencies, not a minimal
trace of only the fields actually read.

Cross-edit diagnostics are not stored in kind 14. Identical expressions can occur
at different source positions, so a structural hash alone cannot relocate a
failure correctly. Errors remain memoized within sessions and under exact-input
kind-3/kind-6 keys. After an edit, evaluation produces a diagnostic at the new
source position.

### Follow these edits through the cache

Suppose `main` imports `lib` and exports `{answer = lib.sum(); label = "old"}`.

1. **First run:** load or check the standard library; check `lib` and `main`; link;
   evaluate `answer`; save checked units, useful scalar results and the selected
   output. Unselected fields are checked but not evaluated.
2. **Restart, same inputs and path:** kind 3 hits. No preparation or evaluation.
3. **Change only `label`:** the whole-input output misses; `lib`'s checked unit
   hits; `main` is rechecked. The unchanged sum computation can hit even though
   its caller's file changed. The edited label is computed when selected.
4. **Change `lib.sum`'s body, keep its exported type:** recheck `lib`; reuse
   `main`'s checked unit. Link the new implementation. The changed computation's
   identity misses, so its result is recomputed. A checked caller is not a promise
   that its old result remains valid.
5. **Change `lib`'s exported type:** recheck direct consumers against the new
   interface. Propagation stops where a consumer's exported interface stays equal.
6. **Add only comments:** the edited unit is reparsed/rechecked, but computation
   identities can hit because positions and comments are excluded.
7. **Delete a reusable artifact:** that lookup misses and the computation is
   reproduced. No language result changes. Deleting an in-session spill record
   instead is an error: its suspended work may already have been released.

### Bytes, budgets and failures

Artifacts use bounded Postcard serialization, with an 8-byte format marker, the
32-byte expected key and a 32-byte payload checksum. Loading verifies the envelope,
checksum, trailing bytes and recursive decoding depth. Checked units additionally
validate graph edges, owners, parameter/import bounds, types and source spans.
Only private constructors turn these graphs into prepared programs.
Kind-3 diagnostics embed their module's source text, so these failed-output
artifacts are source-sized. Kind-6 diagnostics store the diagnostic alone and
reconstruct the source from the keyed module input.

The producer/store is trusted. Checksums detect accidental damage, not malicious
replacement with newly checksummed executable data. Serialized `Arc`s do not
preserve sharing; loading reconstructs values. Do not infer disk size from RAM
sharing or treat this interface as an untrusted bytecode loader.

`CompilerPolicy.memory_bytes` bounds **estimated retained cache entries**, not
process RSS. Active evaluation graphs, frames, thunk metadata, caller-held values
and the fixed standard-library seed are outside that budget. Set it to zero for
one-shot hosts. `artifact_bytes` caps each encoded or decoded record; oversized
results are usable but are not saved. Hosts control total disk admission and
reclamation. Production hosts should pack small records, bound their index memory,
choose atomic publication and durability guarantees, and enforce total storage
budgets. The small single-writer log adapter in
[`tests/support/file_cache.rs`](../crates/lonk/tests/support/file_cache.rs) exercises
restarts and incomplete appends; it is not a production disk cache.

An absent artifact is a normal miss. A malformed artifact, failed read or failed
write is an infrastructure error, including a write failure after successful
computation. The core does not silently ignore storage failures. A host that wants
best-effort caching can deliberately decline writes by returning success; the
next lookup then misses. Failed infrastructure operations are not saved as
language failures. Session spill has a stricter retention contract because it
replaces resident data instead of merely accelerating reproducible work.
`artifact_writes` counts successful write attempts; a host can decline admission,
so it is not a count of newly stored records. `computation_writes` counts completed
scalar save requests, including records too large to encode under the policy.

## Session spill

The language core performs no filesystem or network operations. A host supplies
`CacheStore`; the evaluator encodes immutable records and controls eviction.
`Program::evaluator()` retains its original interface. The separate
`Program::evaluator_with_cache(store, policy)` returns a `CachedEvaluation` whose
`select` distinguishes language failures from storage failures.

```rust
use lonk::{CachePolicy, MemoryStore, Program};

let program = Program::prepare("{answer = [40, 2]; unused = 1 / 0}")?;
let mut evaluation = program.evaluator_with_cache(
    Box::new(MemoryStore::new()),
    CachePolicy::new(0),
);
let first = evaluation.select(&["answer"])?;
let again = evaluation.select(&["answer"])?;
assert_eq!(first, again);
let stats = evaluation.cache_stats();
assert!(stats.records_reloaded > 0);
evaluation.close()?;
```

`MemoryStore` demonstrates the protocol and supports tests. It retains its bytes
in RAM, so it is not a memory-saving host implementation. The test-only directory
adapter in `../crates/lonk/tests/cache_adapter.rs` demonstrates filesystem storage
outside the core. Its one-file-per-record layout is deliberately simple and is
not a recommended disk-efficient production layout.

### Scope and budget

This implementation is **lossless spill within one evaluation session**. Stored
container payloads refer to that session's thunk IDs. They cannot be reused by a
new evaluator or after restarting the process. Each store instance must have a
fresh key namespace and retain all accepted records until release.

The policy counts the estimated reachable allocations of ready strings and
containers. Shared allocations are conservatively counted once per ready root.
Eviction brings that estimate within budget at successful force boundaries.
It does not bound the entire process:

- Checked graphs, source text, frames, thunk metadata and closure/type metadata
  remain resident. Inline scalars stay in their thunk slots.
- A demand chain retains its active working set until it finishes. Evicting its
  dependencies on every retry could prevent progress with a small budget.
- Reload reconstructs a whole container, preserving sharing within that load.
  Containers are not yet paged on individual field/index access.
- Exported `Value` objects and temporarily held container roots belong to the
  caller or active computation and are outside the cache budget.

These are architectural limits, not small tuning details. Bounded metadata,
paged traversal and streaming export are needed for a total small-host memory
guarantee.

### Shared records

Persistent container nodes get version IDs when first stored. Every mutation
invalidates the ID on its changed path, even when an `Rc` is uniquely owned.
Unchanged subtrees retain their identity. Records contain child IDs and lazy
payload thunk IDs, so storing a snapshot does not force its fields or serialize
every earlier snapshot again.

An array node's current encoded record is 29 bytes: format/kind, two child IDs,
and a thunk ID. A record-field node adds the UTF-8 field name. Strings use a
four-byte format marker followed by UTF-8. Host indexing, allocation and
filesystem overhead are additional. These session record formats are internal,
versioned implementation details, not a stable interchange format.
Different thunk keys can carry identical string records. The core does not keep
a content-hash index for them; a production host should deduplicate those bytes.

`contains` lets a write stop at an already stored subtree. Child records are
accepted before their parent; a ready value becomes spilled only after all its
records are accepted. The core does not retain a second index of stored nodes.
The embedder can pack, compress or deduplicate records and choose its own index.
`contains` must truthfully report accepted records. A record must remain invisible
under its key until it has been written completely; the directory adapter uses a
temporary name followed by a rename for that publication step.

### Errors and lifetime

Language failures remain memoized diagnostics. Missing data, malformed records,
and host failures are `EvaluationError::Infrastructure`; they never become a
language-level failure. Failed writes retain the ready value. Failed reloads
retain the spilled reference, and suspended demand chains can be retried after
the host recovers. `forced_nodes` counts computation attempts, so an interrupted
attempt can count again when retried; ordinary reload adds no forced nodes.

A failed eviction may report an infrastructure error after the computation has
already succeeded: the next request can reuse that result and retry storage.
The decoder checks kinds, UTF-8, thunk bounds and container structure. The store
must return identical accepted bytes; structural validation is not cryptographic
integrity verification.

Call `close()` to observe storage-release failures. Dropping a session only
attempts release on a best-effort basis. No record is individually deleted while
the session is live; live-root tracing and reclamation remain future work.

## Remaining work

Session metadata reclamation, bounded/paged container reload and streaming data
export are not implemented. Restart caches recover checked units, exact outputs
and scalar computations, not live closure/container graphs. Computation identities
describe structural dependencies rather than observed-demand traces, so some
irrelevant edits still cause misses. These limits matter when designing hosts
for very small memory and disk budgets.
