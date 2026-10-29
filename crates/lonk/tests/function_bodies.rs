use lonk::{Program, Syntax};

#[test]
fn inferred_results_keep_short_functions_at_every_arity() {
    for (source, expected) in [
        ("let f = fn() 42; f()", "42"),
        ("let f = fn(x: int) x * 2; f(21)", "42"),
        ("let f = fn(x: int, y: int) x + y; f(20, 22)", "42"),
        ("[1, 2, 3].map(fn(x) x * 2)", "[2,4,6]"),
        ("[1, 2, 3].fold(0, fn(sum, x) sum + x)", "6"),
        ("let f = fn<T>(x: T) x; f(42)", "42"),
        ("let f = fn() ({answer = 42}); f().answer", "42"),
    ] {
        roundtrip(source, expected);
    }
}

#[test]
fn explicit_results_use_blocks_for_values_records_and_functions() {
    for (source, expected) in [
        ("let f = fn() -> int { 42 }; f()", "42"),
        ("let f = fn(x: int) -> int { return x * 2; }; f(21)", "42"),
        ("let f = fn() -> [int] { [1, 2] }; f()", "[1,2]"),
        ("let f = fn() -> {x: int} { {x = 42} }; f().x", "42"),
        (
            "let f = fn(n: int) -> fn(int) -> int { fn(x) x + n }; f(20)(22)",
            "42",
        ),
        ("let f = fn<T>(x: T) -> T { x }; f(42)", "42"),
        (
            "let f = fn<R>(v: {x: any; ...R}) -> {...R} { v }; f({x = 1 / 0; y = 42}).y",
            "42",
        ),
        (
            "let v = {x = 42; get = fn(self) -> int { self.x }}; v.get()",
            "42",
        ),
        (
            "let f = fn(x: &int) -> int { x += 1; x }; let n = 41; let result = f(&n); result + n",
            "84",
        ),
        (
            "let F = type fn(int) -> int; let f: F = fn(x) x * 2; f(21)",
            "42",
        ),
        (
            "let f = fn(g: fn() -> int = fn() -> int { 42 }) -> int { g() }; f()",
            "42",
        ),
    ] {
        roundtrip(source, expected);
    }
}

#[test]
fn explicit_results_reject_every_unbraced_body_with_a_useful_diagnostic() {
    for source in [
        "fn() -> int 42",
        "fn() -> int (42)",
        "fn() -> [int] [42]",
        "fn() -> {x: int} ({x = 42})",
        "fn() -> int ({ let x = 42; x })",
        "fn() -> int if true { 42 } else { 0 }",
        "fn() -> int match 1 { int => 42; else => 0; }",
        "fn() -> fn(int) -> int fn(x) x",
        "fn<T>(x: T) -> T x",
        "{get = fn(self) -> int self.x}",
        "fn() -> int /* before body */ -1",
    ] {
        let error = Syntax::parse(source).expect_err(source);
        assert!(
            error.message.contains("requires a block body"),
            "{source}: {error}"
        );
        assert!(!source[error.span.start..error.span.end].is_empty());
    }
    for source in ["fn(x) => x", "|x| x", "|| 42"] {
        assert!(Syntax::parse(source).is_err(), "{source}");
    }
}

#[test]
fn comments_survive_typed_body_and_record_boundaries() {
    roundtrip(
        "// leading\nlet f = fn(/* parameter */ x: int) /* arrow */ -> /* type */ {value: int} /* body */ { /* record */ ({value = x /* field */}) /* end */ }; f(42).value // trailing\n",
        "42",
    );
    roundtrip(
        "let f = fn() -> int { // first\n /* expression */ 42 // last\n }; f()",
        "42",
    );
}

fn roundtrip(source: &str, expected: &str) {
    let syntax = Syntax::parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    let canonical = syntax.format();
    let reparsed = Syntax::parse(&canonical).unwrap_or_else(|e| panic!("{canonical}: {e}"));
    assert_eq!(canonical, reparsed.format());
    assert_eq!(
        syntax
            .comments()
            .iter()
            .map(|c| &c.text)
            .collect::<Vec<_>>(),
        reparsed
            .comments()
            .iter()
            .map(|c| &c.text)
            .collect::<Vec<_>>(),
    );
    for text in [source, canonical.as_str()] {
        let program = Program::prepare(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(
            program.evaluate(&[]).unwrap().to_string(),
            expected,
            "{text}"
        );
    }
}
