//! Behavioral regression tests for ordinary generics and captured rows.
//! Whole programs; `1 / 0` is the must-not-force canary. Borrows are not used here,
//! so every row works by value: rows travel through parameters and results.

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

/// The helper every row-generic test uses: increments `count`, keeps the caller's remainder.
const INC: &str = "let inc = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { ({ ...v; count = v.count + 1 }) };";

fn rigid_bodies() -> Vec<Case> {
    vec![
        Case {
            id: "R1",
            source: "let id = fn<T>(x: T) -> T { x }; { a = id(1); b = id(\"s\"); c = id({ x = 1; y = 2 }) }",
            expect: Value(&[], r#"{"a":1,"b":"s","c":{"x":1,"y":2}}"#),
        },
        Case {
            id: "R2",
            source: "let f = fn<T>(x: T) -> int { x + 1 }; f(1)",
            expect: Reject(""),
        },
        Case {
            id: "R3",
            source: "let f = fn<T>(x: T) -> int { x.n }; f({ n = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "R4",
            source: "let f = fn<T>(a: T, b: T) -> bool { a == b }; f(1, 1)",
            expect: Reject(""),
        },
        Case {
            id: "R5",
            source: "let f = fn<T>(x: T) -> { value: T } { ({ value = x }) }; f(41).value + 1",
            expect: Value(&[], "42"),
        },
        Case {
            id: "R6",
            source: "let f = fn<T>(x: T) -> [T] { [x, x] }; f(\"s\")",
            expect: Value(&[], r#"["s","s"]"#),
        },
        Case {
            id: "R7",
            source: "let f = fn<T>(xs: [T]) -> T { xs[0] }; f([7, 8])",
            expect: Value(&[], "7"),
        },
        Case {
            id: "R8",
            source: "let f = fn<T>(xs: [{ v: T }]) -> T { xs[0].v }; f([{ v = \"a\" }])",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "R9",
            source: "let f = fn<T>(x: T) -> fn() -> T { fn() -> T { x } }; f(5)()",
            expect: Value(&[], "5"),
        },
        Case {
            id: "R10",
            source: "let f = fn<T>(x: T) -> T { let y = x; y }; f(3)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "R11",
            source: "let f = fn<T>(x: int) -> int { x }; f(1)",
            expect: Reject(""),
        },
        Case {
            id: "R12",
            source: "let f = fn<R>(v: R) -> R { v }; f(1)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "R13",
            source: "let f = fn<T>(v: T, w: { x: int, ...T }) -> int { w.x }; f(1, { x = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "R14",
            source: "let f = fn<T>(x: T) -> T { x = x; x }; f(1)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "R15",
            source: "let f = fn<R>(v: { x: int, ...R }) -> int { match v { { y: int; ...R } => v.y; else => 0; } }; f({ x = 1; y = 2 })",
            expect: Reject(""),
        },
    ]
}

fn inference() -> Vec<Case> {
    vec![
        Case {
            id: "I1",
            source: "let pair = fn<T>(a: T, b: T) -> [T] { [a, b] }; pair(1, \"s\")",
            expect: Reject(""),
        },
        Case {
            id: "I2",
            source: "let pair = fn<T>(a: T, b: T) -> [T] { [a, b] }; pair({ x = 1 }, { x = 1; y = 2 })",
            expect: Reject(""),
        },
        Case {
            id: "I3",
            source: "let empty = fn<T>() -> [T] { [] }; let xs: [int] = empty(); xs",
            expect: Value(&[], "[]"),
        },
        Case {
            id: "I4",
            source: "let empty = fn<T>() -> [T] { [] }; empty()",
            expect: Reject(""),
        },
        Case {
            id: "I5",
            source: "let apply = fn<T>(f: fn(T) -> T, x: T) -> T { f(x) }; apply(fn(n: int) -> int { n + 1 }, 1)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "I6",
            source: "let id = fn<T>(x: T) -> T { x }; let apply = fn<T>(f: fn(T) -> T, x: T) -> T { f(x) }; apply(id, 1)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "I7",
            source: "let id = fn<T>(x: T) -> T { x }; let g = id; { a = g(1); b = g(\"s\") }",
            expect: Value(&[], r#"{"a":1,"b":"s"}"#),
        },
        Case {
            id: "I8",
            source: "let lib = { id = fn<T>(x: T) -> T { x } }; { a = lib.id(1); b = lib.id(\"s\") }",
            expect: Value(&[], r#"{"a":1,"b":"s"}"#),
        },
        Case {
            id: "I9",
            source: "let k = 1; let tag = fn<T>(x: T) -> { k: int; x: T } { ({ k = k; x = x }) }; tag(\"s\")",
            expect: Value(&[], r#"{"k":1,"x":"s"}"#),
        },
        Case {
            id: "I10",
            source: "let f = fn<T>(x: T) -> T { x }; let g: fn(int) -> int = f; g(2)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "I11",
            source: "let f = fn<T>(x: T) -> T { x }; f(1 / 0); 1",
            expect: Reject("unused"),
        },
        Case {
            id: "I12",
            source: "let first = fn<T>(xs: [T]) -> T { xs[0] }; first([1, 1 / 0])",
            expect: Value(&[], "1"),
        },
        Case {
            id: "I13",
            source: "let first = fn<T>(xs: [T]) -> T { xs[0] }; first([1 / 0, 1])",
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "I14",
            source: "let id = fn<T>(x: T) -> T { x }; id<int>(1)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "I15",
            source: "let id = fn<T>(x: T) -> T { x }; id<str>(1)",
            expect: Reject(""),
        },
        Case {
            id: "I16",
            source: "let id = fn<T>(x: T) -> T { x }; id<int, str>(1)",
            expect: Reject(""),
        },
        Case {
            id: "I17",
            source: "let empty = fn<T>() -> [T] { [] }; empty<int>()",
            expect: Value(&[], "[]"),
        },
        Case {
            id: "I18",
            source: "let pair = fn<A, B>(a: A, b: B) -> { a: A; b: B } { ({ a = a; b = b }) }; pair<int, str>(1, \"s\")",
            expect: Value(&[], r#"{"a":1,"b":"s"}"#),
        },
        Case {
            id: "I19",
            source: "let box = fn<T>(x: T) -> { value: T } { ({ value = x }) }; let b: { value: int } = box<int>(1 / 0); 1",
            expect: Reject("unused"),
        },
        Case {
            id: "I20",
            source: "let f = fn(x: int) -> int { x }; f<int>(1)",
            expect: Reject(""),
        },
        Case {
            id: "I21",
            source: "let id = fn<T>(x: T) -> T { x }; let q = { x = 1; y = 2 }; let p: { x: int, ... } = id(q); p",
            expect: Value(&[], r#"{"x":1,"y":2}"#),
        },
        Case {
            id: "I22",
            source: "let id = fn<T>(x: T) -> T { x }; let q = { x = 1; y = 2 }; let p: { x: int } = id(q); p",
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "I23 a fresh literal where T is established is checked like an annotated binding",
            source: r#"let push = fn<T>(values: &[T], value: T) { values = [...values, value]; }; let xs: [{ name: str; port?: int }] = []; push(&xs, { name = "sshd"; port = 22 }); push(&xs, { name = "editor" }); xs"#,
            expect: Value(&[], r#"[{"name":"sshd","port":22},{"name":"editor"}]"#),
        },
        Case {
            id: "I24 an unknown field in that literal is still a typo",
            source: r#"let push = fn<T>(values: &[T], value: T) { values = [...values, value]; }; let xs: [{ name: str; port?: int }] = []; push(&xs, { name = "x"; nope = 1 }); xs"#,
            expect: Reject("fresh literal"),
        },
        Case {
            id: "I25 a non-literal wider value is not projected: inference stays exact",
            source: r#"let push = fn<T>(values: &[T], value: T) { values = [...values, value]; }; let xs: [{ name: str; port?: int }] = []; let wide = { name = "x"; extra = 1 }; push(&xs, wide); xs"#,
            expect: Reject("conflicting inference"),
        },
        Case {
            id: "I26 established by an earlier value argument",
            source: r#"let pair = fn<T>(a: T, b: T) -> [T] { [a, b] }; let f = fn(s: { n: int; tag?: str }) -> [{ n: int; tag?: str }] { pair(s, { n = 2; tag = "t" }) }; f({ n = 1 })"#,
            expect: Value(&[], r#"[{"n":1},{"n":2,"tag":"t"}]"#),
        },
        Case {
            id: "I27 a literal first still determines T, so order does not silently change meaning",
            source: r#"let pair = fn<T>(a: T, b: T) -> [T] { [a, b] }; let f = fn(s: { n: int; tag?: str }) -> [{ n: int; tag?: str }] { pair({ n = 2; tag = "t" }, s) }; f({ n = 1 })"#,
            expect: Reject("conflicting inference"),
        },
        Case {
            id: "I28 a literal whose parameter still mentions an open variable infers as before",
            source: r#"let both = fn<T, U>(a: T, b: { t: T; u: U }) -> U { b.u }; both(1, { t = 1; u = "s" })"#,
            expect: Value(&[], r#""s""#),
        },
        Case {
            id: "I29 the findings case through the builtin push",
            source: r#"let Service = type { name: str; port?: int }; let services: [Service] = []; services.push({ name = "sshd"; port = 22 }); services.push({ name = "editor" }); services"#,
            expect: Value(&[], r#"[{"name":"sshd","port":22},{"name":"editor"}]"#),
        },
        Case {
            id: "I30 explicit type arguments keep working",
            source: r#"let Service = type { name: str; port?: int }; let services: [Service] = []; services.push<Service>({ name = "x" }); services"#,
            expect: Value(&[], r#"[{"name":"x"}]"#),
        },
    ]
}

fn rows() -> Vec<Case> {
    vec![
        Case {
            id: "W1",
            source: "INC let p = { count = 1; label = \"a\" }; inc(p)",
            expect: Value(&[], r#"{"count":2,"label":"a"}"#),
        },
        Case {
            id: "W2",
            source: "INC let p = { count = 1; label = \"a\" }; inc(p).label",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "W3",
            source: "INC let p = { count = 1; label = \"a\" }; let q = inc(p); q == { count = 2; label = \"a\" }",
            expect: Value(&[], "true"),
        },
        Case {
            id: "W4",
            source: "INC inc({ count = 1; label = \"a\" }).label",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "W5",
            source: "INC let p: { count: int, ... } = { count = 1; hidden = \"h\" }; inc(p)",
            expect: Value(&[], r#"{"count":2,"hidden":"h"}"#),
        },
        Case {
            id: "W6",
            source: "INC let p: { count: int, ... } = { count = 1; hidden = \"h\" }; inc(p).hidden",
            expect: Reject("not known"),
        },
        Case {
            id: "W7",
            source: "INC let p: { count: int, ... } = { count = 1 }; p.label = \"a\"; inc(p).label",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "W8",
            source: "INC let p = { count = 1; bad = 1 / 0 }; inc(p).count",
            expect: Value(&[], "2"),
        },
        Case {
            id: "W9",
            source: "INC let p = { count = 1; f = fn(n: int) -> int { n * 2 } }; inc(p).f(21)",
            expect: Value(&[], "42"),
        },
        Case {
            id: "W10",
            source: "INC let p: { count: int; label?: str } = { count = 1 }; match inc(p) { { label: any; ... } => 1; else => 0; }",
            expect: Value(&[], "0"),
        },
        Case {
            id: "W11",
            source: "INC let p: { count: int; label?: str } = { count = 1; label = \"a\" }; let q = inc(p); match q { { label: any; ... } => { q.label }; else => \"none\"; }",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "W12",
            source: "INC inc({ count = 1 })",
            expect: Value(&[], r#"{"count":2}"#),
        },
        Case {
            id: "W13",
            source: "INC inc({ label = \"a\" })",
            expect: Reject(""),
        },
        Case {
            id: "W14",
            source: "INC let twice = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { inc(inc(v)) }; twice({ count = 1; label = \"a\" })",
            expect: Value(&[], r#"{"count":3,"label":"a"}"#),
        },
        Case {
            id: "W15",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { ({ count = 10 }) }; f({ count = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "W16",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { ({ ...v; count = \"s\" }) }; f({ count = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "W17",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { ({ ...v; extra = 1 }) }; f({ count = 1; extra = \"s\" })",
            expect: Reject(""),
        },
        Case {
            id: "W18",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { v.extra = 1; v }; f({ count = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "W19",
            source: "let f = fn<R>(v: { count: int, ...R }) -> int { v.label }; f({ count = 1; label = 2 })",
            expect: Reject(""),
        },
        Case {
            id: "W20",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { v.count += 1; v }; f({ count = 1; label = \"a\" })",
            expect: Value(&[], r#"{"count":2,"label":"a"}"#),
        },
        Case {
            id: "W21",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { ({ count = 5; ...v }) }; f({ count = 1; label = \"a\" })",
            expect: Value(&[], r#"{"count":1,"label":"a"}"#),
        },
        Case {
            id: "W22",
            source: "let f = fn<R>(v: { ...R }) -> { ...R } { v }; f({ x = 1; y = 2 })",
            expect: Value(&[], r#"{"x":1,"y":2}"#),
        },
        Case {
            id: "W23",
            source: "let f = fn<R>(v: { a: { x: int }, ...R }) -> { a: { x: int }, ...R } { v }; let p = { a = { x = 1; y = 2 }; z = 3 }; f(p)",
            expect: Value(&[], r#"{"a":{"x":1},"z":3}"#),
        },
        Case {
            id: "W23b",
            source: "let f = fn<R>(v: { a: { x: int }, ...R }) -> { a: { x: int }, ...R } { v }; f({ a = { x = 1; y = 2 }; z = 3 })",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "W27",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { match v { { label: str; ... } => ({ ...v; label = \"b\" }); else => v; } }; f({ count = 1; label = \"a\" })",
            expect: Reject("captured generic remainder"),
        },
        Case {
            id: "W28",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { match v { { label: str; ... } => ({ ...v; label = 1 }); else => v; } }; f({ count = 1; label = \"a\" })",
            expect: Reject(""),
        },
        Case {
            id: "W24",
            source: "let f = fn<R>(v: { a: { x: int, ...R } }) -> { a: { x: int, ...R } } { v }; f({ a = { x = 1; y = 2 } })",
            expect: Value(&[], r#"{"a":{"x":1,"y":2}}"#),
        },
        Case {
            id: "W25",
            source: "let f = fn<A, B>(a: { ...A }, b: { ...B }) -> { ...A, ...B } ({ ...a; ...b }); f({ x = 1 }, { y = 2 })",
            expect: Reject(""),
        },
        Case {
            id: "W26",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { match v { { label: str; ... } => { v.count = v.count + 1; }; else => { }; }; v }; f({ count = 1; label = \"a\" })",
            expect: Value(&[], r#"{"count":2,"label":"a"}"#),
        },
    ]
}

fn extraction() -> Vec<Case> {
    vec![
        Case {
            id: "X1",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { x: int, ...R } { let rest: { ...R } = v; { ...rest; x = 10 } }; f({ x = 1; name = \"n\"; enabled = true })",
            expect: Value(&[], r#"{"enabled":true,"name":"n","x":10}"#),
        },
        Case {
            id: "X2",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { x: int, ...R } { let rest: { ...R } = v; { x = 10; ...rest } }; f({ x = 1; name = \"n\" })",
            expect: Value(&[], r#"{"name":"n","x":10}"#),
        },
        Case {
            id: "X3",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { ...R } { let rest: { ...R } = v; rest }; f({ x = 1; name = \"n\" })",
            expect: Value(&[], r#"{"name":"n"}"#),
        },
        Case {
            id: "X4",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { ...R } { let rest: { ...R } = v; rest }; let p: { x: int, ... } = { x = 1; name = \"n\"; extra = true }; f(p)",
            expect: Value(&[], r#"{"extra":true,"name":"n"}"#),
        },
        Case {
            id: "X5",
            source: "let f = fn<R>(v: { x: int, y?: str, ...R }) -> { ...R } { let rest: { ...R } = v; rest }; f({ x = 1; name = \"n\" })",
            expect: Value(&[], r#"{"name":"n"}"#),
        },
        Case {
            id: "X6",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { x: int, ...R } { let rest: { ...R } = v; { ...rest } }; f({ x = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "X7",
            source: "let f = fn<R>(v: { x: int, ...R }, w: { y: int, ... }) -> { ...R } { let rest: { ...R } = w; rest }; f({ x = 1 }, { y = 2 })",
            expect: Reject(""),
        },
        Case {
            id: "X8",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { y: str, ...R } { let rest: { y: str, ...R } = v; rest }; f({ x = 1; y = \"s\" })",
            expect: Reject(""),
        },
        Case {
            id: "X9",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { ...R } { let rest: { ...R } = v; rest }; f({ x = 1 / 0; name = \"n\" })",
            expect: Value(&[], r#"{"name":"n"}"#),
        },
        Case {
            id: "X10",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { x: int, ...R } { let rest: { ...R } = v; v.x = 5; { ...rest; x = v.x } }; f({ x = 1; name = \"n\" })",
            expect: Value(&[], r#"{"name":"n","x":5}"#),
        },
        Case {
            id: "X11",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { x: int, ...R } { let whole: { ... } = v; { ...whole; x = 10 } }; f({ x = 1 })",
            expect: Reject(""),
        },
        Case {
            id: "X12",
            source: "let p = { x = 1; name = \"n\" }; let rest: { ...R } = p; rest",
            expect: Reject(""),
        },
        Case {
            id: "X13",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { ...R } { match v { { label: str; ... } => { let rest: { ...R } = v; rest }; else => { let rest: { ...R } = v; rest }; } }; f({ count = 1; label = \"a\" })",
            expect: Value(&[], r#"{"label":"a"}"#),
        },
        Case {
            id: "X14",
            source: "let f = fn<R>(v: { a: { x: int, ...R } }) -> { ...R } { let rest: { ...R } = v.a; rest }; f({ a = { x = 1; y = 2 } })",
            expect: Value(&[], r#"{"y":2}"#),
        },
        Case {
            id: "X15",
            source: "let f = fn<R>(v: { count: int, ...R }) -> { count: int, ...R } { match v { { label: str; ... } => ({ ...v; count = 0 }); else => v; } }; f({ count = 1; label = \"a\" })",
            expect: Value(&[], r#"{"count":0,"label":"a"}"#),
        },
    ]
}

fn aliases() -> Vec<Case> {
    vec![
        Case {
            id: "G1",
            source: "let Box = type<T> { value: T }; let b: Box<int> = { value = 1 }; b.value",
            expect: Value(&[], "1"),
        },
        Case {
            id: "G2",
            source: "let Box = type<T> { value: T }; let b: Box<int> = { value = \"s\" }; b.value",
            expect: Reject(""),
        },
        Case {
            id: "G3",
            source: "let Pair = type<A, B> { first: A; second: B }; let p: Pair<int, str> = { first = 1; second = \"s\" }; p",
            expect: Value(&[], r#"{"first":1,"second":"s"}"#),
        },
        Case {
            id: "G4",
            source: "let Box = type<T> { value: T }; let unbox = fn<T>(b: Box<T>) -> T { b.value }; unbox({ value = 7 })",
            expect: Value(&[], "7"),
        },
        Case {
            id: "G5",
            source: "let Box = type<T> { value: T }; let b: Box = { value = 1 }; b",
            expect: Reject(""),
        },
        Case {
            id: "G6",
            source: "let Box = type<T> { value: T }; let b: Box<int, int> = { value = 1 }; b",
            expect: Reject(""),
        },
        Case {
            id: "G7",
            source: "let Box = type<T> { value: T }; let b: Box<int> = { value = 1; extra = 2 }; b",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "G8 a dotted type path selects an exported type value",
            source: r#"let types = { P = type { name: str } }; let p: types.P = { name = "x" }; p"#,
            expect: Value(&[], r#"{"name":"x"}"#),
        },
        Case {
            id: "G9 and checks a fresh literal exactly",
            source: r#"let types = { P = type { name: str } }; let p: types.P = { name = "x"; extra = 1 }; p"#,
            expect: Reject("fresh literal"),
        },
        Case {
            id: "G10 a generic alias through a path",
            source: r#"let types = { Box = type<T> { v: T } }; let b: types.Box<int> = { v = 1 }; b.v"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "G11 a value field is not a type",
            source: r#"let types = { n = 1 }; let p: types.n = 1; p"#,
            expect: Reject("not a type"),
        },
        Case {
            id: "G12 a path through a non-object",
            source: r#"let types = 1; let p: types.P = 1; p"#,
            expect: Reject("not an object"),
        },
        Case {
            id: "G13 nested paths",
            source: r#"let m = { types = { P = type { a: int } } }; let p: m.types.P = { a = 1 }; p.a"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "G14 a generic alias needs its arguments",
            source: r#"let types = { Box = type<T> { v: T } }; let b: types.Box = { v = 1 }; b"#,
            expect: Reject("needs type arguments"),
        },
        Case {
            id: "G15 an unknown field on the path",
            source: r#"let types = { P = type { a: int } }; let p: types.Q = { a = 1 }; p"#,
            expect: Reject("not known"),
        },
        Case {
            id: "G16 paths in parameter and result positions",
            source: r#"let types = { P = type { a: int } }; let f = fn(p: types.P) -> types.P { p }; f({ a = 1 })"#,
            expect: Value(&[], r#"{"a":1}"#),
        },
        Case {
            id: "G17 a path still resolves after the alias form",
            source: r#"let types = { P = type { a: int } }; let P = types.P; let f = fn(p: P) -> types.P { p }; f({ a = 2 }).a"#,
            expect: Value(&[], "2"),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(rigid_bodies());
    all.extend(inference());
    all.extend(rows());
    all.extend(extraction());
    all.extend(aliases());
    for case in &mut all {
        if case.source.starts_with("INC ") {
            let rest = &case.source[4..];
            case.source = Box::leak(format!("{INC} {rest}").into_boxed_str());
        }
    }
    all
}

fn table(prefix: char) -> Vec<Case> {
    all_cases()
        .into_iter()
        .filter(|c| c.id.starts_with(prefix))
        .collect()
}

#[test]
fn rigid_generic_bodies() {
    run_table(&table('R'));
}
#[test]
fn signature_based_inference_and_generic_values() {
    run_table(&table('I'));
}
#[test]
fn captured_rows_through_values() {
    run_table(&table('W'));
}
#[test]
fn row_extraction_by_typed_copy() {
    run_table(&table('X'));
}
#[test]
fn generic_type_aliases() {
    run_table(&table('G'));
}

fn outcome(source: &str, path: &[&str]) -> String {
    match Program::prepare(source) {
        Err(e) => format!("reject: {}", e.message),
        Ok(p) => match p.evaluate(path) {
            Ok(v) => format!("value: {v}"),
            Err(e) => format!("eval error: {}", e.message),
        },
    }
}

#[test]
fn dotted_type_paths_format_canonically() {
    let formatted = Syntax::parse(
        "let types = { P = type { a: int } };
let p: types . P = { a = 1 };
p",
    )
    .unwrap()
    .format();
    assert!(formatted.contains("let p: types.P = "), "{formatted}");
}

#[test]
fn formatter_round_trips_every_generics_program() {
    let mut failures = Vec::new();
    for case in all_cases() {
        let path: &[&str] = match &case.expect {
            Value(path, _) | EvalError(path, _) => path,
            Reject(_) => &[],
        };
        let Ok(syntax) = Syntax::parse(case.source) else {
            continue;
        };
        let once = syntax.format();
        let reparsed = match Syntax::parse(&once) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!(
                    "{}: formatted text does not parse: {}\n{once}",
                    case.id,
                    e.render(&once)
                ));
                continue;
            }
        };
        if reparsed.format() != once {
            failures.push(format!("{}: not idempotent\n{once}", case.id));
        }
        let (a, b) = (outcome(case.source, path), outcome(&once, path));
        if a != b {
            failures.push(format!(
                "{}: meaning changed\n  original: {a}\n  formatted: {b}\n{once}",
                case.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} formatter failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
