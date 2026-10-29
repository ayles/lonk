use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn methods_infer_later_data_fields_and_capture_their_original_environment() {
    assert_eq!(
        value(
            r#"
        let seed=1;
        let o={early=fn(self) self.x+seed;data={seed+=1;seed};x=10;late=fn(self) self.x+seed};
        {early=o.early();late=o.late();seed=seed}
    "#
        ),
        r#"{"early":11,"late":12,"seed":2}"#
    );
    assert_eq!(
        value("let data={x=42};let o={read=fn(self) self.x;...data};o.read()"),
        "42"
    );
    assert_eq!(
        value(
            "let o:{x:1;read:fn(self:{x:1;...})->int}={x=1;read=fn(self:{x:1;...})->int { self.x }};o.read()"
        ),
        "1"
    );
}

#[test]
fn receiver_signatures_support_callbacks_generics_and_unbound_calls() {
    assert_eq!(
        value(
            r#"
        let o={x=1;choose=fn<T>(self,a:T,b:T)->T { if self.x==1 {a} else {b} }};
        let extracted=o.choose;
        {direct=o.choose<str>("a","b");ordinary=extracted<int>(o,7,8);grouped=(o.choose)<int>(o,3,4)}
    "#
        ),
        r#"{"direct":"a","grouped":3,"ordinary":7}"#
    );
    assert_eq!(
        value(
            r#"
        let o={x=2;map=fn<T>(self,v:T,f:fn(T)->int)->int { f(v)+self.x }};
        o.map(7,fn(n) n*2)
    "#
        ),
        "16"
    );
    assert_eq!(
        value(
            r#"
        let m:fn(self:{x:int;...},int=...)->int=fn(self:{x:int;...},n:int=1)->int { self.x+n };
        let o={x=5;m=m};{member=o.m();unbound=(o.m)(o,3)}
    "#
        ),
        r#"{"member":6,"unbound":8}"#
    );
    assert_eq!(
        value(
            r#"
        let apply=fn(v:{x:int},m:fn(self:{x:int})->int)->int { m(v) };
        apply({x=42},fn(self:{x:int})->int { self.x })
    "#
        ),
        "42"
    );
}

#[test]
fn writable_receivers_preserve_extensions_locations_and_signature_replacements() {
    assert_eq!(
        value(
            r#"
        let create=fn(n:int) ({x=n;inc=fn(&self,step:int=1){self.x+=step;}});
        let xs=[create(1),create(10)];let index=1;let before=xs;
        xs[index].inc(2);let extended={...xs[1];extra="kept"};
        let old=extended.inc;extended.inc=fn(&self,step:int=1){self.x+=step*10;};
        old(&extended,3);extended.inc();
        {before=before[1].x;array=xs[1].x;extended=extended.x;extra=extended.extra}
    "#
        ),
        r#"{"array":12,"before":10,"extended":25,"extra":"kept"}"#
    );
    assert_eq!(
        value(
            r#"
        let o={x=1;inc=fn(&self){self.x+=1;}};
        {let a=&o;a.inc();};o.x
    "#
        ),
        "2"
    );
    for source in [
        "let o={x=1;inc=fn(&self){self.x+=1;}};let f=fn(){o.inc();};f()",
        "let create=fn() ({x=1;inc=fn(&self){self.x+=1;}});create().inc();1",
        "let o={x=1;inc=fn(&self){self.x+=1;}};let p={...o;x=\"bad\"};p.inc();p.x",
        "let o={x=1;inc=fn(&self){self.x+=1;}};o.inc=fn(&self){self.x=\"bad\";};o.x",
        "let o={x=1;f=fn(self:{x:int}) self.x};let f:fn({x:int})->int=o.f;f({x=1})",
        "let o={x=1;f=fn(n:int) n};o.f=fn(self:int) self;o.f(1)",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
}

#[test]
fn spread_overrides_and_method_boundaries_keep_known_types_honest() {
    assert_eq!(
        value(
            r#"
        let override={m=42};let o={m=fn(self) self.x;...override;x=1};o.m
    "#
        ),
        "42"
    );
    for source in [
        "let extra:{m?:int}={m=7};let o={m=fn(self) self.x;...extra;x=1};o.m()",
        "let extra:{...}={m=7};let o={m=fn(self) self.x;...extra;x=1};o.m()",
        "let o={x=1;read=fn(self) self.x;another=fn(self) self.read()};o.another()",
        "let f=fn(n:int,self:int) n+self;f(1,2)",
        "let f=fn(self:int=1) self;f()",
        "let f=fn(&x) 1;f(1)",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
    assert_eq!(
        value("let o={x=42;read=fn(self) self.x};let o=o;({read=o.read;x=7}).read()"),
        "7"
    );
}

#[test]
fn formatting_preserves_member_calls_and_parenthesized_extraction() {
    let source = "let o={x=1;inc=fn(&self,step:int=1){self.x+=step;}};o.inc();(o.inc)(&o,2);o.x";
    let syntax = Syntax::parse(source).unwrap();
    let canonical = syntax.format();
    assert!(canonical.contains("(o.inc)(&o, 2)"));
    assert_eq!(canonical, Syntax::parse(&canonical).unwrap().format());
    assert_eq!(value(source), value(&canonical));
    assert_eq!(value(source), "4");
    let nested = format!(
        "let T=type {}int{};1",
        "fn(self: ".repeat(1000),
        ") -> int".repeat(1000)
    );
    assert!(Syntax::parse(&nested).is_err());
}

#[test]
fn independent_generic_handles_share_a_signature_but_keep_their_own_binders() {
    assert_eq!(
        value(
            r#"
        let xs=[{x=1;inc=fn(&self){self.x+=1;}},{x=10;inc=fn(&self){self.x+=2;}}];
        xs[0].inc();xs[1].inc();[xs[0].x,xs[1].x]
    "#
        ),
        "[2,12]"
    );
    assert_eq!(
        value(
            r#"
        let fs=[fn<T>(v:T,a:any)->bool { match a {T=>true;else=>false;} },
                fn<U>(v:U,a:any)->bool { match a {U=>true;else=>false;} }];
        [fs[0](1,1),fs[1](1,1),fs[1]("s",1),fs[1]({x=1},{x=2})]
    "#
        ),
        "[true,true,false,true]"
    );
    assert_eq!(
        value(
            r#"
        let a=fn<T>(x:T)->T { x };let b=fn<U>(x:U)->U { x };
        let a=if false {a} else {b};a(42)
    "#
        ),
        "42"
    );
    assert!(
        Program::prepare(
            r#"
        let make=fn<T>(x:T) fn<U>(y:U) ({x=x;y=y});
        let fs=[make(1),make("s")];fs[0](7)
    "#
        )
        .is_err()
    );
}

#[test]
fn generic_specialization_through_fields_and_arrays_remains_lazy() {
    assert_eq!(
        value(
            r#"
        let o={x=1;inc=fn(&self){self.x+=1;}};
        let q:{x:int;inc:fn(self:&{x:int})}=o;
        let p:{x:int}=q;let inc=q.inc;inc(&p);p.x
    "#
        ),
        "2"
    );
    assert_eq!(
        value(
            r#"
        let o={bad=1/0;f=fn<T>(v:T)->bool { match v {int=>true;else=>false;} }};
        let p:{f:fn(str)->bool}=o;{specialized=p.f("s");original=o.f(1)}
    "#
        ),
        r#"{"original":true,"specialized":false}"#
    );
    assert_eq!(
        value(
            r#"
        let specialize=fn<T>(sample:T)->[fn(T)->T] {
            let fs=[fn<U>(v:U)->U { v }];let out:[fn(T)->T]=fs;out
        };
        specialize(1)[0](42)
    "#
        ),
        "42"
    );
    assert_eq!(
        value(
            r#"
        let o={x=3;read=fn(self:{x:int;...})->int { self.x };
            twice=fn<R>(self:&{x:int;read:fn(self:{x:int;...})->int;...R}) {
                let old=self.read();self.x=old*2;
            }};
        o.twice();o.x
    "#
        ),
        "6"
    );
}
