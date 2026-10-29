use super::*;

#[derive(Clone)]
pub(super) struct Flow {
    pub guard: Option<Id>,
    pub exits: Vec<Exit>,
    pub expected: Option<Type>,
    pub parameters: Vec<syntax::Parameter>,
    pub types: Vec<Type>,
    pub aliases: BTreeSet<Id>,
    pub scope_base: usize,
}
#[derive(Clone)]
pub(super) struct Exit {
    pub guard: Id,
    pub output: Id,
    pub ty: Type,
    pub span: Span,
}

impl Checker {
    pub(super) fn reachable(&self) -> bool {
        self.flow.as_ref().is_none_or(|f| f.guard.is_some())
    }

    pub(super) fn guard(&self) -> Option<Id> {
        self.flow.as_ref().and_then(|f| f.guard)
    }

    pub(super) fn set_guard(&mut self, guard: Option<Id>) {
        if let Some(flow) = &mut self.flow {
            flow.guard = guard;
        }
    }

    pub(super) fn branch_guard(
        &mut self,
        parent: Option<Id>,
        condition: Id,
        yes: bool,
        span: Span,
    ) -> Option<Id> {
        let parent = parent?;
        let zero = self.node(Kind::Bool(false), Type::Bool, span).id;
        let condition = if yes {
            condition
        } else {
            self.node(Kind::Unary(Unary::Not, condition), Type::Bool, span)
                .id
        };
        Some(
            self.node(Kind::If(parent, condition, zero), Type::Bool, span)
                .id,
        )
    }

    pub(super) fn join_guards(&mut self, guards: &[Id], span: Span) -> Option<Id> {
        let (&last, rest) = guards.split_last()?;
        let one = self.node(Kind::Bool(true), Type::Bool, span).id;
        Some(rest.iter().rev().fold(last, |next, guard| {
            self.node(Kind::If(*guard, one, next), Type::Bool, span).id
        }))
    }

    pub(super) fn return_statement(
        &mut self,
        value: Option<&syntax::Expr>,
        env: &mut Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let flow = self
            .flow
            .clone()
            .ok_or_else(|| Diagnostic::new(span, "return is only allowed in a function body"))?;
        let result = match value {
            Some(value) => self.expr(value, env, flow.expected.as_ref())?,
            None => self.node(Kind::Unit, Type::Unit, span),
        };
        if !self.reachable() {
            return Ok(());
        }
        if value.is_none() && flow.expected.as_ref().is_some_and(|t| *t != Type::Unit) {
            return Err(Diagnostic::new(
                span,
                "return needs a value matching the function result type",
            ));
        }
        // A return observes the current versions and closes every alias owned
        // by this function. Keep the checking state for the other branch intact.
        let aliases = self.aliases.clone();
        let loans = self.loans.clone();
        let mut output_env = env.clone();
        self.close_aliases(&flow.aliases, &mut output_env, span)?;
        self.aliases = aliases;
        self.loans = loans;
        for entering in self.scopes[flow.scope_base..].iter().rev() {
            let leaving = output_env;
            output_env = entering.clone();
            for (name, binding) in &mut output_env {
                if let Some(updated) = leaving.get(name) {
                    if binding.identity == updated.identity {
                        *binding = updated.clone();
                    }
                }
            }
        }
        let output = if flow.types.iter().any(|t| matches!(t, Type::Borrow(_))) {
            self.borrowed_outputs(
                &flow.parameters,
                &flow.types,
                result.clone(),
                &output_env,
                span,
            )?
        } else {
            result.id
        };
        let guard = self
            .guard()
            .ok_or_else(|| Diagnostic::new(span, "unreachable return"))?;
        let current = self
            .flow
            .as_mut()
            .ok_or_else(|| Diagnostic::new(span, "internal invariant: checked function flow"))?;
        current.exits.push(Exit {
            guard,
            output,
            ty: result.ty,
            span,
        });
        current.guard = None;
        Ok(())
    }

    pub(super) fn finish_returns(
        &mut self,
        mut exits: Vec<Exit>,
        fallthrough: Option<(Checked, &Env)>,
        flow: &Flow,
        span: Span,
    ) -> Result<(Type, Id), Diagnostic> {
        if let Some((value, env)) = fallthrough {
            let output = if flow.types.iter().any(|t| matches!(t, Type::Borrow(_))) {
                self.borrowed_outputs(&flow.parameters, &flow.types, value.clone(), env, span)?
            } else {
                value.id
            };
            exits.push(Exit {
                guard: flow.guard.ok_or_else(|| {
                    Diagnostic::new(span, "internal invariant: reachable fallthrough")
                })?,
                output,
                ty: value.ty,
                span,
            });
        }
        let last = exits
            .pop()
            .ok_or_else(|| Diagnostic::new(span, "function has no result path"))?;
        let mut result = self
            .node(
                Kind::If(last.guard, last.output, last.output),
                self.nodes[last.output].ty.clone(),
                span,
            )
            .id;
        let mut ty = last.ty;
        for exit in exits.into_iter().rev() {
            if (ty == Type::Unit) != (exit.ty == Type::Unit) {
                return Err(Diagnostic::new(
                    exit.span,
                    "all function exits must return values, or all must return without a value",
                ));
            }
            ty = ty.join(&exit.ty);
            let output_ty = self.nodes[exit.output].ty.join(&self.nodes[result].ty);
            result = self
                .node(Kind::If(exit.guard, exit.output, result), output_ty, span)
                .id;
        }
        Ok((ty, result))
    }
}

// Used only for the implicit no-result signature of recursive borrowed
// functions. Nested function declarations have their own return boundary.
pub(super) fn returns_value(expr: &syntax::Expr) -> bool {
    use syntax::ExprKind as E;
    match &expr.kind {
        E::Block(b) => block_returns_value(b),
        E::If(c, a, b) => {
            returns_value(c)
                || block_returns_value(a)
                || b.as_ref().is_some_and(block_returns_value)
        }
        E::For(_, _, source, body) => returns_value(source) || block_returns_value(body),
        E::Match(v, arms) => returns_value(v) || arms.iter().any(|a| returns_value(&a.body)),
        E::Borrow(v) | E::Field(v, _) | E::Unary(_, v) => returns_value(v),
        E::Binary(_, a, b) | E::Index(a, b) => returns_value(a) || returns_value(b),
        E::Slice(v, a, b) => {
            returns_value(v)
                || a.as_ref().is_some_and(|v| returns_value(v))
                || b.as_ref().is_some_and(|v| returns_value(v))
        }
        E::Call(f, _, args) | E::MemberCall(f, _, _, args) => {
            returns_value(f) || args.iter().any(returns_value)
        }
        E::Array(items) => items.iter().any(|i| returns_value(&i.value)),
        E::Record(entries) => entries.iter().any(|e| match e {
            syntax::Entry::Field(_, _, v) | syntax::Entry::Spread(_, v) => returns_value(v),
        }),
        _ => false,
    }
}
fn block_returns_value(block: &syntax::Block) -> bool {
    block.result.as_ref().is_some_and(|v| returns_value(v))
        || block.statements.iter().any(|s| match s {
            syntax::Statement::Return(value, _) => value.is_some(),
            syntax::Statement::Let(_, _, v, _) | syntax::Statement::Discard(v) => returns_value(v),
            syntax::Statement::Assign(p, _, v, _) => returns_value(p) || returns_value(v),
        })
}
