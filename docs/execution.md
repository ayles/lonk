# IR and execution model

Proposed architecture; not implemented. The current evaluator executes the
checker's graph directly. [internals.md](internals.md) describes that implementation.
This design separates language checking, program transformation and execution.
Inlining, loop distribution, range analysis, parallel scheduling and JIT compilation
are later work. The first implementation must preserve behavior without them.

## Pipeline

```text
source -> syntax and whole-source checking -> typed lazy SSA
       -> optional transformations -> bytecode lowering -> runtime
```

The checker owns source-language rules: structural types, flow refinements,
generics, usefulness and writable-borrow admission. It translates assignments and
borrowed writes into immutable value versions and explicit results. Neither the
IR nor the runtime performs source-level variable mutation.

The IR owns its types, operations, regions, verifier and deterministic printer.
It can use shared type definitions, but cannot depend on checker environments,
AST node layouts or source variable names. The runtime consumes executable chunks,
constants and diagnostic metadata; it does not interpret checker nodes.

## Typed lazy SSA

Each value has one definition. A region contains basic blocks with typed arguments,
operations and a terminator. Branch operands supply the destination block's
arguments: these serve the purpose of phi nodes. Back-edges pass the next loop
state explicitly. A block argument defines a new value on each dynamic visit.

The essential operations are constants, primitive unary/binary operations,
immutable container construction/access/update, function closures and calls,
delayed regions, force, branches and returns. Type tests and projections retain
their existing structural semantics. Primitive operations specify which operands
are demanded and the required error order.

Two internal representations are distinguished:

- `Lazy<T>` is a handle to a suspended or already completed value.
- `Ready<T>` reveals the outer value. A ready record or array still contains lazy
  handles for its members; it does not recursively materialize its contents.

`delay` captures explicit immutable inputs and returns a lazy handle without
executing its region. `force` demands a handle and returns a shallow ready value.
Wrapping an already ready value is cheap and need not allocate a thunk. These
are compiler/runtime operations, not additions to Lonk syntax.

The conservative translation preserves existing lazy boundaries. Later demand
analysis may remove delays or fuse instructions when it proves that doing so
preserves demand, failure and termination behavior. Instruction cost alone cannot
justify moving a potentially failing computation across a lazy boundary.

Region captures and operands remain visible to passes. A delayed region is not
an opaque blob of native code. Function calls can be inlined by substituting
explicit arguments and captures while preserving the delayed boundaries.

Loops initially carry `Lazy<T>` state through block arguments. An iteration can
construct the next state's suspended computations without forcing their payloads.
Branches force their predicates; only the chosen successor executes. A source
conditional affecting a suspended update stays inside that delayed region; it
must not become eager loop control merely because it appears inside a loop.
Early returns have explicit control flow and may require state needed to decide
that return.
This preserves independently lazy loop outputs, including the existing `a`/`b`
example. It does not initially eliminate per-iteration allocation.

A loop pass can later clone a loop's blocks, divide independent state parameters
and outputs, and change induction bounds. Both the original and split loops use
the same operations. Loop membership, induction information and demand summaries
are analyses over the IR, not undocumented evaluator rules. A strict scalar loop
is the same control flow carrying ready scalars after a valid transformation.

SSA block arguments follow the established approach described in the
[MLIR reference](https://mlir.llvm.org/docs/LangRef/#blocks). This does not require
an LLVM or MLIR dependency. The
[RVSDG paper](https://arxiv.org/abs/1912.05036) provides another useful reference
for representing dependencies and structured control flow in an optimizer.

## Runtime chunks and thunks

Bytecode lowering assigns registers and linearizes blocks into chunks containing
primitive instructions, branches, calls, container operations, `make_thunk`,
`force` and returns. Chunk signatures distinguish lazy handles from ready values.
Function application produces a lazy result handle; forcing that handle evaluates
the selected function body far enough to obtain its outer result.

A thunk contains a chunk entry, immutable captures and a memo state. The chunk is
shared code; each application supplies its own captures. Successful forcing stores
the shallow result. Language failures are memoized; storage failures remain
retryable infrastructure errors. Pending evaluator continuations are scheduling
state, separate from the thunk's computation identity.

Conceptually, a shared local and two returned fields lower like this. This is
illustrative bytecode, not an implemented textual format:

```text
foo(bar, baz, zzz):
    a = make_thunk shared(bar)
    x = make_thunk field(a, baz)
    y = make_thunk field(a, zzz)
    return record {x: x, y: y}

shared(bar):
    p = call bar()
    v = force p
    return checked_add v, 1

field(a, f):
    u = force a
    p = call f()
    v = force p
    return checked_add u, v
```

This illustration groups arithmetic to explain the representation, not to specify
an optimization in the first implementation. A conservative emitter may generate
more chunks. Task boundaries need not coincide with chunks or basic blocks; a
single worker can execute many chunks without scheduling each one separately.

The runtime owns memoization, immutable containers, roots, reclamation, spill,
resource accounting and cancellation. `force` can suspend the current task on a
dependency without recursively growing the native stack. A future parallel
scheduler must distinguish an actual demand cycle from another worker computing
the same thunk, share completion, and preserve language error precedence.

A native backend can eventually implement the same chunk entry/return, force,
ownership and diagnostic contracts. JIT compilation still requires those runtime
contracts and valid safepoints; bytecode alone does not make it automatic.

## Demand and identity

Evaluation starts with a requested output. Demand travels toward prerequisites;
ready results allow their consumers to proceed. A condition must be evaluated to
discover which branch is needed. Loops discover dynamic iterations and state in
the same way. There is no requirement to instantiate the entire runtime graph
before computing a result, but this does not itself eliminate lazy loop chains.

Executing instructions directly inside a demanded chunk avoids a heap thunk for
every primitive. Escaping or shared suspended values retain explicit handles.
Minimal thunk allocation and minimal captured environments require later analysis.

Persistent identities describe computation code and accounted-for captured inputs,
within the host's compatibility namespace. Bytecode addresses, register numbers,
worker placement and source spans are not identities. Diagnostic provenance is
retained separately. Initially, identities can use deterministic semantic IR
fingerprints; arbitrary equivalent rewrites are not promised identical hashes.
The identity is retained when a thunk becomes ready and keys its reusable result.
Identities must never force captures merely to hash them.

## Verification and implementation sequence

The IR verifier checks references, operand/result types, region capture scopes,
dominance, terminators, branch arity and types, function signatures and legal
lazy/ready operations. A transformation may not silently introduce a force.
Source-to-IR checking is distinct from IR structural verification. Structural
verification alone does not prove that an optimization preserves behavior.

The first implementation should:

1. Define independent IR types, verifier, printer and execution contracts.
2. Lower the existing fully checked representation conservatively into this IR,
   keeping the current evaluator available as a migration reference.
3. Lower to bytecode and run the existing behavioral suites against both engines,
   comparing values, diagnostic provenance and observable demand/error behavior.
4. Carry existing module preparation, cache compatibility and spill contracts
   across the boundary; check restart reuse and storage failure recovery.
5. Make bytecode the normal execution path once those contracts pass, then remove
   the transitional representation where it no longer has a purpose.

Do not add optimization passes as part of this migration. In particular, baseline
tests must preserve lazy siblings, common local sharing, independent borrowed
outputs, generic projections, recursive calls, loop return decisions and storage
retry behavior. Resource tests must distinguish runtime overhead from caller-owned
materialized data. The goal is a stable semantic boundary for future passes, not
a promise that the internal instruction set or serialized format can never evolve.
