# Lonk language

This describes the implemented language. Unsupported features are listed at the
end. Lonk evaluates pure computations to structured values: it has no filesystem,
network, clock, process or package-manager operations. Hosts supply external data
and modules through the [embedding API](docs/embedding.md).

## Start here

```text
let original = { version = "1.0" };
let updated = original;
updated.version = "2.0";
{ before = original.version; after = updated.version }
```

The result is `{"after":"2.0","before":"1.0"}`. Every value is an independent
snapshot. Assignment constructs a new version; existing copies and closures keep
their original values. The runtime shares immutable storage to make copies cheap.

A file is a sequence of statements followed by a result expression. Top-level
values can be data, functions or type descriptors. JSON export requires data;
library objects can contain functions or types when consumed inside Lonk.

Run a [complete script](examples/showcase.lonk) with
`cargo run -p lonk-cli -- eval examples/showcase.lonk`.

## Types, copies and evidence

| Spelling | Meaning |
| --- | --- |
| `int` | Checked signed 64-bit integer |
| `bool` | `true` or `false` |
| `str` | Unicode string |
| `[T]` | Array of values satisfying `T` |
| `{x: int}` | Closed record containing exactly `x` |
| `{x: int; ...}` | Record with `x` and an unnamed remainder |
| `{x: int; ...R}` | Record with `x` and a preserved generic remainder |
| `{x?: int}` | Optional field; presence must be checked before reading |
| `any` | Unknown type; use `match` to establish a usable type |
| `fn(int) -> int` | Function contract |
| `"off" \| {on: int}` | Disjoint literal/record alternatives |

`let` fixes a binding's contract; reassignment must satisfy that contract and
current refinements. A new `let` may shadow a name with a different contract.
Values are copied at assignment and by-value calls, including typed projections:

```text
let p = { x = 1; y = "hello" };
let q: {x: int} = p;
{ original = p; projected = q }
```

Here `q` contains only `x`, while `p` retains both fields. Fresh record literals
at closed destinations reject extra fields, catching construction mistakes.
Borrowed values are not copies: a borrow cannot discard the caller's fields.
Generic rows express preservation, and `let rest: {...R} = value` extracts the
captured remainder without forcing the fields it excludes.

`match value { {x: int; ...} => value.x; else => 0; }` establishes that `x`
exists and is an integer in its arm. `{x: any; ...}` checks presence only. An
optional field already has a known payload type, so presence is enough. Boolean
conditions do not refine types. Ordinary reassignment cannot weaken established
facts; use a new binding when deliberately changing the contract.

## Evaluation and static checking

All source is checked before evaluation, including unused fields and uncalled
function bodies. A type error in an unused field rejects the program. A runtime
error such as division by zero is observed only if its value is demanded.
Selecting one field leaves unrelated fields lazy; exporting an entire data
object demands all its fields in UTF-8 lexical key order. Arrays export in index
order. Object order does not depend on insertion history or locale.

Unused expression results and locals that do not contribute to a function's
outputs are errors. Returned record fields count as outputs, including lazy
helpers exported for callers. Formal parameters may be unused.

## Syntax and behavior

The prelude exposes ordinary `array`, `object`, `string`, `integer`, and `boolean`
records. Most collection algorithms are checked Lonk source in
[`std/collections.lonk`](crates/lonk/std/collections.lonk); representation and string primitives
live in the evaluator. There is no builtin dictionary type.

- `return value;` and bare `return;` end the current function, including from
  nested branches, matches and finite loops. Bare return requires borrowed
  outputs. All exit paths agree on ordinary result presence and satisfy its type;
  aliases close and borrowed updates are preserved on the chosen exit. Unreachable
  following statements are errors. Tail expressions remain supported. Short-circuit
  `&&` and `||` also guard returns in their right operand.
- Array `xs[i]` reads/writes and half-open `xs[start:end]` slices. Either bound
  can be omitted; bounds require `0 <= start <= end <= len`. Invalid bounds fail
  when demanded. Slices share interior structure and immutable element thunks.
  Reads and slices demand their subject before index/bound expressions. Updates
  demand the root subject, capture path indices left to right, and check each
  array's bounds when that level is demanded. Unselected updated children stay
  lazy. A negative slice start fails before evaluating its end bound.
- Checked signed 64-bit `int`, `bool`, `str`, arrays, records, and type descriptors.
  Arithmetic overflow and division by zero are evaluation errors. Integer division
  truncates toward zero. Strings concatenate with `+`; ordering and object key
  output use locale-independent UTF-8 lexical order.
- `let`, shadowing, whole-name assignments and nested/array writes; fixed closed,
  open and optional record contracts; recursive typed-copy projection. Fresh
  records, including records with spreads, reject extras at closed destinations.
- Ordinary functions with explicitly typed or contextually typed parameters,
  optional result annotations, nested closures, and independent call frames.
- Receiver methods use `fn(self)` or `fn(&self)` and direct calls such as
  `counter.add(2)`. Field extraction is unbound: `let add = counter.add;
  add(&counter, 2)` and `(counter.add)(&counter, 2)` supply all arguments.
  Ordinary callable fields receive no implicit argument. Explicit receiver
  signatures use `fn(self: &Type, int)`, and receiver status is part of callable
  compatibility. See [`methods.lonk`](examples/methods.lonk).
  Inferred self sees the containing object's ordinary fields, including later
  ones, plus a generic remainder that preserves methods and extensions. Bodies
  capture the lexical environment at their original source position. Receiver
  methods themselves are excluded from inferred self to avoid recursive type
  expansion; sibling calls need a finite explicit receiver contract or an ordinary
  callable field. Copies retain the original method signature. Replacement bodies
  can use that signature as context. Writable receivers must be local places;
  auto-borrows obey ordinary overlap rules. Compute mutating indices beforehand.
  Explicit sibling contracts can quantify their own receiver row, such as
  `fn<R>(self: &{x: int; ...R})`, preserving the caller's complete shape.
- Trailing positional defaults (`fn(attempts: int = 3)`), with type inference
  from a default when the annotation is omitted. Defaults are checked at
  preparation and evaluated lazily in each call. They capture definition-time
  values and earlier value-parameter entry snapshots; body writes cannot change
  them. Defaults cannot read borrowed or later parameters or mutate their inputs.
  Callable types mark availability with `fn(int, int = ...) -> int`. Copying to
  a signature requiring every argument forgets omission availability; a function
  without a default cannot satisfy a signature promising one.
- Rigidly checked generic functions (`fn<T>`), inferred and explicit calls
  (`identity<int>(42)`), generic type aliases (`type<T>` and `Box<int>`), and
  contextual specialization of generic function values. Inference requires
  consistent evidence; it does not join conflicting arguments to `any`.
  Bound generic names do not affect compatibility: independently defined methods
  can share an array element type. Typed copies specialize generic callables inside
  records and arrays lazily, as well as at a direct callable destination.
  Callable types may quantify parameters (`fn<T>(T) -> T`), including nested
  callbacks and receiver rows. A lambda can inherit a quantified expected signature
  and is checked with rigid variables. Type parameters stay inside their signature;
  a nested quantifier may shadow a name with a distinct type variable.
  Typed copies can expose an inferred receiver row as an explicit quantified row;
  binder order, constraints and the callable body must still agree.
  See [`callable-contracts.lonk`](examples/callable-contracts.lonk).
- Captured object rows (`...R`) through value parameters/results and typed-copy
  extraction (`let rest: {...R} = value`). Nested projections retain unknown
  runtime tails and drop excluded fields without forcing them. Match-discovered
  remainder fields are readable, but cannot be overwritten by a generic body.
- Writable arguments (`fn(v: &T)` and `f(&place)`), nested reborrows, disjoint
  field/index updates, and independently lazy ordinary and borrowed outputs.
  Admission and writeback must preserve the caller's current guarantees;
  generic rows preserve unknown remainder fields. While a call holds a writable
  borrow, a later argument cannot read an overlapping place: copy it to a local
  first. This is a conservative restriction under snapshot semantics.
- Scoped aliases (`let alias = &place`) live until their block ends. Reads and
  read-only closure captures are snapshots; assignments and nested reborrows
  update the target. Overlapping parent access is suspended, disjoint siblings
  remain available, and an indexed target retains its original index. An alias
  cannot escape as a reference. Shadowing a live alias or its target is currently
  rejected; end its block first. Optional annotations use `let alias: &T = &place`
  and must preserve the target's guarantees. See
  [`aliases.lonk`](examples/aliases.lonk).
- Array spread (`[...items, value]`) and finite `for item in array` loops.
  Iteration uses a source snapshot and leaves element payloads lazy. Loop state
  retains binding contracts; facts account for zero iterations and all subsequent
  iterations. A loop body must update surrounding state and cannot discard a
  result. Its item parameter may be unused, allowing counting without forcing
  payloads. Generic map, fold and dictionary functions can be ordinary library
  code; see [`collections.lonk`](examples/collections.lonk).
- Direct recursion in a new `let` function binding, including generics and borrowed
  outputs. Recursive calls use the original definition handle after later outer
  reassignment. A recursive result needs an annotation or an expected function
  type; a borrowed statement body can infer no ordinary result. A shadowing
  initializer continues to see the previous binding. Mutual forward references
  and recursion through an enclosing record are not yet supported.
- Literal types, literal unions, and variants with distinct required single-field
  record alternatives. Union order and duplicates do not affect type identity.
  Admission checks the entire case set before projecting: a value with both tags
  is rejected. An explicit earlier closed copy can select one part intentionally.
  Ordinary literals widen to primitive types; variant contexts and match arms
  retain singleton evidence. Generic payloads support library options such as
  `type<T> ("none" | {some: T})`. See
  [`variants.lonk`](examples/variants.lonk).
- `if`/`else if`/`else` blocks, short-circuit boolean operators, and `match` with ordered
  bare type patterns and literal patterns. A final `else` is optional when static
  evidence proves all cases covered, including both boolean values; `any` still
  requires a fallback. Impossible patterns on finite variants are rejected. Structural
  patterns establish presence/payload facts within their arm: `{x: any; ...}`
  checks presence without demanding the payload, while retaining a declared
  optional field's known type. `{x: int; ...}` checks a dynamic payload too, but
  does not demand a payload whose declared type already proves it is an integer.
  Variant tags are checked without forcing statically known payloads. Branch and
  loop merges preserve variant exclusivity; a refined case supports payload writes.
  Ordinary boolean conditions do not refine types; `in` is a boolean membership
  test only. The old `is` expression is rejected. Equality requires fully known data
  types, including closed records; `any`, callables and unknown tails need checks.
- Unused expression results, dead locals and definitely overwritten whole binding
  versions are errors. Returned record fields count as outputs. Unused formal
  parameters are allowed. Liveness for field updates is conservative: the base
  object counts as an input, even if a later field write replaces the same payload.

Syntax uses semicolons after statements, `=` for record values, `:` for field
types, and commas for arguments/arrays. Record fields accept semicolons, commas,
or newlines; trailing separators are accepted. Functions with inferred results
may use an expression body: `fn(x) x * 2`, `fn(a, b) a + b`, or `fn() 42`.
An explicit result type requires a block: `fn(x: int) -> int { x * 2 }`.
`fn() { ... }` is a statement block; use `fn() ({ answer = 42 })` for a directly
returned record with an inferred result, or
`fn() -> {answer: int} { return {answer = 42}; }` with an explicit result.
There is no separate lambda spelling. Quoted field
names and literal string indexing are supported. `//` comments and nested
`/* ... */` comments are retained. Strings preserve literal newlines; `%%{`
escapes a literal `%{`, `${HOME}` is ordinary text, and `\%` is also accepted.
No interpolation is implemented yet.

Standalone braces beginning with a declaration or other statement, such as
`{ let alias = &value; alias += 1; }`, form a block. Empty braces or braces starting
with `name =`, a quoted field name followed by `=`, or spread form a record.
Loops and standalone statement blocks accept an omitted trailing semicolon;
the formatter always prints it.

## Collections

Arrays, strings, integers and booleans have fixed built-in methods, available in
every scope. Their lookup is independent of lexical `array`, `string`, `integer`
and `boolean` bindings; shadowing or extending those records cannot change member
behavior. Some implementations are checked language code internally.
A borrowed first parameter auto-borrows a local place: `xs.push(1)` updates `xs`.
`xs.len` extracts an unbound function: `let length = xs.len; length(xs)`.
The prelude namespaces also expose the standard functions for ordinary calls
such as `array.push(&xs, 1)`. Custom helpers use ordinary calls, for example
`let second = fn<T>(xs: [T]) -> T { xs[1] }; second([1, 2])`.
Objects always use their own fields; helpers use `object.method(value, ...)`.

| Value | Available operations |
| --- | --- |
| Array | `len`, `append`, `concat`, `slice`, `splice`, `insert`, `remove_at`, `take`, `drop`, `map`, `filter`, `fold`, `find`, `any`, `all`, `reverse`, `first`, `last`, `contains`, `index_of`, `push`, `extend`; `[str]` also has `join` |
| Object namespace | `len`, `keys`, `remove`, `lookup`, `with` |
| String | `len`, `split`, `contains`, `starts_with`, `ends_with`, `replace`, `trim`, plus `[start:end]` slices |
| Integer / boolean | `to_str` |

Array operations return copies except `push` and `extend`, which write through
borrows. `splice(start, end, replacement)` replaces a half-open range. `find`,
`first`, and `last` return `{value?: T}`; `index_of` returns `{index?: int}`.
`contains(value, equal)` and `index_of(value, equal)` accept an equality callback.
`find`, `any`, and `all` stop early. `filter` must evaluate predicates to discover
its output shape. `len`, slices, and object key operations do not force payloads.

`object.remove(value, key)` returns a copy, is idempotent when absent, and currently
has result type `{...}`. Recover retained field facts with `match`; this operation
cannot weaken an existing borrowed/binding contract. `object.lookup` returns
`{value?: any}`, and `object.with(value, key, payload)` returns an open copy.
These support library dictionaries over ordinary records. Keys use stable UTF-8
lexical order. No object fields are reserved for helpers.

The current array representation uses one tree node per element:
indexing is O(log n), length is O(1), and traversal is O(n). Chunked leaves remain
a possible memory/locality optimization. Sharing is in-memory within one session;
persistent artifact reuse is a separate mechanism and does not preserve this
session's shared container graph. Reclamation of unreachable session thunks and
restart recovery of lazy container/closure structure remain future work. Exported Rust `Value` is fully materialized owned data.

String lengths and slices count Unicode scalar values, not bytes or grapheme
clusters. `split("")` returns individual scalars with no leading/trailing empty
items. `trim()` removes ASCII space, tab, CR and LF. `replace` replaces all
occurrences; an empty search inserts at scalar boundaries. `string.join(xs, sep)`
and `xs.join(sep)` are equivalent. Integer conversion uses decimal notation.

## Modules

A module exports its ordinary result, including functions and types. Hosts supply
an explicit acyclic map of source texts and dependency binding names; there is no
filesystem import syntax. Dependency bindings are immutable. Copy one with `let`
to construct a local version. All reachable modules are checked, shared dependencies
are prepared once, and unrelated supplied modules are not evaluated. See
[modules in the embedding guide](docs/embedding.md#modules).

## Canonical formatting

The formatter has no style options. It prints four-space indentation, no
discretionary blank lines, one final newline for nonempty output, multiline
records and arrays, semicolon record separators, trailing array commas, normalized
literal spelling, and only the parentheses required by syntax and precedence.
Statement and field order are preserved. Comments are printed on separate lines
in source order, retaining their text; whitespace inside comments and string values
is content, not layout. The parsed syntax owns both the AST and comments. There is
no separate formatting AST. Formatting works before name/type resolution.

## Limits and planned work

Recursive receiver inference, general overlapping variants, direct computed record
keys, body-driven parameter inference, cyclic module interfaces, string interpolation
and tagged templates remain unimplemented. Host-declared acyclic modules work. Unsupported
constructs receive diagnostics. No unchecked interpreter fallback exists. Function
type compatibility requires matching argument/result types and borrow modes;
it can forget default availability but does not infer variance.

The checker conservatively requires whole replacements to preserve current flow
guarantees as well as the original contract. Resetting an incrementally refined
open binding requires an explicit new `let`. Field presence/type proofs are
not silently weakened by later writes. Mutations in binary right operands are
currently rejected; express those updates in `if` blocks.

Parser nesting, expression chains, structural types and data export have an
initial depth limit of 64. Evaluation uses an explicit heap work stack, including
recursive calls, with a limit of 131072 pending demands. Call frames are allocated
once; resumed work reuses memoized dependencies. Tests cover 10000 recursive
calls, 2000-element recursive arrays and deep lazy record traversal. Loop type
inference has an eight-pass convergence limit. These implementation limits are
diagnostics, not language numeric or semantic commitments. Reclamation of session metadata, paged container reload, bytecode, a language server
and production build/cache adapters remain future work. The CLI does not yet expose
persistent caching.

Computed-key patterns and typed homogeneous record tails are design proposals,
not implemented features. Typed dictionaries belong in library code: ordinary
generic factories should return objects with storage fields and receiver methods,
using general language features rather than a built-in dictionary type. A
`{*: T}` record constraint is a possible storage building block, not a dictionary
API. Optional-propagation syntax is not part of the language.
