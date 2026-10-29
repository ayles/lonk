use lonk::{Program, Syntax, format};

fn canonical(source: &str) -> String {
    let syntax = Syntax::parse(source).unwrap();
    let formatted = syntax.format();
    let reparsed = Syntax::parse(&formatted).unwrap_or_else(|e| panic!("{e}\n{formatted}"));
    assert_eq!(
        reparsed.format(),
        formatted,
        "formatter is not idempotent:\n{source}"
    );
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
        "comment content/order changed:\n{source}"
    );
    formatted
}

#[test]
fn one_layout_independent_of_whitespace_separators_and_redundant_parentheses() {
    let a = "let x=({z=001,a = (2)});\n\n\n {out=x; numbers=[1,2,]}";
    let b = "let x = { z = 1\n a = 2 }; {out = x\n numbers = [1, 2]}";
    let expected = "let x = {\n    z = 1;\n    a = 2;\n};\n{\n    out = x;\n    numbers = [\n        1,\n        2,\n    ];\n}\n";
    assert_eq!(canonical(a), expected);
    assert_eq!(canonical(b), expected);
}

#[test]
fn comments_survive_in_all_syntactic_positions() {
    let cases = [
        "// trailing spaces  \n1",
        "// before\nlet /* name */ x /* annotation */ : /* type */ int /* assign */ = /* value */ 1 /* end */; // after\n x // eof",
        "{ /* first */ a /* eq */ = /* value */ 1; // next\n b = [/* element */ 2, /* end array */]; /* end record */ }",
        "let F=type { /* first */ x?: /* field type */ [int /* element */]; /* before tail */ ... /* tail */ }; F",
        "let f = fn(/* parameter */ x: /* type */ int, /* last */ y) /* arrow */ -> /* result type */ int /* body */ { /* stmt */ let z=x; z /* close */ }; f(1, /* arg */ 2 /* close call */)",
        "if /* condition */ true /* open */ { /* yes */ 1 /* close */ } /* else */ else /* open */ { 2 /* close */ }",
        "(1 /* left */ + /* right */ 2) * /* next */ 3",
        "let x = { a = [1] }; x /* dot */ .a /* bracket */ [0 /* close */]",
        "{/* dangling */}",
        "[// dangling\n]",
        "/* only /* nested */ comment */",
        "// λ\n{\"a-b\" = \"/* not a comment */ // 💡\"; /* 多言語 */ \"if\" = 1}",
        "let x = (/* inside */ 1 /* outside */); x",
        "let x = ({} /* record */); x",
    ];
    for source in cases {
        canonical(source);
    }
}

#[test]
fn printing_preserves_precedence_function_bodies_and_results() {
    let cases = [
        "1 + 2 * 3",
        "(1 + 2) * 3",
        "8 - (3 - 1)",
        "8 / (4 / 2)",
        "-(-2)",
        "-9223372036854775808",
        "!(true && false)",
        "let f = fn(x: int) ({a = x}); f(2).a",
        "let f = fn(x: int) fn(y: int) x + y; f(1)(2)",
        "(fn(x: int) x + 1)(2)",
        "(if true { 1 } else { 2 }) + 3",
        "let x: any = 1; match x { int => x + 1; else => 0; }",
        "let f = fn() ({a = 1}).a; f()",
        "match 1 { int => ({a = 1}).a; else => 0; }",
        "let x = [1]; x[0] += 1; x",
        "let F = type fn(int) -> int; let f: F = fn(x) x; f(2)",
        "{ x = \"line\\n\\t\\\" %%{literal} ${HOME}\" }",
    ];
    for source in cases {
        let formatted = canonical(source);
        let original = Program::prepare(source).unwrap();
        let after = Program::prepare(&formatted).unwrap_or_else(|e| panic!("{e}\n{formatted}"));
        assert_eq!(original.result_type(), after.result_type(), "{source}");
        assert_eq!(
            original.evaluate(&[]).unwrap(),
            after.evaluate(&[]).unwrap(),
            "{source}"
        );
    }
}

#[test]
fn formatting_needs_syntax_only_and_never_demands_values() {
    assert!(format("let unused = unknown + 1; { bad = 1 / 0 }").is_ok());
    assert!(format("let = broken").is_err());
}
