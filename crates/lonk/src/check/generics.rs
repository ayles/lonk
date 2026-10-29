use super::*;

impl Checker {
    pub(super) fn specialization(
        &self,
        parameters: &[GenericParameter],
        body: &Type,
        target: &Type,
        span: Span,
    ) -> Result<(Type, Vec<Type>), Diagnostic> {
        let mut substitutions = BTreeMap::new();
        self.infer(body, target, parameters, &mut substitutions, span)?;
        self.complete_arguments(parameters, &substitutions, span)?;
        let specialized = body.substitute(&substitutions);
        self.can_copy(&specialized, target, false)
            .map_err(|e| Diagnostic::new(span, e))?;
        let arguments = parameters
            .iter()
            .map(|p| substitutions[&p.id].clone())
            .collect();
        Ok((specialized, arguments))
    }
    pub(super) fn row_exclusions(
        &self,
        id: usize,
        span: Span,
    ) -> Result<&BTreeSet<String>, Diagnostic> {
        self.generics
            .get(&id)
            .and_then(|state| state.parameter.row.as_ref())
            .ok_or_else(|| Diagnostic::new(span, "missing captured row metadata"))
    }
    fn inference_argument(
        &mut self,
        ast: &syntax::Expr,
        env: &mut Env,
        context: &Type,
        ids: &BTreeSet<usize>,
    ) -> Result<Argument, Diagnostic> {
        let previous = self.inference.clone();
        self.inference.extend(ids);
        let result = self.argument(ast, context, env);
        self.inference = previous;
        result
    }
    pub(super) fn bind_generics(
        &mut self,
        declarations: &[(String, Span)],
        env: &mut Env,
    ) -> Result<Vec<usize>, Diagnostic> {
        let mut names = BTreeSet::new();
        let mut ids = Vec::new();
        for (name, span) in declarations {
            if !names.insert(name)
                || matches!(name.as_str(), "int" | "str" | "bool" | "any" | "self")
            {
                return Err(Diagnostic::new(
                    *span,
                    "duplicate or reserved generic parameter name",
                ));
            }
            let id = self.generics.len();
            self.generics.insert(
                id,
                GenericState {
                    parameter: GenericParameter {
                        id,
                        name: name.clone(),
                        row: None,
                        implicit: false,
                    },
                    kind: None,
                },
            );
            let ty = Type::Descriptor(Box::new(Type::Parameter {
                id,
                name: name.clone(),
            }));
            let node = self.node(Kind::Descriptor, ty.clone(), *span);
            env.insert(
                name.clone(),
                Binding {
                    identity: node.id,
                    value: node.id,
                    contract: ty.clone(),
                    known: ty,
                    owner: self.owner,
                },
            );
            ids.push(id);
        }
        Ok(ids)
    }

    pub(super) fn use_generic(
        &mut self,
        id: usize,
        row: bool,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let state = self
            .generics
            .get_mut(&id)
            .ok_or_else(|| Diagnostic::new(span, "unknown generic parameter"))?;
        if state.kind.is_some_and(|previous| previous != row) {
            return Err(Diagnostic::new(
                span,
                format!(
                    "generic `{}` cannot be both a value type and a row",
                    state.parameter.name
                ),
            ));
        }
        state.kind = Some(row);
        if row {
            state.parameter.row.get_or_insert_with(BTreeSet::new);
        }
        Ok(())
    }

    pub(super) fn quantify(
        &self,
        ids: &[usize],
        body: Type,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        if ids.is_empty() {
            return Ok(body);
        }
        let mut parameters = Vec::new();
        for id in ids {
            let state = &self.generics[id];
            if state.kind.is_none() {
                return Err(Diagnostic::new(
                    span,
                    format!("unused generic parameter `{}`", state.parameter.name),
                ));
            }
            parameters.push(state.parameter.clone());
        }
        Ok(Type::Generic {
            parameters,
            body: Box::new(body),
        })
    }

    pub(super) fn explicit_arguments(
        &mut self,
        parameters: &[GenericParameter],
        arguments: &[syntax::TypeExpr],
        env: &Env,
        span: Span,
    ) -> Result<BTreeMap<usize, Type>, Diagnostic> {
        if parameters.len() != arguments.len() {
            return Err(Diagnostic::new(
                span,
                format!(
                    "expected {} type arguments, found {}",
                    parameters.len(),
                    arguments.len()
                ),
            ));
        }
        let mut substitutions = BTreeMap::new();
        for (parameter, ast) in parameters.iter().zip(arguments) {
            let ty = self.resolve_type(ast, env)?;
            if !ty.storable() {
                return Err(Diagnostic::new(
                    ast.span,
                    "a generic argument must be a storable type",
                ));
            }
            self.check_row_argument(parameter, &ty, span)?;
            substitutions.insert(parameter.id, ty);
        }
        Ok(substitutions)
    }

    fn check_row_argument(
        &self,
        parameter: &GenericParameter,
        actual: &Type,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if let Some(excluded) = &parameter.row {
            let Type::Record { fields, row, .. } = actual else {
                return Err(Diagnostic::new(
                    span,
                    format!("row `{}` needs an object remainder", parameter.name),
                ));
            };
            if fields.keys().any(|name| excluded.contains(name)) {
                return Err(Diagnostic::new(
                    span,
                    format!("row `{}` contains an excluded named field", parameter.name),
                ));
            }
            if let Some((id, _)) = row {
                let outer = self.row_exclusions(*id, span)?;
                if !excluded.is_subset(outer) {
                    return Err(Diagnostic::new(
                        span,
                        "captured row cannot prove the required field exclusions",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn infer(
        &self,
        template: &Type,
        actual: &Type,
        parameters: &[GenericParameter],
        substitutions: &mut BTreeMap<usize, Type>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        match (template, actual) {
            (Type::Union(cases), Type::Union(actual_cases)) => {
                for actual in actual_cases {
                    if let Ok(template) = actual.variant_case(cases) {
                        self.infer(template, actual, parameters, substitutions, span)?;
                    }
                }
            }
            (Type::Union(cases), actual) => {
                if let Ok(template) = actual.variant_case(cases) {
                    self.infer(template, actual, parameters, substitutions, span)?;
                }
            }
            (Type::Default(t), Type::Default(a)) => {
                self.infer(t, a, parameters, substitutions, span)?
            }
            (Type::Default(t), a) => self.infer(t, a, parameters, substitutions, span)?,
            (t, Type::Default(a)) => self.infer(t, a, parameters, substitutions, span)?,
            (Type::Parameter { id, .. }, _) if parameters.iter().any(|p| p.id == *id) => {
                if actual.contains_unresolved() {
                    return Ok(());
                }
                self.bind_inference(*id, actual.clone(), substitutions, span)?;
            }
            (Type::Array(t), Type::Array(a)) | (Type::Borrow(t), Type::Borrow(a)) => {
                self.infer(t, a, parameters, substitutions, span)?
            }
            (Type::Descriptor(t), Type::Descriptor(a)) => {
                self.infer(t, a, parameters, substitutions, span)?
            }
            (
                Type::Function {
                    parameters: tp,
                    result: tr,
                    receiver: tm,
                },
                Type::Function {
                    parameters: ap,
                    result: ar,
                    receiver: am,
                },
            ) => {
                if tm != am {
                    return Err(Diagnostic::new(
                        span,
                        "method and ordinary callable contracts have different receiver modes",
                    ));
                }
                if tp.len() != ap.len() {
                    return Err(Diagnostic::new(span, "incompatible callback arity"));
                }
                for (t, a) in tp.iter().zip(ap) {
                    self.infer(t, a, parameters, substitutions, span)?;
                }
                self.infer(tr, ar, parameters, substitutions, span)?;
            }
            (
                Type::Record {
                    fields: tf,
                    row: tr,
                    ..
                },
                Type::Record {
                    fields: af,
                    open: ao,
                    row: ar,
                },
            ) => {
                for (name, field) in tf.iter() {
                    if let Some(actual) = af.get(name) {
                        self.infer(&field.ty, &actual.ty, parameters, substitutions, span)?;
                    }
                }
                if let Some((id, _)) = tr {
                    if let Some(parameter) = parameters.iter().find(|p| p.id == *id) {
                        let rest = Type::Record {
                            fields: af
                                .iter()
                                .filter(|(name, _)| !tf.contains_key(*name))
                                .map(|(n, f)| (n.clone(), f.clone()))
                                .collect::<BTreeMap<_, _>>()
                                .into(),
                            open: *ao,
                            row: ar.clone(),
                        };
                        self.check_row_argument(parameter, &rest, span)?;
                        if *ao
                            && self
                                .row_exclusions(parameter.id, span)?
                                .iter()
                                .any(|n| !tf.contains_key(n))
                        {
                            return Err(Diagnostic::new(
                                span,
                                "open remainder may contain a field excluded by another use of this row",
                            ));
                        }
                        self.bind_inference(*id, rest, substitutions, span)?;
                    }
                }
            }
            _ => (),
        }
        Ok(())
    }

    fn bind_inference(
        &self,
        id: usize,
        actual: Type,
        substitutions: &mut BTreeMap<usize, Type>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if let Some(previous) = substitutions.get(&id) {
            if previous != &actual {
                return Err(Diagnostic::new(
                    span,
                    format!(
                        "conflicting inference for `{}`: {previous} and {actual}; annotate or supply explicit type arguments",
                        self.generics[&id].parameter.name
                    ),
                ));
            }
        } else {
            substitutions.insert(id, actual);
        }
        Ok(())
    }

    pub(super) fn complete_arguments(
        &self,
        parameters: &[GenericParameter],
        substitutions: &BTreeMap<usize, Type>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        for parameter in parameters {
            if !substitutions.contains_key(&parameter.id) {
                return Err(Diagnostic::new(
                    span,
                    format!(
                        "cannot infer `{}`; annotate the result or supply explicit type arguments",
                        parameter.name
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn generic_call(
        &mut self,
        callee: (Checked, Vec<Argument>),
        type_args: &[syntax::TypeExpr],
        args: &[syntax::Expr],
        env: &mut Env,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let (function, prefix) = callee;
        let prefix_len = prefix.len();
        let Type::Generic {
            parameters: generics,
            body,
        } = &function.ty
        else {
            unreachable!()
        };
        let (generics, body) = (generics.clone(), *body.clone());
        let Type::Function {
            parameters, result, ..
        } = &body
        else {
            return Err(Diagnostic::new(span, "generic value is not callable"));
        };
        Self::check_arity(parameters, args.len() + prefix_len, span)?;
        let explicit = !type_args.is_empty();
        let mut substitutions = if explicit {
            self.explicit_arguments(
                &generics
                    .iter()
                    .filter(|p| !p.implicit)
                    .cloned()
                    .collect::<Vec<_>>(),
                type_args,
                env,
                span,
            )?
        } else {
            BTreeMap::new()
        };
        let ids: BTreeSet<_> = generics.iter().map(|p| p.id).collect();
        let inferred: Vec<_> = generics
            .iter()
            .filter(|p| !explicit || p.implicit)
            .cloned()
            .collect();
        let loan_start = self.loans.len() - prefix.iter().filter(|a| a.loan.is_some()).count();
        let mut checked = Vec::new();
        let mut delayed = Vec::new();
        for (value, template) in prefix.into_iter().zip(parameters) {
            let actual = if value.loan.is_some() {
                Type::Borrow(Box::new(value.value.ty.clone()))
            } else {
                value.value.ty.clone()
            };
            self.infer(template, &actual, &inferred, &mut substitutions, value.span)?;
            checked.push(Some(value));
        }
        for (offset, (arg, template)) in args
            .iter()
            .zip(parameters.iter().skip(prefix_len))
            .enumerate()
        {
            let i = prefix_len + offset;
            let context = template.argument_type().substitute(&substitutions);
            let pending_callback = matches!(&arg.kind, syntax::ExprKind::Function(_, ps, _, _) if ps.iter().any(|p| p.ty.is_none() && p.default.is_none()))
                && matches!(&context, Type::Function { parameters, .. } if parameters.iter().any(|t| t.mentions(&ids)));
            if pending_callback {
                checked.push(None);
                delayed.push((i, env.clone(), self.loans.clone()));
                continue;
            }
            // A fresh literal written where every variable of its parameter is already
            // established contributes no inference of its own: finish_call checks it
            // against that type exactly like an annotated binding (fresh flag), so an
            // optional field may be present or absent and an unknown field is a typo.
            // Context becomes available left to right, from the receiver and earlier
            // arguments: a literal in a position whose variables are still open
            // determines them as before, so `pair({ n = 2; tag = "t" }, s)` fixes T
            // from the literal while `pair(s, { n = 2; tag = "t" })` checks the literal
            // against s. Non-literal arguments remain strict and an established type
            // never widens; only the contextual checking of fresh literals is
            // position dependent.
            let contextual = matches!(
                arg.kind,
                syntax::ExprKind::Record(_) | syntax::ExprKind::Array(_)
            ) && !context.mentions(&ids);
            let value = self.inference_argument(arg, env, &context, &ids)?;
            if !inferred.is_empty() && !contextual {
                let actual = if value.loan.is_some() {
                    Type::Borrow(Box::new(value.value.ty.clone()))
                } else {
                    value.value.ty.clone()
                };
                self.infer(template, &actual, &inferred, &mut substitutions, arg.span)?;
            }
            checked.push(Some(value));
        }
        // An annotated result can project a wider call result. Use it only to
        // solve variables the arguments did not determine, never to silently
        // constrain their independently inferred types to a narrower shape.
        if !explicit {
            if let Some(expected) = expected {
                let missing: Vec<_> = generics
                    .iter()
                    .filter(|p| !substitutions.contains_key(&p.id))
                    .cloned()
                    .collect();
                self.infer(result, expected, &missing, &mut substitutions, span)?;
            }
        }
        for (i, mut snapshot, prior_loans) in delayed {
            let arg = &args[i - prefix_len];
            let context = parameters[i].argument_type().substitute(&substitutions);
            if matches!(&context, Type::Function { parameters, .. } if parameters.iter().any(|t| t.mentions(&ids)))
            {
                return Err(Diagnostic::new(
                    arg.span,
                    "cannot infer callback parameter types; annotate its parameters",
                ));
            }
            let current_loans = std::mem::replace(&mut self.loans, prior_loans);
            let value = self.inference_argument(arg, &mut snapshot, &context, &ids);
            self.loans = current_loans;
            let value = value?;
            self.infer(
                &parameters[i],
                &value.value.ty,
                &inferred,
                &mut substitutions,
                arg.span,
            )?;
            checked[i] = Some(value);
        }
        self.complete_arguments(&generics, &substitutions, span)?;
        let arguments = checked
            .into_iter()
            .map(|arg| arg.ok_or_else(|| Diagnostic::new(span, "unresolved generic argument")))
            .collect::<Result<_, _>>()?;
        let parameters: Vec<_> = parameters
            .iter()
            .map(|p| p.substitute(&substitutions))
            .collect();
        let result = result.substitute(&substitutions);
        let instantiated = self.node(
            Kind::Instantiate(
                function.id,
                generics
                    .iter()
                    .map(|p| substitutions[&p.id].clone())
                    .collect(),
            ),
            body.substitute(&substitutions),
            span,
        );
        self.loans.truncate(loan_start);
        self.finish_call(instantiated.id, &parameters, result, arguments, env, span)
    }
}
