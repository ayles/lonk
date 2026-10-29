use lonk::{Program, Type, Value};

fn run(source: &str) -> Value {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap()
}

fn reject(source: &str, message: &str) {
    let error = Program::prepare(source).expect_err(source);
    assert!(
        error.message.contains(message),
        "{}\n{source}",
        error.render(source)
    );
}

#[test]
fn literal_patterns_check_kind_and_value_in_order() {
    let source = r#"
        let describe = fn(v: any) match v {
            1 => "one";
            "1" => "string one";
            int => "another integer";
            str => "another string";
            true => "true";
            else => "other";
        };
        [describe(1), describe("1"), describe(2), describe("two"), describe(true), describe(false)]
    "#;
    assert_eq!(
        run(source).to_string(),
        r#"["one","string one","another integer","another string","true","other"]"#
    );
    assert_eq!(
        run("match -9223372036854775808 { -9223372036854775808 => 1; else => 0; }"),
        Value::Int(1)
    );
    reject(
        "match 1 { --9223372036854775808 => 1; else => 0; }",
        "outside",
    );
}

#[test]
fn presence_patterns_preserve_known_payload_types_and_laziness() {
    assert_eq!(
        run("let p: {x?: int} = {x = 1 / 0}; match p { {x: any; ...} => 42; else => 0; }"),
        Value::Int(42)
    );
    assert_eq!(
        run("let p: {x?: int} = {x = 5}; match p { {x: any; ...} => p.x + 1; else => 0; }"),
        Value::Int(6)
    );
    assert_eq!(
        run("let p: {...} = {x = 1 / 0}; match p { {x: any; ...} => 42; else => 0; }"),
        Value::Int(42)
    );
    assert_eq!(
        run("let v = 1 / 0; match v { int => 42; else => 0; }"),
        Value::Int(42)
    );
    reject(
        "let p: {...} = {x = 1}; match p { {x: any; ...} => p.x + 1; else => 0; }",
        "does not accept",
    );
    let program =
        Program::prepare("let p: {...} = {x = 1 / 0}; match p { {x: int; ...} => 42; else => 0; }")
            .unwrap();
    assert!(
        program
            .evaluate(&[])
            .unwrap_err()
            .message
            .contains("division")
    );
}

#[test]
fn nested_patterns_check_only_needed_paths_and_do_not_project() {
    assert_eq!(
        run(r#"
        let p: any = {inner = {x = 7; other = 1 / 0}; extra = "kept"};
        match p {
            {inner: {x: int; ...}; ...} => {
                let q = {...p};
                match q { {extra: str; ...} => q.extra; else => "missing"; }
            };
            else => "missing";
        }
    "#),
        Value::Str("kept".into())
    );
    assert_eq!(
        run("let p: any = {x = 1; y = 2}; match p { {x: int} => 0; else => 1; }"),
        Value::Int(1)
    );
    assert_eq!(
        run("match {x = 1 / 0} { {x: int; ...} => 42; else => 0; }"),
        Value::Int(42)
    );
}

#[test]
fn arms_are_checked_eagerly_and_proofs_stay_in_their_arm() {
    reject(
        r#"match 1 { 1 => 42; str => 1 + "bad"; else => 0; }"#,
        "does not accept",
    );
    reject(
        "let p: any = 1; let x = match p { int => p + 1; else => 0; }; p + x",
        "does not accept",
    );
    reject(
        "let p: any = 1; match p { int => p; else => p + 1; }",
        "does not accept",
    );
    assert_eq!(run("match 1 { int => 2; }"), Value::Int(2));
    reject("match 1 { 1 => 2; }", "final `else`");
    reject(
        "match 1 { else => 0; int => 1; else => 2; }",
        "last match arm",
    );
    reject(
        "let p: any = 1; if p is int {p} else {0}",
        "type tests belong in match",
    );
    reject(
        r#"let p: {x?: int} = {x = 1}; if "x" in p {p.x} else {0}"#,
        "optional",
    );
}

#[test]
fn match_writes_join_and_old_closures_keep_their_snapshot() {
    let source = r#"
        let p: {...} = {x = 1};
        let result = match p {
            {x: int; ...} => {
                let old = fn() p.x;
                p.x += 1;
                {old = old(); current = p.x}
            };
            else => {p.x = 0; ({old = 0; current = 0})};
        };
        {result = result; later = p.x}
    "#;
    assert_eq!(
        run(source).to_string(),
        r#"{"later":2,"result":{"current":2,"old":1}}"#
    );
    let program = Program::prepare(
        "let v = 1; let p: {...} = {}; match v { 1 => {p.x = 2;}; else => {p.x = 3;}; }; p.x",
    )
    .unwrap();
    assert_eq!(program.result_type(), &Type::Int);
    assert_eq!(program.evaluate(&[]).unwrap(), Value::Int(2));
}
