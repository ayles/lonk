use super::*;

impl Checker {
    pub(super) fn is_method(ty: &Type) -> bool {
        match ty {
            Type::Function { receiver, .. } => *receiver,
            Type::Generic { body, .. } => Self::is_method(body),
            _ => false,
        }
    }

    pub(super) fn inferred_receiver(
        &mut self,
        parameters: &[syntax::Parameter],
        context: Option<Type>,
        ids: &mut Vec<usize>,
    ) -> Option<Type> {
        let parameter = parameters.first()?;
        if parameter.name != "self" || parameter.ty.is_some() {
            return None;
        }
        let Type::Record { fields, .. } = context? else {
            return None;
        };
        let id = self.generics.len();
        let name = "$Self".to_owned();
        self.generics.insert(
            id,
            GenericState {
                parameter: GenericParameter {
                    id,
                    name: name.clone(),
                    row: Some(fields.keys().cloned().collect()),
                    implicit: true,
                },
                kind: Some(true),
            },
        );
        ids.push(id);
        Some(Type::Record {
            fields,
            open: false,
            row: Some((id, name)),
        })
    }

    pub(super) fn invoke(
        &mut self,
        callee: (Checked, Vec<Argument>),
        type_args: &[syntax::TypeExpr],
        args: &[syntax::Expr],
        env: &mut Env,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let (function, mut arguments) = callee;
        if matches!(function.ty, Type::Generic { .. }) {
            return self.generic_call((function, arguments), type_args, args, env, expected, span);
        }
        if !type_args.is_empty() {
            return Err(Diagnostic::new(
                span,
                "this function does not take generic arguments",
            ));
        }
        let Type::Function {
            parameters, result, ..
        } = &function.ty
        else {
            return Err(Diagnostic::new(
                span,
                "calling requires a checked function type",
            ));
        };
        Self::check_arity(parameters, args.len() + arguments.len(), span)?;
        let parameters = parameters.clone();
        let result = *result.clone();
        let start = self.loans.len() - arguments.iter().filter(|a| a.loan.is_some()).count();
        for (arg, ty) in args.iter().zip(parameters.iter().skip(arguments.len())) {
            arguments.push(self.argument(arg, ty, env)?);
        }
        self.loans.truncate(start);
        self.finish_call(function.id, &parameters, result, arguments, env, span)
    }

    pub(super) fn member_call(
        &mut self,
        member: (&syntax::Expr, &str),
        type_args: &[syntax::TypeExpr],
        args: &[syntax::Expr],
        env: &mut Env,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let (base, name) = member;
        let field = syntax::Expr {
            kind: syntax::ExprKind::Field(Box::new(base.clone()), name.into()),
            span,
        };
        self.check_access(&field, env)?;
        // Resolve a place once. Method lookup and the receiver share its pinned
        // index nodes, so computed indices cannot select a different location.
        let (receiver, loan) = if Self::syntax_place(base) {
            let before = env.clone();
            let (root, path) = self.place(base, env)?;
            if env
                .iter()
                .any(|(n, b)| before.get(n).is_some_and(|old| old.value != b.value))
            {
                return Err(Diagnostic::new(
                    base.span,
                    "receiver indices must not mutate bindings; compute the index before the member call",
                ));
            }
            let binding = env
                .get(&root)
                .ok_or_else(|| Diagnostic::new(base.span, "unknown receiver"))?;
            let receiver = self.read_location(binding, &path, base.span)?;
            let loan = Loan {
                root,
                identity: binding.identity,
                base: binding.value,
                path,
                ty: receiver.ty.clone(),
                handle: None,
                guard: self.aliases.get(&binding.identity).map(|a| a.guard),
            };
            (receiver, Some(loan))
        } else {
            (self.projection_base(base, env)?, None)
        };
        let primitive = self.primitive_member(&receiver.ty, name, span)?;
        let ty = if let Some(function) = &primitive {
            function.ty.clone()
        } else {
            Self::field_type(&receiver.ty,name,span).map_err(|error| {
            if matches!(&base.kind,syntax::ExprKind::Name(n) if n=="self") && matches!(&receiver.ty,Type::Record{row:Some((_,n)),..} if n=="$Self") {
                Diagnostic::new(span,format!("{}; inferred self excludes receiver-method fields: use an explicit receiver contract or an ordinary callable field",error.message))
            } else {error}
        })?
        };
        let method = primitive.is_some() || Self::is_method(&ty);
        let signature = if let Type::Generic { body, .. } = &ty {
            body.as_ref()
        } else {
            &ty
        };
        let borrowed = matches!(signature,Type::Function{parameters,..} if matches!(parameters.first(),Some(Type::Borrow(_))));
        let function =
            primitive.unwrap_or_else(|| self.node(Kind::Field(receiver.id, name.into()), ty, span));
        let mut prefix = Vec::new();
        if method {
            self.check_access(base, env)?;
            let loan = if borrowed {
                let loan=loan.ok_or_else(||Diagnostic::new(base.span,"a writable receiver requires a local place; bind this value with let first"))?;
                let binding = &env[&loan.root];
                if binding.owner != self.owner {
                    return Err(Diagnostic::new(
                        base.span,
                        "a closure cannot borrow a captured receiver for writing",
                    ));
                }
                Self::path_type(&binding.contract, &binding.known, &loan.path, base.span)?;
                self.loans.push(self.normalize_loan(&loan));
                Some(loan)
            } else {
                None
            };
            prefix.push(Argument {
                value: receiver,
                loan,
                span: base.span,
                fresh: false,
            });
        }
        self.invoke((function, prefix), type_args, args, env, expected, span)
    }

    fn syntax_place(ast: &syntax::Expr) -> bool {
        match &ast.kind {
            syntax::ExprKind::Name(_) => true,
            syntax::ExprKind::Field(base, _) | syntax::ExprKind::Index(base, _) => {
                Self::syntax_place(base)
            }
            _ => false,
        }
    }
}
