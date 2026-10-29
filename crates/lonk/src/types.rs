use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};
mod equality;
mod identity;
pub(crate) use identity::TypeIds;
mod variants;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum LiteralType {
    Int(i64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GenericParameter {
    pub id: usize,
    pub name: String,
    /// None is a value type; Some records the names excluded from a captured row.
    pub row: Option<BTreeSet<String>>,
    /// Compiler-inferred receiver remainder, omitted from explicit type arguments.
    pub implicit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Field {
    pub ty: Type,
    pub optional: bool,
}

impl Field {
    pub(crate) fn required(ty: Type) -> Self {
        Self {
            ty,
            optional: false,
        }
    }
}

#[derive(Clone, Debug, Eq, serde::Serialize, serde::Deserialize)]
pub enum Type {
    Literal(LiteralType),
    Union(#[serde(deserialize_with = "crate::codec::nested")] Vec<Type>),
    Int,
    Bool,
    Str,
    Any,
    Borrow(#[serde(deserialize_with = "crate::codec::nested")] Box<Type>),
    /// Availability of a trailing value argument, only inside a callable signature.
    Default(#[serde(deserialize_with = "crate::codec::nested")] Box<Type>),
    Parameter {
        id: usize,
        name: String,
    },
    Generic {
        parameters: Vec<GenericParameter>,
        #[serde(deserialize_with = "crate::codec::nested")]
        body: Box<Type>,
    },
    Array(#[serde(deserialize_with = "crate::codec::nested")] Box<Type>),
    Record {
        #[serde(deserialize_with = "crate::codec::nested")]
        fields: Arc<BTreeMap<String, Field>>,
        open: bool,
        row: Option<(usize, String)>,
    },
    Function {
        receiver: bool,
        #[serde(deserialize_with = "crate::codec::nested")]
        parameters: Vec<Type>,
        #[serde(deserialize_with = "crate::codec::nested")]
        result: Box<Type>,
    },
    Descriptor(#[serde(deserialize_with = "crate::codec::nested")] Box<Type>),
    /// An internal result for statement blocks, never an ordinary source value.
    #[doc(hidden)]
    Unit,
    /// Only used while contextualizing an empty array.
    #[doc(hidden)]
    Never,
}

impl Type {
    pub(crate) fn within_depth(&self, limit: usize) -> bool {
        let mut pending = vec![(self, 0)];
        while let Some((ty, depth)) = pending.pop() {
            if depth > limit {
                return false;
            }
            match ty {
                Self::Union(cases) => pending.extend(cases.iter().map(|t| (t, depth + 1))),
                Self::Array(t)
                | Self::Borrow(t)
                | Self::Default(t)
                | Self::Descriptor(t)
                | Self::Generic { body: t, .. } => pending.push((t, depth + 1)),
                Self::Function {
                    parameters, result, ..
                } => {
                    pending.push((result, depth + 1));
                    pending.extend(parameters.iter().map(|t| (t, depth + 1)));
                }
                Self::Record { fields, .. } => {
                    pending.extend(fields.values().map(|f| (&f.ty, depth + 1)))
                }
                _ => (),
            }
        }
        true
    }

    pub(crate) fn storable(&self) -> bool {
        match self {
            Self::Union(cases) => cases.iter().all(Self::storable),
            Self::Borrow(_) | Self::Default(_) | Self::Unit => false,
            Self::Array(t) | Self::Descriptor(t) => t.storable(),
            Self::Generic { body, .. } => body.storable(),
            Self::Record { fields, .. } => fields.values().all(|f| f.ty.storable()),
            Self::Function {
                parameters,
                result,
                receiver,
            } => {
                let first_default = Self::required_arguments(parameters);
                (!receiver
                    || (!parameters.is_empty() && !matches!(parameters[0], Self::Default(_))))
                    && parameters
                        .iter()
                        .skip(first_default)
                        .all(|p| matches!(p, Self::Default(_)))
                    && parameters.iter().all(|p| match p {
                        Self::Borrow(t) | Self::Default(t) => t.storable(),
                        t => t.storable(),
                    })
                    && (result.storable()
                        || (**result == Self::Unit
                            && parameters.iter().any(|p| matches!(p, Self::Borrow(_)))))
            }
            _ => true,
        }
    }
    pub(crate) fn substitute(&self, arguments: &BTreeMap<usize, Type>) -> Self {
        match self {
            Self::Union(cases) => {
                Self::Union(cases.iter().map(|t| t.substitute(arguments)).collect())
            }
            Self::Parameter { id, .. } => {
                arguments.get(id).cloned().unwrap_or_else(|| self.clone())
            }
            Self::Array(t) => Self::Array(Box::new(t.substitute(arguments))),
            Self::Borrow(t) => Self::Borrow(Box::new(t.substitute(arguments))),
            Self::Default(t) => Self::Default(Box::new(t.substitute(arguments))),
            Self::Descriptor(t) => Self::Descriptor(Box::new(t.substitute(arguments))),
            Self::Function {
                parameters,
                result,
                receiver,
            } => Self::Function {
                receiver: *receiver,
                parameters: parameters.iter().map(|t| t.substitute(arguments)).collect(),
                result: Box::new(result.substitute(arguments)),
            },
            Self::Generic { parameters, body } => {
                let mut free_arguments = arguments.clone();
                for parameter in parameters {
                    free_arguments.remove(&parameter.id);
                }
                Self::Generic {
                    parameters: parameters.clone(),
                    body: Box::new(body.substitute(&free_arguments)),
                }
            }
            Self::Record { fields, open, row } => {
                let mut fields: BTreeMap<_, _> = fields
                    .iter()
                    .map(|(name, field)| {
                        (
                            name.clone(),
                            Field {
                                ty: field.ty.substitute(arguments),
                                optional: field.optional,
                            },
                        )
                    })
                    .collect();
                let mut open = *open;
                let mut row = row.clone();
                if let Some(Type::Record {
                    fields: tail,
                    open: tail_open,
                    row: tail_row,
                }) = row.as_ref().and_then(|(id, _)| arguments.get(id))
                {
                    fields.extend(tail.iter().map(|(n, f)| (n.clone(), f.clone())));
                    open |= tail_open;
                    row = tail_row.clone();
                }
                Self::Record {
                    fields: fields.into(),
                    open,
                    row,
                }
            }
            _ => self.clone(),
        }
    }

    pub(crate) fn mentions(&self, ids: &BTreeSet<usize>) -> bool {
        match self {
            Self::Union(cases) => cases.iter().any(|t| t.mentions(ids)),
            Self::Parameter { id, .. } => ids.contains(id),
            Self::Array(t) | Self::Descriptor(t) | Self::Borrow(t) | Self::Default(t) => {
                t.mentions(ids)
            }
            Self::Generic { body, .. } => body.mentions(ids),
            Self::Function {
                parameters, result, ..
            } => parameters.iter().any(|t| t.mentions(ids)) || result.mentions(ids),
            Self::Record { fields, row, .. } => {
                row.as_ref().is_some_and(|(id, _)| ids.contains(id))
                    || fields.values().any(|f| f.ty.mentions(ids))
            }
            _ => false,
        }
    }
    /// A test can use trusted static evidence without demanding payloads. This
    /// is a proof about the original value, never a projecting copy conversion.
    pub(crate) fn guarantees(&self, target: &Self) -> bool {
        if self == target || *target == Self::Any {
            return true;
        }
        match (self, target) {
            (Self::Union(cases), target) => cases.iter().all(|t| t.guarantees(target)),
            (source, Self::Union(cases)) => source
                .variant_case(cases)
                .is_ok_and(|case| source.guarantees(case)),
            (Self::Literal(value), target) if value.primitive() == *target => true,
            (
                Self::Function {
                    parameters: a,
                    result: ar,
                    receiver: am,
                },
                Self::Function {
                    parameters: b,
                    result: br,
                    receiver: bm,
                },
            ) => {
                am == bm
                    && ar == br
                    && a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        a.argument_type() == b.argument_type()
                            && (!matches!(b, Self::Default(_)) || matches!(a, Self::Default(_)))
                    })
            }
            (Self::Array(a), Self::Array(b)) => a.guarantees(b),
            (
                Self::Record {
                    fields: from,
                    open: from_open,
                    row: from_row,
                },
                Self::Record {
                    fields: to,
                    open: to_open,
                    row: to_row,
                },
            ) => {
                (to_row.is_none() || to_row == from_row)
                    && (*to_open
                        || (from_row == to_row
                            && !from_open
                            && from.keys().all(|name| to.contains_key(name))))
                    && to.iter().all(|(name, expected)| match from.get(name) {
                        Some(actual) => {
                            (!actual.optional || expected.optional)
                                && actual.ty.guarantees(&expected.ty)
                        }
                        None => expected.optional && !from_open,
                    })
            }
            _ => false,
        }
    }

    pub(crate) fn argument_type(&self) -> &Self {
        if let Self::Default(t) = self { t } else { self }
    }
    pub(crate) fn required_arguments(parameters: &[Self]) -> usize {
        parameters
            .iter()
            .position(|p| matches!(p, Self::Default(_)))
            .unwrap_or(parameters.len())
    }

    pub(crate) fn select(
        &self,
        path: &[&str],
        span: crate::Span,
    ) -> Result<&Self, crate::Diagnostic> {
        let mut current = self;
        for part in path {
            current = match current {
                Self::Record { fields, .. } => match fields.get(*part) {
                    Some(field) if !field.optional => &field.ty,
                    Some(_) => {
                        return Err(crate::Diagnostic::new(
                            span,
                            format!(
                                "selected field `{part}` is optional; prove presence in the source first"
                            ),
                        ));
                    }
                    None => {
                        return Err(crate::Diagnostic::new(
                            span,
                            format!("selected field `{part}` is not statically known"),
                        ));
                    }
                },
                Self::Array(element) if part.parse::<usize>().is_ok() => element,
                _ => {
                    return Err(crate::Diagnostic::new(
                        span,
                        format!("cannot select `{part}` from {current}"),
                    ));
                }
            };
        }
        Ok(current)
    }

    pub(crate) fn contains_unresolved(&self) -> bool {
        match self {
            Self::Union(cases) => cases.iter().any(Self::contains_unresolved),
            Self::Never => true,
            Self::Array(t) => t.contains_unresolved(),
            Self::Record { fields, .. } => fields.values().any(|f| f.ty.contains_unresolved()),
            _ => false,
        }
    }

    pub(crate) fn join(&self, other: &Self) -> Self {
        if self == other {
            return self.clone();
        }
        if let (Some(a), Some(b)) = (self.primitive(), other.primitive()) {
            if a == b {
                return a;
            }
        }
        match (self, other) {
            (Self::Never, t) | (t, Self::Never) => t.clone(),
            (Self::Array(a), Self::Array(b)) => Self::Array(Box::new(a.join(b))),
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
                let mut fields = BTreeMap::new();
                for name in a.keys().chain(b.keys()) {
                    let field = match (a.get(name), b.get(name)) {
                        (Some(x), Some(y)) => Field {
                            ty: x.ty.join(&y.ty),
                            optional: x.optional || y.optional,
                        },
                        (Some(x), None) => Field {
                            ty: if *bo { Self::Any } else { x.ty.clone() },
                            optional: true,
                        },
                        (None, Some(y)) => Field {
                            ty: if *ao { Self::Any } else { y.ty.clone() },
                            optional: true,
                        },
                        _ => continue,
                    };
                    fields.insert(name.clone(), field);
                }
                Self::Record {
                    fields: fields.into(),
                    open: *ao || *bo || ar != br,
                    row: if ar == br { ar.clone() } else { None },
                }
            }
            _ => Self::Any,
        }
    }

    /// Facts established by a successful guard add to existing guarantees.
    /// Contradictory guards describe an unreachable branch; retaining the new
    /// fact there is sound without introducing a bottom type into source types.
    pub(crate) fn refine(&self, evidence: &Self) -> Self {
        match (self, evidence) {
            (Self::Union(cases), evidence) => {
                let compatible: Vec<_> = cases
                    .iter()
                    .filter(|case| case.can_overlap(evidence))
                    .map(|case| case.refine(evidence))
                    .collect();
                match compatible.as_slice() {
                    [] => evidence.clone(),
                    [only] => only.clone(),
                    _ => Self::Union(compatible),
                }
            }
            (known, Self::Union(cases)) => {
                let compatible: Vec<_> = cases
                    .iter()
                    .filter(|case| known.can_overlap(case))
                    .map(|case| known.refine(case))
                    .collect();
                match compatible.as_slice() {
                    [] => evidence.clone(),
                    [only] => only.clone(),
                    _ => Self::Union(compatible),
                }
            }
            (Self::Literal(value), evidence) if *evidence == value.primitive() => self.clone(),
            (known, Self::Any) => known.clone(),
            (Self::Array(a), Self::Array(b)) => Self::Array(Box::new(a.refine(b))),
            (
                Self::Record {
                    fields: old,
                    open: old_open,
                    row: old_row,
                },
                Self::Record {
                    fields: new,
                    open: new_open,
                    row: new_row,
                },
            ) => {
                let mut fields = old.clone();
                if !new_open && new_row.is_none() {
                    Arc::make_mut(&mut fields)
                        .retain(|name, field| !field.optional || new.contains_key(name));
                }
                for (name, field) in new.iter() {
                    let refined = match old.get(name) {
                        Some(previous) => Field {
                            ty: previous.ty.refine(&field.ty),
                            optional: previous.optional && field.optional,
                        },
                        None => field.clone(),
                    };
                    Arc::make_mut(&mut fields).insert(name.clone(), refined);
                }
                Self::Record {
                    fields,
                    open: *old_open && *new_open,
                    row: old_row.clone().or_else(|| new_row.clone()),
                }
            }
            (_, evidence) => evidence.clone(),
        }
    }

    /// A merge may forget branch-local facts but must retain facts established
    /// before the branch, including exclusivity of variant alternatives.
    pub(crate) fn join_preserving(&self, other: &Self, prior: &Self) -> Self {
        let mut joined = self.join(other);
        match (&mut joined, self, other, prior) {
            (
                Self::Record { fields, .. },
                Self::Record { fields: a, .. },
                Self::Record { fields: b, .. },
                Self::Record { fields: p, .. },
            ) => {
                for (name, field) in Arc::make_mut(fields) {
                    if let (Some(a), Some(b), Some(p)) = (a.get(name), b.get(name), p.get(name)) {
                        field.ty = a.ty.join_preserving(&b.ty, &p.ty);
                    }
                }
            }
            (Self::Array(t), Self::Array(a), Self::Array(b), Self::Array(p)) => {
                **t = a.join_preserving(b, p)
            }
            _ => (),
        }
        if !joined.guarantees(prior) && self.guarantees(prior) && other.guarantees(prior) {
            prior.clone()
        } else {
            joined
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(value) => match value {
                LiteralType::Int(n) => write!(f, "{n}"),
                LiteralType::Bool(b) => write!(f, "{b}"),
                LiteralType::Str(s) => f.write_str(&crate::format::quoted(s)),
            },
            Self::Union(cases) => {
                f.write_str("(")?;
                for (i, case) in cases.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{case}")?;
                }
                f.write_str(")")
            }
            Self::Int => f.write_str("int"),
            Self::Bool => f.write_str("bool"),
            Self::Str => f.write_str("str"),
            Self::Any => f.write_str("any"),
            Self::Borrow(t) => write!(f, "&{t}"),
            Self::Default(t) => write!(f, "{t} = ..."),
            Self::Parameter { name, .. } => f.write_str(name),
            Self::Generic { parameters, body } => {
                let callable = matches!(body.as_ref(), Self::Function { .. });
                if callable {
                    f.write_str("fn")?;
                }
                f.write_str("<")?;
                for (i, p) in parameters.iter().enumerate() {
                    if i != 0 {
                        f.write_str(", ")?;
                    }
                    f.write_str(&p.name)?;
                }
                if callable {
                    write!(
                        f,
                        ">{}",
                        body.to_string().strip_prefix("fn").unwrap_or_default()
                    )
                } else {
                    write!(f, "> {body}")
                }
            }
            Self::Unit => f.write_str("<no result>"),
            Self::Never => f.write_str("<unresolved>"),
            Self::Array(t) => write!(f, "[{t}]"),
            Self::Descriptor(t) => write!(f, "type {t}"),
            Self::Record { fields, open, row } => {
                f.write_str("{")?;
                for (i, (name, field)) in fields.iter().enumerate() {
                    if i != 0 {
                        f.write_str("; ")?;
                    }
                    write!(
                        f,
                        "{name}{}: {}",
                        if field.optional { "?" } else { "" },
                        field.ty
                    )?;
                }
                if *open || row.is_some() {
                    if !fields.is_empty() {
                        f.write_str("; ")?;
                    }
                    f.write_str("...")?;
                    if let Some((_, name)) = row {
                        f.write_str(name)?;
                    }
                }
                f.write_str("}")
            }
            Self::Function {
                parameters,
                result,
                receiver,
            } => {
                f.write_str("fn(")?;
                for (i, p) in parameters.iter().enumerate() {
                    if i != 0 {
                        f.write_str(", ")?;
                    }
                    if i == 0 && *receiver {
                        f.write_str("self: ")?;
                    }
                    write!(f, "{p}")?;
                }
                if **result == Self::Unit {
                    f.write_str(")")
                } else {
                    write!(f, ") -> {result}")
                }
            }
        }
    }
}
