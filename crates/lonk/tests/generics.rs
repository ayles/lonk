use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn nested_projection_keeps_unknown_rows_and_does_not_force_removed_fields() {
    assert_eq!(
        value(
            r#"
        let rest = fn<R>(v: { nested: { x: int; ...R } }) -> { nested: { ...R } } { v };
        let p: { nested: { x: int; ... } } = { nested = { x = 1 / 0; hidden = "kept" } };
        rest(p)
    "#
        ),
        r#"{"nested":{"hidden":"kept"}}"#
    );
    assert_eq!(
        value(
            r#"
        let rest = fn<R>(v: [{ x: int; ...R }]) -> [{ ...R }] { v };
        let p: [{ x: int; ... }] = [{ x = 1 / 0; hidden = "kept" }];
        rest(p)
    "#
        ),
        r#"[{"hidden":"kept"}]"#
    );
}

#[test]
fn generic_calls_keep_independent_type_environments_in_nested_closures() {
    assert_eq!(
        value(
            r#"
        let make = fn<T>(x: T) fn<U>(y: U) ({ x = x; y = y });
        let a = make(1);
        let b = make("s");
        { a = a(true); b = b(2); c = a("t") }
    "#
        ),
        r#"{"a":{"x":1,"y":true},"b":{"x":"s","y":2},"c":{"x":1,"y":"t"}}"#
    );
    assert_eq!(
        value(
            r#"
        let id = fn<T>(v: T) -> T { v };
        let outer = fn<T>(v: T) -> T { id<T>(v) };
        { a = outer(7); b = outer("s") }
    "#
        ),
        r#"{"a":7,"b":"s"}"#
    );
}

#[test]
fn generic_type_tests_use_static_evidence_and_dynamic_substitution() {
    assert_eq!(
        value("let f = fn<T>(v: T) match v { T => 42; else => 0; }; f(1 / 0)"),
        "42"
    );
    assert_eq!(
        value(
            r#"
        let checked = fn<T>(v: any) -> { value?: T } { match v {
            T => ({ value = v }); else => ({});
        } };
        { yes = checked<int>(7); no = checked<int>("s"); text = checked<str>("s") }
    "#
        ),
        r#"{"no":{},"text":{"value":"s"},"yes":{"value":7}}"#
    );
    assert_eq!(
        value("let id = fn<T>(v: T) -> T { match v { int => v; else => v; } }; id(7)"),
        "7"
    );
    assert_eq!(
        value(
            "let get = fn<T>(v: {x:T}) -> T { match v { {x:int} => v.x; else => v.x; } }; get({x=7})"
        ),
        "7"
    );
}

#[test]
fn refinement_cannot_authorize_overwriting_an_arbitrary_generic_type() {
    for source in [
        "let f = fn<T>(v: T) -> T { match v { int => { v = 2; }; else => {}; }; v }; f(1)",
        "let f = fn<T>(v: {x:T}) -> {x:T} { match v { {x:int} => { v.x = 2; }; else => {}; }; v }; f({x=1})",
        "let f = fn<R>(v: {x:int;...R}) -> {x:int;...R} { match v { {label:str;...} => { v.label = \"changed\"; }; else => {}; }; v }; f({x=1;label=\"original\"})",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inference_contexts_cover_callbacks_empty_arrays_and_generic_records() {
    assert_eq!(
        value("let apply = fn<T,U>(x: T, f: fn(T) -> U) -> U { f(x) }; apply(5, fn(n) n + 1)"),
        "6"
    );
    assert_eq!(
        value("let apply = fn<T,U>(f: fn(T) -> U, x: T) -> U { f(x) }; apply(fn(n) n + 1, 5)"),
        "6"
    );
    assert_eq!(
        value("let f = fn<T>(v: [T]) -> [T] { v }; let xs: [int] = f([]); xs"),
        "[]"
    );
    assert_eq!(
        value(
            "let id = fn<T>(v: T) -> T { v }; let p = {x = 1; hidden = 1 / 0}; let q: {x: int} = id(p); q"
        ),
        r#"{"x":1}"#
    );
    assert_eq!(
        value("let id = fn<T>(v: T) -> T { v }; let q: {x: int} = id({x = 1; hidden = 1 / 0}); q"),
        r#"{"x":1}"#
    );
}

#[test]
fn row_projection_does_not_drop_fields_discovered_by_matching() {
    assert_eq!(
        value(
            r#"
        let rest = fn<R>(v: { x: int; ...R }) -> { ...R } {
            match v { { hidden: str; ... } => { let q: { ...R } = v; q }; else => { let q: { ...R } = v; q }; }
        };
        let p: { x: int; ... } = { x = 1 / 0; hidden = "kept" };
        rest(p)
    "#
        ),
        r#"{"hidden":"kept"}"#
    );
}

#[test]
fn generic_syntax_preserves_comments_and_comparison_parsing() {
    let source = "// generic\nlet id = fn</* binder */T>(v: T) -> T { v }; { a = id</* argument */int>(4); b = 1 < 2; c = 3 > 2 }";
    let parsed = Syntax::parse(source).unwrap();
    let canonical = parsed.format();
    assert_eq!(value(source), value(&canonical));
    assert_eq!(canonical, Syntax::parse(&canonical).unwrap().format());
    assert_eq!(
        parsed.comments().len(),
        Syntax::parse(&canonical).unwrap().comments().len()
    );
}
