use super::*;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Builtin {
    ArrayLen,
    ObjectLen,
    ObjectKeys,
    ObjectRemove,
    ObjectLookup,
    ObjectWith,
    StringLen,
    StringSplit,
    StringJoin,
    StringContains,
    StringStartsWith,
    StringEndsWith,
    StringReplace,
    StringTrim,
    IntString,
    BoolString,
}

impl Builtin {
    pub(crate) fn arity(self) -> usize {
        use Builtin::*;
        match self {
            ObjectWith | StringReplace => 3,
            ObjectRemove | ObjectLookup | StringSplit | StringJoin | StringContains
            | StringStartsWith | StringEndsWith => 2,
            ArrayLen | ObjectLen | ObjectKeys | StringLen | StringTrim | IntString | BoolString => {
                1
            }
        }
    }
}

impl Checker {
    pub(super) fn standard_library(&mut self) -> Result<(Env, Id), Diagnostic> {
        let origin = self.origin;
        self.origin = 0;
        use Builtin::*;
        let specs = [
            ("array_len", "fn<T>([T]) -> int", ArrayLen),
            ("object_len", "fn({...}) -> int", ObjectLen),
            ("object_keys", "fn({...}) -> [str]", ObjectKeys),
            ("object_remove", "fn({...}, str) -> {...}", ObjectRemove),
            (
                "object_lookup",
                "fn({...}, str) -> {value?: any}",
                ObjectLookup,
            ),
            ("object_with", "fn({...}, str, any) -> {...}", ObjectWith),
            ("string_len", "fn(str) -> int", StringLen),
            ("string_split", "fn(str, str) -> [str]", StringSplit),
            ("string_join", "fn([str], str) -> str", StringJoin),
            ("string_contains", "fn(str, str) -> bool", StringContains),
            (
                "string_starts_with",
                "fn(str, str) -> bool",
                StringStartsWith,
            ),
            ("string_ends_with", "fn(str, str) -> bool", StringEndsWith),
            ("string_replace", "fn(str, str, str) -> str", StringReplace),
            ("string_trim", "fn(str) -> str", StringTrim),
            ("int_string", "fn(int) -> str", IntString),
            ("bool_string", "fn(bool) -> str", BoolString),
        ];
        let mut fields = BTreeMap::new();
        let mut values = Vec::new();
        for (name, signature, builtin) in specs {
            let syntax = syntax::parse(&format!("type {signature}"))?;
            let descriptor = self.expr(
                syntax.body.result.as_ref().ok_or_else(|| {
                    Diagnostic::new(Span::default(), "internal invariant: builtin signature")
                })?,
                &mut Env::new(),
                None,
            )?;
            let Type::Descriptor(signature) = descriptor.ty else {
                unreachable!()
            };
            let (parameters, body) = match signature.as_ref() {
                Type::Generic { parameters, body } => {
                    (parameters.iter().map(|p| p.id).collect(), body.as_ref())
                }
                ty => (Vec::new(), ty),
            };
            let Type::Function {
                parameters: inputs,
                result,
                ..
            } = body
            else {
                unreachable!()
            };
            debug_assert_eq!(inputs.len(), builtin.arity());
            let outside = self.owner;
            let owner = self.next_owner;
            self.next_owner += 1;
            self.owner = owner;
            let arguments = inputs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    self.node(Kind::Parameter(i, None), t.clone(), Span::default())
                        .id
                })
                .collect();
            let output = self
                .node(
                    Kind::Builtin(builtin, arguments),
                    *result.clone(),
                    Span::default(),
                )
                .id;
            self.owner = outside;
            let function = self.node(
                Kind::Function {
                    owner,
                    body: output,
                    defaults: Vec::new(),
                    parameters,
                },
                *signature,
                Span::default(),
            );
            fields.insert(name.to_owned(), Field::required(function.ty));
            values.push((Some(name.to_owned()), function.id));
        }
        let core = self.node(
            Kind::Record(values),
            Type::Record {
                fields: fields.into(),
                open: false,
                row: None,
            },
            Span::default(),
        );
        let mut env = Env::new();
        env.insert(
            "core".into(),
            Binding {
                identity: core.id,
                value: core.id,
                contract: core.ty.clone(),
                known: core.ty,
                owner: 0,
            },
        );
        let library = syntax::parse(include_str!("../../std/collections.lonk"))?;
        let library = self
            .block(&library.body, &mut env, None)
            .map_err(|e| e.with_origin(0))?;
        let Type::Record { fields, .. } = &library.ty else {
            unreachable!()
        };
        let mut env = Env::new();
        for (name, field) in fields.iter() {
            let value = self.node(
                Kind::Field(library.id, name.clone()),
                field.ty.clone(),
                Span::default(),
            );
            env.insert(
                name.clone(),
                Binding {
                    identity: value.id,
                    value: value.id,
                    known: value.ty.clone(),
                    contract: value.ty,
                    owner: 0,
                },
            );
        }
        self.primitive_libraries = env.clone();
        self.origin = origin;
        Ok((env, library.id))
    }

    pub(super) fn primitive_member(
        &mut self,
        ty: &Type,
        name: &str,
        span: Span,
    ) -> Result<Option<Checked>, Diagnostic> {
        let namespace = match (ty, ty.primitive()) {
            (Type::Array(_), _) => "array",
            (_, Some(Type::Str)) => "string",
            (_, Some(Type::Int)) => "integer",
            (_, Some(Type::Bool)) => "boolean",
            _ => return Ok(None),
        };
        let binding = self.primitive_libraries.get(namespace).ok_or_else(|| {
            Diagnostic::new(
                span,
                format!("internal invariant: missing builtin library `{namespace}`"),
            )
        })?;
        let ty = Self::field_type(&binding.known, name, span)?;
        Ok(Some(self.node(
            Kind::Field(binding.value, name.into()),
            ty,
            span,
        )))
    }
}
