use crate::cache::{CacheError, CachePolicy, CacheStats, CacheStore, EvaluationError};
use crate::{
    Diagnostic, LiteralType, Span, Type,
    check::{Drops, Graph, Id, Kind, Part},
    syntax::{Binary, Unary},
};
use std::{collections::BTreeMap, fmt, rc::Rc};
mod builtins;
mod identity;
mod incremental;
pub(crate) use incremental::{Computations, Scalar};
mod shared;
mod storage;
use shared::{Fields, Items};
use storage::{Storage, Stored};

/// Fully demanded data at the requested output path. Object keys are lexical.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Str(String),
    Array(#[serde(deserialize_with = "crate::codec::nested")] Vec<Value>),
    Record(#[serde(deserialize_with = "crate::codec::nested")] BTreeMap<String, Value>),
}

fn write_string(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in text.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if c < '\u{20}' => write!(f, "\\u{:04x}", c as u32)?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Str(s) => write_string(f, s),
            Self::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Self::Record(fields) => {
                f.write_str("{")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write_string(f, name)?;
                    write!(f, ":{value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

type ThunkId = usize;
type FrameId = usize;
#[derive(Clone, Debug)]
enum Ready {
    Int(i64),
    Bool(bool),
    Str(Rc<str>),
    Record(Fields),
    Array(Items),
    Function(Rc<Function>),
    Descriptor,
    Unit,
}
#[derive(Clone, Debug)]
struct Function {
    owner: usize,
    body: Id,
    captured: FrameId,
    types: Rc<BTreeMap<usize, Type>>,
    parameters: Vec<usize>,
}
#[derive(Clone)]
enum State {
    Suspended,
    Evaluating,
    Ready(Ready),
    Spilled(Stored),
    Failed(Rc<Diagnostic>),
}
#[derive(Clone)]
enum Work {
    Expr(Id, FrameId),
    Project(ThunkId, Type, Drops),
    Update(ThunkId, Vec<Key>, ThunkId),
    Forward(ThunkId),
    Loop(LoopWork),
}
#[derive(Clone)]
struct LoopWork {
    elements: Items,
    at: usize,
    state: Vec<ThunkId>,
    owner: usize,
    parent: FrameId,
    iteration: Option<FrameId>,
    updates: Vec<(String, usize, Id)>,
    exit: (Id, Id),
}
#[derive(Clone, serde::Serialize)]
enum Key {
    Field(String),
    Index(i64),
}
enum DemandError {
    Need(ThunkId),
    Failed(Diagnostic),
    Infrastructure(CacheError),
}
impl From<CacheError> for DemandError {
    fn from(error: CacheError) -> Self {
        Self::Infrastructure(error)
    }
}
impl From<Diagnostic> for DemandError {
    fn from(error: Diagnostic) -> Self {
        Self::Failed(error)
    }
}

struct Thunk {
    // Pending continuation storage is released when its result is memoized.
    // Keep its largest variant out of every scalar/container cache entry.
    work: Option<Box<Work>>,
    state: State,
    span: Span,
    origin: usize,
}
struct Frame {
    owner: usize,
    parent: Option<FrameId>,
    arguments: Vec<ThunkId>,
    cache: BTreeMap<Id, ThunkId>,
    types: Rc<BTreeMap<usize, Type>>,
}

/// A demand session. Storage and caches belong to the session, not global state.
pub struct Evaluation<'a> {
    graph: &'a Graph,
    frames: Vec<Frame>,
    thunks: Vec<Thunk>,
    forced: usize,
    materialize_depth: usize,
    storage: Option<Storage>,
    incremental: Option<incremental::Incremental<'a>>,
}

/// A session with host-provided spill storage. Infrastructure failures are
/// separate from memoized language failures and can be retried.
pub struct CachedEvaluation<'a> {
    inner: Evaluation<'a>,
}

impl<'a> CachedEvaluation<'a> {
    pub(crate) fn new(graph: &'a Graph, store: Box<dyn CacheStore>, policy: CachePolicy) -> Self {
        let mut inner = Evaluation::new(graph);
        inner.storage = Some(Storage::new(store, policy));
        Self { inner }
    }
    pub fn select(&mut self, path: &[&str]) -> Result<Value, EvaluationError> {
        let value = self.inner.select_inner(path)?;
        self.inner.trim_cache()?;
        Ok(value)
    }
    pub fn forced_nodes(&self) -> usize {
        self.inner.forced_nodes()
    }
    pub fn cache_stats(&self) -> CacheStats {
        self.inner
            .storage
            .as_ref()
            .map_or_else(CacheStats::default, |s| s.stats.clone())
    }
    /// Explicitly release storage, reporting cleanup failure. Drop only makes
    /// a best-effort release when this method was not called.
    pub fn close(mut self) -> Result<(), CacheError> {
        if let Some(mut storage) = self.inner.storage.take() {
            storage.store.release()?;
        }
        Ok(())
    }
}

impl Drop for CachedEvaluation<'_> {
    fn drop(&mut self) {
        if let Some(storage) = &mut self.inner.storage {
            let _ = storage.store.release();
        }
    }
}

impl<'a> Evaluation<'a> {
    pub(crate) fn new(graph: &'a Graph) -> Self {
        Self {
            graph,
            frames: vec![Frame {
                owner: 0,
                parent: None,
                arguments: Vec::new(),
                cache: BTreeMap::new(),
                types: Rc::default(),
            }],
            thunks: Vec::new(),
            forced: 0,
            materialize_depth: 0,
            storage: None,
            incremental: None,
        }
    }

    /// Number of evaluations started from suspended nodes. Retrying a demand
    /// interrupted by an infrastructure failure can count an attempt again;
    /// reloading ready data and repeating memoized language failures do not.
    pub fn forced_nodes(&self) -> usize {
        self.forced
    }

    pub fn select(&mut self, path: &[&str]) -> Result<Value, Diagnostic> {
        self.select_inner(path).map_err(|error| match error {
            EvaluationError::Language(error) => error,
            EvaluationError::Infrastructure(_) => {
                unreachable!("uncached evaluation has no storage")
            }
        })
    }

    fn select_inner(&mut self, path: &[&str]) -> Result<Value, EvaluationError> {
        let root = &self.graph.nodes[self.graph.root];
        root.ty.select(path, root.span)?;
        let mut current = self.thunk(self.graph.root, 0)?;
        for field in path {
            let span = self.thunks[current].span;
            current = match self.force(current)? {
                Ready::Record(fields) => fields.get(field).copied().ok_or_else(|| {
                    Diagnostic::new(span, format!("selected field `{field}` is absent"))
                })?,
                Ready::Array(items) => field
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| items.get(i).copied())
                    .ok_or_else(|| {
                        Diagnostic::new(span, "selected array index is out of bounds")
                    })?,
                _ => {
                    return Err(Diagnostic::new(
                        span,
                        format!("cannot select `{field}` from a non-object"),
                    )
                    .into());
                }
            };
        }
        self.materialize(current)
    }

    fn thunk(&mut self, id: Id, mut frame: FrameId) -> Result<ThunkId, Diagnostic> {
        let node = &self.graph.nodes[id];
        while self.frames[frame].owner != node.owner {
            frame = self.frames[frame]
                .parent
                .ok_or_else(|| Diagnostic::new(node.span, "missing lexical evaluation frame"))?;
        }
        if let Some(thunk) = self.frames[frame].cache.get(&id) {
            return Ok(*thunk);
        }
        let index = self.thunks.len();
        if let Some(memo) = &mut self.incremental {
            memo.work(index, &Work::Expr(id, frame));
        }
        self.thunks.push(Thunk {
            work: Some(Box::new(Work::Expr(id, frame))),
            state: State::Suspended,
            span: node.span,
            origin: node.origin,
        });
        self.frames[frame].cache.insert(id, index);
        Ok(index)
    }

    fn deferred(&mut self, work: Work, span: Span) -> ThunkId {
        let id = self.thunks.len();
        if let Some(memo) = &mut self.incremental {
            memo.work(id, &work);
        }
        let origin = match &work {
            Work::Expr(id, _) => self.graph.nodes[*id].origin,
            Work::Project(id, ..) | Work::Update(id, ..) | Work::Forward(id) => {
                self.thunks[*id].origin
            }
            Work::Loop(_) => 0,
        };
        self.thunks.push(Thunk {
            work: Some(Box::new(work)),
            state: State::Suspended,
            span,
            origin,
        });
        id
    }

    fn force_expr(&mut self, id: Id, frame: FrameId) -> Result<Ready, DemandError> {
        let thunk = self.thunk(id, frame)?;
        self.demand(thunk)
    }

    // Internal evaluation only requests dependencies. The driver below runs
    // them on an explicit stack, so an Lonk call never nests a Rust call.
    fn demand(&mut self, id: ThunkId) -> Result<Ready, DemandError> {
        if matches!(self.thunks[id].state, State::Suspended) {
            self.memoized(id)?;
        }
        if let State::Spilled(stored) = self.thunks[id].state {
            let storage = self
                .storage
                .as_mut()
                .unwrap_or_else(|| unreachable!("spilled value has a store"));
            let value = stored.load(storage, self.thunks.len())?;
            self.register_ready(id, &value);
            self.thunks[id].state = State::Ready(value);
        }
        match &self.thunks[id].state {
            State::Ready(value) => Ok(value.clone()),
            State::Failed(error) => Err(DemandError::Failed((**error).clone())),
            State::Suspended | State::Evaluating => Err(DemandError::Need(id)),
            State::Spilled(_) => unreachable!("loaded above"),
        }
    }

    fn force(&mut self, id: ThunkId) -> Result<Ready, EvaluationError> {
        let value = self.force_inner(id)?;
        self.trim_cache()?;
        Ok(value)
    }

    fn force_inner(&mut self, id: ThunkId) -> Result<Ready, EvaluationError> {
        match self.demand(id) {
            Ok(value) => return Ok(value),
            Err(DemandError::Failed(error)) => return Err(error.into()),
            Err(DemandError::Infrastructure(error)) => return Err(error.into()),
            Err(DemandError::Need(_)) => (),
        }
        let mut pending = vec![id];
        self.thunks[id].state = State::Evaluating;
        self.forced += 1;
        while let Some(current) = pending.last().copied() {
            let outcome = self.run_work(current).and_then(|value| {
                if let Some(memo) = &mut self.incremental {
                    memo.save(current, &value)?;
                }
                Ok(value)
            });
            let error = match outcome {
                Ok(value) => {
                    self.register_ready(current, &value);
                    self.thunks[current].state = State::Ready(value.clone());
                    self.thunks[current].work = None;
                    pending.pop();
                    if pending.is_empty() {
                        return Ok(value);
                    }
                    continue;
                }
                Err(DemandError::Need(dependency)) => {
                    if matches!(self.thunks[dependency].state, State::Evaluating) {
                        Diagnostic::new(
                            self.thunks[dependency].span,
                            "demand cycle while evaluating this value",
                        )
                    } else if pending.len() >= 131_072 {
                        Diagnostic::new(
                            self.thunks[current].span,
                            "evaluation nesting limit exceeded (131072 pending demands)",
                        )
                    } else {
                        self.thunks[dependency].state = State::Evaluating;
                        self.forced += 1;
                        pending.push(dependency);
                        continue;
                    }
                }
                Err(DemandError::Failed(error)) => error,
                Err(DemandError::Infrastructure(error)) => {
                    // Work and completed dependencies remain valid. An I/O
                    // outage must not poison this chain as a language failure.
                    for waiting in pending {
                        self.thunks[waiting].state = State::Suspended;
                    }
                    return Err(error.into());
                }
            };
            let error = error.with_origin(self.thunks[current].origin);
            let shared = Rc::new(error.clone());
            for waiting in pending {
                self.thunks[waiting].state = State::Failed(shared.clone());
                self.thunks[waiting].work = None;
            }
            return Err(error.into());
        }
        Err(Diagnostic::new(self.thunks[id].span, "empty evaluation work stack").into())
    }

    fn run_work(&mut self, id: ThunkId) -> Result<Ready, DemandError> {
        let span = self.thunks[id].span;
        let work = self.thunks[id].work.as_deref().cloned().ok_or_else(|| {
            Diagnostic::new(span, "internal invariant: suspended value has no work")
        })?;
        match work {
            Work::Expr(node, frame) => self.eval(node, frame, id),
            Work::Forward(target) => self.demand(target),
            Work::Loop(work) => self.run_loop(id, work),
            Work::Project(source, ty, drops) => self.project(source, &ty, &drops, span),
            Work::Update(source, path, value) => self.update(source, &path, value, span),
        }
    }

    fn eval(&mut self, id: Id, frame: FrameId, current: ThunkId) -> Result<Ready, DemandError> {
        let node = &self.graph.nodes[id];
        let span = node.span;
        Ok(match &node.kind {
            Kind::Int(n) => Ready::Int(*n),
            Kind::Bool(b) => Ready::Bool(*b),
            Kind::Str(s) => Ready::Str(s.as_str().into()),
            Kind::Unit => Ready::Unit,
            Kind::Builtin(op, args) => self.builtin(*op, args, frame, span)?,
            Kind::Slice(base, start, end) => self.slice(*base, *start, *end, frame, span)?,
            Kind::Descriptor => Ready::Descriptor,
            Kind::Alias(source) => self.force_expr(*source, frame)?,
            Kind::After(check, value) => {
                self.force_expr(*check, frame)?;
                self.force_expr(*value, frame)?
            }
            Kind::BorrowCheck(base, path, guard) => {
                if let Some(guard) = guard {
                    self.force_expr(*guard, frame)?;
                }
                self.validate_borrow(*base, path, frame, span)?;
                Ready::Unit
            }
            Kind::Record(entries) => {
                let mut fields = Fields::default();
                for (name, value) in entries {
                    if let Some(name) = name {
                        fields.insert(name.clone(), self.thunk(*value, frame)?);
                    } else {
                        let Ready::Record(more) = self.force_expr(*value, frame)? else {
                            return Err(
                                Diagnostic::new(span, "spread source is not an object").into()
                            );
                        };
                        fields.extend(more);
                    }
                }
                Ready::Record(fields)
            }
            Kind::Array(elements) => {
                let mut values = Items::default();
                for (spread, id) in elements {
                    if *spread {
                        let Ready::Array(more) = self.force_expr(*id, frame)? else {
                            return Err(Diagnostic::new(
                                span,
                                "array spread source is not an array",
                            )
                            .into());
                        };
                        values.len().checked_add(more.len()).ok_or_else(|| {
                            Diagnostic::new(span, "array length exceeds the supported size")
                        })?;
                        values.extend(more);
                    } else {
                        values.len().checked_add(1).ok_or_else(|| {
                            Diagnostic::new(span, "array length exceeds the supported size")
                        })?;
                        values.push(self.thunk(*id, frame)?);
                    }
                }
                Ready::Array(values)
            }
            Kind::Field(base, name) => {
                let Ready::Record(fields) = self.force_expr(*base, frame)? else {
                    return Err(Diagnostic::new(span, "field access on a non-object").into());
                };
                let value = fields
                    .get(name)
                    .ok_or_else(|| Diagnostic::new(span, format!("field `{name}` is absent")))?;
                self.demand(*value)?
            }
            Kind::Index(base, index) => {
                let value = self.force_expr(*base, frame)?;
                let index = self.force_expr(*index, frame)?;
                let selected = match (value, index) {
                    (Ready::Array(items), Ready::Int(index)) => usize::try_from(index)
                        .ok()
                        .and_then(|i| items.get(i).copied()),
                    (Ready::Record(fields), Ready::Str(key)) => fields.get(&key).copied(),
                    _ => None,
                }
                .ok_or_else(|| Diagnostic::new(span, "index is absent or out of bounds"))?;
                self.demand(selected)?
            }
            Kind::Unary(op, value) => match (op, self.force_expr(*value, frame)?) {
                (Unary::Not, Ready::Bool(b)) => Ready::Bool(!b),
                (Unary::Negate, Ready::Int(n)) => Ready::Int(
                    n.checked_neg()
                        .ok_or_else(|| Diagnostic::new(span, "integer overflow"))?,
                ),
                _ => return Err(Diagnostic::new(span, "invalid unary operand").into()),
            },
            Kind::Binary(op, left, right) => self.binary(*op, *left, *right, frame, span)?,
            Kind::Project(source, ty, dropped) => {
                let source = self.thunk(*source, frame)?;
                let ty = ty.substitute(&self.frames[frame].types);
                let dropped = dropped.substitute(&self.frames[frame].types);
                self.project(source, &ty, &dropped, span)?
            }
            Kind::Update(source, path, value) => {
                let source = self.thunk(*source, frame)?;
                // As with a read, demand the subject before the index expressions.
                // Nested children and their bounds remain lazy in Work::Update.
                self.demand(source)?;
                let value = self.thunk(*value, frame)?;
                let mut keys = Vec::new();
                for part in path {
                    keys.push(match part {
                        Part::Field(name) => Key::Field(name.clone()),
                        Part::Index(index) => match self.force_expr(*index, frame)? {
                            Ready::Int(n) => Key::Index(n),
                            _ => {
                                return Err(Diagnostic::new(span, "array index must be int").into());
                            }
                        },
                    });
                }
                self.update(source, &keys, value, span)?
            }
            Kind::If(condition, yes, no) => {
                let Ready::Bool(condition) = self.force_expr(*condition, frame)? else {
                    return Err(Diagnostic::new(span, "condition must be bool").into());
                };
                self.force_expr(if condition { *yes } else { *no }, frame)?
            }
            Kind::Is(value, target) => {
                let target = target.substitute(&self.frames[frame].types);
                let known = self.graph.nodes[*value]
                    .ty
                    .substitute(&self.frames[frame].types);
                if known.guarantees(&target) {
                    Ready::Bool(true)
                } else {
                    let value = self.thunk(*value, frame)?;
                    Ready::Bool(self.matches(value, &target, &known)?)
                }
            }
            Kind::LiteralTest(value, literal) => {
                Ready::Bool(match (self.force_expr(*value, frame)?, literal) {
                    (Ready::Int(a), crate::syntax::Literal::Int(b)) => a == *b,
                    (Ready::Bool(a), crate::syntax::Literal::Bool(b)) => a == *b,
                    (Ready::Str(a), crate::syntax::Literal::Str(b)) => a.as_ref() == b,
                    _ => false,
                })
            }
            Kind::Function {
                owner,
                body,
                parameters,
                ..
            } => Ready::Function(Rc::new(Function {
                owner: *owner,
                body: *body,
                captured: frame,
                types: self.frames[frame].types.clone(),
                parameters: parameters.clone(),
            })),
            Kind::Instantiate(function, arguments) => {
                let value = self.force_expr(*function, frame)?;
                let arguments: Vec<_> = arguments
                    .iter()
                    .map(|t| t.substitute(&self.frames[frame].types))
                    .collect();
                Self::instantiate(value, &arguments, span)?
            }
            Kind::Parameter(index, default) => {
                let value = if let Some(value) = self.frames[frame].arguments.get(*index) {
                    *value
                } else if let Some(default) = default {
                    self.thunk(*default, frame)?
                } else {
                    return Err(Diagnostic::new(span, "missing function argument").into());
                };
                self.demand(value)?
            }
            Kind::Import(_) => {
                return Err(
                    Diagnostic::new(span, "internal invariant: unlinked module import").into(),
                );
            }
            Kind::Call(function, arguments, places) => {
                for check in places {
                    self.force_expr(*check, frame)?;
                }
                let Ready::Function(function) = self.force_expr(*function, frame)? else {
                    return Err(Diagnostic::new(span, "call target is not a function").into());
                };
                let args = arguments
                    .iter()
                    .map(|id| self.thunk(*id, frame))
                    .collect::<Result<Vec<_>, _>>()?;
                let call = self.push_frame(Frame {
                    owner: function.owner,
                    parent: Some(function.captured),
                    arguments: args,
                    cache: BTreeMap::new(),
                    types: function.types.clone(),
                });
                let body = self.thunk(function.body, call)?;
                // Allocate each call frame once, even if its body suspends.
                self.thunks[current].work = Some(Box::new(Work::Forward(body)));
                self.demand(body)?
            }
            Kind::For {
                source,
                owner,
                initial,
                updates,
                exit,
                ..
            } => {
                let Ready::Array(elements) = self.force_expr(*source, frame)? else {
                    return Err(Diagnostic::new(span, "iteration source is not an array").into());
                };
                let mut state = initial
                    .iter()
                    .map(|id| self.thunk(*id, frame))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(exit) = exit {
                    let work = LoopWork {
                        elements,
                        at: 0,
                        state,
                        owner: *owner,
                        parent: frame,
                        iteration: None,
                        updates: updates.clone(),
                        exit: *exit,
                    };
                    self.thunks[current].work = Some(Box::new(Work::Loop(work.clone())));
                    return self.run_loop(current, work);
                }
                for element in elements.iter().copied() {
                    let iteration = self.push_frame(Frame {
                        owner: *owner,
                        parent: Some(frame),
                        arguments: std::iter::once(element)
                            .chain(state.iter().copied())
                            .collect(),
                        cache: BTreeMap::new(),
                        types: self.frames[frame].types.clone(),
                    });
                    for (_, index, value) in updates {
                        state[*index] = self.thunk(*value, iteration)?;
                    }
                }
                Ready::Record(
                    updates
                        .iter()
                        .map(|(name, index, _)| (name.clone(), state[*index]))
                        .collect(),
                )
            }
        })
    }

    fn run_loop(&mut self, current: ThunkId, mut work: LoopWork) -> Result<Ready, DemandError> {
        let span = self.thunks[current].span;
        while work.at < work.elements.len() {
            let iteration = if let Some(iteration) = work.iteration {
                iteration
            } else {
                let iteration =
                    self.push_frame(Frame {
                        owner: work.owner,
                        parent: Some(work.parent),
                        arguments: std::iter::once(*work.elements.get(work.at).ok_or_else(
                            || Diagnostic::new(span, "internal invariant: loop index"),
                        )?)
                        .chain(work.state.iter().copied())
                        .collect(),
                        cache: BTreeMap::new(),
                        types: self.frames[work.parent].types.clone(),
                    });
                work.iteration = Some(iteration);
                iteration
            };
            // Preserve the iteration frame across a suspended predicate.
            self.thunks[current].work = Some(Box::new(Work::Loop(work.clone())));
            let flag = self.thunk(work.exit.0, iteration)?;
            let Ready::Bool(returned) = self.demand(flag)? else {
                return Err(Diagnostic::new(span, "invalid loop return condition").into());
            };
            if returned {
                let output = self.thunk(work.exit.1, iteration)?;
                let mut fields: Fields = work
                    .updates
                    .iter()
                    .map(|(n, i, _)| (n.clone(), work.state[*i]))
                    .collect();
                fields.insert("$returned", flag);
                fields.insert("$exit", output);
                return Ok(Ready::Record(fields));
            }
            for (_, index, value) in &work.updates {
                work.state[*index] = self.thunk(*value, iteration)?;
            }
            work.at += 1;
            work.iteration = None;
        }
        let flag = self.thunks.len();
        if let Some(memo) = &mut self.incremental {
            memo.ready(flag, &Ready::Bool(false));
        }
        self.thunks.push(Thunk {
            work: None,
            state: State::Ready(Ready::Bool(false)),
            span,
            origin: self.thunks[current].origin,
        });
        let mut fields: Fields = work
            .updates
            .iter()
            .map(|(n, i, _)| (n.clone(), work.state[*i]))
            .collect();
        fields.insert("$returned", flag);
        Ok(Ready::Record(fields))
    }

    fn validate_borrow(
        &mut self,
        base: Id,
        path: &[Part],
        frame: FrameId,
        span: Span,
    ) -> Result<(), DemandError> {
        let mut current = self.thunk(base, frame)?;
        for part in path {
            current = match (self.demand(current)?, part) {
                (Ready::Record(fields), Part::Field(name)) => fields
                    .get(name)
                    .copied()
                    .ok_or_else(|| Diagnostic::new(span, "borrowed field is absent"))?,
                (Ready::Array(items), Part::Index(index)) => {
                    let Ready::Int(index) = self.force_expr(*index, frame)? else {
                        return Err(Diagnostic::new(span, "borrow index must be int").into());
                    };
                    usize::try_from(index)
                        .ok()
                        .and_then(|index| items.get(index).copied())
                        .ok_or_else(|| {
                            Diagnostic::new(span, "borrow index is absent or out of bounds")
                        })?
                }
                _ => return Err(Diagnostic::new(span, "invalid borrowed path").into()),
            };
        }
        // Establish the location without demanding the old leaf payload.
        Ok(())
    }

    fn instantiate(value: Ready, arguments: &[Type], span: Span) -> Result<Ready, DemandError> {
        let Ready::Function(mut function) = value else {
            return Err(Diagnostic::new(span, "generic target is not a function").into());
        };
        if function.parameters.len() != arguments.len() {
            return Err(Diagnostic::new(span, "generic callable parameter count mismatch").into());
        }
        let data = Rc::make_mut(&mut function);
        let types = Rc::make_mut(&mut data.types);
        for (id, ty) in data.parameters.iter().copied().zip(arguments) {
            if !ty.within_depth(64) {
                return Err(
                    Diagnostic::new(span, "generic type nesting limit exceeded (64)").into(),
                );
            }
            types.insert(id, ty.clone());
        }
        data.parameters.clear();
        Ok(Ready::Function(function))
    }

    fn project(
        &mut self,
        source: ThunkId,
        ty: &Type,
        drops: &Drops,
        span: Span,
    ) -> Result<Ready, DemandError> {
        let value = self.demand(source)?;
        self.project_value(value, ty, drops, span)
    }

    fn project_value(
        &mut self,
        value: Ready,
        ty: &Type,
        drops: &Drops,
        span: Span,
    ) -> Result<Ready, DemandError> {
        let value = if let Some(arguments) = &drops.instantiate {
            Self::instantiate(value, arguments, span)?
        } else {
            value
        };
        if let Type::Union(cases) = ty {
            let case = Self::variant_case(&value, cases)
                .ok_or_else(|| Diagnostic::new(span, "value has no unambiguous variant case"))?;
            return self.project_value(value, case, drops, span);
        }
        match (value, ty) {
            (value, Type::Literal(literal)) if Self::literal_matches(&value, literal) => Ok(value),
            (
                Ready::Record(mut fields),
                Type::Record {
                    fields: target,
                    open,
                    row: None,
                },
            ) => {
                if *open {
                    for name in &drops.names {
                        fields.remove(name);
                    }
                } else {
                    fields = target
                        .keys()
                        .filter_map(|name| {
                            fields
                                .get(name)
                                .filter(|_| !drops.names.contains(name))
                                .map(|id| (name.clone(), *id))
                        })
                        .collect();
                }
                for (name, field) in target.iter() {
                    if let Some(source) = fields.get(name).copied() {
                        fields.insert(
                            name.clone(),
                            self.deferred(
                                Work::Project(
                                    source,
                                    field.ty.clone(),
                                    drops.fields.get(name).cloned().unwrap_or_default(),
                                ),
                                span,
                            ),
                        );
                    } else if !field.optional {
                        return Err(Diagnostic::new(
                            span,
                            format!("missing projected field `{name}`"),
                        )
                        .into());
                    }
                }
                Ok(Ready::Record(fields))
            }
            (Ready::Array(elements), Type::Array(element)) => Ok(Ready::Array(
                elements
                    .iter()
                    .copied()
                    .map(|source| {
                        self.deferred(
                            Work::Project(
                                source,
                                *element.clone(),
                                drops.element.as_deref().cloned().unwrap_or_default(),
                            ),
                            span,
                        )
                    })
                    .collect(),
            )),
            (value, Type::Any) => Ok(value),
            (value @ Ready::Int(_), Type::Int)
            | (value @ Ready::Bool(_), Type::Bool)
            | (value @ Ready::Str(_), Type::Str)
            | (value @ Ready::Function(_), Type::Function { .. })
            | (value @ Ready::Function(_), Type::Generic { .. })
            | (value @ Ready::Descriptor, Type::Descriptor(_)) => Ok(value),
            _ => Err(Diagnostic::new(span, "value does not satisfy its checked projection").into()),
        }
    }

    fn update(
        &mut self,
        source: ThunkId,
        path: &[Key],
        value: ThunkId,
        span: Span,
    ) -> Result<Ready, DemandError> {
        let Some((key, rest)) = path.split_first() else {
            return self.demand(value);
        };
        match (self.demand(source)?, key) {
            (Ready::Record(mut fields), Key::Field(name)) => {
                let replacement = if rest.is_empty() {
                    value
                } else {
                    let child = fields.get(name).copied().ok_or_else(|| {
                        Diagnostic::new(span, format!("nested field `{name}` is absent"))
                    })?;
                    self.deferred(Work::Update(child, rest.to_vec(), value), span)
                };
                fields.insert(name.clone(), replacement);
                Ok(Ready::Record(fields))
            }
            (Ready::Array(mut elements), Key::Index(index)) => {
                let index = usize::try_from(*index)
                    .map_err(|_| Diagnostic::new(span, "array update index out of bounds"))?;
                let previous = elements
                    .get(index)
                    .copied()
                    .ok_or_else(|| Diagnostic::new(span, "array update index out of bounds"))?;
                let replacement = if rest.is_empty() {
                    value
                } else {
                    self.deferred(Work::Update(previous, rest.to_vec(), value), span)
                };
                elements.set(index, replacement);
                Ok(Ready::Array(elements))
            }
            _ => Err(Diagnostic::new(span, "update path does not match the value").into()),
        }
    }

    fn literal_matches(value: &Ready, literal: &LiteralType) -> bool {
        match (value, literal) {
            (Ready::Int(a), LiteralType::Int(b)) => a == b,
            (Ready::Bool(a), LiteralType::Bool(b)) => a == b,
            (Ready::Str(a), LiteralType::Str(b)) => a.as_ref() == b,
            _ => false,
        }
    }

    // Selection uses only the outer kind, literal value, and required tag
    // presence. Looking at payloads here would destroy variant laziness.
    fn variant_case<'t>(value: &Ready, cases: &'t [Type]) -> Option<&'t Type> {
        let mut selected = None;
        for case in cases {
            let applies = match (value, case) {
                (_, Type::Literal(literal)) => Self::literal_matches(value, literal),
                (Ready::Record(fields), Type::Record { fields: tags, .. }) => {
                    tags.keys().all(|tag| fields.contains_key(tag))
                }
                _ => false,
            };
            if applies {
                if selected.is_some() {
                    return None;
                }
                selected = Some(case);
            }
        }
        selected
    }

    fn matches(&mut self, id: ThunkId, ty: &Type, known: &Type) -> Result<bool, DemandError> {
        if known.guarantees(ty) {
            return Ok(true);
        }
        let value = self.demand(id)?;
        let known = if let Type::Union(cases) = known {
            Self::variant_case(&value, cases).unwrap_or(&Type::Any)
        } else {
            known
        };
        if known.guarantees(ty) {
            return Ok(true);
        }
        if let Type::Union(cases) = ty {
            return match Self::variant_case(&value, cases) {
                Some(case) => self.matches(id, case, known),
                None => Ok(false),
            };
        }
        Ok(match (value, ty) {
            (value, Type::Literal(literal)) => Self::literal_matches(&value, literal),
            (Ready::Int(_), Type::Int)
            | (Ready::Bool(_), Type::Bool)
            | (Ready::Str(_), Type::Str) => true,
            (
                Ready::Record(fields),
                Type::Record {
                    fields: target,
                    open,
                    row: None,
                },
            ) => {
                if !open && fields.keys().any(|name| !target.contains_key(name)) {
                    return Ok(false);
                }
                for (name, field) in target.iter() {
                    let child_known = match known {
                        Type::Record { fields, .. } => {
                            fields.get(name).map(|f| &f.ty).unwrap_or(&Type::Any)
                        }
                        _ => &Type::Any,
                    };
                    match fields.get(name) {
                        Some(value) if !self.matches(*value, &field.ty, child_known)? => {
                            return Ok(false);
                        }
                        None if !field.optional => return Ok(false),
                        _ => (),
                    }
                }
                true
            }
            (Ready::Array(values), Type::Array(element)) => {
                let child_known = match known {
                    Type::Array(t) => t.as_ref(),
                    _ => &Type::Any,
                };
                for value in values.iter().copied() {
                    if !self.matches(value, element, child_known)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => false,
        })
    }

    fn binary(
        &mut self,
        op: Binary,
        left: Id,
        right: Id,
        frame: FrameId,
        span: Span,
    ) -> Result<Ready, DemandError> {
        use Binary::*;
        let a = self.force_expr(left, frame)?;
        if op == And && matches!(a, Ready::Bool(false)) {
            return Ok(Ready::Bool(false));
        }
        if op == Or && matches!(a, Ready::Bool(true)) {
            return Ok(Ready::Bool(true));
        }
        let b = self.force_expr(right, frame)?;
        if matches!(op, Equal | NotEqual) {
            let left = self.thunk(left, frame)?;
            let right = self.thunk(right, frame)?;
            let equal = self.equal(left, right, 0, span)?;
            return Ok(Ready::Bool(if op == Equal { equal } else { !equal }));
        }
        match (a, b) {
            (Ready::Int(a), Ready::Int(b)) => {
                let arithmetic = match op {
                    Add => Some(a.checked_add(b)),
                    Subtract => Some(a.checked_sub(b)),
                    Multiply => Some(a.checked_mul(b)),
                    Divide => Some(a.checked_div(b)),
                    Remainder => Some(a.checked_rem(b)),
                    _ => None,
                };
                if let Some(value) = arithmetic {
                    return value
                        .map(Ready::Int)
                        .ok_or_else(|| {
                            Diagnostic::new(
                                span,
                                if b == 0 && matches!(op, Divide | Remainder) {
                                    "division by zero"
                                } else {
                                    "integer overflow"
                                },
                            )
                        })
                        .map_err(Into::into);
                }
                Ok(Ready::Bool(match op {
                    Less => a < b,
                    LessEqual => a <= b,
                    Greater => a > b,
                    GreaterEqual => a >= b,
                    _ => return Err(Diagnostic::new(span, "invalid integer operator").into()),
                }))
            }
            (Ready::Str(a), Ready::Str(b)) => match op {
                Add => Ok(Ready::Str(format!("{a}{b}").into())),
                Less => Ok(Ready::Bool(a < b)),
                LessEqual => Ok(Ready::Bool(a <= b)),
                Greater => Ok(Ready::Bool(a > b)),
                GreaterEqual => Ok(Ready::Bool(a >= b)),
                _ => Err(Diagnostic::new(span, "invalid string operator").into()),
            },
            (Ready::Bool(a), Ready::Bool(b)) => match op {
                And => Ok(Ready::Bool(a && b)),
                Or => Ok(Ready::Bool(a || b)),
                _ => Err(Diagnostic::new(span, "invalid boolean operator").into()),
            },
            (Ready::Str(key), Ready::Record(fields)) if op == In => {
                Ok(Ready::Bool(fields.contains_key(&key)))
            }
            _ => Err(Diagnostic::new(span, "invalid binary operands").into()),
        }
    }

    fn equal(
        &mut self,
        a: ThunkId,
        b: ThunkId,
        depth: usize,
        span: Span,
    ) -> Result<bool, DemandError> {
        if depth >= 64 {
            return Err(Diagnostic::new(span, "equality nesting limit exceeded (64)").into());
        }
        Ok(match (self.demand(a)?, self.demand(b)?) {
            (Ready::Int(a), Ready::Int(b)) => a == b,
            (Ready::Bool(a), Ready::Bool(b)) => a == b,
            (Ready::Str(a), Ready::Str(b)) => a == b,
            (Ready::Array(a), Ready::Array(b)) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                for (a, b) in a.iter().copied().zip(b.iter().copied()) {
                    if !self.equal(a, b, depth + 1, span)? {
                        return Ok(false);
                    }
                }
                true
            }
            (Ready::Record(a), Ready::Record(b)) => {
                if !a.keys().eq(b.keys()) {
                    return Ok(false);
                }
                for (a, b) in a.values().zip(b.values()) {
                    if !self.equal(*a, *b, depth + 1, span)? {
                        return Ok(false);
                    }
                }
                true
            }
            (
                Ready::Int(_) | Ready::Bool(_) | Ready::Str(_) | Ready::Array(_) | Ready::Record(_),
                Ready::Int(_) | Ready::Bool(_) | Ready::Str(_) | Ready::Array(_) | Ready::Record(_),
            ) => false,
            _ => return Err(Diagnostic::new(span, "equality requires checked data values").into()),
        })
    }

    fn materialize(&mut self, id: ThunkId) -> Result<Value, EvaluationError> {
        if self.materialize_depth >= 64 {
            return Err(Diagnostic::new(
                self.thunks[id].span,
                "export nesting limit exceeded (64)",
            )
            .with_origin(self.thunks[id].origin)
            .into());
        }
        self.materialize_depth += 1;
        let result = self.materialize_value(id);
        self.materialize_depth -= 1;
        result.map_err(|e| match e {
            EvaluationError::Language(error) => {
                EvaluationError::Language(error.with_origin(self.thunks[id].origin))
            }
            other => other,
        })
    }

    fn materialize_value(&mut self, id: ThunkId) -> Result<Value, EvaluationError> {
        Ok(match self.force(id)? {
            Ready::Int(n) => Value::Int(n),
            Ready::Bool(b) => Value::Bool(b),
            Ready::Str(s) => Value::Str(s.to_string()),
            Ready::Array(items) => Value::Array(
                items
                    .iter()
                    .copied()
                    .map(|id| self.materialize(id))
                    .collect::<Result<_, _>>()?,
            ),
            Ready::Record(fields) => Value::Record(
                fields
                    .iter()
                    .map(|(name, id)| self.materialize(*id).map(|value| (name.to_owned(), value)))
                    .collect::<Result<_, _>>()?,
            ),
            _ => {
                return Err(Diagnostic::new(
                    self.thunks[id].span,
                    "only data values can be exported; select a data field instead",
                )
                .into());
            }
        })
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::Program;

    #[test]
    fn completed_thunks_release_continuations_and_share_failures() {
        let program = Program::prepare(
            "let identity = fn<T>(x: T) -> T { x }; {good = identity(42); bad = identity(1 / 0); pending = identity(7)}",
        )
        .unwrap();
        let mut evaluation = program.evaluator();
        assert_eq!(evaluation.select(&["good"]).unwrap(), Value::Int(42));
        let forced = evaluation.forced_nodes();
        assert_eq!(evaluation.select(&["good"]).unwrap(), Value::Int(42));
        assert_eq!(evaluation.forced_nodes(), forced);
        let first_error = evaluation.select(&["bad"]).unwrap_err();
        let after_failure = evaluation.forced_nodes();
        assert_eq!(evaluation.select(&["bad"]).unwrap_err(), first_error);
        assert_eq!(evaluation.forced_nodes(), after_failure);
        let mut shared_error = None;
        for thunk in &evaluation.thunks {
            match &thunk.state {
                State::Ready(_) => assert!(thunk.work.is_none()),
                State::Failed(error) => {
                    assert!(thunk.work.is_none());
                    if let Some(first) = &shared_error {
                        assert!(Rc::ptr_eq(first, error));
                    } else {
                        shared_error = Some(error.clone());
                    }
                }
                State::Suspended => assert!(thunk.work.is_some()),
                State::Spilled(_) => panic!("uncached evaluation cannot spill"),
                State::Evaluating => panic!("no work should remain active after select"),
            }
        }
        assert!(shared_error.is_some());
        assert_eq!(evaluation.select(&["pending"]).unwrap(), Value::Int(7));
    }

    #[test]
    fn cached_slots_do_not_embed_closure_or_loop_storage() {
        assert!(std::mem::size_of::<Ready>() <= 3 * std::mem::size_of::<usize>());
        assert!(std::mem::size_of::<State>() <= 4 * std::mem::size_of::<usize>());
        assert!(std::mem::size_of::<Thunk>() <= 8 * std::mem::size_of::<usize>());
    }
}
