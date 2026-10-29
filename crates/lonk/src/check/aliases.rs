use super::*;

impl Checker {
    pub(super) fn normalize_loan(&self, loan: &Loan) -> Loan {
        let mut normalized = loan.clone();
        if let Some(alias) = self.aliases.get(&loan.identity) {
            normalized.root = alias.access.root.clone();
            normalized.identity = alias.access.identity;
            normalized.path = alias
                .access
                .path
                .iter()
                .cloned()
                .chain(loan.path.iter().cloned())
                .collect();
        }
        normalized
    }

    pub(super) fn check_shadow(&self, name: &str, env: &Env, span: Span) -> Result<(), Diagnostic> {
        if let Some(binding) = env.get(name) {
            if self.aliases.contains_key(&binding.identity)
                || self
                    .aliases
                    .values()
                    .any(|a| a.target.identity == binding.identity)
            {
                return Err(Diagnostic::new(
                    span,
                    "cannot shadow a live alias or its target; end the alias's block first",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn declare_alias(
        &mut self,
        name: &str,
        annotation: Option<&syntax::TypeExpr>,
        place: &syntax::Expr,
        env: &mut Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let argument = syntax::Expr {
            kind: syntax::ExprKind::Borrow(Box::new(place.clone())),
            span,
        };
        let start = self.loans.len();
        let borrowed = self.argument(&argument, &Type::Borrow(Box::new(Type::Any)), env)?;
        let mut access = self
            .loans
            .pop()
            .ok_or_else(|| Diagnostic::new(span, "missing alias access"))?;
        self.loans.truncate(start);
        let target = borrowed
            .loan
            .ok_or_else(|| Diagnostic::new(span, "missing alias target"))?;
        if target.root == name {
            return Err(Diagnostic::new(
                span,
                "an alias cannot shadow its own target",
            ));
        }
        let contract = if let Some(annotation) = annotation {
            let Type::Borrow(ty) = self.resolve_type(annotation, env)? else {
                return Err(Diagnostic::new(
                    annotation.span,
                    "an alias annotation must use &Type",
                ));
            };
            if !borrowed.value.ty.guarantees(&ty) || !ty.guarantees(&borrowed.value.ty) {
                return Err(Diagnostic::new(
                    span,
                    "alias annotation must preserve the target's current guarantees; an alias cannot project or widen",
                ));
            }
            *ty
        } else {
            borrowed.value.contract.clone()
        };
        let guard = self.node(
            Kind::BorrowCheck(target.base, target.path.clone(), target.guard),
            Type::Unit,
            span,
        );
        let value = self.node(
            Kind::After(guard.id, borrowed.value.id),
            borrowed.value.ty.clone(),
            span,
        );
        let mut permits = self
            .aliases
            .get(&target.identity)
            .map(|a| a.permits.clone())
            .unwrap_or_default();
        permits.insert(guard.id);
        access.handle = Some(guard.id);
        self.loans.push(access.clone());
        self.definitions.push(Definition {
            id: guard.id,
            name: name.into(),
            span,
            owner: self.owner,
        });
        self.aliases.insert(
            value.id,
            ScopedAlias {
                name: name.into(),
                target,
                access,
                permits,
                guard: guard.id,
            },
        );
        env.insert(
            name.into(),
            Binding {
                identity: value.id,
                value: value.id,
                known: borrowed.value.ty,
                contract,
                owner: self.owner,
            },
        );
        Ok(())
    }

    pub(super) fn guard_alias_write(
        &mut self,
        identity: Id,
        value: Checked,
        span: Span,
    ) -> Checked {
        match self.aliases.get(&identity) {
            Some(alias) => self.node(Kind::After(alias.guard, value.id), value.ty, span),
            None => value,
        }
    }

    pub(super) fn close_aliases(
        &mut self,
        entering: &BTreeSet<Id>,
        env: &mut Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let closing: Vec<_> = self
            .aliases
            .keys()
            .filter(|id| !entering.contains(id))
            .copied()
            .rev()
            .collect();
        for identity in closing {
            let alias = self
                .aliases
                .remove(&identity)
                .ok_or_else(|| Diagnostic::new(span, "missing scoped alias"))?;
            self.loans.retain(|loan| loan.handle != Some(alias.guard));
            let value = env
                .get(&alias.name)
                .filter(|b| b.identity == identity)
                .cloned()
                .ok_or_else(|| Diagnostic::new(span, "alias binding was shadowed"))?;
            let mut target = env
                .get(&alias.target.root)
                .filter(|b| b.identity == alias.target.identity)
                .cloned()
                .ok_or_else(|| Diagnostic::new(span, "alias target was shadowed"))?;
            if value.value == identity {
                continue;
            }
            let known = Self::updated_type(&target.known, &alias.target.path, &value.known);
            self.can_copy(&known, &target.contract, false)
                .map_err(|e| {
                    Diagnostic::new(
                        span,
                        format!("alias output violates its target's contract: {e}"),
                    )
                })?;
            let updated = if alias.target.path.is_empty() {
                self.node(Kind::Alias(value.value), known.clone(), span)
            } else {
                self.node(
                    Kind::Update(target.value, alias.target.path, value.value),
                    known.clone(),
                    span,
                )
            };
            let updated = self.guard_alias_write(target.identity, updated, span);
            self.definitions.push(Definition {
                id: updated.id,
                name: alias.target.root.clone(),
                span,
                owner: self.owner,
            });
            self.implicit_outputs.insert(updated.id, vec![value.value]);
            target.value = updated.id;
            target.known = known;
            env.insert(alias.target.root, target);
        }
        Ok(())
    }
}
