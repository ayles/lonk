use super::*;

impl Checker {
    pub(super) fn for_loop(
        &mut self,
        name: &str,
        name_span: Span,
        source: &syntax::Expr,
        body: &syntax::Block,
        env: &mut Env,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let source = self.expr(source, env, None)?;
        let Type::Array(element) = &source.ty else {
            return Err(Diagnostic::new(
                span,
                "for requires a checked array type; object iteration is not implemented yet",
            ));
        };
        if element.contains_unresolved() {
            return Err(Diagnostic::new(
                span,
                "an empty iteration source needs an element type annotation",
            ));
        }
        let outside = self.owner;
        let entry = env.clone();
        let checkpoint = self.clone();
        let mut invariant: BTreeMap<_, _> = entry
            .iter()
            .map(|(n, b)| (n.clone(), b.known.clone()))
            .collect();
        // Infer facts valid both before the first iteration and at every
        // back-edge. Rechecking uses a checkpoint, so trial graphs and liveness
        // roots never leak into the compiled program.
        for attempt in 0..8 {
            if attempt != 0 {
                *self = checkpoint.clone();
            }
            let owner = self.next_owner;
            self.next_owner += 1;
            self.owner = owner;
            let outer_flow = self.flow.clone();
            if self.flow.is_some() {
                let guard = self.node(Kind::Bool(true), Type::Bool, span).id;
                let flow = self
                    .flow
                    .as_mut()
                    .ok_or_else(|| Diagnostic::new(span, "internal invariant: function flow"))?;
                flow.guard = Some(guard);
                flow.exits.clear();
            }
            let mut scope = entry.clone();
            let mut parameters = BTreeMap::new();
            for (n, binding) in &mut scope {
                if binding.owner == outside {
                    let node = self.node(
                        Kind::Parameter(parameters.len() + 1, None),
                        invariant[n].clone(),
                        span,
                    );
                    parameters.insert(n.clone(), node.id);
                    binding.value = node.id;
                    binding.known = invariant[n].clone();
                    binding.owner = owner;
                }
            }
            let item = self.node(Kind::Parameter(0, None), *element.clone(), name_span);
            scope.insert(
                name.to_owned(),
                Binding {
                    identity: item.id,
                    value: item.id,
                    contract: *element.clone(),
                    known: *element.clone(),
                    owner,
                },
            );
            let result = self.block(body, &mut scope, None)?;
            if self.reachable() && result.ty != Type::Unit {
                return Err(Diagnostic::new(
                    body.span,
                    "unused loop body result; assign it to a surrounding binding",
                ));
            }
            let mut changed = BTreeMap::new();
            let mut next = invariant.clone();
            for (n, parameter) in &parameters {
                let binding = &scope[n];
                if self.reachable()
                    && binding.identity == entry[n].identity
                    && binding.value != *parameter
                {
                    let joined = entry[n]
                        .known
                        .join_preserving(&binding.known, &entry[n].known);
                    if !joined.guarantees(&entry[n].known) {
                        return Err(Diagnostic::new(
                            span,
                            format!("loop would lose an established guarantee for `{n}`"),
                        ));
                    }
                    next.insert(n.clone(), joined);
                    changed.insert(n.clone(), binding.value);
                }
            }
            let exits = self
                .flow
                .as_ref()
                .map_or_else(Vec::new, |f| f.exits.clone());
            if changed.is_empty() && exits.is_empty() {
                return Err(Diagnostic::new(
                    span,
                    "loop does not update a surrounding binding",
                ));
            }
            if next != invariant {
                invariant = next;
                continue;
            }
            let mut fields: Vec<_> = changed
                .iter()
                .map(|(n, id)| (Some(n.clone()), *id))
                .collect();
            let exit = if exits.is_empty() {
                None
            } else {
                let guards: Vec<_> = exits.iter().map(|e| e.guard).collect();
                let flag = self
                    .join_guards(&guards, span)
                    .ok_or_else(|| Diagnostic::new(span, "internal invariant: return paths"))?;
                let flow = self
                    .flow
                    .clone()
                    .ok_or_else(|| Diagnostic::new(span, "internal invariant: function flow"))?;
                let (ty, output) = self.finish_returns(exits, None, &flow, span)?;
                fields.push((Some("$returned".into()), flag));
                fields.push((Some("$exit".into()), output));
                Some((flag, output, ty))
            };
            let body_type = Type::Record {
                fields: changed
                    .keys()
                    .map(|n| (n.clone(), Field::required(scope[n].known.clone())))
                    .collect::<BTreeMap<_, _>>()
                    .into(),
                open: false,
                row: None,
            };
            let root = self.node(Kind::Record(fields), body_type, body.span);
            self.roots.insert(owner, root.id);
            let mut live = BTreeSet::new();
            let mut pending = vec![root.id];
            while let Some(id) = pending.pop() {
                if live.insert(id) {
                    pending.extend(self.nodes[id].kind.dependencies());
                }
            }
            let mut initial = Vec::new();
            let mut updates = Vec::new();
            // State used to decide or produce a return remains useful even
            // when its final value after an exhausted loop is never selected.
            let back_edges: BTreeMap<_, _> = parameters
                .iter()
                .filter_map(|(n, p)| changed.get(n).map(|v| (*p, *v)))
                .collect();
            let mut return_live = BTreeSet::new();
            let mut pending: Vec<_> = exit
                .as_ref()
                .map_or_else(Vec::new, |(flag, output, _)| vec![*flag, *output]);
            while let Some(id) = pending.pop() {
                if return_live.insert(id) {
                    pending.extend(self.nodes[id].kind.dependencies());
                    pending.extend(back_edges.get(&id).copied());
                }
            }
            let return_state: BTreeSet<_> = parameters
                .iter()
                .filter(|(_, p)| return_live.contains(p))
                .map(|(n, _)| n.clone())
                .collect();
            let mut state_users: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (output_name, output) in &changed {
                let mut dependencies = BTreeSet::new();
                let mut pending = vec![*output];
                while let Some(id) = pending.pop() {
                    if dependencies.insert(id) {
                        pending.extend(self.nodes[id].kind.dependencies());
                        pending.extend(back_edges.get(&id).copied());
                    }
                }
                for (input_name, input) in &parameters {
                    if input_name != output_name && dependencies.contains(input) {
                        state_users
                            .entry(input_name.clone())
                            .or_default()
                            .push(output_name.clone());
                    }
                }
            }
            for (n, parameter) in parameters {
                // An overwritten input is still needed for the zero-iteration
                // case. Unrelated locals must not become useful by accident.
                if live.contains(&parameter) || changed.contains_key(&n) {
                    let index = initial.len();
                    initial.push(entry[&n].value);
                    self.nodes[parameter].kind = Kind::Parameter(index + 1, None);
                    if let Some(output) = changed.get(&n) {
                        updates.push((n, index, *output));
                    }
                }
            }
            self.owner = outside;
            self.flow = outer_flow;
            let ty = Type::Record {
                fields: changed
                    .keys()
                    .map(|n| (n.clone(), Field::required(invariant[n].clone())))
                    .collect::<BTreeMap<_, _>>()
                    .into(),
                open: false,
                row: None,
            };
            let iteration = self.node(
                Kind::For {
                    source: source.id,
                    owner,
                    initial,
                    body: root.id,
                    updates,
                    exit: exit.as_ref().map(|(flag, output, _)| (*flag, *output)),
                },
                ty,
                span,
            );
            if let Some((_, output, ty)) = exit {
                let flag = self
                    .node(
                        Kind::Field(iteration.id, "$returned".into()),
                        Type::Bool,
                        span,
                    )
                    .id;
                let value = self
                    .node(
                        Kind::Field(iteration.id, "$exit".into()),
                        self.nodes[output].ty.clone(),
                        span,
                    )
                    .id;
                let parent = self.guard();
                let returned = self.branch_guard(parent, flag, true, span).ok_or_else(|| {
                    Diagnostic::new(span, "internal invariant: returning function")
                })?;
                let continued = self.branch_guard(parent, flag, false, span);
                let flow = self
                    .flow
                    .as_mut()
                    .ok_or_else(|| Diagnostic::new(span, "internal invariant: function flow"))?;
                flow.exits.push(returns::Exit {
                    guard: returned,
                    output: value,
                    ty,
                    span,
                });
                flow.guard = continued;
            }
            let mut output_ids = BTreeMap::new();
            for n in changed.keys() {
                let value = self.node(
                    Kind::Field(iteration.id, n.clone()),
                    invariant[n].clone(),
                    span,
                );
                if return_state.contains(n) {
                    self.implicit_outputs.insert(value.id, vec![iteration.id]);
                }
                output_ids.insert(n.clone(), value.id);
                self.definitions.push(Definition {
                    id: value.id,
                    name: n.clone(),
                    span,
                    owner: outside,
                });
                if let Some(binding) = env.get_mut(n) {
                    binding.value = value.id;
                    binding.known = invariant[n].clone();
                }
            }
            for (name, users) in state_users {
                if let Some(id) = output_ids.get(&name) {
                    self.implicit_outputs
                        .entry(*id)
                        .or_default()
                        .extend(users.iter().filter_map(|n| output_ids.get(n)).copied());
                }
            }
            return Ok(self.node(Kind::Unit, Type::Unit, span));
        }
        Err(Diagnostic::new(
            span,
            "loop type inference did not converge; use a stable annotated state type",
        ))
    }
}
