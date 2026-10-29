use super::*;

#[derive(Clone)]
enum Access {
    Field(String),
    Index(Option<i64>),
}

impl Checker {
    fn constant_index(&self, mut id: Id) -> Option<i64> {
        for _ in 0..self.nodes.len() {
            match &self.nodes.get(id)?.kind {
                Kind::Int(n) => return Some(*n),
                Kind::Alias(next) => id = *next,
                _ => return None,
            }
        }
        None
    }

    fn access_path(&self, path: &[Part]) -> Vec<Access> {
        path.iter()
            .map(|p| match p {
                Part::Field(name) => Access::Field(name.clone()),
                Part::Index(id) => Access::Index(self.constant_index(*id)),
            })
            .collect()
    }

    fn syntax_access(
        &self,
        ast: &syntax::Expr,
        env: &Env,
    ) -> Option<(Id, Vec<Access>, BTreeSet<Id>)> {
        match &ast.kind {
            syntax::ExprKind::Name(name) => env.get(name).map(|b| {
                if let Some(alias) = self.aliases.get(&b.identity) {
                    (
                        alias.access.identity,
                        self.access_path(&alias.access.path),
                        alias.permits.clone(),
                    )
                } else {
                    (b.identity, Vec::new(), BTreeSet::new())
                }
            }),
            syntax::ExprKind::Field(base, name) => {
                let (root, mut path, permits) = self.syntax_access(base, env)?;
                path.push(Access::Field(name.clone()));
                Some((root, path, permits))
            }
            syntax::ExprKind::Index(base, key) => {
                let (root, mut path, permits) = self.syntax_access(base, env)?;
                path.push(match &key.kind {
                    syntax::ExprKind::Str(name) => Access::Field(name.clone()),
                    syntax::ExprKind::Int(n) => Access::Index(Some(*n)),
                    syntax::ExprKind::Name(name) => {
                        Access::Index(env.get(name).and_then(|b| self.constant_index(b.value)))
                    }
                    _ => Access::Index(None),
                });
                Some((root, path, permits))
            }
            _ => None,
        }
    }

    fn overlaps(a: &[Access], b: &[Access]) -> bool {
        for (a, b) in a.iter().zip(b) {
            match (a, b) {
                (Access::Field(a), Access::Field(b)) if a != b => return false,
                (Access::Index(Some(a)), Access::Index(Some(b))) if a != b => return false,
                (Access::Index(None), _) | (_, Access::Index(None)) => return true,
                _ => (),
            }
        }
        true
    }

    pub(super) fn check_access(&self, ast: &syntax::Expr, env: &Env) -> Result<(), Diagnostic> {
        if let Some((identity, path, permits)) = self.syntax_access(ast, env) {
            if self.loans.iter().any(|loan| {
                !loan.handle.is_some_and(|handle| permits.contains(&handle))
                    && loan.identity == identity
                    && Self::overlaps(&path, &self.access_path(&loan.path))
            }) {
                return Err(Diagnostic::new(
                    ast.span,
                    "access overlaps an active writable borrow; copy the value to a local before borrowing",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn projection_base(
        &mut self,
        ast: &syntax::Expr,
        env: &mut Env,
    ) -> Result<Checked, Diagnostic> {
        let previous = self.projection_read;
        if self.syntax_access(ast, env).is_some() {
            self.projection_read += 1;
        }
        let value = self.expr(ast, env, None);
        self.projection_read = previous;
        value
    }

    pub(super) fn read_location(
        &mut self,
        binding: &Binding,
        path: &[Part],
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let mut value = self.node(Kind::Alias(binding.value), binding.known.clone(), span);
        value.contract = binding.contract.clone();
        for part in path {
            value = match part {
                Part::Field(name) => {
                    let ty = Self::field_type(&value.ty, name, span)?;
                    let contract = match &value.contract {
                        Type::Record { fields, .. } => fields.get(name).map(|f| f.ty.clone()),
                        _ => None,
                    };
                    let mut field = self.node(Kind::Field(value.id, name.clone()), ty, span);
                    if let Some(contract) = contract {
                        field.contract = contract;
                    }
                    field
                }
                Part::Index(index) => {
                    let Type::Array(element) = &value.ty else {
                        return Err(Diagnostic::new(span, "borrowed indexing requires an array"));
                    };
                    let contract = if let Type::Array(t) = &value.contract {
                        Some(*t.clone())
                    } else {
                        None
                    };
                    let mut item = self.node(Kind::Index(value.id, *index), *element.clone(), span);
                    if let Some(contract) = contract {
                        item.contract = contract;
                    }
                    item
                }
            };
        }
        Ok(value)
    }

    pub(super) fn argument(
        &mut self,
        arg: &syntax::Expr,
        parameter: &Type,
        env: &mut Env,
    ) -> Result<Argument, Diagnostic> {
        let parameter = parameter.argument_type();
        match (&arg.kind, parameter) {
            (syntax::ExprKind::Borrow(place), Type::Borrow(_)) => {
                self.check_access(place, env)?;
                let before = env.clone();
                let (root, path) = self.place(place, env)?;
                if env
                    .iter()
                    .any(|(name, b)| before.get(name).is_some_and(|old| old.value != b.value))
                {
                    return Err(Diagnostic::new(
                        arg.span,
                        "borrow indices must not mutate bindings; compute the index before borrowing",
                    ));
                }
                let binding = env
                    .get(&root)
                    .cloned()
                    .ok_or_else(|| Diagnostic::new(arg.span, "unknown borrowed location"))?;
                if binding.owner != self.owner {
                    return Err(Diagnostic::new(
                        arg.span,
                        "a closure cannot borrow a captured binding for writing",
                    ));
                }
                Self::path_type(&binding.contract, &binding.known, &path, arg.span)?;
                let value = self.read_location(&binding, &path, arg.span)?;
                let loan = Loan {
                    root,
                    identity: binding.identity,
                    base: binding.value,
                    path,
                    ty: value.ty.clone(),
                    handle: None,
                    guard: self.aliases.get(&binding.identity).map(|alias| alias.guard),
                };
                self.loans.push(self.normalize_loan(&loan));
                Ok(Argument {
                    value,
                    loan: Some(loan),
                    span: arg.span,
                    fresh: false,
                })
            }
            (_, Type::Borrow(_)) => Err(Diagnostic::new(
                arg.span,
                "borrowed parameter requires an explicit &place argument",
            )),
            (syntax::ExprKind::Borrow(_), _) => Err(Diagnostic::new(
                arg.span,
                "a value parameter does not accept a borrow",
            )),
            _ => Ok(Argument {
                value: self.expr_unprojected(arg, env, Some(parameter))?,
                loan: None,
                span: arg.span,
                fresh: matches!(arg.kind, syntax::ExprKind::Record(_)),
            }),
        }
    }

    pub(super) fn borrowed_outputs(
        &mut self,
        parameters: &[syntax::Parameter],
        types: &[Type],
        result: Checked,
        env: &Env,
        span: Span,
    ) -> Result<Id, Diagnostic> {
        let mut values = Vec::new();
        let mut fields = BTreeMap::new();
        if result.ty != Type::Unit {
            fields.insert("$return".into(), Field::required(result.ty));
            values.push((Some("$return".into()), result.id));
        }
        for (index, (parameter, ty)) in parameters.iter().zip(types).enumerate() {
            if let Type::Borrow(payload) = ty {
                let value = &env[&parameter.name];
                self.can_copy(&value.known, payload, false).map_err(|e| {
                    Diagnostic::new(
                        span,
                        format!(
                            "borrowed output `{}` violates its contract: {e}",
                            parameter.name
                        ),
                    )
                })?;
                let name = format!("$arg{index}");
                fields.insert(name.clone(), Field::required(*payload.clone()));
                values.push((Some(name), value.value));
            }
        }
        Ok(self
            .node(
                Kind::Record(values),
                Type::Record {
                    fields: fields.into(),
                    open: false,
                    row: None,
                },
                span,
            )
            .id)
    }

    pub(super) fn finish_call(
        &mut self,
        function: Id,
        parameters: &[Type],
        result: Type,
        checked: Vec<Argument>,
        env: &mut Env,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let mut values = Vec::new();
        let mut loans = Vec::new();
        let mut output_fields = BTreeMap::new();
        for (index, (parameter, checked)) in parameters.iter().zip(checked).enumerate() {
            if let Type::Borrow(payload) = parameter {
                let loan = checked
                    .loan
                    .ok_or_else(|| Diagnostic::new(checked.span, "missing borrowed place"))?;
                if !checked.value.ty.guarantees(payload) {
                    return Err(Diagnostic::new(
                        checked.span,
                        format!(
                            "borrow entry requires {payload}, found {}; borrowing cannot project fields",
                            checked.value.ty
                        ),
                    ));
                }
                if !payload.guarantees(&loan.ty) {
                    return Err(Diagnostic::new(
                        checked.span,
                        format!(
                            "borrowed output {payload} would lose the caller's established guarantees {}",
                            loan.ty
                        ),
                    ));
                }
                output_fields.insert(format!("$arg{index}"), Field::required(*payload.clone()));
                values.push(checked.value.id);
                loans.push((index, loan));
            } else {
                values.push(
                    self.project(
                        checked.value,
                        parameter.argument_type(),
                        checked.fresh,
                        checked.span,
                    )?
                    .id,
                );
            }
        }
        if loans.is_empty() {
            return Ok(self.node(Kind::Call(function, values, Vec::new()), result, span));
        }
        if result != Type::Unit {
            output_fields.insert("$return".into(), Field::required(result.clone()));
        }
        let places = loans
            .iter()
            .map(|(_, loan)| {
                self.node(
                    Kind::BorrowCheck(loan.base, loan.path.clone(), loan.guard),
                    Type::Unit,
                    span,
                )
                .id
            })
            .collect();
        let call = self.node(
            Kind::Call(function, values, places),
            Type::Record {
                fields: output_fields.into(),
                open: false,
                row: None,
            },
            span,
        );
        for (index, loan) in loans {
            let mut binding = env[&loan.root].clone();
            if binding.identity != loan.identity {
                return Err(Diagnostic::new(
                    span,
                    "borrowed binding was shadowed during argument evaluation",
                ));
            }
            let output = self.node(
                Kind::Field(call.id, format!("$arg{index}")),
                loan.ty.clone(),
                span,
            );
            let known = Self::updated_type(&binding.known, &loan.path, &loan.ty);
            self.can_copy(&known, &binding.contract, false)
                .map_err(|e| {
                    Diagnostic::new(
                        span,
                        format!("borrowed output violates the caller's binding contract: {e}"),
                    )
                })?;
            let updated = if loan.path.is_empty() {
                self.node(Kind::Alias(output.id), known.clone(), span)
            } else {
                self.node(
                    Kind::Update(binding.value, loan.path, output.id),
                    known.clone(),
                    span,
                )
            };
            let updated = self.guard_alias_write(binding.identity, updated, span);
            self.definitions.push(Definition {
                id: updated.id,
                name: loan.root.clone(),
                span,
                owner: self.owner,
            });
            self.implicit_outputs.insert(updated.id, vec![call.id]);
            binding.value = updated.id;
            binding.known = known;
            env.insert(loan.root, binding);
        }
        if result == Type::Unit {
            Ok(self.node(Kind::Unit, Type::Unit, span))
        } else {
            Ok(self.node(Kind::Field(call.id, "$return".into()), result, span))
        }
    }
}
