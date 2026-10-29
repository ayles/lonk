use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn quantified_callbacks_accept_independent_definitions_and_contextual_lambdas() {
    assert_eq!(
        value(
            r#"
        let apply=fn(f:fn<T>(T)->T) ({a=f(42);b=f("s")});
        let id=fn<U>(x:U)->U { x };
        {explicit=apply(id);contextual=apply(fn(x) x)}
    "#
        ),
        r#"{"contextual":{"a":42,"b":"s"},"explicit":{"a":42,"b":"s"}}"#
    );
    assert_eq!(
        value(
            r#"
        let box:{check:fn<T>(T,any)->bool}={check=fn(x,other) match other {int=>true;else=>false;}};
        [box.check(1,2),box.check("s","s")]
    "#
        ),
        "[true,false]"
    );
    assert_eq!(
        value(
            r#"
        let check:fn<T>(T,any)->bool=fn<U>(sample:U,other:any)->bool { match other {U=>true;else=>false;} };
        [check(1,2),check("s",2)]
    "#
        ),
        "[true,false]"
    );
}

#[test]
fn quantified_receiver_fields_allow_borrowed_sibling_calls() {
    assert_eq!(
        value(
            r#"
        let base={x=1;inc=fn(&self,n:int=1){self.x+=n;}};
        let view:{x:int;inc:fn<R>(self:&{x:int;...R},int=...)}=base;
        let o={...view;extra="kept";
            twice=fn<S>(self:&{x:int;inc:fn<R>(self:&{x:int;...R},int=...);...S}) {
                self.inc();self.inc(2);
            }};
        o.twice();{x=o.x;extra=o.extra;base=base.x}
    "#
        ),
        r#"{"base":1,"extra":"kept","x":4}"#
    );
    assert_eq!(
        value(
            r#"
        let inc:fn<R>(self:&{x:int;...R})=fn(self){self.x+=1;};
        let o={x=1;inc=inc};o.inc();o.x
    "#
        ),
        "2"
    );
    assert_eq!(
        value(
            r#"
        let f:fn<R>({x:int;...R})->{...R}=fn<Q>(v:{x:int;...Q})->{...Q} {let rest:{...Q}=v;rest};
        f({x=1;y="kept"})
    "#
        ),
        r#"{"y":"kept"}"#
    );
}

#[test]
fn nested_quantifiers_preserve_free_variables_and_keep_their_scope() {
    assert_eq!(
        value(
            r#"
        let maker:fn<T>(T)->fn<U>(U)->{first:T;second:U}=fn(first) fn(second) ({first=first;second=second});
        {a=maker(1)("s");b=maker("s")(2)}
    "#
        ),
        r#"{"a":{"first":1,"second":"s"},"b":{"first":"s","second":2}}"#
    );
    assert_eq!(
        value(
            r#"
        let adapter=fn<T>(v:T,f:fn<U>(U)->U)->T { f(v) };
        adapter(42,fn(x) x)
    "#
        ),
        "42"
    );
    for source in [
        "let f:fn<T>(T)->T=fn(x:int)->int { x };f(1)",
        "let f:fn<T>(T)->T=fn(x) 1;f(1)",
        "let f:fn<T>(T)->T=fn(x)x;let v:T=1;f(v)",
        "let f:fn<T>(int)->int=fn(x)x;f(1)",
        "let f:fn<R>({x:int;...R})->{...R}=fn<S>(v:{x:int;y:int;...S})->{...S}{let r:{...S}=v;r};f({x=1;y=2})",
        "let f:fn<T>(T)->T=fn(x)x;let a:any=f;match a {fn<U>(U)->U=>1;else=>0;}",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
}

#[test]
fn quantified_callable_signatures_round_trip_with_comments() {
    let source = "let f:fn<T>(/* value */T)->T=fn(x)x;f<int>(42)";
    let syntax = Syntax::parse(source).unwrap();
    let canonical = syntax.format();
    let parsed = Syntax::parse(&canonical).unwrap();
    assert_eq!(canonical, parsed.format());
    assert_eq!(syntax.comments().len(), parsed.comments().len());
    assert_eq!(value(&canonical), "42");
}
