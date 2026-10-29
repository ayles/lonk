use super::*;

impl LiteralType {
    pub(crate) fn primitive(&self) -> Type {
        match self {
            Self::Int(_) => Type::Int,
            Self::Bool(_) => Type::Bool,
            Self::Str(_) => Type::Str,
        }
    }
}
impl From<&crate::syntax::Literal> for LiteralType {
    fn from(value: &crate::syntax::Literal) -> Self {
        match value {
            crate::syntax::Literal::Int(n) => Self::Int(*n),
            crate::syntax::Literal::Bool(b) => Self::Bool(*b),
            crate::syntax::Literal::Str(s) => Self::Str(s.clone()),
        }
    }
}

impl Type {
    pub(crate) fn primitive(&self) -> Option<Self> {
        match self {
            Self::Int | Self::Bool | Self::Str => Some(self.clone()),
            Self::Literal(v) => Some(v.primitive()),
            Self::Union(cases) => {
                let first = cases.first()?.primitive()?;
                cases
                    .iter()
                    .all(|case| case.primitive() == Some(first.clone()))
                    .then_some(first)
            }
            _ => None,
        }
    }

    pub(crate) fn union(cases: Vec<Self>) -> Result<Self, String> {
        let mut normalized = Vec::new();
        for case in cases {
            let cases = if let Self::Union(cases) = case {
                cases
            } else {
                vec![case]
            };
            for case in cases {
                if !normalized.contains(&case) {
                    normalized.push(case);
                }
            }
        }
        let mut tags = BTreeSet::new();
        for case in &normalized {
            match case {
                Self::Literal(_) => (),
                Self::Record { fields, open: false, row: None }
                    if fields.len() == 1 && fields.values().all(|f| !f.optional) => {
                    for name in fields.keys() {
                        if !tags.insert(name.clone()) {
                            return Err(format!("variant alternatives share the tag `{name}`"));
                        }
                    }
                }
                _ => return Err(
                    "variant alternatives must be literals or closed records with one required field".into()
                ),
            }
        }
        normalized.sort_by_key(|case| case.to_string());
        if normalized.len() == 1 {
            Ok(normalized.remove(0))
        } else {
            Ok(Self::Union(normalized))
        }
    }

    pub(crate) fn variant_case<'a>(&self, cases: &'a [Self]) -> Result<&'a Self, String> {
        match self {
            Self::Literal(value) => cases
                .iter()
                .find(|case| matches!(case,Self::Literal(want) if want==value))
                .ok_or_else(|| "literal is not a case of this variant".into()),
            Self::Record { fields, open, row } => {
                let mut selected = None;
                for case in cases {
                    if let Self::Record { fields: tag, .. } = case {
                        for name in tag.keys() {
                            match fields.get(name) {
                                Some(field) if field.optional => return Err(
                                    "variant tag presence is uncertain; check it before conversion".into()
                                ),
                                Some(_) => {
                                    if selected.is_some() {
                                        return Err("ambiguous variant value contains multiple case tags".into());
                                    }
                                    selected = Some(case);
                                }
                                None if *open || row.is_some() => return Err(
                                    "open value may contain another variant tag; use a checked exact shape or an explicit earlier projection".into()
                                ),
                                None => (),
                            }
                        }
                    }
                }
                selected.ok_or_else(|| "value has no required variant tag".into())
            }
            _ => Err("variant conversion needs checked case evidence".into()),
        }
    }

    pub(crate) fn can_overlap(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Union(cases), other) | (other, Self::Union(cases)) => {
                cases.iter().any(|case| case.can_overlap(other))
            }
            (Self::Any, _)
            | (_, Self::Any)
            | (Self::Parameter { .. }, _)
            | (_, Self::Parameter { .. }) => true,
            (Self::Literal(a), Self::Literal(b)) => a == b,
            (
                Self::Record {
                    fields: a,
                    open: ao,
                    row: ar,
                },
                Self::Record {
                    fields: b,
                    open: bo,
                    row: br,
                },
            ) => {
                let compatible = |from: &BTreeMap<String, Field>,
                                  open: bool,
                                  to: &BTreeMap<String, Field>| {
                    to.iter().all(|(name, field)| match from.get(name) {
                        Some(actual) => {
                            (actual.optional && field.optional) || actual.ty.can_overlap(&field.ty)
                        }
                        None => field.optional || open,
                    })
                };
                compatible(a, *ao || ar.is_some(), b) && compatible(b, *bo || br.is_some(), a)
            }
            // Empty arrays satisfy either element constraint.
            (Self::Array(_), Self::Array(_)) => true,
            _ => {
                self == other
                    || self
                        .primitive()
                        .is_some_and(|p| other.primitive() == Some(p))
            }
        }
    }
}
