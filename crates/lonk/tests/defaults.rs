use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn defaults_are_lazy_per_call_and_capture_entry_snapshots() {
    assert_eq!(
        value(
            "let f=fn(a:int,b:int=a+1,c:int=b+1)->[int] {a+=10;[a,b,c]};{one=f(1);two=f(10);override=f(1,20)}"
        ),
        r#"{"one":[11,2,3],"override":[11,20,21],"two":[20,11,12]}"#
    );
    assert_eq!(
        value("let seed=7;let f=fn(n:int=seed) n;seed=9;{old=f();new=seed}"),
        r#"{"new":9,"old":7}"#
    );
    assert_eq!(value("let f=fn(bad:int=1/0)->int { 42 };f()"), "42");
    assert_eq!(value("let f=fn(n:int=1/0)->int { n };f(7)"), "7");
    assert_eq!(value("let f=fn(n=42) n;f()"), "42");
    assert_eq!(
        value("let f=fn(n:int={let local=1;local+=1;local}) n;f()"),
        "2"
    );
    assert_eq!(
        value("let f=fn(ignored:int={let local=42;local}) 7;f()"),
        "7"
    );
}

#[test]
fn defaults_are_part_of_callable_contracts_and_support_generics() {
    assert_eq!(
        value("let f=fn(n:int=42)->int { n };let copy:fn(int=...)->int=f;copy()"),
        "42"
    );
    assert_eq!(
        value("let f=fn(n:int=42)->int { n };let required:fn(int)->int=f;required(7)"),
        "7"
    );
    assert_eq!(
        value("let apply=fn(f:fn(int=...)->int)->int { f() };apply(fn(n:int=7) n)"),
        "7"
    );
    assert_eq!(
        value("let pair=fn<T>(a:T,b:T=a)->[T] { [a,b] };{a=pair(7);b=pair(\"x\",\"y\")}"),
        r#"{"a":[7,7],"b":["x","y"]}"#
    );
    assert_eq!(
        value("let f=fn<T>(items:[T]=[])->[T] { items };let a:[int]=f();a"),
        "[]"
    );
    assert_eq!(
        value("let inc=fn(n:&int,amount:int=1){n+=amount;};let n=1;inc(&n);inc(&n,10);n"),
        "12"
    );
    assert_eq!(
        value(
            "let count=fn(n:int,step:int=1)->int {if n==0 {0} else {step+count(n-1)}};count(4,10)"
        ),
        "13"
    );
}

#[test]
fn defaults_cannot_weaken_signatures_or_mutate_parameter_entry_snapshots() {
    for (source, needle) in [
        ("let f=fn(a:int=1,b:int) a+b;f(1,2)", "trailing"),
        ("let f=fn(a:&int=1){a+=1;};f()", "borrowed parameter"),
        ("let f=fn(a:int=\"s\") a;f(1)", "expected int"),
        ("let f=fn(a:int=b,b:int=1) a+b;f()", "unknown name"),
        ("let f=fn(a:int=a) a;f()", "unknown name"),
        ("let f=fn(a:int,b:int={a+=1;a}) a+b;f(1)", "captured"),
        (
            "let f=fn(a:&int,b:int=a){a+=b;};let n=1;f(&n);n",
            "unknown name",
        ),
        ("let f:fn(int=...)->int=fn(n:int) n;f()", "expected"),
        (
            "let f=fn(n:int=42)->int { n };let g:fn(int)->int=f;g()",
            "expected 1",
        ),
        ("let f=fn(a:int,b:int=1) a+b;f()", "expected 1..=2"),
        ("let f=fn(a:int=1) a;f(1,2)", "expected 0..=1"),
        (
            "let edit=fn(f:&fn(int)->int){f=fn(n:int) n;};let f=fn(n:int=42) n;edit(&f);f()",
            "lose",
        ),
    ] {
        let error = Program::prepare(source)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.message.contains(needle), "{source}: {error}");
    }
}

#[test]
fn default_syntax_and_comments_round_trip() {
    let source =
        "let f:fn(int,int=/* available */...)->int=fn(a:int,b:int=/* lazy */a+1)->int { a+b };f(2)";
    let syntax = Syntax::parse(source).unwrap();
    let canonical = syntax.format();
    let reparsed = Syntax::parse(&canonical).unwrap();
    assert_eq!(canonical, reparsed.format());
    assert_eq!(syntax.comments().len(), reparsed.comments().len());
    assert_eq!(value(source), value(&canonical));
}
