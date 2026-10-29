//! Rename arena-local generic IDs when importing interfaces or linking units.
use super::*;

#[derive(Default)]
/// The original types must remain alive for this walk: record pointer keys are
/// valid only while their source Arcs cannot be freed and their addresses reused.
pub(crate) struct TypeIds {
    pub ids: BTreeMap<usize, usize>,
    fields: BTreeMap<usize, Arc<BTreeMap<String, Field>>>,
}

impl TypeIds {
    pub(crate) fn id(&mut self, old: usize, next: &mut usize) -> usize {
        *self.ids.entry(old).or_insert_with(|| {
            let id = *next;
            *next += 1;
            id
        })
    }

    pub(crate) fn ty(&mut self, ty: &Type, next: &mut usize) -> Type {
        match ty {
            Type::Parameter { id, name } => Type::Parameter {
                id: self.id(*id, next),
                name: name.clone(),
            },
            Type::Generic { parameters, body } => {
                let parameters = parameters
                    .iter()
                    .map(|p| {
                        let mut p = p.clone();
                        p.id = self.id(p.id, next);
                        p
                    })
                    .collect();
                Type::Generic {
                    parameters,
                    body: Box::new(self.ty(body, next)),
                }
            }
            Type::Array(t) => Type::Array(Box::new(self.ty(t, next))),
            Type::Borrow(t) => Type::Borrow(Box::new(self.ty(t, next))),
            Type::Default(t) => Type::Default(Box::new(self.ty(t, next))),
            Type::Descriptor(t) => Type::Descriptor(Box::new(self.ty(t, next))),
            Type::Union(cases) => Type::Union(cases.iter().map(|t| self.ty(t, next)).collect()),
            Type::Function {
                receiver,
                parameters,
                result,
            } => Type::Function {
                receiver: *receiver,
                parameters: parameters.iter().map(|t| self.ty(t, next)).collect(),
                result: Box::new(self.ty(result, next)),
            },
            Type::Record { fields, open, row } => {
                let pointer = Arc::as_ptr(fields) as usize;
                let mapped = if let Some(mapped) = self.fields.get(&pointer) {
                    mapped.clone()
                } else {
                    let mapped: Arc<BTreeMap<_, _>> = Arc::new(
                        fields
                            .iter()
                            .map(|(name, field)| {
                                (
                                    name.clone(),
                                    Field {
                                        ty: self.ty(&field.ty, next),
                                        optional: field.optional,
                                    },
                                )
                            })
                            .collect(),
                    );
                    self.fields.insert(pointer, mapped.clone());
                    mapped
                };
                Type::Record {
                    fields: mapped,
                    open: *open,
                    row: row
                        .as_ref()
                        .map(|(id, name)| (self.id(*id, next), name.clone())),
                }
            }
            Type::Literal(_)
            | Type::Int
            | Type::Bool
            | Type::Str
            | Type::Any
            | Type::Unit
            | Type::Never => ty.clone(),
        }
    }
}

impl Type {
    pub(crate) fn generic_parameters(&self) -> BTreeMap<usize, GenericParameter> {
        let mut parameters = BTreeMap::new();
        let mut pending = vec![self];
        while let Some(ty) = pending.pop() {
            match ty {
                Self::Generic {
                    parameters: args,
                    body,
                } => {
                    parameters.extend(args.iter().map(|p| (p.id, p.clone())));
                    pending.push(body);
                }
                Self::Borrow(t) | Self::Default(t) | Self::Descriptor(t) | Self::Array(t) => {
                    pending.push(t)
                }
                Self::Union(cases) => pending.extend(cases),
                Self::Record { fields, .. } => pending.extend(fields.values().map(|f| &f.ty)),
                Self::Function {
                    parameters, result, ..
                } => {
                    pending.extend(parameters);
                    pending.push(result);
                }
                _ => (),
            }
        }
        parameters
    }

    pub(crate) fn canonical_ids(&self) -> Type {
        TypeIds::default().ty(self, &mut 0)
    }
}
