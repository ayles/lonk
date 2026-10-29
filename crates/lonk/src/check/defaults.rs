use super::*;

impl Checker {
    pub(super) fn check_arity(
        parameters: &[Type],
        count: usize,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let required = Type::required_arguments(parameters);
        if count < required || count > parameters.len() {
            let range = if required == parameters.len() {
                required.to_string()
            } else {
                format!("{required}..={}", parameters.len())
            };
            return Err(Diagnostic::new(
                span,
                format!("expected {range} arguments, found {count}"),
            ));
        }
        Ok(())
    }

    pub(super) fn function_parameters(
        &mut self,
        parameters: &[syntax::Parameter],
        type_env: &Env,
        body_env: &mut Env,
        expected: Option<&Type>,
        context_owner: (usize, Option<&Type>),
    ) -> Result<(Vec<Type>, Vec<Id>), Diagnostic> {
        let (outside, receiver_context) = context_owner;
        let mut default_env = type_env.clone();
        // Later parameters and the current parameter are unavailable, even if
        // the surrounding scope happens to contain a name spelled the same.
        for p in parameters {
            default_env.remove(&p.name);
        }
        let context = if let Some(Type::Function { parameters, .. }) = expected {
            Some(parameters)
        } else {
            None
        };
        let mut names = BTreeSet::new();
        let mut types = Vec::new();
        let mut defaults = Vec::new();
        let mut saw_default = false;
        for (index, parameter) in parameters.iter().enumerate() {
            if !names.insert(&parameter.name) {
                return Err(Diagnostic::new(parameter.span, "duplicate parameter name"));
            }
            if parameter.name == "self" && index != 0 {
                return Err(Diagnostic::new(
                    parameter.span,
                    "self must be the first parameter",
                ));
            }
            if parameter.name == "self" && parameter.default.is_some() {
                return Err(Diagnostic::new(
                    parameter.span,
                    "a receiver cannot have a default",
                ));
            }
            if parameter.default.is_none() && saw_default {
                return Err(Diagnostic::new(
                    parameter.span,
                    "only trailing value parameters may have defaults",
                ));
            }
            let annotation = parameter
                .ty
                .as_ref()
                .map(|t| self.resolve_type(t, type_env))
                .transpose()?;
            let receiver = (parameter.name == "self")
                .then_some(receiver_context)
                .flatten()
                .cloned();
            let mut declared = annotation.or(receiver).or_else(|| {
                context
                    .and_then(|ps| ps.get(index))
                    .map(|t| t.argument_type().clone())
            });
            if parameter.borrowed_self {
                declared = declared.map(|t| {
                    if matches!(t, Type::Borrow(_)) {
                        t
                    } else {
                        Type::Borrow(Box::new(t))
                    }
                });
            }
            if parameter.ty.is_none()
                && parameter.default.is_some()
                && declared
                    .as_ref()
                    .is_some_and(|t| t.mentions(&self.inference))
            {
                declared = None;
            }
            if parameter.default.is_some() && matches!(declared, Some(Type::Borrow(_))) {
                return Err(Diagnostic::new(
                    parameter.span,
                    "a borrowed parameter cannot have a default",
                ));
            }
            let default = parameter
                .default
                .as_ref()
                .map(|value| self.expr(value, &mut default_env, declared.as_ref()))
                .transpose()?;
            let ty=declared.or_else(||default.as_ref().map(|value|value.ty.clone())).ok_or_else(||Diagnostic::new(parameter.span,"this parameter needs a type annotation, a typed default, or an expected function type"))?;
            let payload = if let Type::Borrow(t) = &ty {
                *t.clone()
            } else {
                ty.clone()
            };
            if !payload.storable() || payload.contains_unresolved() {
                return Err(Diagnostic::new(
                    parameter.span,
                    "a parameter must have a resolved storable value type",
                ));
            }
            let default_id = default.map(|value| value.id);
            let node = self.node(
                Kind::Parameter(index, default_id),
                payload.clone(),
                parameter.span,
            );
            let binding = Binding {
                identity: node.id,
                value: node.id,
                contract: payload.clone(),
                known: payload,
                owner: self.owner,
            };
            body_env.insert(parameter.name.clone(), binding.clone());
            if !matches!(ty, Type::Borrow(_)) {
                // Defaults see immutable entry versions. Local copies inside
                // a default expression may still be updated normally.
                default_env.insert(
                    parameter.name.clone(),
                    Binding {
                        owner: outside,
                        ..binding
                    },
                );
            }
            if let Some(id) = default_id {
                saw_default = true;
                defaults.push(id);
                types.push(Type::Default(Box::new(ty)));
            } else {
                types.push(ty);
            }
        }
        Ok((types, defaults))
    }
}
