# IR and bytecode execution

Target architecture, not the implemented runtime. The current implementation is
described in [internals.md](internals.md). This document specifies the recommended
replacement and its implementation gates. It does not claim measured whole-language
speedups or a universally optimal evaluator.

## Decision

Keep a separate typed IR and a register bytecode VM. Use demand to choose what
computation to enter, then execute ordinary instructions forward inside it.
Preserve loops as compact, independently selectable recurrences until lowering.
Give simple scalar recurrences a forward execution path from the beginning.

```text
source -> whole-source checking -> typed IR with explicit lazy boundaries
       -> optional optimization passes
       -> bounded execution planning and register bytecode -> runtime
```

The baseline includes local sharing/capture information, register allocation and
conservative demand-order summaries. These are necessary to avoid a thunk and a
dictionary lookup per arithmetic operation. They are deliberately small compiler
passes, not a claim that fast execution needs no analysis whatsoever.

Loop splitting, iteration pruning, range analysis, general inlining, speculative
execution, parallel scheduling and JIT compilation are later work. The baseline
must already run ordinary arithmetic and supported scalar loops without allocating
a heap object for every intermediate value or iteration.

## Research and choices

The following work informs the design; the combination and its Lonk-specific
contracts are our design, not a performance result established by these papers.

| Reference | What we take from it | What we do not assume |
|---|---|---|
| [STG, especially sections 4, 7 and 10](https://www.microsoft.com/en-us/research/wp-content/uploads/1992/04/spineless-tagless-gmachine.pdf) | Shared closures, explicit delayed computation, unboxed arithmetic and updating a suspension with its result | One closure per source expression, or the historical machine's exact layout |
| [Extended call-by-push-value](https://www.cl.cam.ac.uk/~dm606/extended-call-by-push-value.pdf) | An IR can explicitly distinguish execution, suspension and shared call-by-need evaluation | Ordinary CBPV alone supplies memoization, or purity makes all evaluation orders interchangeable |
| [Demand analysis in Haskell, sections 2–3](https://simon.peytonjones.org/assets/pdfs/theory-and-practice.pdf) | Small demand summaries can justify direct scalar calls and avoid unnecessary allocation | Haskell's treatment of errors is Lonk's: the paper permits some changes to how failure manifests; Lonk preserves error precedence |
| [MLIR blocks and regions](https://mlir.llvm.org/docs/LangRef/#blocks), [RVSDG](https://arxiv.org/abs/1912.05036) | Typed values, explicit block arguments, structured regions and visible dependencies | A dependency on LLVM/MLIR or a separate representation for every pass |
| [Program Expression Graphs, sections 3 and 6](https://www.cs.cornell.edu/~ross/publications/eqsat/eqsat_tate_lmcs11.pdf) | Represent loop-varying values as recurrences; distinguish selecting a result from discovering loop termination | Equality saturation in the baseline, or a proof that interpreting PEGs backward is fastest |
| [Virtual Machine Showdown](https://www.complang.tuwien.ac.at/andi/tuonly/SkriptVMShowdown.pdf) | Register instructions reduce operand shuffling and dispatch opportunities | Register bytecode is always smaller or that its published JVM speedups predict Lonk's performance |
| [Optimistic Evaluation](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/07/icfp2003.pdf) | Runtime speculation can reduce laziness overhead, but needs bounded execution, suspension and delayed failures | An unconditional eager fallback is safe or a speculative runtime is simple enough for the baseline |

Choose direct calls for known functions and evaluate an unknown callee before
applying it. Lonk's checked, uncurried calls do not need a general currying
protocol. [Making a Fast Curry](https://simonmar.github.io/bib/papers/eval-apply.pdf)
helps distinguish that calling-convention question from the separate choice of
when to evaluate arguments.

## Semantic boundary

The checker still owns structural types, generics, refinement, usefulness and
writable-borrow admission. Check all source before evaluation. Lower mutation
into immutable versions and borrowed outputs into explicit results. Snapshot
semantics do not change.

Value-producing expressions are logically deferred until demanded. This applies
equally to local arithmetic and function calls: `let b = a + 1` and
`let b = bar(a)`, with `bar(v) = v + 1`, have the same demand behavior.
Inlining a function does not itself authorize earlier evaluation. A source
function boundary is neither a mandatory thunk boundary nor evidence of strictness.

Logical suspension does not require allocating one heap object per expression.
Allocation and instruction grouping implement that demand behavior; they cannot
change it. Executing a required dependency directly after entering its consumer
is different from executing it merely because its source statement was reached.

Preserve these observations in every execution mode:

- Selecting one result does not demand unrelated results, including ordinary
  and borrowed function outputs and independent loop state.
- Object shape is distinct from its field payloads; array shape is distinct
  from its element payloads.
- Demand order determines which error is observed. Preserve left/right operand,
  subject/index/bound and lexical/index export order from the existing evaluator.
- A later update may discard an earlier error or nonterminating computation.
  Merely assigning a value does not make its evaluation mandatory.
- Infrastructure failures remain retryable and distinct from language failures.

For compiler purposes, failure, divergence and demand are observable even though
Lonk has no language-level I/O. Instructions and summaries record those properties.
A transformation may move a force only when its ordering and failure obligations
are satisfied. Structural IR verification alone does not prove that.

## IR

Use one independently owned IR vocabulary, with typed SSA values and nested
regions. Blocks have typed arguments instead of phi instructions. Branches pass
arguments to successor blocks; a dynamic visit gives those arguments new values.
The IR has a verifier and deterministic printer, and no dependency on checker
environments, AST layouts or source names. Diagnostic metadata is attached separately.

Verify SSA dominance, operand/result types, branch arity, region captures, loop
state/result types and legal lazy/ready uses. Verify bytecode register bounds,
instruction boundaries, control-flow targets and root maps before executing
persisted code. A pass invalidates summaries affected by its changes; stale
summaries cannot authorize a faster schedule.

Two internal representations are explicit:

- `Need<T>`: an immediate ready value or a reference to shared suspended work.
- `Ready<T>`: the outer value is known. A ready container can still hold lazy
  members. A ready function is a closure, not its executed result.

Constants and already ready scalars need no heap thunk when used as `Need<T>`.
The operations are constants; typed primitives; immutable container operations;
closures and calls; `delay`, `force` and explicit sharing; branches; returns;
and structured loops. Primitive signatures specify their demanded operands and
order. Short-circuiting is control flow, not an eager two-operand instruction.

A delay region captures explicit immutable inputs. Shared suspended locals have
one memo identity per dynamic binding; distinct dynamic calls do not accidentally
share merely because their code matches. Copying a handle preserves sharing.
Loops and delays retain their bodies as inspectable IR, never opaque native blobs.

The loop operation contains the source snapshot, initial state, invariant
captures, a body region with explicit state parameters and result expressions,
and an optional early-exit result. Source assignments within a body already have
separate SSA versions; a later assignment can depend on an earlier one in that
same iteration. The body yields the final versions for the next iteration.

Keep this structured operation until after optional passes. A later pass can
split it, change its source/range, inline a call inside it or replace it with
ordinary blocks. All resulting code uses the same values, lazy boundaries and
control-flow operations. No new execution model is needed for those transformations.

## Bytecode and ordinary computation

### Function boundaries

A source function is a useful checking and code-ownership boundary, but not a
rule to evaluate all its locals at entry. In
`fn foo() { let a = bar() + 1; return {x = a}; }`, calculating `a` during the call
would force `bar` before `x` is requested. Keeping only the inner call lazy is
insufficient: its addition must also wait. Private locals can become dependencies
of returned lazy fields or closures.

Use compiler-generated chunks as execution units. A chunk may coincide with a
function body or represent a delayed part of it. Inside a demanded chunk, execute
ordinary instructions forward; emit new suspended chunks only for work that must
remain lazy. This preserves the convenience of function-shaped executable code
without changing the language's demand behavior.

[Nix's evaluator](https://github.com/NixOS/nix/blob/master/src/libexpr/eval.cc)
likewise permits thunks for local bindings, arguments, list elements and attributes,
not only entire calls. Its addition evaluator directly evaluates operands and
performs arithmetic; it does not require a distinct heap thunk for every primitive.

### Registers and calls

Lower demanded regions to compact register instructions. Keep constants,
variable-size operands, source locations and register/GC maps in side tables;
do not make every instruction as large as the largest Rust enum variant.
Start with portable Rust dispatch. Threaded dispatch and a native backend are
separate implementation choices requiring measurements.

Typed scalar registers hold raw `i64`/boolean data; heap references use handles.
Generic or possibly suspended slots need a discriminant. Do not force all of
these into one 64-bit tagged word by silently reducing the 64-bit integer range.
Packed homogeneous arrays and alternative generic slot layouts can be added later.

Entering a demanded expression executes its single-use prerequisites directly
in their prescribed order. Operations do not each acquire a thunk, hash key,
memo-table entry or scheduler task. Allocate a suspended cell when work must
remain unevaluated and escape, or when independent consumers must share it.
An uncertain escape/use case takes the general shared-cell path.

Group expressions by their demand and sharing relationships, not merely because
their source statements are adjacent. For `b = bar(a)`, the suspension for `b`
contains the call; after inlining it contains the addition and the required force
of `a`. The surrounding demand boundary remains in place. A conditional within
that chunk still demands only its selected branch.

For example, one field of this function is one demanded instruction sequence:

```text
fn foo() { return { x = 1 + bar(); }; }

foo:
    x = delay x_body(captured_bar)
    return record {x: x}

x_body:
    r0 = constant 1
    r1 = call_demanded captured_bar()
    r2 = checked_add_i64 r0, r1
    return r2
```

`call_demanded` obtains the callee's shallow result. It can execute a direct
known entry or an ordinary closure call. It does not first allocate a separate
application thunk only to force it immediately. An undemanded call instead stays
inside a delayed region. A ready-result entry never implies eager evaluation of
all of its arguments.

For `let a = bar() + 1; {x = a + baz(); y = a + zzz()}`, create one shared cell
for `a` and two field suspensions. Each field's bytecode forces `a` before its own
call. The addition instructions execute directly after those dependencies return.
Taking `x` never calls `zzz`, and later taking `y` reuses live `a`.

Frames use indexed slots and an explicit program counter. Suspend at the actual
force/call instruction and resume there; do not restart an expression and repeat
its bookkeeping. Local use/capture information permits frame-slot reuse and
direct captures instead of retaining whole parent frame chains. Tail calls reuse
continuations when their outputs and outstanding memo updates allow it.

## Compact loops

The runtime's general loop representation is a shared descriptor plus projections:

```text
LoopInstance = (body code, source snapshot, initial state, invariant captures)
StateAt(instance, iteration, result_slot)
```

Conceptually, with `S[0]` the initial state:

```text
S[k + 1, slot] = body_result(slot, source[k], lazy S[k], captures)
```

This equation specifies values, not an array of allocated states. For an ordinary
finite loop, demanding a result first demands the source shape and obtains its
length, then selects `StateAt(instance, length, slot)`. Constructing the suspended
loop does not traverse its iterations. The descriptor retains its source and
initial inputs; those are real retained data and must be accounted for.

Do not expand a loop into an application chain and then search that chain for
repeated or periodic patterns. Its body and iteration interval already describe
those patterns. Dynamic compression would add recognition work and must compare
captured inputs as well as code. Keep that as a possible representation experiment,
not a prerequisite of the execution model.

### General lazy path

Demanding a projection enters only the selected body result's bytecode. Reads of
previous state produce further projections, materialized as work cells only when
needed. Common suspended work in an iteration has a shared identity
`(instance, iteration, shared_region)`; requesting two outputs must not invent two
copies of that work while it is live. Ordinary local temporaries remain registers.

Thus independent `a` and `b` outputs do not require constructing `b`'s chain just
to request `a`. Coupled state remains coupled. A condition can require other state
before deciding a result. This is dynamic demand following the existing body,
not a proof that iterations or branches can be skipped in advance.

With early returns, a shared control computation discovers the first returning
iteration in source order. It demands the flags and state needed for that decision,
and selects the returned payload or exhausted-loop state. It must not presume
that the source length identifies the return point. All result projections share
that control decision, while their payloads remain independently lazy.

General demand can still require O(n) pending continuations for a recurrence.
The compact descriptor avoids preallocation; it does not prove constant-space
execution. That is why the following forward path belongs in the baseline.

### Forward scalar path

Recognize a deliberately narrow case: a scalar state's demanded update first
demands its own preceding scalar state, then computes the next scalar without
depending on another loop state or letting intermediate state escape. The zero-
iteration case returns the initial value. Examples include `a + 1` and a direct
call to a known function `fn(a: int) a + 1`.

For initial admission, an intermediate state captured by another state lane,
an output element or an escaping closure counts as escape. Check that with
local use/dependency information. This excludes coupled recurrences from the
simple path instead of accidentally introducing repeated prefix recomputation.

The local summary is an **ordered demand prefix**, not just a set of arguments
eventually used. Transparent bookkeeping can precede the prefix; another possibly
failing/diverging demand cannot. Known acyclic callees can supply a bounded summary;
unknown or recursive callees remain unknown. Cap summary size and fall back instead
of running an unbounded whole-program analysis.

For an admitted result, enter the initial state once and run its step bytecode
forward with one reusable scalar accumulator and frame. Every original iteration
still runs. There is no range narrowing, algebraic reassociation or loop-fission
pass. Result selection is already part of the loop's representation. Other result
slots retain their lazy projections.

For this class the demand sequence is the initial state followed by step 0,
step 1 and so on, in both evaluators. Executing that sequence directly removes
the otherwise redundant predecessor continuations. Checked arithmetic and error
locations stay identical. The live scalar state/frame overhead is O(1), excluding
the source, captured data, user results and work performed inside a step.

For example, `a + fail(i)` demands earlier `a` before the current failure and may
qualify. `fail(i) + a` cannot use the same forward schedule: demanding the final
iteration observes its failure before any predecessor. A branch that replaces
`a` with 42 may ignore every predecessor. It must use the lazy path unless a
stronger, valid summary is available.

Branching, mutually dependent state, unknown higher-order steps and early exits
need not qualify initially. A constant-time or small function is not automatically
safe to evaluate early. A stronger local summary can expand coverage later without
changing the loop representation or VM. Persist these summaries with prepared
code so ordinary evaluation does not recompute them.

### Sharing versus retention

Memoize live shared dependencies. Retain optional completed iteration results
only under a bounded cache policy; do not turn the sparse memo index into an
unbounded table of every iteration. An escaped snapshot remains a strong root.
A later request for an unretained intermediate result can replay the recipe or
load a compatible cached result. Preserve diagnostics when replaying.

This is an explicit tradeoff: arbitrary future observations, no recomputation and
constant RAM cannot all be promised. Hosts can provide disk storage when retaining
useful results outside RAM is preferable. Simple private scalar folds should not
pay a cache lookup or disk write for every iteration.

## Memory, caching and embedding

Executable modules are immutable and shared between sessions. Runtime cells,
containers, continuations and public lazy handles belong to an evaluation session.
Use traceable handles and reclaimable arenas, not permanently growing vectors of
frames/thunks. Roots include live registers, continuations, escaped values and
explicitly retained caches. Dead registers must not keep their old captures alive.
Generation-checked handles prevent a reclaimed slot from becoming another value
through a stale reference. Cyclic closures require tracing, not reference counting
alone. Begin single-threaded; no atomic operation is needed for ordinary arithmetic.

A suspension has code/captures and a memo state: suspended, running, ready, failed
or stored. Pending continuations are separate from computation identity. Completed
work releases captures unless provenance or a replay policy explicitly needs them.
Session storage must preserve the transitive lifetime of referenced lazy values;
spilling a container cannot make its children reclaimable. Reusable cache records
and lossless session spill keep their different retention contracts.

Computation identity uses semantic code identity and immutable captured-input
identities within the embedder's namespace. Bytecode addresses, register numbers,
worker IDs and source positions are not keys. A loop projection can use its loop
recipe identity, iteration and result identity without expanding its predecessors.
Identity construction never forces an input. Conservative extra dependencies can
cause misses, but incomplete keys cannot be used.

Keep designated cache/export boundaries' semantic identities independent of the
lowered execution schedule. Internal arithmetic registers need no per-operation
hash. A fast loop does not hash every accumulator unless a retained checkpoint or
export actually needs it. General optimizations must preserve a boundary's proven
recipe identity or create a new compatible key; arbitrary equal programs are not
promised equal computation hashes.

Cache probing is opt-in and targets useful suspended work, not every instruction.
Resume with existing host storage interfaces during migration. Lazy handles and
streaming traversal can later expose results without materializing whole trees;
the existing owned `Value` export remains a convenience operation. Host I/O, native
capabilities, compatibility namespaces, storage layout and budgets stay outside
the core. An embedding callback that influences results must have explicit,
accounted-for inputs and identity; hidden host state cannot enter a pure recipe.

Reclamation bounds dead overhead; it does not bound a genuinely live demand graph.
Expose accounting and cancellation safepoints. Further paging/spill is needed to
support a small RAM budget for large live workloads. Do not advertise a total
memory bound from a cache budget alone.

## Optimization and scheduling extension points

| Future work | Extension point |
|---|---|
| Loop splitting, fusion and iteration pruning | Rewrite structured loop regions, state parameters and sources before execution planning |
| Range analysis | Attach conservative facts to SSA values; use them in explicit transformations |
| Inlining and stronger demand analysis | Rewrite call/delay regions or supply verified ordered summaries |
| Better allocation and packed arrays | Representation lowering and container implementations, preserving the same value contracts |
| Parallel evaluation | Schedule sufficiently large demanded cells through the force/completion protocol |
| Native/JIT execution | Implement chunk calls, force, returns, roots and safepoints with the same observable behavior |

The initial scheduler has one worker. Future parallel execution must distinguish
a demand cycle from another task already computing a cell, share completion and
preserve error precedence. A bytecode chunk or primitive operation is not
automatically a parallel task. Independent computation still needs a granularity
policy before spawning work is profitable.

Keep pass interfaces, the verifier and printer stable enough to compose work;
do not promise that internal opcodes or serialization can never evolve. Every
optional pass can be disabled. Basic preparation must not invoke an optimizer
search, loop solver or JIT before evaluating a small one-shot script.

## Implementation and acceptance

Implement in this order:

1. Independent IR, printer, verifier, explicit demand order and loop semantics.
   Translate the existing checked graph conservatively; retain it as a behavioral
   reference during migration.
2. Register VM, shared cells, direct demanded calls, resume PCs, root tracking and
   reclamation. Establish ordinary expressions and independent lazy fields first.
3. Compact loop projections and shared exit control, followed by the narrow
   ordered-prefix scalar path. The baseline is not complete with generic lazy
   chains alone. Unknown cases must keep their correct fallback.
4. Carry preparation reuse, computation identities, host storage and failure
   recovery across the boundary. Remove transitional representations only after
   behavior, caching and resource checks pass.

Run existing behavioral suites through both engines. Add focused checks for
snapshots, shared locals, higher-order calls, borrow outputs, reset/overwrite,
error precedence, early returns, mixed-state loops, repeated projections and
storage retry. Generate small checked IR programs to compare the general and
fast paths; use the old evaluator to validate source-level lowering too.

Acceptance must measure both allocation and work:

- Demanded scalar expression chains execute in registers without per-operation
  work cells or hash lookups.
- Independent field demands retain sharing and leave failing siblings untouched.
- Supported scalar folds over growing inputs retain a constant number of private
  state/frame slots; report input/result storage separately.
- Creating an undemanded loop retains a descriptor, not an iteration-sized chain.
- Generic lazy recurrences remain correct when their active continuation count
  grows. Report that growth instead of calling descriptor size a memory bound.
- Exercise first selection, repeated selection, another field, cold preparation,
  prepared execution, restart reuse and edits. Include allocations, peak retained
  bytes, code size and preparation time, not only elapsed execution time.

An isolated Rust scalar-bytecode probe supports the narrow scheduling choice:
direct forward execution and predecessor-demand execution agree on the admitted
arithmetic/failure cases; reversed failures and overwritten state demonstrate why
the general fallback is required. Its million-step arithmetic case retains one
forward frame versus a million predecessor continuations. This is a kernel check,
not validation of the full lowering, cross-output sharing, storage, GC or Lonk's
performance against Nix. Those remain implementation acceptance work.
