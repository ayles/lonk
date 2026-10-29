use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn aliases_write_through_with_lexical_lifetimes_and_snapshot_reads() {
    assert_eq!(
        value(
            "let p={x=1;y=2};let before=p;{let a=&p.x;let old=a;a+=10;p.y+=old;};{before=before;after=p}"
        ),
        r#"{"after":{"x":11,"y":3},"before":{"x":1,"y":2}}"#
    );
    assert_eq!(
        value("let p={x=1;y=2};{let a=&p;{let b=&a.x;b+=2;a.y+=3;};a.x+=1;};p"),
        r#"{"x":4,"y":5}"#
    );
    assert_eq!(
        value("let p={x=1;y=2};{let a=&p.x;let b=&p.y;a+=10;b+=20;};p"),
        r#"{"x":11,"y":22}"#
    );
    assert_eq!(
        value("let x=1;let capture={let a=&x;a=2;fn() a};x=3;{old=capture();new=x}"),
        r#"{"new":3,"old":2}"#
    );
    assert_eq!(value("let x=1;{let a:&int=&x;a+=1;};x"), "2");
}

#[test]
fn alias_reborrows_loops_and_indices_use_the_original_place() {
    assert_eq!(
        value("let inc=fn(n:&int){n+=1;};let p={n=1};{let a=&p.n;inc(&a);a+=1;};p.n"),
        "3"
    );
    assert_eq!(
        value("let xs=[10,20];let index=0;{let a=&xs[index];index=1;a+=5;};{xs=xs;index=index}"),
        r#"{"index":1,"xs":[15,20]}"#
    );
    assert_eq!(
        value("let sum=0;{let a=&sum;for x in [1,2,3] {a+=x;}} sum"),
        "6"
    );
    assert_eq!(
        value("let sum=0;for x in [1,2,3] {{let a=&sum;a+=x;};} sum"),
        "6"
    );
    assert_eq!(
        value("let set=fn(x:&int){x=42;};let xs=[1/0];{let a=&xs[0];set(&a);};xs[0]"),
        "42"
    );
    assert_eq!(
        value(
            "let x:any=1;{let a=&x;match a {int=>{a+=1;};else=>{a=a;};};};match x {int=>x;else=>0}"
        ),
        "2"
    );
}

#[test]
fn aliases_enforce_exclusivity_contracts_and_non_escape() {
    for (source, needle) in [
        ("let p={x=1};{let a=&p.x;a+=p.x;};p", "overlaps"),
        (
            "let p={x=1};{let a=&p;let b=&a.x;a.x+=1;b+=1;};p",
            "overlaps",
        ),
        ("let x=1;{let a=&x;let b=&x;a+=1;b+=1;};x", "overlaps"),
        ("let x=1;{let a=&x;a=\"s\";};x", "expected int"),
        ("let x=1;{let a=&x;let x=2;a+=x;};x", "shadow"),
        ("let x=1;{let a=&x;let a=2;a};x", "shadow"),
        ("let x=1;{let a=&x;let f=fn(){a+=1;};f();};x", "captured"),
        ("let x=1;{let a=&x;&a}", "not a storable"),
        ("let x=1;{let a=&x;42}", "unused"),
        ("let p={x=1;y=2};{let a:&{x:int}=&p;a.x+=1;};p", "preserve"),
    ] {
        let error = Program::prepare(source)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.message.contains(needle), "{source}: {error}");
    }
}

#[test]
fn alias_existence_is_checked_without_demanding_old_payloads() {
    assert_eq!(
        value("let xs=[1/0];let result={let a=&xs[0];a=42;a};result"),
        "42"
    );
    for source in [
        "let xs=[1];let result={let a=&xs[99];a=42;a};result",
        "let set=fn(x:&int)->int {x=42;7};let xs=[1];let result={let a=&xs[99];set(&a)};result",
    ] {
        let error = Program::prepare(source).unwrap().evaluate(&[]).unwrap_err();
        assert!(error.message.contains("out of bounds"), "{source}: {error}");
    }
    let source = "let x=1;{let a=/* location */&x;a+=1;};x";
    let canonical = Syntax::parse(source).unwrap().format();
    assert_eq!(canonical, Syntax::parse(&canonical).unwrap().format());
    assert_eq!(value(source), value(&canonical));
}

#[test]
fn block_and_record_nesting_are_bounded_before_native_stack_exhaustion() {
    for (open, close) in [
        ("(", ")"),
        ("{", "}"),
        ("[", "]"),
        ("{a=", "}"),
        ("fn() ", ""),
        ("if true {", "}"),
    ] {
        let source = format!("{}1{}", open.repeat(1000), close.repeat(1000));
        let error = Syntax::parse(&source).unwrap_err();
        assert!(error.message.contains("nesting"), "{open}: {error}");
    }
}

#[test]
fn aliases_and_calls_cannot_write_opaque_generic_fields() {
    for source in [
        "let inc=fn(x:&int){x+=1;};let f=fn<R>(v:&{x:int;...R}) {match v {{y:int;...}=>{inc(&v.y);};else=>{v.x+=1;};};};let p={x=1;y=2};f(&p);p",
        "let f=fn<R>(v:&{x:int;...R}) {match v {{y:int;...}=>{let a=&v.y;a+=1;};else=>{v.x+=1;};};};let p={x=1;y=2};f(&p);p",
        "let f=fn<T>(v:&T) {match v {int=>{let a=&v;a=42;};else=>{v=v;};};};let x=1;f(&x);x",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
}
