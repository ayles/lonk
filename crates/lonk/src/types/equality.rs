use super::*;

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        self.same_with(other, &BTreeMap::new())
    }
}

impl Type {
    pub(crate) fn callable_rebinds(&self, target: &Self) -> bool {
        let (
            Self::Generic {
                parameters: a,
                body,
            },
            Self::Generic {
                parameters: b,
                body: target_body,
            },
        ) = (self, target)
        else {
            return false;
        };
        if a.len() != b.len()
            || !matches!(body.as_ref(), Self::Function { .. })
            || !matches!(target_body.as_ref(), Self::Function { .. })
        {
            return false;
        }
        let mut parameters = a.clone();
        for (from, to) in parameters.iter_mut().zip(b) {
            if from.implicit != to.implicit {
                if from.row.is_none() || to.row.is_none() {
                    return false;
                }
                from.implicit = to.implicit;
            }
        }
        Self::Generic {
            parameters,
            body: body.clone(),
        } == *target
    }
    // Quantifier names and arena IDs are not observable type distinctions.
    // Keep free variables rigid and rename only corresponding bound variables.
    fn same_with(&self, other: &Self, names: &BTreeMap<usize, usize>) -> bool {
        use Type::*;
        match (self, other) {
            (Int, Int) | (Bool, Bool) | (Str, Str) | (Any, Any) | (Unit, Unit) | (Never, Never) => {
                true
            }
            (Literal(a), Literal(b)) => a == b,
            (Parameter { id: a, .. }, Parameter { id: b, .. }) => names.get(a).unwrap_or(a) == b,
            (Array(a), Array(b))
            | (Borrow(a), Borrow(b))
            | (Default(a), Default(b))
            | (Descriptor(a), Descriptor(b)) => a.same_with(b, names),
            (Union(a), Union(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|case| b.iter().any(|other| case.same_with(other, names)))
            }
            (
                Function {
                    receiver: am,
                    parameters: a,
                    result: ar,
                },
                Function {
                    receiver: bm,
                    parameters: b,
                    result: br,
                },
            ) => {
                am == bm
                    && a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| a.same_with(b, names))
                    && ar.same_with(br, names)
            }
            (
                Record {
                    fields: a,
                    open: ao,
                    row: ar,
                },
                Record {
                    fields: b,
                    open: bo,
                    row: br,
                },
            ) => {
                let rows = match (ar, br) {
                    (None, None) => true,
                    (Some((a, _)), Some((b, _))) => names.get(a).unwrap_or(a) == b,
                    _ => false,
                };
                ao == bo
                    && rows
                    && a.len() == b.len()
                    && a.iter().all(|(name, a)| {
                        b.get(name).is_some_and(|b| {
                            a.optional == b.optional && a.ty.same_with(&b.ty, names)
                        })
                    })
            }
            (
                Generic {
                    parameters: a,
                    body: ab,
                },
                Generic {
                    parameters: b,
                    body: bb,
                },
            ) => {
                if a.len() != b.len()
                    || !a
                        .iter()
                        .zip(b)
                        .all(|(a, b)| a.implicit == b.implicit && a.row == b.row)
                {
                    return false;
                }
                let mut names = names.clone();
                names.extend(a.iter().zip(b).map(|(a, b)| (a.id, b.id)));
                ab.same_with(bb, &names)
            }
            _ => false,
        }
    }
}
