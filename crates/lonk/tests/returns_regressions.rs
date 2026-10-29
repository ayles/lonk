//! Behavioral regression tests for `return`.
//! A `return` is a statement that ends the innermost function; each exit snapshots its ordinary
//! result and borrowed outputs under the guard of the path that reached it. Outputs of exits that
//! are not taken are never built, code after a definite exit is rejected, and refinements proven
//! by an exiting branch do not leak into the fallthrough path.

use lonk::{Program, Syntax};

enum Expect {
    Value(&'static [&'static str], &'static str),
    Reject(&'static str),
    EvalError(&'static [&'static str], &'static str),
}
use Expect::*;

struct Case {
    id: &'static str,
    source: &'static str,
    expect: Expect,
}

fn check(case: &Case) -> Result<(), String> {
    let program = Program::prepare(case.source);
    match (&case.expect, program) {
        (Reject(needle), Err(e)) => {
            if e.message.contains(needle) {
                Ok(())
            } else {
                Err(format!(
                    "rejected for another reason: {}",
                    e.render(case.source)
                ))
            }
        }
        (Reject(_), Ok(p)) => Err(format!(
            "accepted; type {}; value {:?}",
            p.result_type(),
            p.evaluate(&[]).map(|v| v.to_string())
        )),
        (_, Err(e)) => Err(format!("unexpectedly rejected: {}", e.render(case.source))),
        (Value(path, want), Ok(p)) => match p.evaluate(path) {
            Ok(v) if v.to_string() == *want => Ok(()),
            Ok(v) => Err(format!("value {v} but wanted {want}")),
            Err(e) => Err(format!("evaluation failed: {}", e.render(case.source))),
        },
        (EvalError(path, needle), Ok(p)) => match p.evaluate(path) {
            Ok(v) => Err(format!(
                "evaluated to {v} but an error containing {needle:?} was expected"
            )),
            Err(e) if e.message.contains(needle) => Ok(()),
            Err(e) => Err(format!(
                "failed for another reason: {}",
                e.render(case.source)
            )),
        },
    }
}

fn run_table(cases: &[Case]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| {
            check(c)
                .err()
                .map(|e| format!("{}: {e}\n    {}", c.id, c.source.trim()))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// A: exits in branches and the shape of the result.
const BASICS: &[Case] = &[
    Case {
        id: "A1 early exit, inferred result",
        source: r#"let f = fn(x: int) { if x > 0 { return x; }; 0 - x }; [f(3), f(-2)]"#,
        expect: Value(&[], "[3,2]"),
    },
    Case {
        id: "A2 return inside an if used as an expression",
        source: r#"let f = fn(c: bool) -> int { let v = if c { return 7; } else { 2 }; v + 1 }; [f(true), f(false)]"#,
        expect: Value(&[], "[7,3]"),
    },
    Case {
        id: "A3 return inside a let initializer block",
        source: r#"let f = fn(c: bool) -> int { let v = { if c { return 1; }; 2 }; v * 10 }; [f(true), f(false)]"#,
        expect: Value(&[], "[1,20]"),
    },
    Case {
        id: "A4 return inside a match arm used as an expression",
        source: r#"let f = fn(x: any) -> int { let n = match x { int => x; else => { return 0; }; }; n + 1 }; [f(4), f("s")]"#,
        expect: Value(&[], "[5,0]"),
    },
    Case {
        id: "A5 return as the only statement",
        source: r#"let f = fn() -> int { return 5; }; f()"#,
        expect: Value(&[], "5"),
    },
    Case {
        id: "A6 all arms of a match return, nothing after",
        source: r#"let f = fn(x: any) -> int { match x { int => { return 1; }; else => { return 2; }; }; }; [f(1), f("s")]"#,
        expect: Value(&[], "[1,2]"),
    },
    Case {
        id: "A7 inferred join of record exits keeps the common field",
        source: r#"let f = fn(c: bool) { if c { return { a = 1 }; }; { a = 2; b = 3 } }; f(true).a"#,
        expect: Value(&[], "1"),
    },
    Case {
        id: "A7b the uncommon field is optional after the join",
        source: r#"let f = fn(c: bool) { if c { return { a = 1 }; }; { a = 2; b = 3 } }; f(false).b"#,
        expect: Reject("optional"),
    },
    Case {
        id: "A8 a fresh literal at an annotated exit is checked exactly",
        source: r#"let f = fn(c: bool) -> { a: int } { if c { return { a = 1; b = 2 }; }; { a = 3 } }; f(true).a"#,
        expect: Reject(""),
    },
    Case {
        id: "A8b an existing wider value is projected at the exit",
        source: r#"let f = fn(c: bool) -> { a: int } { let w = { a = 1; b = 1 / 0 }; if c { return w; }; { a = 3 } }; f(true)"#,
        expect: Value(&[], r#"{"a":1}"#),
    },
    Case {
        id: "A9 the returned value stays lazy",
        source: r#"let f = fn(c: bool) -> int { if c { return 1 / 0; }; 2 }; f(false)"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "A9b and is forced only when demanded",
        source: r#"let f = fn(c: bool) -> int { if c { return 1 / 0; }; 2 }; f(true)"#,
        expect: EvalError(&[], "zero"),
    },
    Case {
        id: "A10 an unselected field of a returned record is never forced",
        source: r#"let f = fn() -> { a: int; b: int } { return { a = 1; b = 1 / 0 }; }; f().a"#,
        expect: Value(&[], "1"),
    },
    Case {
        id: "A11 the body of an unselected exit is never evaluated",
        source: r#"let f = fn(c: bool) -> int { if c { let big = 1 / 0; return big; }; 2 }; f(false)"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "A12 an annotated union result admits a literal exit",
        source: r#"let f = fn(c: bool) -> ("a" | "b") { if c { return "a"; }; "b" }; [f(true), f(false)]"#,
        expect: Value(&[], r#"["a","b"]"#),
    },
    Case {
        id: "A13 if-expression function body without braces",
        source: r#"let f = fn(c: bool) -> int { if c { return 1; } else { 2 } }; [f(true), f(false)]"#,
        expect: Value(&[], "[1,2]"),
    },
];

/// X: return nested inside expression positions.
const CONTEXTS: &[Case] = &[
    Case {
        id: "X1 return inside an array element",
        source: r#"let f = fn(c: bool) -> [int] { [1, { if c { return [9]; }; 2 }] }; [f(true), f(false)]"#,
        expect: Value(&[], "[[9],[1,2]]"),
    },
    Case {
        id: "X2 return inside a record field value",
        source: r#"let f = fn(c: bool) -> { a: int } { let r = { a = { if c { return { a = 0 }; }; 1 } }; r }; [f(true).a, f(false).a]"#,
        expect: Value(&[], "[0,1]"),
    },
    Case {
        id: "X3 return inside a call argument",
        source: r#"let f = fn(c: bool) -> int { let g = fn(x: int) -> int { x * 2 }; g({ if c { return 1; }; 5 }) }; [f(true), f(false)]"#,
        expect: Value(&[], "[1,10]"),
    },
    Case {
        id: "X4 return inside an if condition",
        source: r#"let f = fn(c: bool) -> int { if { if c { return 1; }; c == false } { 2 } else { 3 } }; [f(true), f(false)]"#,
        expect: Value(&[], "[1,2]"),
    },
    Case {
        id: "X5 return inside the right operand of && fires only when the left is true",
        source: r#"let f = fn(a: bool, c: bool) -> int { if a && { if c { return 7; }; true } { 1 } else { 2 } }; [f(false, true), f(true, true), f(true, false)]"#,
        expect: Value(&[], "[2,7,1]"),
    },
    Case {
        id: "X6 return inside the right operand of || fires only when the left is false",
        source: r#"let f = fn(a: bool, c: bool) -> int { if a || { if c { return 7; }; false } { 1 } else { 2 } }; [f(true, true), f(false, true), f(false, false)]"#,
        expect: Value(&[], "[1,7,2]"),
    },
    Case {
        id: "X7 return inside a closure ends the closure only",
        source: r#"let f = fn() -> int { let g = fn() -> int { return 1; }; g() + 1 }; f()"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "X8 a closure's return does not make the outer body unreachable",
        source: r#"let f = fn(c: bool) -> int { let g = fn(y: int) -> int { if c { return y; }; 0 }; g(1) + g(2) }; [f(true), f(false)]"#,
        expect: Value(&[], "[3,0]"),
    },
    Case {
        id: "X9 return inside a method ends the method",
        source: r#"let c = { n = 0; bump = fn(&self, by: int) { if by == 0 { return; }; self.n += by; }; get = fn(self) -> int { return self.n; }; }; c.bump(0); c.bump(5); c.get()"#,
        expect: Value(&[], "5"),
    },
    Case {
        id: "X10 return inside a default value is not in a function body",
        source: r#"let f = fn(a: int = { return 1; }) -> int { a }; f()"#,
        expect: Reject("return"),
    },
    Case {
        id: "X11 a closure default may return from itself",
        source: r#"let f = fn(g: fn(int) -> int = fn(x: int) -> int { return x + 1; }) -> int { g(1) }; f()"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "X12 a defaulted inner function returns from itself",
        source: r#"let f = fn() -> int { let g = fn(a: int = 3) -> int { if a > 2 { return a; }; 0 }; g() + g(1) }; f()"#,
        expect: Value(&[], "3"),
    },
    Case {
        id: "X13 generic early exit",
        source: r#"let first = fn<T>(xs: [T], fallback: T) -> T { for x in xs { return x; }; fallback }; [first([1, 2], 0), first([], 9)]"#,
        expect: Value(&[], "[1,9]"),
    },
    Case {
        id: "X14 row-preserving early exit",
        source: r#"let bump = fn<R>(v: { n: int; ...R }, c: bool) -> { n: int; ...R } { if c { return { ...v; n = 0 }; }; { ...v; n = v.n + 1 } }; [bump({ n = 1; tag = "t" }, true).tag, bump({ n = 1; tag = "u" }, false).tag]"#,
        expect: Value(&[], r#"["t","u"]"#),
    },
    Case {
        id: "X15 a fully returning right operand",
        source: r#"let f = fn(a: bool) -> int { if a && { return 7; } { 1 } else { 2 } }; let g = fn(a: bool) -> int { if a || { return 7; } { 1 } else { 2 } }; [f(true), f(false), g(true), g(false)]"#,
        expect: Value(&[], "[7,2,1,7]"),
    },
    Case {
        id: "X16 nested short-circuit guards",
        source: r#"let f = fn(a: bool, b: bool, c: bool) -> int { if a && (b || { if c { return 7; }; true }) { 1 } else { 2 } }; [f(false, false, true), f(true, true, true), f(true, false, true), f(true, false, false)]"#,
        expect: Value(&[], "[2,1,7,1]"),
    },
    Case {
        id: "X17 short-circuit with an exit used as a value",
        source: r#"let f = fn(a: bool, c: bool) -> int { let v = a && { if c { return 7; }; true }; if v { 1 } else { 2 } }; [f(false, true), f(true, true), f(true, false)]"#,
        expect: Value(&[], "[2,7,1]"),
    },
    Case {
        id: "X18 an exit in the left operand is unconditional",
        source: r#"let f = fn(a: bool, c: bool) -> int { if { if c { return 7; }; true } && a { 1 } else { 2 } }; [f(true, true), f(false, true), f(true, false), f(false, false)]"#,
        expect: Value(&[], "[7,7,1,2]"),
    },
    Case {
        id: "X19 the exit output of the skipped operand is never built",
        source: r#"let f = fn(a: bool) -> int { if a && { return 1 / 0; } { 1 } else { 2 } }; f(false)"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "X20 a fully returning left operand makes the rest unreachable",
        source: r#"let f = fn() -> int { if { return 7; } && true { 1 } else { 2 } }; f()"#,
        expect: Reject(""),
    },
];

/// D: dead code, liveness and exit shape rules.
const DEAD: &[Case] = &[
    Case {
        id: "D1 statement after return",
        source: r#"let f = fn() -> int { return 1; let x = 2; }; f()"#,
        expect: Reject("unreachable"),
    },
    Case {
        id: "D2 trailing expression after return",
        source: r#"let f = fn() -> int { return 1; 2 }; f()"#,
        expect: Reject("unreachable"),
    },
    Case {
        id: "D3 statement after an if whose branches both return",
        source: r#"let f = fn(c: bool) -> int { if c { return 1; } else { return 2; }; let y = 3; y }; f(true)"#,
        expect: Reject("unreachable"),
    },
    Case {
        id: "D4 a binding used only by an exit is live",
        source: r#"let f = fn(c: bool) -> int { let x = 5; if c { return x; }; 1 }; [f(true), f(false)]"#,
        expect: Value(&[], "[5,1]"),
    },
    Case {
        id: "D5 an unused binding in an exiting branch",
        source: r#"let f = fn(c: bool) -> int { if c { let x = 1; return 2; }; 3 }; f(true)"#,
        expect: Reject("unused"),
    },
    Case {
        id: "D6 a write that no path reads before the exit",
        source: r#"let f = fn(c: bool) -> int { let x = 1; if c { x = 2; return 5; }; x }; f(true)"#,
        expect: Reject(""),
    },
    Case {
        id: "D7 bare return in a borrowed function",
        source: r#"let f = fn(x: &int) { x += 1; return; }; let n = 1; f(&n); n"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "D8 bare return in a pure function",
        source: r#"let f = fn() { return; }; f()"#,
        expect: Reject(""),
    },
    Case {
        id: "D9 mixed bare and valued exits",
        source: r#"let f = fn(x: &int, c: bool) { if c { return 1; }; x = 2; return; }; let n = 0; f(&n, false); n"#,
        expect: Reject("all function exits"),
    },
    Case {
        id: "D9b valued exit with a statement fallthrough",
        source: r#"let f = fn(x: &int) -> int { if x > 0 { return 1; }; x = 0; }; let n = 0; f(&n)"#,
        expect: Reject("no result"),
    },
    Case {
        id: "D10 return at file level",
        source: r#"let x = 1; return x;"#,
        expect: Reject("function body"),
    },
    Case {
        id: "D11 return in a loop with no other effect is a contribution",
        source: r#"let f = fn(xs: [int]) -> bool { for x in xs { if x == 2 { return true; }; }; false }; [f([1, 2, 1 / 0]), f([])]"#,
        expect: Value(&[], "[true,false]"),
    },
    Case {
        id: "D12 a loop with neither update nor exit is still rejected",
        source: r#"let f = fn(xs: [int]) -> int { for x in xs { }; 0 }; f([1])"#,
        expect: Reject("does not update"),
    },
    Case {
        id: "D13 return value of the wrong type",
        source: r#"let f = fn() -> int { return "s"; }; f()"#,
        expect: Reject(""),
    },
    Case {
        id: "D14 bare return where a value is required",
        source: r#"let f = fn() -> int { return; }; f()"#,
        expect: Reject(""),
    },
];

/// G: refinements and guards around exits.
const GUARDS: &[Case] = &[
    Case {
        id: "G1 presence proven by the non-exiting arm survives the match",
        source: r#"let f = fn(v: { a?: int }) -> int { match v { { a: any; } => { }; else => { return 0; }; }; v.a }; [f({ a = 5 }), f({})]"#,
        expect: Value(&[], "[5,0]"),
    },
    Case {
        id: "G2 a refinement from the exiting arm must not leak",
        source: r#"let f = fn(x: any) -> int { match x { int => { return x; }; else => { }; }; x + 1 }; f(1)"#,
        expect: Reject(""),
    },
    Case {
        id: "G3 the non-exiting arm refinement persists past the exit",
        source: r#"let f = fn(m: ("a" | "b" | "c")) -> int { match m { "a" => { }; else => { return 0; }; }; match m { "a" => 1; } }; [f("a"), f("b")]"#,
        expect: Value(&[], "[1,0]"),
    },
    Case {
        id: "G4 the output of an untaken exit is never built",
        source: r#"let f = fn(c: bool) -> { a: int } { if c { return { a = 1 / 0 }; }; { a = 2 } }; f(false).a"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "G5 nested exits in both arms with a fallthrough",
        source: r#"let f = fn(x: int) -> int { if x > 10 { if x > 20 { return 2; }; return 1; }; 0 }; [f(25), f(15), f(5)]"#,
        expect: Value(&[], "[2,1,0]"),
    },
    Case {
        id: "G6 the guard of a deeper exit is not evaluated when a shallower one fires",
        source: r#"let f = fn(c: bool, n: int) -> int { if c { return 1; }; if n / 0 == 0 { return 2; }; 3 }; f(true, 1)"#,
        expect: Value(&[], "1"),
    },
];

/// B: borrowed outputs and aliases at exits.
const BORROWS: &[Case] = &[
    Case {
        id: "B1 the write before an exit reaches the caller",
        source: r#"let f = fn(x: &int, c: bool) -> int { if c { x = 10; return 1; }; x = 20; 2 }; let n = 0; let r = f(&n, true); [n, r]"#,
        expect: Value(&[], "[10,1]"),
    },
    Case {
        id: "B1b and the fallthrough write on the other path",
        source: r#"let f = fn(x: &int, c: bool) -> int { if c { x = 10; return 1; }; x = 20; 2 }; let n = 0; let r = f(&n, false); [n, r]"#,
        expect: Value(&[], "[20,2]"),
    },
    Case {
        id: "B2 nested block writes before an exit reach the caller",
        source: r#"let f = fn(x: &int) -> int { { let y = 5; x = y; { let z = 1; x += z; return 1; }; }; }; let n = 0; let v = f(&n); [n, v]"#,
        expect: Value(&[], "[6,1]"),
    },
    Case {
        id: "B3 a shadow inside a nested block does not hide the parameter's output",
        source: r#"let f = fn(x: &int, c: bool) -> int { x = 2; { let x = 10; if c { return x; }; }; x + 1 }; let n = 0; let a = f(&n, true); let b = f(&n, false); [n, a, b]"#,
        expect: Value(&[], "[2,10,3]"),
    },
    Case {
        id: "B4 an alias is closed by an exit inside its block",
        source: r#"let f = fn(c: bool) -> int { let n = 1; { let r = &n; r += 1; if c { return r; }; r += 1; }; n }; [f(true), f(false)]"#,
        expect: Value(&[], "[2,3]"),
    },
    Case {
        id: "B5 an alias of a borrowed parameter writes back at the exit",
        source: r#"let f = fn(x: &int, c: bool) -> int { let r = &x; r = 5; if c { return 1; }; r = 6; 2 }; let n = 0; let v = f(&n, true); [n, v]"#,
        expect: Value(&[], "[5,1]"),
    },
    Case {
        id: "B6 alias live across a loop, no return (control)",
        source: r#"let f = fn(xs: [int]) -> int { let n = 0; let r = &n; for x in xs { r += x; }; r }; f([1, 2, 3])"#,
        expect: Value(&[], "6"),
    },
    Case {
        id: "B6b alias live across a loop with a return inside",
        source: r#"let f = fn(xs: [int]) -> int { let n = 0; let r = &n; for x in xs { r += x; if r > 3 { return r; }; }; r }; [f([1, 2, 3, 4]), f([1])]"#,
        expect: Value(&[], "[6,1]"),
    },
    Case {
        id: "B7 alias opened inside the loop body, return inside",
        source: r#"let f = fn(xs: [{ v: int }]) -> int { let total = 0; for x in xs { let p = &x; total += p.v; if total > 2 { return total; }; }; total }; f([{ v = 1 }, { v = 2 }, { v = 1 / 0 }])"#,
        expect: Value(&[], "3"),
    },
    Case {
        id: "B8 borrowed method exits early without writing",
        source: r#"let c = { n = 1; set = fn(&self, v: int, skip: bool) { if skip { return; }; self.n = v; }; }; c.set(5, true); c.set(7, false); c.n"#,
        expect: Value(&[], "7"),
    },
];

/// L: loops.
const LOOPS: &[Case] = &[
    Case {
        id: "L1 matching element returns its lazy payload only",
        source: r#"let f = fn(xs: [{ k: int; v: int }]) -> int { for x in xs { if x.k == 1 { return x.v; }; }; 0 }; f([{ k = 0; v = 1 / 0 }, { k = 1; v = 7 }, { k = 1 / 0; v = 0 }])"#,
        expect: Value(&[], "7"),
    },
    Case {
        id: "L2 inner loop returns with the outer accumulator",
        source: r#"let f = fn() -> int { let acc = 0; for x in [1, 2, 3] { for y in [10, 20] { acc += y; if acc > 25 { return acc + x; }; }; }; acc }; f()"#,
        expect: Value(&[], "31"),
    },
    Case {
        id: "L3 zero iterations, updates and exits",
        source: r#"let f = fn(xs: [int]) -> int { let t = 1; for x in xs { t = t * x; if t > 100 { return 0 - t; }; }; t }; [f([]), f([2, 3]), f([50, 3, 1 / 0])]"#,
        expect: Value(&[], "[1,6,-150]"),
    },
    Case {
        id: "L4 unconditional return in the body",
        source: r#"let f = fn(xs: [int]) -> int { for x in xs { return x; }; 9 }; [f([5, 1 / 0]), f([])]"#,
        expect: Value(&[], "[5,9]"),
    },
    Case {
        id: "L5 outer loop exits after an inner loop updated the accumulator",
        source: r#"let f = fn() -> int { let acc = 0; for x in [1, 2, 3] { for y in [1, 2] { acc += x * y; }; if acc > 5 { return acc; }; }; 0 }; f()"#,
        expect: Value(&[], "9"),
    },
    Case {
        id: "L6 return inside a match inside a loop",
        source: r#"let f = fn(xs: [any]) -> int { for x in xs { match x { int => { if x > 1 { return x; }; }; else => { }; }; }; 0 }; [f(["s", 1, 5, 1 / 0]), f(["s"])]"#,
        expect: Value(&[], "[5,0]"),
    },
    Case {
        id: "L7 exit value computed from loop state and the element",
        source: r#"let f = fn(xs: [int]) -> int { let i = 0; for x in xs { if x == 7 { return i; }; i += 1; }; 0 - 1 }; [f([3, 7, 1 / 0]), f([3])]"#,
        expect: Value(&[], "[1,-1]"),
    },
    Case {
        id: "L8 borrowed accumulator written in the loop before the exit",
        source: r#"let f = fn(out: &[int], xs: [int]) -> int { for x in xs { out = [...out, x]; if x > 1 { return x; }; }; 0 }; let o: [int] = []; let r = f(&o, [1, 2, 3]); [o, [r]]"#,
        expect: Value(&[], "[[1,2],[2]]"),
    },
    Case {
        id: "L9 loop variable shadowed by a let before the exit",
        source: r#"let f = fn(xs: [int]) -> int { for x in xs { let x = x * 2; if x > 2 { return x; }; }; 0 }; f([1, 2, 1 / 0])"#,
        expect: Value(&[], "4"),
    },
];

/// R: recursion.
const RECURSION: &[Case] = &[
    Case {
        id: "R1 fib with a base-case return",
        source: r#"let fib = fn(n: int) -> int { if n < 2 { return n; }; fib(n - 1) + fib(n - 2) }; fib(15)"#,
        expect: Value(&[], "610"),
    },
    Case {
        id: "R2 both arms return through recursion",
        source: r#"let count = fn(n: int) -> int { if n == 0 { return 0; } else { return 1 + count(n - 1); } }; count(3000)"#,
        expect: Value(&[], "3000"),
    },
    Case {
        id: "R3 linear search by recursion with slices of state",
        source: r#"let find = fn(xs: [int], i: int, want: int) -> int { if i == 3 { return 0 - 1; }; if xs[i] == want { return i; }; find(xs, i + 1, want) }; [find([4, 5, 6], 0, 6), find([4, 5, 6], 0, 9)]"#,
        expect: Value(&[], "[2,-1]"),
    },
];

fn all_cases() -> Vec<&'static Case> {
    [BASICS, CONTEXTS, DEAD, GUARDS, BORROWS, LOOPS, RECURSION]
        .into_iter()
        .flatten()
        .collect()
}

#[test]
fn basics_and_result_shape() {
    run_table(BASICS);
}

#[test]
fn exits_in_expression_contexts() {
    run_table(CONTEXTS);
}

#[test]
fn dead_code_and_exit_shape() {
    run_table(DEAD);
}

#[test]
fn guards_and_refinements() {
    run_table(GUARDS);
}

#[test]
fn borrowed_outputs_and_aliases() {
    run_table(BORROWS);
}

#[test]
fn loops_with_exits() {
    run_table(LOOPS);
}

#[test]
fn recursion_with_exits() {
    run_table(RECURSION);
}

#[test]
fn deep_loop_exit_does_not_depend_on_the_native_stack() {
    let source = r#"
let build = fn(n: int) -> [int] { if n == 0 { return []; }; [...build(n - 1), n - 1] };
let f = fn(xs: [int]) -> int { for x in xs { if x == 1999 { return x; }; }; 0 };
f(build(2000))
"#;
    let program = Program::prepare(source).unwrap_or_else(|e| panic!("{}", e.render(source)));
    assert_eq!(
        program
            .evaluate(&[])
            .unwrap_or_else(|e| panic!("{}", e.render(source)))
            .to_string(),
        "1999"
    );
}

#[test]
fn canonical_form_of_return() {
    let formatted = Syntax::parse(
        "let f = fn() -> int { return 1; };\nlet g = fn(x: &int) { x += 1; return; };\nf()",
    )
    .unwrap()
    .format();
    assert!(formatted.contains("    return 1;\n"), "{formatted}");
    assert!(formatted.contains("    return;\n"), "{formatted}");
}

#[test]
fn formatter_round_trips_every_returns_program() {
    for case in all_cases() {
        let Ok(syntax) = Syntax::parse(case.source) else {
            continue;
        };
        let formatted = syntax.format();
        let again = Syntax::parse(&formatted)
            .unwrap_or_else(|e| panic!("{}: {}\n{formatted}", case.id, e.render(&formatted)))
            .format();
        assert_eq!(again, formatted, "{}", case.id);
        if let (Ok(a), Ok(b)) = (Program::prepare(case.source), Program::prepare(&formatted)) {
            assert_eq!(
                a.evaluate(&[]).map(|v| v.to_string()).ok(),
                b.evaluate(&[]).map(|v| v.to_string()).ok(),
                "{}",
                case.id
            );
        }
    }
}
