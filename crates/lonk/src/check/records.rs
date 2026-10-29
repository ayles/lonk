use super::*;

impl Checker {
    pub(super) fn record(
        &mut self,
        entries: &[syntax::Entry],
        env: &mut Env,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        let mut fields = BTreeMap::new();
        let mut explicit = BTreeSet::new();
        let mut open = false;
        let mut row = None;
        let mut values = Vec::new();
        let mut methods = Vec::new();
        for entry in entries {
            match entry {
                syntax::Entry::Field(name, _, value) => {
                    if let Some((id, _)) = &row {
                        if !self.row_exclusions(*id, span)?.contains(name) {
                            return Err(Diagnostic::new(
                                value.span,
                                format!(
                                    "field `{name}` may overwrite a captured generic remainder"
                                ),
                            ));
                        }
                    }
                    if !explicit.insert(name) {
                        return Err(Diagnostic::new(
                            value.span,
                            format!("duplicate explicit field `{name}`"),
                        ));
                    }
                    let context = match expected {
                        Some(Type::Record { fields, .. }) => fields.get(name).map(|f| &f.ty),
                        Some(Type::Union(cases)) => cases.iter().find_map(|case| match case {
                            Type::Record { fields, .. } => fields.get(name).map(|f| &f.ty),
                            _ => None,
                        }),
                        _ => None,
                    };
                    if matches!(&value.kind,syntax::ExprKind::Function(_,ps,_,_) if ps.first().is_some_and(|p|p.name=="self"))
                    {
                        methods.push((
                            values.len(),
                            name.clone(),
                            value,
                            env.clone(),
                            context.cloned(),
                        ));
                        fields.insert(name.clone(), Field::required(Type::Never));
                        values.push((Some(name.clone()), usize::MAX));
                        continue;
                    }
                    let value = self.expr(value, env, context)?;
                    if value.ty == Type::Unit {
                        return Err(Diagnostic::new(span, "a record field must have a value"));
                    }
                    fields.insert(name.clone(), Field::required(value.ty));
                    values.push((Some(name.clone()), value.id));
                }
                syntax::Entry::Spread(_, value) => {
                    let value = self.expr(value, env, None)?;
                    let Type::Record {
                        fields: additional,
                        open: more_open,
                        row: more_row,
                    } = value.ty
                    else {
                        return Err(Diagnostic::new(span, "record spread requires an object"));
                    };
                    if let Some((id, _)) = &more_row {
                        if row.is_some() && row != more_row {
                            return Err(Diagnostic::new(
                                span,
                                "cannot combine unrelated captured rows",
                            ));
                        }
                        let excluded = self.row_exclusions(*id, span)?;
                        if open
                            || fields.keys().any(|name| {
                                !additional.contains_key(name) && !excluded.contains(name)
                            })
                        {
                            return Err(Diagnostic::new(
                                span,
                                "spread may overlap a captured generic remainder",
                            ));
                        }
                        row = more_row;
                    } else if let Some((id, _)) = &row {
                        let excluded = self.row_exclusions(*id, span)?;
                        if more_open || additional.keys().any(|name| !excluded.contains(name)) {
                            return Err(Diagnostic::new(
                                span,
                                "spread may overwrite a captured generic remainder",
                            ));
                        }
                    }
                    // An unknown spread tail could replace any preceding payload.
                    if more_open {
                        for field in fields.values_mut() {
                            field.ty = Type::Any;
                        }
                    }
                    for (name, field) in std::sync::Arc::unwrap_or_clone(additional) {
                        if field.optional {
                            if let Some(previous) = fields.get(&name) {
                                fields.insert(
                                    name,
                                    Field {
                                        ty: previous.ty.join(&field.ty),
                                        optional: previous.optional,
                                    },
                                );
                                continue;
                            }
                        }
                        fields.insert(name, field);
                    }
                    open |= more_open;
                    values.push((None, value.id));
                }
            }
        }
        if !methods.is_empty() {
            let receiver = Type::Record {
                fields: fields
                    .iter()
                    .filter(|(name, field)| {
                        !Self::is_method(&field.ty)
                            && !methods.iter().any(|(_, method, _, _, _)| method == *name)
                    })
                    .map(|(n, f)| (n.clone(), f.clone()))
                    .collect::<BTreeMap<_, _>>()
                    .into(),
                open: false,
                row: None,
            };
            for (index, _, ast, mut captured, context) in methods {
                self.receiver_context = Some(receiver.clone());
                let method = self.expr(ast, &mut captured, context.as_ref())?;
                values[index].1 = method.id;
            }
            // Rebuild the final shape in source order: a later optional or open
            // spread can still override an earlier method field.
            fields.clear();
            for (name, id) in &values {
                if let Some(name) = name {
                    fields.insert(name.clone(), Field::required(self.nodes[*id].ty.clone()));
                } else if let Type::Record {
                    fields: additional,
                    open,
                    ..
                } = &self.nodes[*id].ty
                {
                    if *open {
                        for field in fields.values_mut() {
                            field.ty = Type::Any;
                        }
                    }
                    for (name, field) in additional.iter() {
                        let field =
                            if let Some(previous) = fields.get(name).filter(|_| field.optional) {
                                Field {
                                    ty: previous.ty.join(&field.ty),
                                    optional: previous.optional,
                                }
                            } else {
                                field.clone()
                            };
                        fields.insert(name.clone(), field);
                    }
                }
            }
        }
        Ok(self.node(
            Kind::Record(values),
            Type::Record {
                fields: fields.into(),
                open,
                row,
            },
            span,
        ))
    }
}
