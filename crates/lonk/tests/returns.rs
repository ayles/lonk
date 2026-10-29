use lonk::{Program, Syntax};

fn run(source: &str, wanted: &str) {
    let program =
        Program::prepare(source).unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)));
    assert_eq!(
        program
            .evaluate(&[])
            .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
            .to_string(),
        wanted
    );
    let formatted = Syntax::parse(source).unwrap().format();
    assert_eq!(Syntax::parse(&formatted).unwrap().format(), formatted);
    assert_eq!(
        Program::prepare(&formatted)
            .unwrap()
            .evaluate(&[])
            .unwrap()
            .to_string(),
        wanted
    );
}

#[test]
fn branches_and_inference() {
    run(
        "let f = fn(x: int) { if x < 0 { return 1; }; return x + 2; }; [f(-3), f(5)]",
        "[1,7]",
    );
    run(
        "let f = fn(x: bool) -> int { if x { return 2; } else { return 3; } }; [f(true), f(false)]",
        "[2,3]",
    );
    run(
        "let f = fn(x: any) -> int { match x { int => { return x; }; else => { return 4; }; } }; [f(3), f(false)]",
        "[3,4]",
    );
    run(
        "let f = fn(x: any) -> int { match x { int => { }; else => { return 4; }; }; x + 1 }; [f(3), f(false)]",
        "[4,4]",
    );
    run(
        "let f = fn(x: bool) -> int { if x { return 42; }; 1 / 0 }; f(true)",
        "42",
    );
    run(
        "let f = fn(x: bool) -> int { if x { if 1 / 0 == 0 { return 2; }; }; return 3; }; f(false)",
        "3",
    );
}

#[test]
fn borrowed_exits_and_aliases() {
    run(
        "let f = fn(x: &int) { x = 2; return; }; let x = 1; f(&x); x",
        "2",
    );
    run(
        "let f = fn(x: &int, yes: bool) -> int { let a = &x; a = 2; if yes { return a; }; a = 3; a }; let x = 1; let y = f(&x, true); [x, y]",
        "[2,2]",
    );
    run(
        "let f = fn(x: &int) -> int { x = 2; let x = 3; return x; }; let x = 1; let y = f(&x); [x, y]",
        "[2,3]",
    );
}

#[test]
fn loop_returns_and_recursion() {
    run(
        "let f = fn(xs: [int]) -> int { for x in xs { if x > 2 { return x; }; }; return 0; }; [f([1, 2, 3, 1 / 0]), f([])]",
        "[3,0]",
    );
    run(
        "let f = fn(xs: [int]) -> int { let total = 0; for x in xs { total += x; if total > 3 { return total; }; }; total }; [f([1, 2, 3, 1 / 0]), f([1]) ]",
        "[6,1]",
    );
    run(
        "let f = fn(out: &int) -> int { for x in [1, 2, 3] { out += x; if out > 2 { return out; }; }; out }; let out = 0; let v = f(&out); [out, v]",
        "[3,3]",
    );
    run(
        "let f = fn(n: int) -> int { if n == 0 { return 0; }; return f(n - 1); }; f(2000)",
        "0",
    );
    run(
        "let f = fn() -> int { for x in [1, 2] { for y in [1, 2] { if x + y == 3 { return x * 10 + y; }; }; }; 0 }; f()",
        "12",
    );
}

#[test]
fn invalid_returns() {
    for source in [
        "return 1;",
        "let f = fn() { return 1; let x = 2; x }; f()",
        "let f = fn() -> int { return; }; f()",
        "let f = fn(x: bool) { if x { return 1; }; }; f(true)",
        "let f = fn() -> int { return false; }; f()",
    ] {
        assert!(Program::prepare(source).is_err(), "{source}");
    }
}
