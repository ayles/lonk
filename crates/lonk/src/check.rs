use crate::types::{GenericParameter, LiteralType};
use crate::{
    Diagnostic, Field, Span, Type,
    syntax::{self, Binary, Unary},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
mod aliases;
mod borrows;
mod builtins;
mod defaults;
mod env;
use env::Env;
mod generics;
mod graph;
mod units;
pub(crate) use units::{compile_unit, link_units, prelude_graph};
mod loops;
mod methods;
mod modules;
pub(crate) use modules::compile_modules;
mod records;
mod returns;
pub(crate) use builtins::Builtin;

pub(crate) type Id = usize;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Graph {
    pub nodes: Vec<Node>,
    pub root: Id,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Node {
    pub kind: Kind,
    pub ty: Type,
    pub span: Span,
    pub owner: usize,
    pub origin: usize,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Part {
    Field(String),
    Index(Id),
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Drops {
    pub names: BTreeSet<String>,
    #[serde(deserialize_with = "crate::codec::nested")]
    pub fields: BTreeMap<String, Drops>,
    #[serde(deserialize_with = "crate::codec::nested")]
    pub element: Option<Box<Drops>>,
    pub instantiate: Option<Vec<Type>>,
}
impl Drops {
    pub(crate) fn substitute(&self, types: &BTreeMap<usize, Type>) -> Self {
        Self {
            names: self.names.clone(),
            fields: self
                .fields
                .iter()
                .map(|(n, d)| (n.clone(), d.substitute(types)))
                .collect(),
            element: self.element.as_ref().map(|d| Box::new(d.substitute(types))),
            instantiate: self
                .instantiate
                .as_ref()
                .map(|args| args.iter().map(|t| t.substitute(types)).collect()),
        }
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Kind {
    Int(i64),
    Bool(bool),
    Str(String),
    Unit,
    Descriptor,
    /// An interface-only reference in an unlinked compilation unit.
    Import(usize),
    Alias(Id),
    Record(Vec<(Option<String>, Id)>),
    Array(Vec<(bool, Id)>),
    Field(Id, String),
    Index(Id, Id),
    Slice(Id, Option<Id>, Option<Id>),
    Builtin(Builtin, Vec<Id>),
    Unary(Unary, Id),
    Binary(Binary, Id, Id),
    Project(Id, Type, Drops),
    Instantiate(Id, Vec<Type>),
    Update(Id, Vec<Part>, Id),
    If(Id, Id, Id),
    Is(Id, Type),
    LiteralTest(Id, syntax::Literal),
    Function {
        owner: usize,
        body: Id,
        defaults: Vec<Id>,
        parameters: Vec<usize>,
    },
    Parameter(usize, Option<Id>),
    Call(Id, Vec<Id>, Vec<Id>),
    BorrowCheck(Id, Vec<Part>, Option<Id>),
    After(Id, Id),
    For {
        source: Id,
        owner: usize,
        initial: Vec<Id>,
        body: Id,
        updates: Vec<(String, usize, Id)>,
        exit: Option<(Id, Id)>,
    },
}

impl Kind {
    pub(crate) fn dependencies(&self) -> Vec<Id> {
        match self {
            Self::Alias(a)
            | Self::Unary(_, a)
            | Self::Field(a, _)
            | Self::Project(a, _, _)
            | Self::Instantiate(a, _)
            | Self::Is(a, _)
            | Self::LiteralTest(a, _) => vec![*a],
            Self::Binary(_, a, b) | Self::Index(a, b) | Self::After(a, b) => vec![*a, *b],
            Self::If(a, b, c) => vec![*a, *b, *c],
            Self::Record(fields) => fields.iter().map(|(_, id)| *id).collect(),
            Self::Array(elements) => elements.iter().map(|(_, id)| *id).collect(),
            Self::Slice(base, start, end) => {
                std::iter::once(*base).chain(*start).chain(*end).collect()
            }
            Self::Builtin(_, args) => args.clone(),
            Self::Function { body, defaults, .. } => std::iter::once(*body)
                .chain(defaults.iter().copied())
                .collect(),
            Self::Parameter(_, default) => default.iter().copied().collect(),
            Self::For {
                source,
                initial,
                body,
                ..
            } => std::iter::once(*source)
                .chain(initial.iter().copied())
                .chain(std::iter::once(*body))
                .collect(),
            Self::Call(function, args, places) => std::iter::once(*function)
                .chain(args.iter().copied())
                .chain(places.iter().copied())
                .collect(),
            Self::BorrowCheck(base, path, guard) => std::iter::once(*base)
                .chain(guard.iter().copied())
                .chain(path.iter().filter_map(|p| match p {
                    Part::Index(id) => Some(*id),
                    _ => None,
                }))
                .collect(),
            Self::Update(base, path, value) => std::iter::once(*base)
                .chain(std::iter::once(*value))
                .chain(path.iter().filter_map(|p| {
                    if let Part::Index(id) = p {
                        Some(*id)
                    } else {
                        None
                    }
                }))
                .collect(),
            Self::Int(_)
            | Self::Bool(_)
            | Self::Str(_)
            | Self::Unit
            | Self::Descriptor
            | Self::Import(_) => Vec::new(),
        }
    }
}

#[derive(Clone)]
struct Binding {
    identity: Id,
    value: Id,
    contract: Type,
    known: Type,
    owner: usize,
}
#[derive(Clone)]
struct Checked {
    id: Id,
    ty: Type,
    contract: Type,
}
#[derive(Clone)]
struct Loan {
    root: String,
    identity: Id,
    base: Id,
    path: Vec<Part>,
    ty: Type,
    handle: Option<Id>,
    guard: Option<Id>,
}
#[derive(Clone)]
struct ScopedAlias {
    name: String,
    target: Loan,
    access: Loan,
    permits: BTreeSet<Id>,
    guard: Id,
}
struct Argument {
    value: Checked,
    loan: Option<Loan>,
    span: Span,
    fresh: bool,
}
#[derive(Clone)]
struct Definition {
    id: Id,
    name: String,
    span: Span,
    owner: usize,
}
#[derive(Clone)]
struct Checker {
    nodes: Vec<Node>,
    definitions: Vec<Definition>,
    roots: BTreeMap<usize, Id>,
    used_types: BTreeSet<Id>,
    owner: usize,
    next_owner: usize,
    generics: BTreeMap<usize, GenericState>,
    inference: BTreeSet<usize>,
    loans: Vec<Loan>,
    projection_read: usize,
    implicit_outputs: BTreeMap<Id, Vec<Id>>,
    function_name: Option<String>,
    incomplete_recursion: Vec<String>,
    aliases: BTreeMap<Id, ScopedAlias>,
    receiver_context: Option<Type>,
    // Fixed primitive method libraries, independent of user lexical bindings.
    primitive_libraries: Env,
    flow: Option<returns::Flow>,
    scopes: Vec<Env>,
    origin: usize,
}

#[derive(Clone)]
struct GenericState {
    parameter: GenericParameter,
    kind: Option<bool>,
}

impl Checker {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            definitions: Vec::new(),
            roots: BTreeMap::new(),
            used_types: BTreeSet::new(),
            owner: 0,
            next_owner: 1,
            generics: BTreeMap::new(),
            inference: BTreeSet::new(),
            loans: Vec::new(),
            projection_read: 0,
            implicit_outputs: BTreeMap::new(),
            function_name: None,
            incomplete_recursion: Vec::new(),
            aliases: BTreeMap::new(),
            receiver_context: None,
            primitive_libraries: Env::new(),
            flow: None,
            scopes: Vec::new(),
            origin: 1,
        }
    }
}

pub(crate) fn compile(ast: &syntax::Block) -> Result<Graph, Diagnostic> {
    Prelude::new()?.compile(ast)
}

/// Explicitly owned by a compiler session; no global cache or host I/O.
pub(crate) struct Prelude {
    checker: Checker,
    env: Env,
    library: Id,
}

impl Prelude {
    pub(crate) fn new() -> Result<Self, Diagnostic> {
        let mut checker = Checker::new();
        let (env, library) = checker.standard_library()?;
        Ok(Self {
            checker,
            env,
            library,
        })
    }

    pub(crate) fn compile(&self, ast: &syntax::Block) -> Result<Graph, Diagnostic> {
        let mut c = self.checker.clone();
        let mut env = self.env.clone();
        let library = self.library;
        let result = c.block(ast, &mut env, None)?;
        if result.ty == Type::Unit {
            return Err(Diagnostic::new(
                ast.span,
                "a source unit must produce a value",
            ));
        }
        if result.ty.contains_unresolved() {
            return Err(Diagnostic::new(
                ast.span,
                "an empty array needs an element type annotation",
            ));
        }
        let useful = c.node(
            Kind::Record(vec![
                (Some("$library".into()), library),
                (Some("$program".into()), result.id),
            ]),
            Type::Any,
            ast.span,
        );
        c.roots.insert(0, useful.id);
        c.check_usefulness()?;
        Ok(Graph {
            nodes: c.nodes,
            root: result.id,
        }
        .compact())
    }
}

impl Checker {
    fn node(&mut self, kind: Kind, ty: Type, span: Span) -> Checked {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            ty: ty.clone(),
            span,
            owner: self.owner,
            origin: self.origin,
        });
        Checked {
            id,
            contract: ty.clone(),
            ty,
        }
    }

    fn resolve_type(&mut self, ast: &syntax::TypeExpr, env: &Env) -> Result<Type, Diagnostic> {
        Ok(match &ast.kind {
            syntax::TypeKind::Literal(value) => Type::Literal(value.into()),
            syntax::TypeKind::Union(cases) => Type::union(
                cases
                    .iter()
                    .map(|case| self.resolve_type(case, env))
                    .collect::<Result<_, _>>()?,
            )
            .map_err(|e| Diagnostic::new(ast.span, e))?,
            syntax::TypeKind::NoResult => Type::Unit,
            syntax::TypeKind::Default(t) => Type::Default(Box::new(self.resolve_type(t, env)?)),
            syntax::TypeKind::Borrow(t) => {
                let ty = self.resolve_type(t, env)?;
                if !ty.storable() {
                    return Err(Diagnostic::new(
                        ast.span,
                        "a borrowed parameter requires a value type, not another borrow",
                    ));
                }
                Type::Borrow(Box::new(ty))
            }
            syntax::TypeKind::Name(name, span) => match name.as_str() {
                "int" => Type::Int,
                "bool" => Type::Bool,
                "str" => Type::Str,
                "any" => Type::Any,
                _ => match env.get(name) {
                    Some(Binding {
                        known: Type::Descriptor(t),
                        identity,
                        ..
                    }) => {
                        self.used_types.insert(*identity);
                        if let Type::Parameter { id, .. } = t.as_ref() {
                            self.use_generic(*id, false, *span)?;
                        }
                        if matches!(t.as_ref(), Type::Generic { .. }) {
                            return Err(Diagnostic::new(
                                *span,
                                format!("generic type `{name}` needs type arguments"),
                            ));
                        }
                        *t.clone()
                    }
                    _ => return Err(Diagnostic::new(*span, format!("unknown type `{name}`"))),
                },
            },
            syntax::TypeKind::Array(t) => {
                let ty = self.resolve_type(t, env)?;
                if !ty.storable() {
                    return Err(Diagnostic::new(
                        ast.span,
                        "borrowed references cannot be stored in arrays",
                    ));
                }
                Type::Array(Box::new(ty))
            }
            syntax::TypeKind::Apply(name, span, arguments) => {
                let Some(binding) = env.get(name) else {
                    return Err(Diagnostic::new(*span, format!("unknown type `{name}`")));
                };
                self.used_types.insert(binding.identity);
                let Type::Descriptor(descriptor) = &binding.known else {
                    return Err(Diagnostic::new(*span, "expected a generic type alias"));
                };
                let Type::Generic { parameters, body } = descriptor.as_ref() else {
                    return Err(Diagnostic::new(
                        *span,
                        "this type does not take generic arguments",
                    ));
                };
                let parameters = parameters.clone();
                let body = body.clone();
                let substitutions = self.explicit_arguments(&parameters, arguments, env, *span)?;
                body.substitute(&substitutions)
            }
            syntax::TypeKind::Path(segments, arguments) => {
                // A type value reached through object fields, such as `types.Package`
                // or `types.Box<int>`. Resolution is static: it walks known field
                // types and never evaluates the object.
                let (root, root_span) = &segments[0];
                let Some(binding) = env.get(root) else {
                    return Err(Diagnostic::new(
                        *root_span,
                        format!("unknown name `{root}` at the start of a type path"),
                    ));
                };
                self.used_types.insert(binding.identity);
                let mut current = binding.known.clone();
                let mut described = root.clone();
                for (name, span) in &segments[1..] {
                    let Type::Record { fields, .. } = &current else {
                        return Err(Diagnostic::new(
                            *span,
                            format!(
                                "`{described}` is not an object; a type path selects object fields"
                            ),
                        ));
                    };
                    let Some(field) = fields.get(name) else {
                        return Err(Diagnostic::new(
                            *span,
                            format!("field `{name}` is not known on `{described}`"),
                        ));
                    };
                    if field.optional {
                        return Err(Diagnostic::new(
                            *span,
                            format!(
                                "field `{name}` of `{described}` is optional; a type path needs a present field"
                            ),
                        ));
                    }
                    current = field.ty.clone();
                    described = format!("{described}.{name}");
                }
                let Type::Descriptor(descriptor) = current else {
                    return Err(Diagnostic::new(
                        ast.span,
                        format!("`{described}` is a value, not a type"),
                    ));
                };
                match (*descriptor, arguments.is_empty()) {
                    (Type::Generic { parameters, body }, false) => {
                        let substitutions =
                            self.explicit_arguments(&parameters, arguments, env, ast.span)?;
                        body.substitute(&substitutions)
                    }
                    (Type::Generic { .. }, true) => {
                        return Err(Diagnostic::new(
                            ast.span,
                            format!("generic type `{described}` needs type arguments"),
                        ));
                    }
                    (ty, true) => {
                        if let Type::Parameter { id, .. } = &ty {
                            self.use_generic(*id, false, ast.span)?;
                        }
                        ty
                    }
                    (_, false) => {
                        return Err(Diagnostic::new(
                            ast.span,
                            format!("`{described}` does not take generic arguments"),
                        ));
                    }
                }
            }
            syntax::TypeKind::Record(fields, open, row) => {
                let mut result = BTreeMap::new();
                for (name, span, optional, ast) in fields {
                    let field = Field {
                        ty: self.resolve_type(ast, env)?,
                        optional: *optional,
                    };
                    if !field.ty.storable() {
                        return Err(Diagnostic::new(
                            *span,
                            "borrowed references cannot be stored in object fields",
                        ));
                    }
                    if result.insert(name.clone(), field).is_some() {
                        return Err(Diagnostic::new(
                            *span,
                            format!("duplicate type field `{name}`"),
                        ));
                    }
                }
                let row = if let Some((name, span)) = row {
                    let Some(Binding {
                        known: Type::Descriptor(t),
                        identity,
                        ..
                    }) = env.get(name)
                    else {
                        return Err(Diagnostic::new(
                            *span,
                            format!("unknown generic row `{name}`"),
                        ));
                    };
                    let Type::Parameter { id, name } = t.as_ref() else {
                        return Err(Diagnostic::new(
                            *span,
                            "captured row must name a generic parameter",
                        ));
                    };
                    self.used_types.insert(*identity);
                    self.use_generic(*id, true, *span)?;
                    self.generics
                        .get_mut(id)
                        .and_then(|state| state.parameter.row.as_mut())
                        .ok_or_else(|| Diagnostic::new(*span, "missing captured row metadata"))?
                        .extend(result.keys().cloned());
                    Some((*id, name.clone()))
                } else {
                    None
                };
                Type::Record {
                    fields: result.into(),
                    open: *open,
                    row,
                }
            }
            syntax::TypeKind::Function(generics, receiver, parameters, result) => {
                let mut scope = env.clone();
                let ids = self.bind_generics(generics, &mut scope)?;
                let body = Type::Function {
                    receiver: *receiver,
                    parameters: parameters
                        .iter()
                        .map(|p| self.resolve_type(p, &scope))
                        .collect::<Result<_, _>>()?,
                    result: Box::new(self.resolve_type(result, &scope)?),
                };
                self.quantify(&ids, body, ast.span)?
            }
        })
    }

    fn can_copy(&self, source: &Type, target: &Type, fresh: bool) -> Result<(), String> {
        if source == target || *target == Type::Any || *source == Type::Never {
            return Ok(());
        }
        match (source, target) {
            (Type::Generic { .. }, Type::Generic { .. }) if source.callable_rebinds(target) => {
                Ok(())
            }
            (Type::Generic { parameters, body }, Type::Function { .. }) => self
                .specialization(parameters, body, target, Span::default())
                .map(|_| ())
                .map_err(|e| e.message),
            (Type::Union(cases), target) => cases
                .iter()
                .try_for_each(|case| self.can_copy(case, target, fresh)),
            (source, Type::Union(cases)) => {
                self.can_copy(source, source.variant_case(cases)?, fresh)
            }
            (Type::Literal(value), target) if value.primitive() == *target => Ok(()),
            (
                Type::Record {
                    fields: from,
                    open: from_open,
                    row: from_row,
                },
                Type::Record {
                    fields: to,
                    open: to_open,
                    row: to_row,
                },
            ) => {
                if to_row.is_some() && to_row != from_row {
                    return Err("cannot construct or recover an arbitrary generic remainder".into());
                }
                if fresh
                    && !to_open
                    && to_row.is_none()
                    && (*from_open
                        || from_row.is_some()
                        || from.keys().any(|k| !to.contains_key(k)))
                {
                    return Err("unexpected fields in a fresh literal for a closed type".into());
                }
                for (name, expected) in to.iter() {
                    match from.get(name) {
                        Some(actual) => {
                            if actual.optional && !expected.optional {
                                return Err(format!("field `{name}` may be absent"));
                            }
                            self.can_copy(&actual.ty, &expected.ty, false)
                                .map_err(|e| format!("field `{name}`: {e}"))?;
                        }
                        None if expected.optional && !from_open && from_row.is_none() => (),
                        None if *from_open || from_row.is_some() => {
                            return Err(format!(
                                "field `{name}` needs checked presence/payload evidence"
                            ));
                        }
                        None => return Err(format!("missing required field `{name}`")),
                    }
                }
                Ok(())
            }
            (Type::Array(from), Type::Array(to)) => self.can_copy(from, to, false),
            (Type::Function { .. }, Type::Function { .. }) if source.guarantees(target) => Ok(()),
            _ => Err(format!("expected {target}, found {source}")),
        }
    }

    fn project(
        &mut self,
        mut value: Checked,
        target: &Type,
        fresh: bool,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        // Only contextual literal syntax carries singleton evidence. A widened
        // binding, call result, or typed copy needs a match to narrow it again.
        if matches!(target, Type::Literal(_) | Type::Union(_)) {
            let literal = match &self.nodes[value.id].kind {
                Kind::Int(n) => Some(LiteralType::Int(*n)),
                Kind::Bool(b) => Some(LiteralType::Bool(*b)),
                Kind::Str(s) => Some(LiteralType::Str(s.clone())),
                Kind::Unary(Unary::Negate, id) => match self.nodes[*id].kind {
                    Kind::Int(n) => n.checked_neg().map(LiteralType::Int),
                    _ => None,
                },
                _ => None,
            };
            if let Some(literal) = literal {
                value.ty = Type::Literal(literal);
                self.nodes[value.id].ty = value.ty.clone();
            }
        }
        if let Type::Generic { parameters, body } = &value.ty {
            if matches!(target, Type::Function { .. }) {
                let (ty, arguments) = self.specialization(parameters, body, target, span)?;
                value = self.node(Kind::Instantiate(value.id, arguments), ty, span);
            }
        }
        if let Err(error) = self.can_copy(&value.ty, target, fresh) {
            // A checked read still satisfies its binding contract after a
            // successful match, even when its refined display type is int
            // and its original generic contract is the rigid type T.
            if value.contract != *target {
                return Err(Diagnostic::new(span, error));
            }
        }
        if value.ty == *target {
            return Ok(Checked {
                contract: target.clone(),
                ..value
            });
        }
        let dropped = self.projection_drops(&value.ty, target, span)?;
        Ok(self.node(
            Kind::Project(value.id, target.clone(), dropped),
            target.clone(),
            span,
        ))
    }

    fn projection_drops(
        &self,
        source: &Type,
        target: &Type,
        span: Span,
    ) -> Result<Drops, Diagnostic> {
        let mut drops = Drops::default();
        match (source, target) {
            (Type::Generic { parameters, body }, Type::Function { .. }) => {
                drops.instantiate = Some(self.specialization(parameters, body, target, span)?.1);
            }
            (Type::Union(cases), target) => {
                for case in cases {
                    let more = self.projection_drops(case, target, span)?;
                    drops.fields.extend(more.fields);
                    drops.names.extend(more.names);
                }
            }
            (source, Type::Union(cases)) => {
                if let Ok(case) = source.variant_case(cases) {
                    return self.projection_drops(source, case, span);
                }
            }
            (
                Type::Record {
                    fields: from,
                    row: from_row,
                    ..
                },
                Type::Record {
                    fields: to,
                    row: to_row,
                    ..
                },
            ) => {
                if let (Some(a), Some(b)) = (from_row, to_row) {
                    if a == b {
                        let excluded = self.row_exclusions(a.0, span)?;
                        drops.names = from
                            .keys()
                            .filter(|name| !to.contains_key(*name) && excluded.contains(*name))
                            .cloned()
                            .collect();
                    }
                }
                for (name, field) in to.iter() {
                    if let Some(source) = from.get(name) {
                        drops.fields.insert(
                            name.clone(),
                            self.projection_drops(&source.ty, &field.ty, span)?,
                        );
                    }
                }
            }
            (Type::Array(a), Type::Array(b)) => {
                drops.element = Some(Box::new(self.projection_drops(a, b, span)?))
            }
            _ => (),
        }
        Ok(drops)
    }

    fn expr(
        &mut self,
        ast: &syntax::Expr,
        env: &mut Env,
        expected: Option<&Type>,
    ) -> Result<Checked, Diagnostic> {
        let result = self.expr_unprojected(ast, env, expected)?;
        if !self.reachable() {
            return Ok(result);
        }
        if !result.ty.within_depth(64) {
            return Err(Diagnostic::new(
                ast.span,
                "static type nesting depth limit exceeded (64); use an explicit checked interface",
            ));
        }
        if let Some(target) = expected {
            if target.mentions(&self.inference) {
                return Ok(result);
            }
            self.project(
                result,
                target,
                matches!(ast.kind, syntax::ExprKind::Record(_)),
                ast.span,
            )
        } else {
            Ok(result)
        }
    }

    fn expr_unprojected(
        &mut self,
        ast: &syntax::Expr,
        env: &mut Env,
        expected: Option<&Type>,
    ) -> Result<Checked, Diagnostic> {
        use syntax::ExprKind as E;
        let span = ast.span;
        if self.projection_read == 0 {
            self.check_access(ast, env)?;
        }
        let result = match &ast.kind {
            E::Borrow(_) => {
                return Err(Diagnostic::new(
                    span,
                    "borrowing is only supported at borrowed argument positions; a borrow is not a storable value",
                ));
            }
            E::Int(n) => self.node(Kind::Int(*n), Type::Int, span),
            E::Bool(b) => self.node(Kind::Bool(*b), Type::Bool, span),
            E::Str(s) => self.node(Kind::Str(s.clone()), Type::Str, span),
            E::Name(name) => {
                let b = env
                    .get(name)
                    .ok_or_else(|| {
                        if self.incomplete_recursion.contains(name) {
                            Diagnostic::new(span, format!("recursive function `{name}` needs a result type annotation or an expected function type"))
                        } else {
                            Diagnostic::new(span, format!("unknown name `{name}`"))
                        }
                    })?;
                let mut value = self.node(Kind::Alias(b.value), b.known.clone(), span);
                value.contract = b.contract.clone();
                value
            }
            E::Type(generics, t) => {
                let mut scope = env.clone();
                let ids = self.bind_generics(generics, &mut scope)?;
                let ty = self.resolve_type(t, &scope)?;
                let ty = self.quantify(&ids, ty, span)?;
                if !ty.storable() {
                    return Err(Diagnostic::new(
                        span,
                        "a type descriptor must describe a storable value or callable",
                    ));
                }
                self.node(Kind::Descriptor, Type::Descriptor(Box::new(ty)), span)
            }
            E::Record(entries) => self.record(entries, env, expected, span)?,
            E::Array(elements) => {
                let context = if let Some(Type::Array(t)) = expected {
                    Some(t.as_ref())
                } else {
                    None
                };
                let mut ty = context
                    .filter(|t| !t.mentions(&self.inference))
                    .cloned()
                    .unwrap_or(Type::Never);
                let mut values = Vec::new();
                for e in elements {
                    let array_context = context.map(|t| Type::Array(Box::new(t.clone())));
                    let value = self.expr(
                        &e.value,
                        env,
                        if e.spread {
                            array_context.as_ref()
                        } else {
                            context
                        },
                    )?;
                    if value.ty == Type::Unit {
                        return Err(Diagnostic::new(
                            e.span,
                            "an array element must have a value",
                        ));
                    }
                    let element = if e.spread {
                        let Type::Array(element) = &value.ty else {
                            return Err(Diagnostic::new(
                                e.span,
                                "array spread requires a checked array type",
                            ));
                        };
                        element.as_ref()
                    } else {
                        &value.ty
                    };
                    ty = ty.join(element);
                    values.push((e.spread, value.id));
                }
                self.node(Kind::Array(values), Type::Array(Box::new(ty)), span)
            }
            E::Field(base, name) => {
                let value = self.projection_base(base, env)?;
                if let Some(function) = self.primitive_member(&value.ty, name, span)? {
                    let proof = self.node(Kind::Is(value.id, value.ty.clone()), Type::Bool, span);
                    return Ok(self.node(
                        Kind::If(proof.id, function.id, function.id),
                        function.ty,
                        span,
                    ));
                }
                let ty = Self::field_type(&value.ty, name, span)?;
                let mut node = self.node(Kind::Field(value.id, name.clone()), ty, span);
                if let Type::Record { fields, .. } = &value.contract {
                    if let Some(field) = fields.get(name) {
                        node.contract = field.ty.clone();
                    }
                }
                node
            }
            E::Index(base, index) => {
                let value = self.projection_base(base, env)?;
                let previous = self.projection_read;
                self.projection_read = 0;
                let key = self.expr(index, env, None);
                self.projection_read = previous;
                let key = key?;
                let ty = match &value.ty {
                    Type::Array(element) if key.ty.primitive() == Some(Type::Int) => {
                        *element.clone()
                    }
                    Type::Record { .. } => {
                        let E::Str(name) = &index.kind else {
                            return Err(Diagnostic::new(
                                index.span,
                                "computed record access needs literal-key evidence in this initial slice",
                            ));
                        };
                        Self::field_type(&value.ty, name, span)?
                    }
                    _ => {
                        return Err(Diagnostic::new(
                            span,
                            "indexing requires an array and int, or an object and a proven string key",
                        ));
                    }
                };
                self.node(Kind::Index(value.id, key.id), ty, span)
            }
            E::Slice(base, start, end) => {
                self.check_access(base, env)?;
                let value = self.expr(base, env, None)?;
                let ty = if matches!(value.ty, Type::Array(_)) {
                    value.ty.clone()
                } else if value.ty.primitive() == Some(Type::Str) {
                    Type::Str
                } else {
                    return Err(Diagnostic::new(span, "slicing requires an array or str"));
                };
                let start = start
                    .as_ref()
                    .map(|v| self.expr(v, env, Some(&Type::Int)))
                    .transpose()?;
                let end = end
                    .as_ref()
                    .map(|v| self.expr(v, env, Some(&Type::Int)))
                    .transpose()?;
                self.node(
                    Kind::Slice(value.id, start.map(|v| v.id), end.map(|v| v.id)),
                    ty,
                    span,
                )
            }
            E::Unary(op, operand) => {
                let wanted = if *op == Unary::Not {
                    Type::Bool
                } else {
                    Type::Int
                };
                let operand = self.expr(operand, env, Some(&wanted))?;
                self.node(Kind::Unary(*op, operand.id), wanted, span)
            }
            E::Binary(op, a, b) => {
                let left = self.expr(a, env, None)?;
                let parent = self.guard();
                let short_circuit = matches!(op, Binary::And | Binary::Or);
                let skipped = if short_circuit {
                    let selected = self.branch_guard(parent, left.id, *op == Binary::And, span);
                    self.set_guard(selected);
                    self.branch_guard(parent, left.id, *op == Binary::Or, span)
                } else {
                    None
                };
                let mut rhs_env = env.clone();
                let mut right = self.expr(b, &mut rhs_env, None)?;
                let rhs_live = self.reachable();
                // Expressions may contain mutation-producing blocks: do not silently drop them.
                if rhs_env.iter().any(|(name, binding)| {
                    env.get(name).is_some_and(|old| old.value != binding.value)
                }) {
                    return Err(Diagnostic::new(
                        b.span,
                        "mutating the right operand of a binary expression is not supported; use an if block",
                    ));
                }
                if short_circuit {
                    let guards: Vec<_> = skipped.into_iter().chain(self.guard()).collect();
                    let joined = self.join_guards(&guards, span);
                    self.set_guard(joined);
                    if !rhs_live {
                        right = self.node(Kind::Bool(false), Type::Bool, b.span);
                    }
                } else if !rhs_live {
                    return Ok(right);
                }
                self.binary(*op, left, right, span)?
            }
            E::Match(value, arms) => self.match_expr(value, arms, env, expected, span)?,
            E::For(name, name_span, source, body) => {
                self.for_loop(name, *name_span, source, body, env, span)?
            }
            E::Block(block) => self.block(block, env, expected)?,
            E::If(condition, yes, no) => {
                let condition_value = self.expr(condition, env, Some(&Type::Bool))?;
                let parent = self.guard();
                let mut yes_env = env.clone();
                let mut no_env = env.clone();
                let yes_guard = self.branch_guard(parent, condition_value.id, true, span);
                self.set_guard(yes_guard);
                let a = self.block(yes, &mut yes_env, expected)?;
                let yes_live = self.reachable();
                let yes_guard = self.guard();
                let no_guard = self.branch_guard(parent, condition_value.id, false, span);
                self.set_guard(no_guard);
                let b = match no {
                    Some(block) => self.block(block, &mut no_env, expected)?,
                    None => self.node(Kind::Unit, Type::Unit, span),
                };
                let no_live = self.reachable();
                let guards: Vec<_> = yes_guard.into_iter().chain(self.guard()).collect();
                let joined_guard = self.join_guards(&guards, span);
                self.set_guard(joined_guard);
                if yes_live && no_live && (a.ty == Type::Unit) != (b.ty == Type::Unit) {
                    return Err(Diagnostic::new(
                        span,
                        "both branches must return values, or both must be statement blocks",
                    ));
                }
                for (name, original) in env.iter_mut() {
                    let (Some(y), Some(n)) = (yes_env.get(name), no_env.get(name)) else {
                        continue;
                    };
                    if !yes_live || !no_live {
                        if yes_live {
                            *original = y.clone();
                        }
                        if no_live {
                            *original = n.clone();
                        }
                        continue;
                    }
                    if y.value != original.value || n.value != original.value {
                        let joined = y.known.join_preserving(&n.known, &original.known);
                        let node = self.node(
                            Kind::If(condition_value.id, y.value, n.value),
                            joined.clone(),
                            span,
                        );
                        original.value = node.id;
                        original.known = joined;
                    }
                }
                let ty = match (yes_live, no_live) {
                    (true, true) => a.ty.join(&b.ty),
                    (true, false) => a.ty,
                    (false, true) => b.ty,
                    _ => Type::Unit,
                };
                self.node(Kind::If(condition_value.id, a.id, b.id), ty, span)
            }
            E::Function(generics, parameters, result, body) => {
                let outer_flow = self.flow.take();
                let receiver_context = self.receiver_context.take();
                // Replacing an inferred method checks the new body against the
                // existing rigid receiver row; its signature does not change.
                let inherited = match expected {
                    Some(Type::Generic { parameters, body }) if generics.is_empty() => {
                        Some((parameters, body.as_ref()))
                    }
                    _ => None,
                };
                let expected = inherited.map(|(_, body)| body).or(expected);
                let function_name = self.function_name.take();
                let outside = self.owner;
                let owner = self.next_owner;
                self.next_owner += 1;
                self.owner = owner;
                let mut body_env = env.clone();
                let mut generic_ids = self.bind_generics(generics, &mut body_env)?;
                let receiver_context = if let Some((parameters, _)) = inherited {
                    generic_ids.extend(parameters.iter().map(|p| p.id));
                    None
                } else {
                    self.inferred_receiver(parameters, receiver_context, &mut generic_ids)
                };
                let type_env = body_env.clone();
                let (types, defaults) = self.function_parameters(
                    parameters,
                    &type_env,
                    &mut body_env,
                    expected,
                    (outside, receiver_context.as_ref()),
                )?;
                let names: BTreeSet<_> = parameters.iter().map(|p| &p.name).collect();
                let has_borrows = types.iter().any(|t| matches!(t, Type::Borrow(_)));
                let result_type = result
                    .as_ref()
                    .map(|t| self.resolve_type(t, &type_env))
                    .transpose()?
                    .or_else(|| match expected {
                        Some(Type::Function { result, .. }) => Some(*result.clone()),
                        _ if has_borrows
                            && matches!(&body.kind, E::Block(b) if b.result.is_none())
                            && !returns::returns_value(body) =>
                        {
                            Some(Type::Unit)
                        }
                        _ => None,
                    });
                if result_type
                    .as_ref()
                    .is_some_and(|t| !t.storable() && *t != Type::Unit)
                {
                    return Err(Diagnostic::new(
                        span,
                        "a function cannot return a live borrow",
                    ));
                }
                // A recursive name points to this definition, never a later
                // version of the outer variable. The graph arena owns the cycle.
                let recursive_handle = match (&function_name, &result_type) {
                    (Some(name), Some(result)) => {
                        let signature = Type::Function {
                            receiver: parameters.first().is_some_and(|p| p.name == "self"),
                            parameters: types.clone(),
                            result: Box::new(result.clone()),
                        };
                        let signature = if generic_ids.is_empty() {
                            signature
                        } else {
                            Type::Generic {
                                parameters: generic_ids
                                    .iter()
                                    .map(|id| self.generics[id].parameter.clone())
                                    .collect(),
                                body: Box::new(signature),
                            }
                        };
                        self.owner = outside;
                        let handle = self.node(Kind::Unit, signature.clone(), span);
                        self.owner = owner;
                        // An explicit parameter of the same name shadows the
                        // definition just like any other lexical binding.
                        if !names.contains(name) && !generics.iter().any(|(n, _)| n == name) {
                            body_env.insert(
                                name.clone(),
                                Binding {
                                    identity: handle.id,
                                    value: handle.id,
                                    contract: signature.clone(),
                                    known: signature,
                                    owner: outside,
                                },
                            );
                        }
                        Some(handle.id)
                    }
                    _ => None,
                };
                if let Some(name) = &function_name {
                    if recursive_handle.is_none() {
                        self.incomplete_recursion.push(name.clone());
                    }
                }
                let guard = self.node(Kind::Bool(true), Type::Bool, span).id;
                self.flow = Some(returns::Flow {
                    guard: Some(guard),
                    exits: Vec::new(),
                    expected: result_type.clone(),
                    parameters: parameters.clone(),
                    types: types.clone(),
                    aliases: self.aliases.keys().copied().collect(),
                    scope_base: self.scopes.len(),
                });
                let body_value = self.expr(body, &mut body_env, result_type.as_ref())?;
                let flow = self
                    .flow
                    .take()
                    .ok_or_else(|| Diagnostic::new(span, "internal invariant: function flow"))?;
                let fallthrough = flow.guard.map(|_| (body_value, &body_env));
                let (result_ty, body_id) =
                    self.finish_returns(flow.exits.clone(), fallthrough, &flow, span)?;
                self.flow = outer_flow;
                if function_name.is_some() && recursive_handle.is_none() {
                    self.incomplete_recursion.pop();
                }
                if result_ty == Type::Unit && !has_borrows {
                    return Err(Diagnostic::new(
                        body.span,
                        "a function without borrowed outputs must return a value",
                    ));
                }
                if result_ty.contains_unresolved() {
                    return Err(Diagnostic::new(
                        body.span,
                        "the function result needs a concrete element type",
                    ));
                }
                let usefulness = if defaults.is_empty() {
                    body_id
                } else {
                    let mut outputs = vec![(Some("$body".into()), body_id)];
                    outputs.extend(
                        defaults
                            .iter()
                            .enumerate()
                            .map(|(i, id)| (Some(format!("$default{i}")), *id)),
                    );
                    self.node(Kind::Record(outputs), Type::Any, span).id
                };
                self.roots.insert(owner, usefulness);
                self.owner = outside;
                let ty = Type::Function {
                    receiver: parameters.first().is_some_and(|p| p.name == "self"),
                    parameters: types,
                    result: Box::new(result_ty),
                };
                let ty = self.quantify(&generic_ids, ty, span)?;
                let function = self.node(
                    Kind::Function {
                        owner,
                        body: body_id,
                        defaults,
                        parameters: generic_ids,
                    },
                    ty,
                    span,
                );
                if let Some(handle) = recursive_handle {
                    self.nodes[handle].kind = Kind::Alias(function.id);
                    self.nodes[handle].ty = function.ty.clone();
                }
                function
            }
            E::Call(function, type_args, args) => {
                let function = self.expr(function, env, None)?;
                self.invoke((function, Vec::new()), type_args, args, env, expected, span)?
            }
            E::MemberCall(base, name, type_args, args) => {
                self.member_call((base, name), type_args, args, env, expected, span)?
            }
        };
        Ok(result)
    }

    fn field_type(ty: &Type, name: &str, span: Span) -> Result<Type, Diagnostic> {
        if let Type::Record { fields, .. } = ty {
            if let Some(field) = fields.get(name) {
                if field.optional {
                    return Err(Diagnostic::new(
                        span,
                        format!("field `{name}` is optional; prove presence first"),
                    ));
                }
                return Ok(field.ty.clone());
            }
        }
        Err(Diagnostic::new(
            span,
            format!("field `{name}` is not known on {ty}; establish its type with a guard"),
        ))
    }

    fn checkable(ty: &Type) -> bool {
        match ty {
            Type::Literal(_) => true,
            Type::Union(cases) => cases.iter().all(Self::checkable),
            Type::Int | Type::Bool | Type::Str | Type::Any | Type::Parameter { .. } => true,
            Type::Array(t) => Self::checkable(t),
            Type::Record { fields, .. } => fields.values().all(|f| Self::checkable(&f.ty)),
            _ => false,
        }
    }

    fn comparable(ty: &Type) -> bool {
        match ty {
            Type::Literal(_) => true,
            Type::Union(cases) => cases.iter().all(Self::comparable),
            Type::Int | Type::Bool | Type::Str => true,
            Type::Array(element) => Self::comparable(element),
            Type::Record {
                fields,
                open: false,
                row: None,
            } => fields.values().all(|field| Self::comparable(&field.ty)),
            _ => false,
        }
    }

    fn binary(
        &mut self,
        op: Binary,
        a: Checked,
        b: Checked,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        use Binary::*;
        let ap = a.ty.primitive();
        let bp = b.ty.primitive();
        let ty = match op {
            Add if ap == Some(Type::Str) && bp == ap => Type::Str,
            Add | Subtract | Multiply | Divide | Remainder if ap == Some(Type::Int) && bp == ap => {
                Type::Int
            }
            And | Or if ap == Some(Type::Bool) && bp == ap => Type::Bool,
            Equal | NotEqual
                if (a.ty == b.ty || (ap.is_some() && ap == bp)) && Self::comparable(&a.ty) =>
            {
                Type::Bool
            }
            Less | LessEqual | Greater | GreaterEqual
                if ap == bp && matches!(ap, Some(Type::Int | Type::Str)) =>
            {
                Type::Bool
            }
            In if ap == Some(Type::Str) && matches!(b.ty, Type::Record { .. }) => Type::Bool,
            _ => {
                return Err(Diagnostic::new(
                    span,
                    format!("operator {op:?} does not accept {} and {}", a.ty, b.ty),
                ));
            }
        };
        Ok(self.node(Kind::Binary(op, a.id, b.id), ty, span))
    }

    fn match_expr(
        &mut self,
        value: &syntax::Expr,
        arms: &[syntax::Arm],
        env: &mut Env,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Checked, Diagnostic> {
        use syntax::Pattern;
        if arms.is_empty() {
            return Err(Diagnostic::new(span, "match needs at least one arm"));
        }
        let source = self.expr(value, env, None)?;
        let mut branches = Vec::new();
        let mut coverage = Vec::new();
        let mut remaining = self.guard();
        let mut live_guards = Vec::new();
        let finite = matches!(source.ty, Type::Union(_) | Type::Literal(_) | Type::Bool);
        for (index, arm) in arms.iter().enumerate() {
            let mut branch = env.clone();
            let (condition, evidence) = match &arm.pattern {
                Pattern::Type(ast) => {
                    let target = self.resolve_type(ast, env)?;
                    if !Self::checkable(&target) {
                        return Err(Diagnostic::new(
                            ast.span,
                            "this type cannot be checked by a data pattern in this initial slice",
                        ));
                    }
                    (
                        Some(
                            self.node(Kind::Is(source.id, target.clone()), Type::Bool, ast.span)
                                .id,
                        ),
                        Some(target),
                    )
                }
                Pattern::Literal(literal) => {
                    let ty = Type::Literal(literal.into());
                    (
                        Some(
                            self.node(
                                Kind::LiteralTest(source.id, literal.clone()),
                                Type::Bool,
                                arm.span,
                            )
                            .id,
                        ),
                        Some(ty),
                    )
                }
                Pattern::Else if index == arms.len() - 1 => (None, None),
                Pattern::Else => {
                    return Err(Diagnostic::new(
                        arm.span,
                        "`else` must be the last match arm",
                    ));
                }
            };
            if let Some(evidence) = evidence {
                if finite && !source.ty.can_overlap(&evidence) {
                    return Err(Diagnostic::new(
                        arm.span,
                        "pattern cannot match any case of this variant",
                    ));
                }
                if let Some(known) = Self::guarded_type(value, &mut branch) {
                    *known = known.refine(&evidence);
                }
                coverage.push(evidence);
            }
            let guard = if let Some(condition) = condition {
                let guard = self.branch_guard(remaining, condition, true, span);
                remaining = self.branch_guard(remaining, condition, false, span);
                guard
            } else {
                remaining
            };
            self.set_guard(guard);
            let result = self.expr(&arm.body, &mut branch, expected)?;
            if self.reachable() {
                live_guards.extend(self.guard());
                branches.push((condition, result, branch));
            }
        }
        if !matches!(arms.last().map(|a| &a.pattern), Some(Pattern::Else)) {
            let cases = match &source.ty {
                Type::Union(cases) => cases.clone(),
                Type::Bool => vec![
                    Type::Literal(LiteralType::Bool(false)),
                    Type::Literal(LiteralType::Bool(true)),
                ],
                ty => vec![ty.clone()],
            };
            if source.ty == Type::Any
                || !cases
                    .iter()
                    .all(|case| coverage.iter().any(|pattern| case.guarantees(pattern)))
            {
                return Err(Diagnostic::new(
                    span,
                    "non-exhaustive match; cover every case or add a final `else` arm",
                ));
            }
        }
        let joined_guard = self.join_guards(&live_guards, span);
        self.set_guard(joined_guard);
        let Some((_, fallback, fallback_env)) = branches.pop() else {
            return Ok(self.node(Kind::Unit, Type::Unit, span));
        };
        let mut result = fallback;
        if !matches!(arms.last().map(|a| &a.pattern), Some(Pattern::Else)) {
            // Even a statically exhaustive singleton match uses the scrutinee's
            // evidence. Record that dependency without demanding its payload.
            let proof = self.node(Kind::Is(source.id, source.ty.clone()), Type::Bool, span);
            result = self.node(Kind::If(proof.id, result.id, result.id), result.ty, span);
        }
        for (condition, arm, _) in branches.iter().rev() {
            if (result.ty == Type::Unit) != (arm.ty == Type::Unit) {
                return Err(Diagnostic::new(
                    span,
                    "all match arms must produce values, or all must be statement blocks",
                ));
            }
            if let Some(condition) = condition {
                let ty = arm.ty.join(&result.ty);
                result = self.node(Kind::If(*condition, arm.id, result.id), ty, span);
            }
        }
        for (name, original) in env.iter_mut() {
            let Some(last) = fallback_env.get(name) else {
                continue;
            };
            if last.value == original.value
                && last.known == original.known
                && branches.iter().all(|(_, _, branch)| {
                    branch
                        .get(name)
                        .is_some_and(|b| b.value == original.value && b.known == original.known)
                })
            {
                continue;
            }
            let mut id = last.value;
            let mut known = last.known.clone();
            for (condition, _, branch) in branches.iter().rev() {
                if let (Some(condition), Some(binding)) = (condition, branch.get(name)) {
                    known = binding.known.join_preserving(&known, &original.known);
                    if binding.value != id {
                        id = self
                            .node(Kind::If(*condition, binding.value, id), known.clone(), span)
                            .id;
                    }
                }
            }
            original.value = id;
            original.known = known;
        }
        Ok(result)
    }

    fn guarded_type<'a>(value: &syntax::Expr, env: &'a mut Env) -> Option<&'a mut Type> {
        use syntax::ExprKind as E;
        match &value.kind {
            E::Name(name) => Some(&mut env.get_mut(name)?.known),
            E::Field(base, name) => {
                let Type::Record { fields, .. } = Self::guarded_type(base, env)? else {
                    return None;
                };
                let field = Arc::make_mut(fields).get_mut(name)?;
                (!field.optional).then_some(&mut field.ty)
            }
            E::Index(base, key) => {
                let E::Str(name) = &key.kind else { return None };
                let Type::Record { fields, .. } = Self::guarded_type(base, env)? else {
                    return None;
                };
                let field = Arc::make_mut(fields).get_mut(name)?;
                (!field.optional).then_some(&mut field.ty)
            }
            _ => None,
        }
    }

    fn block(
        &mut self,
        block: &syntax::Block,
        env: &mut Env,
        expected: Option<&Type>,
    ) -> Result<Checked, Diagnostic> {
        let mut entering = env.clone();
        self.scopes.push(entering.clone());
        let entering_aliases: BTreeSet<_> = self.aliases.keys().copied().collect();
        for statement in &block.statements {
            if !self.reachable() {
                return Err(Diagnostic::new(
                    block.span,
                    "unreachable statement after return",
                ));
            }
            match statement {
                syntax::Statement::Return(value, span) => {
                    self.return_statement(value.as_ref(), env, *span)?
                }
                syntax::Statement::Let(name, annotation, expression, span) => {
                    self.check_shadow(name, env, *span)?;
                    if let syntax::ExprKind::Borrow(place) = &expression.kind {
                        if let (Some(outer), Some(current)) =
                            (entering.get_mut(name), env.get(name))
                        {
                            if outer.identity == current.identity {
                                *outer = current.clone();
                            }
                        }
                        *self.scopes.last_mut().ok_or_else(|| {
                            Diagnostic::new(block.span, "internal invariant: block scope")
                        })? = entering.clone();
                        self.declare_alias(name, annotation.as_ref(), place, env, *span)?;
                        continue;
                    }
                    let expected = annotation
                        .as_ref()
                        .map(|t| self.resolve_type(t, env))
                        .transpose()?;
                    // Initializers of shadows keep seeing the previous binding.
                    // Only a new directly bound function gets a recursive name.
                    if matches!(expression.kind, syntax::ExprKind::Function(..))
                        && !env.contains_key(name)
                    {
                        self.function_name = Some(name.clone());
                    }
                    let value = self.expr(expression, env, expected.as_ref())?;
                    if value.ty == Type::Unit || value.ty.contains_unresolved() {
                        return Err(Diagnostic::new(
                            *span,
                            "binding requires a value with a resolved type; annotate empty arrays",
                        ));
                    }
                    let contract = expected.unwrap_or(value.contract);
                    let alias = self.node(Kind::Alias(value.id), value.ty.clone(), *span);
                    self.definitions.push(Definition {
                        id: alias.id,
                        name: name.clone(),
                        span: *span,
                        owner: self.owner,
                    });
                    // Keep writes made before a same-block shadow. The shadow
                    // leaves scope, but those writes still belong to the outer binding.
                    if let (Some(outer), Some(current)) = (entering.get_mut(name), env.get(name)) {
                        if outer.identity == current.identity {
                            *outer = current.clone();
                        }
                    }
                    *self.scopes.last_mut().ok_or_else(|| {
                        Diagnostic::new(block.span, "internal invariant: block scope")
                    })? = entering.clone();
                    env.insert(
                        name.clone(),
                        Binding {
                            identity: alias.id,
                            value: alias.id,
                            contract,
                            known: value.ty,
                            owner: self.owner,
                        },
                    );
                }
                syntax::Statement::Assign(place, op, expression, span) => {
                    self.assign(place, *op, expression, *span, env)?
                }
                syntax::Statement::Discard(expression) => {
                    let exits_before = self.flow.as_ref().map_or(0, |f| f.exits.len());
                    let before: BTreeMap<_, _> =
                        env.iter().map(|(n, b)| (n.clone(), b.value)).collect();
                    let value = self.expr(expression, env, None)?;
                    if self.reachable() && value.ty != Type::Unit {
                        return Err(Diagnostic::new(expression.span, "unused expression result"));
                    }
                    if self.reachable()
                        && self.flow.as_ref().map_or(0, |f| f.exits.len()) == exits_before
                        && !env
                            .iter()
                            .any(|(n, b)| before.get(n).is_some_and(|old| *old != b.value))
                    {
                        return Err(Diagnostic::new(
                            expression.span,
                            "statement does not contribute a value or update",
                        ));
                    }
                }
            }
        }
        let result = match &block.result {
            Some(value) if !self.reachable() => {
                return Err(Diagnostic::new(
                    value.span,
                    "unreachable expression after return",
                ));
            }
            Some(value) => self.expr(value, env, expected)?,
            None => self.node(Kind::Unit, Type::Unit, block.span),
        };
        if self.reachable() {
            self.close_aliases(&entering_aliases, env, block.span)?;
        } else {
            // Exit snapshots already contain alias writebacks.
            let closing: BTreeSet<_> = self
                .aliases
                .keys()
                .filter(|id| !entering_aliases.contains(id))
                .copied()
                .collect();
            let handles: BTreeSet<_> = closing.iter().map(|id| self.aliases[id].guard).collect();
            self.aliases.retain(|id, _| !closing.contains(id));
            self.loans
                .retain(|l| !l.handle.is_some_and(|h| handles.contains(&h)));
        }
        self.scopes.pop();
        let leaving = env.clone();
        *env = entering;
        for (name, binding) in env.iter_mut() {
            if let Some(updated) = leaving.get(name) {
                if binding.identity == updated.identity {
                    *binding = updated.clone();
                }
            }
        }
        Ok(result)
    }

    fn place(
        &mut self,
        ast: &syntax::Expr,
        env: &mut Env,
    ) -> Result<(String, Vec<Part>), Diagnostic> {
        match &ast.kind {
            syntax::ExprKind::Name(name) => Ok((name.clone(), Vec::new())),
            syntax::ExprKind::Field(base, name) => {
                let (root, mut path) = self.place(base, env)?;
                path.push(Part::Field(name.clone()));
                Ok((root, path))
            }
            syntax::ExprKind::Index(base, key) => {
                let (root, mut path) = self.place(base, env)?;
                if let syntax::ExprKind::Str(name) = &key.kind {
                    path.push(Part::Field(name.clone()));
                } else {
                    path.push(Part::Index(self.expr(key, env, Some(&Type::Int))?.id));
                }
                Ok((root, path))
            }
            _ => Err(Diagnostic::new(
                ast.span,
                "assignment requires a local name followed by fields or indices",
            )),
        }
    }

    fn path_type(
        contract: &Type,
        known: &Type,
        path: &[Part],
        span: Span,
    ) -> Result<Option<Type>, Diagnostic> {
        let Some((part, rest)) = path.split_first() else {
            return Ok(Some(known.clone()));
        };
        if *contract == Type::Any && *known != Type::Any {
            return Self::path_type(known, known, path, span);
        }
        if let Type::Union(cases) = contract {
            if let Ok(case) = known.variant_case(cases) {
                return Self::path_type(case, known, path, span);
            }
        }
        match (part, contract, known) {
            (
                Part::Field(name),
                Type::Record { fields, open, row },
                Type::Record {
                    fields: current, ..
                },
            ) => {
                let declared = fields.get(name);
                let fact = current.get(name);
                if row.is_some() && declared.is_none() {
                    return Err(Diagnostic::new(
                        span,
                        format!(
                            "field `{name}` belongs to a captured generic remainder and cannot be written"
                        ),
                    ));
                }
                if rest.is_empty() {
                    if let Some(field) = fact.or(declared) {
                        return Ok(Some(field.ty.clone()));
                    }
                    // An explicit open contract permits insertion even after
                    // matching an exact current shape. The update computes the
                    // new exact shape; it does not forget existing fields.
                    if *open {
                        return Ok(None);
                    }
                    return Err(Diagnostic::new(
                        span,
                        format!("cannot add field `{name}` to a closed binding type"),
                    ));
                }
                let fact = fact.ok_or_else(|| {
                    Diagnostic::new(span, format!("nested field `{name}` is not known present"))
                })?;
                if fact.optional {
                    return Err(Diagnostic::new(
                        span,
                        format!("nested field `{name}` may be absent"),
                    ));
                }
                Self::path_type(declared.map_or(&fact.ty, |f| &f.ty), &fact.ty, rest, span)
            }
            (Part::Index(_), Type::Array(element), Type::Array(current)) => {
                Self::path_type(element, current, rest, span)
            }
            _ => Err(Diagnostic::new(
                span,
                "assignment path does not match its binding type",
            )),
        }
    }

    fn updated_type(known: &Type, path: &[Part], value: &Type) -> Type {
        let Some((part, rest)) = path.split_first() else {
            return value.clone();
        };
        match (part, known) {
            (Part::Field(name), Type::Record { fields, open, row }) => {
                let mut fields = fields.clone();
                let old = fields.get(name).map_or(value, |f| &f.ty);
                let next = Self::updated_type(old, rest, value);
                Arc::make_mut(&mut fields).insert(name.clone(), Field::required(next));
                Type::Record {
                    fields,
                    open: *open,
                    row: row.clone(),
                }
            }
            (Part::Index(_), Type::Array(element)) => Type::Array(Box::new(
                element.join(&Self::updated_type(element, rest, value)),
            )),
            _ => known.clone(),
        }
    }

    fn assign(
        &mut self,
        place: &syntax::Expr,
        op: Option<Binary>,
        expression: &syntax::Expr,
        span: Span,
        env: &mut Env,
    ) -> Result<(), Diagnostic> {
        self.check_access(place, env)?;
        let (name, path) = self.place(place, env)?;
        let binding = env
            .get(&name)
            .cloned()
            .ok_or_else(|| Diagnostic::new(span, format!("unknown assignment target `{name}`")))?;
        if binding.owner != self.owner {
            return Err(Diagnostic::new(
                span,
                "a closure cannot mutate a captured binding; copy it to a local first",
            ));
        }
        let target = Self::path_type(&binding.contract, &binding.known, &path, span)?;
        let value = if let Some(op) = op {
            let before = self.expr(place, env, None)?;
            let rhs = self.expr(expression, env, None)?;
            let value = self.binary(op, before, rhs, span)?;
            if let Some(target) = &target {
                self.project(value, target, false, span)?
            } else {
                value
            }
        } else {
            self.expr(expression, env, target.as_ref())?
        };
        if value.ty.contains_unresolved() || value.ty == Type::Unit {
            return Err(Diagnostic::new(
                span,
                "assignment requires a resolved value type",
            ));
        }
        let known = Self::updated_type(&binding.known, &path, &value.ty);
        self.can_copy(&known, &binding.contract, false)
            .map_err(|e| {
                Diagnostic::new(
                    span,
                    format!("assignment violates its binding contract: {e}"),
                )
            })?;
        // Whole replacements may not erase current field/presence evidence.
        if path.is_empty() {
            self.can_copy(&known, &binding.known, false).map_err(|e| {
                Diagnostic::new(
                    span,
                    format!("assignment would lose an established guarantee: {e}"),
                )
            })?;
        }
        let node = if path.is_empty() {
            self.node(Kind::Alias(value.id), known.clone(), span)
        } else {
            self.node(
                Kind::Update(binding.value, path, value.id),
                known.clone(),
                span,
            )
        };
        let node = self.guard_alias_write(binding.identity, node, span);
        self.definitions.push(Definition {
            id: node.id,
            name: name.clone(),
            span,
            owner: self.owner,
        });
        let mut changed = binding;
        changed.value = node.id;
        changed.known = known;
        env.insert(name, changed);
        Ok(())
    }

    fn check_usefulness(&self) -> Result<(), Diagnostic> {
        let mut live_by_owner = BTreeMap::new();
        for (owner, root) in &self.roots {
            let mut live = BTreeSet::new();
            let mut pending = vec![*root];
            while let Some(id) = pending.pop() {
                if live.insert(id) {
                    pending.extend(self.nodes[id].kind.dependencies());
                }
            }
            live_by_owner.insert(*owner, live);
        }
        for definition in self.definitions.iter().rev() {
            if !live_by_owner.get(&definition.owner).is_some_and(|live| {
                let mut pending = vec![definition.id];
                let mut seen = BTreeSet::new();
                while let Some(id) = pending.pop() {
                    if live.contains(&id) {
                        return true;
                    }
                    if seen.insert(id) {
                        if let Some(sources) = self.implicit_outputs.get(&id) {
                            pending.extend(sources);
                        }
                    }
                }
                false
            }) {
                // Type-level use is a real use, even though it is erased before evaluation.
                if self.used_types.contains(&definition.id) {
                    continue;
                }
                return Err(Diagnostic::new(
                    definition.span,
                    format!(
                        "unused computation or overwritten value for `{}`",
                        definition.name
                    ),
                )
                .with_origin(self.nodes[definition.id].origin));
            }
        }
        Ok(())
    }
}
