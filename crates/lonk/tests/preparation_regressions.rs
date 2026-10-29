//! Regression checks for the preparation optimizations.
//! Sharing checker environments and record-type payloads copy-on-write must not
//! change what the checker accepts, which refinements it keeps per branch, how
//! snapshots and generic copies stay independent, or which borrowed outputs
//! reach the caller. Rows deliberately exercise joins, siblings and copies
//! rather than the basic forms the ordinary suites already cover.

use lonk::{Program, Syntax};

enum Expect {
    Value(&'static [&'static str], &'static str),
    Reject(&'static str),
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
        (Value(path, expected), Ok(p)) => match p.evaluate(path) {
            Ok(v) if v.to_string() == *expected => Ok(()),
            Ok(v) => Err(format!("value {v} but expected {expected}")),
            Err(e) => Err(format!("evaluation failed: {}", e.render(case.source))),
        },
    }
}

fn run(cases: &[Case]) {
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

/// A: refinements and writes made in one branch are joined, not leaked.
const BRANCHES: &[Case] = &[
    Case {
        id: "A1 match refinement selects the live payload",
        source: "let V = type ({a: int} | {b: int}); let v: V = {a = 1}; match v { {a: any; ...} => v.a; {b: any; ...} => v.b; }",
        expect: Value(&[], "1"),
    },
    Case {
        id: "A2 a refinement used once does not change the binding for a second match",
        source: "let V = type ({a: int} | {b: int}); let v: V = {a = 1}; let r = match v { {a: any; ...} => v.a; {b: any; ...} => v.b; }; match v { {a: any; ...} => v.a + r; {b: any; ...} => v.b; }",
        expect: Value(&[], "2"),
    },
    Case {
        id: "A3 branches writing different fields join to one version each",
        source: "let p = {x = 1; y = 2}; if p.x == 1 { p.x = 10; } else { p.y = 20; }; p.x + p.y",
        expect: Value(&[], "12"),
    },
    Case {
        id: "A4 the untaken branch's write is not applied",
        source: "let p = {x = 1; y = 2}; if p.x == 2 { p.x = 10; } else { p.y = 20; }; p.x * 100 + p.y",
        expect: Value(&[], "120"),
    },
    Case {
        id: "A5 a block-local shadow does not leak into the outer binding",
        source: "let x = 1; let y = { let x = 2; x + 1 }; x * 10 + y",
        expect: Value(&[], "13"),
    },
    Case {
        id: "A6 a write inside a taken branch reaches the outer binding",
        source: "let p = {x = 1}; if true { p.x = 5; }; p.x",
        expect: Value(&[], "5"),
    },
    Case {
        id: "A7 nested branches join inner writes before the outer join",
        source: "let p = {x = 1; y = 1}; if true { if p.x == 1 { p.x = 2; } else { p.x = 3; }; p.y = 4; } else { p.y = 9; }; p.x * 10 + p.y",
        expect: Value(&[], "24"),
    },
];

/// B: siblings and snapshots never share a mutable payload.
const SIBLINGS: &[Case] = &[
    Case {
        id: "B1 a copy keeps the old field while the original moves on",
        source: "let p = {x = 1; y = 2}; let q = p; if true { p.x = 5; }; q.x * 100 + p.y * 10 + p.x",
        expect: Value(&[], "125"),
    },
    Case {
        id: "B2 nested record copies are independent",
        source: "let p = {inner = {x = 1}}; let q = p; p.inner.x = 5; q.inner.x * 10 + p.inner.x",
        expect: Value(&[], "15"),
    },
    Case {
        id: "B3 array copies are independent",
        source: "let xs = [1, 2]; let ys = xs; xs[0] = 9; ys[0] * 10 + xs[0]",
        expect: Value(&[], "19"),
    },
    Case {
        id: "B4 a parameter is a copy of the argument",
        source: "let f = fn(v: {x: int}) -> int { v.x = 7; v.x }; let p = {x = 1}; f(p) * 10 + p.x",
        expect: Value(&[], "71"),
    },
    Case {
        id: "B5 a refined copy of a variant is written without touching the source",
        source: "let V = type ({a: int} | {b: int}); let v: V = {a = 1}; let w = v; match w { {a: any; ...} => { w.a += 5; }; {b: any; ...} => { w.b += 5; }; }; let x = match v { {a: any; ...} => v.a; {b: any; ...} => v.b; }; let y = match w { {a: any; ...} => w.a; {b: any; ...} => w.b; }; x * 10 + y",
        expect: Value(&[], "16"),
    },
    Case {
        id: "B6 three siblings from one base each keep their own payload",
        source: "let base = {x = 1; y = 1}; let a = base; let b = base; a.x = 2; b.y = 3; base.x * 100 + a.y * 10 + b.x + a.x + b.y",
        expect: Value(&[], "116"),
    },
    Case {
        id: "B7 the selected field of a copy is the copied version",
        source: "let p = {x = 1; y = 2}; let q = p; p.x = 5; {old = q; new = p}",
        expect: Value(&["old", "x"], "1"),
    },
];

/// C: generic row and type copies are instantiated per use, never aliased.
const GENERICS: &[Case] = &[
    Case {
        id: "C1 a row remainder projection keeps the other fields",
        source: "let strip = fn<R>(v: {x: int, ...R}) -> {...R} { v }; let r = strip({x = 1; y = 2}); r.y",
        expect: Value(&[], "2"),
    },
    Case {
        id: "C2 the stripped field is gone from the projected type",
        source: "let strip = fn<R>(v: {x: int, ...R}) -> {...R} { v }; let r = strip({x = 1; y = 2}); r.x",
        expect: Reject("`x`"),
    },
    Case {
        id: "C3 an identity-instantiated record is a copy",
        source: "let id = fn<T>(x: T) -> T { x }; let p = {x = 1}; let q = id(p); q.x = 5; p.x * 10 + q.x",
        expect: Value(&[], "15"),
    },
    Case {
        id: "C4 row-preserving increments produce independent versions",
        source: "let inc = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { ({ ...v; count = v.count + 1 }) }; let a = {count = 1; tag = \"a\"}; let b = inc(a); let c = inc(b); a.count * 100 + b.count * 10 + c.count",
        expect: Value(&[], "123"),
    },
    Case {
        id: "C5 generic array elements are separate copies",
        source: "let dup = fn<T>(x: T) -> [T] { [x, x] }; let xs = dup({x = 1}); xs[0].x = 9; xs[0].x * 10 + xs[1].x",
        expect: Value(&[], "91"),
    },
    Case {
        id: "C6 two instantiations of one generic do not share the parameter type",
        source: "let id = fn<T>(x: T) -> T { x }; let p = id({x = 1}); let s = id(\"s\"); p.x = 2; p.x + s.len()",
        expect: Value(&[], "3"),
    },
];

/// D: bodies that are never selected or called are still checked.
const REJECTED_BODIES: &[Case] = &[
    Case {
        id: "D1 an uncalled function body with a type error is rejected",
        source: "let f = fn() -> int { 1 + true }; let r = {g = f; h = 2}; r.h",
        expect: Reject("does not accept"),
    },
    Case {
        id: "D2 an uncalled body writing an unknown field is rejected",
        source: "let f = fn(p: {x: int}) { p.y = 1; }; let r = {g = f; h = 2}; r.h",
        expect: Reject("cannot add field"),
    },
    Case {
        id: "D3 a branch that is never taken is still checked",
        source: "let r = if false { 1 + true } else { 2 }; r",
        expect: Reject("does not accept"),
    },
    Case {
        id: "D4 the usefulness rule still applies",
        source: "let f = fn() -> int { 1 }; 2",
        expect: Reject("unused computation"),
    },
    Case {
        id: "D5 an unselected record field with a bad body is rejected",
        source: "let r = {bad = fn() -> int { 1 + true }; good = 2}; r.good",
        expect: Reject("does not accept"),
    },
];

/// E: borrowed outputs reach the caller through branches, loops and copies.
const BORROWED_OUTPUTS: &[Case] = &[
    Case {
        id: "E1 a borrow in one branch updates the caller's field",
        source: "let inc = fn(n: &int) { n += 1; }; let p = {a = 1; b = 2}; if true { inc(&p.a); } else { inc(&p.b); }; p.a * 10 + p.b",
        expect: Value(&[], "22"),
    },
    Case {
        id: "E2 borrows in a loop accumulate with plain writes",
        source: "let inc = fn(n: &int) { n += 1; }; let p = {a = 0}; for i in [1, 2, 3] { inc(&p.a); p.a += i; }; p.a",
        expect: Value(&[], "9"),
    },
    Case {
        id: "E3 a copy taken before a borrow keeps the earlier version",
        source: "let inc = fn(n: &int) { n += 1; }; let p = {a = 1}; let before = p; inc(&p.a); let after = p; before.a * 10 + after.a",
        expect: Value(&[], "12"),
    },
    Case {
        id: "E4 a borrow of a refined variant payload applied twice",
        source: "let V = type ({a: int} | {b: int}); let inc = fn(n: &int) { n += 1; }; let v: V = {b = 1}; match v { {a: any; ...} => { inc(&v.a); inc(&v.a); }; {b: any; ...} => { inc(&v.b); inc(&v.b); }; }; match v { {a: any; ...} => v.a; {b: any; ...} => v.b; }",
        expect: Value(&[], "3"),
    },
    Case {
        id: "E5 a borrowed write inside an untaken branch is not applied",
        source: "let inc = fn(n: &int) { n += 1; }; let p = {a = 1}; if p.a == 2 { inc(&p.a); }; p.a",
        expect: Value(&[], "1"),
    },
];

fn all_cases() -> Vec<&'static Case> {
    BRANCHES
        .iter()
        .chain(SIBLINGS)
        .chain(GENERICS)
        .chain(REJECTED_BODIES)
        .chain(BORROWED_OUTPUTS)
        .collect()
}

#[test]
fn branch_refinements_join_without_leaking() {
    run(BRANCHES);
}

#[test]
fn siblings_and_snapshots_stay_independent() {
    run(SIBLINGS);
}

#[test]
fn generic_row_and_type_copies_are_instantiated_per_use() {
    run(GENERICS);
}

#[test]
fn unselected_and_uncalled_bodies_are_still_checked() {
    run(REJECTED_BODIES);
}

#[test]
fn borrowed_outputs_propagate_through_branches_loops_and_copies() {
    run(BORROWED_OUTPUTS);
}

#[test]
fn every_accepted_program_round_trips_through_the_formatter() {
    for case in all_cases() {
        if matches!(case.expect, Reject(_)) {
            continue;
        }
        let formatted = Syntax::parse(case.source)
            .unwrap_or_else(|e| panic!("{}: {}", case.id, e.render(case.source)))
            .format();
        let original = Program::prepare(case.source)
            .unwrap_or_else(|e| panic!("{}: {}", case.id, e.render(case.source)))
            .evaluate(&[])
            .map(|v| v.to_string());
        let again = Program::prepare(&formatted)
            .unwrap_or_else(|e| panic!("{} formatted: {}", case.id, e.render(&formatted)))
            .evaluate(&[])
            .map(|v| v.to_string());
        assert_eq!(
            original, again,
            "{}: formatting changed the result",
            case.id
        );
    }
}
