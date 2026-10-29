use super::*;
use crate::check::Builtin;

impl Evaluation<'_> {
    fn ready_value(&mut self, value: Ready, span: Span) -> ThunkId {
        let id = self.thunks.len();
        if let Some(memo) = &mut self.incremental {
            memo.ready(id, &value);
        }
        self.register_ready(id, &value);
        self.thunks.push(Thunk {
            work: None,
            state: State::Ready(value),
            span,
            origin: 0,
        });
        id
    }

    fn string_arg(&mut self, id: Id, frame: FrameId, span: Span) -> Result<Rc<str>, DemandError> {
        match self.force_expr(id, frame)? {
            Ready::Str(value) => Ok(value),
            _ => Err(Diagnostic::new(span, "expected str argument").into()),
        }
    }

    pub(super) fn builtin(
        &mut self,
        op: Builtin,
        args: &[Id],
        frame: FrameId,
        span: Span,
    ) -> Result<Ready, DemandError> {
        use Builtin::*;
        Ok(match op {
            ArrayLen => {
                let Ready::Array(values) = self.force_expr(args[0], frame)? else {
                    return Err(Diagnostic::new(span, "expected array").into());
                };
                Ready::Int(
                    i64::try_from(values.len())
                        .map_err(|_| Diagnostic::new(span, "array length exceeds int"))?,
                )
            }
            ObjectLen | ObjectKeys | ObjectRemove | ObjectLookup | ObjectWith => {
                let Ready::Record(mut fields) = self.force_expr(args[0], frame)? else {
                    return Err(Diagnostic::new(span, "expected object").into());
                };
                match op {
                    ObjectLen => Ready::Int(
                        i64::try_from(fields.len())
                            .map_err(|_| Diagnostic::new(span, "object length exceeds int"))?,
                    ),
                    ObjectKeys => Ready::Array(
                        fields
                            .keys()
                            .map(|key| self.ready_value(Ready::Str(key.into()), span))
                            .collect(),
                    ),
                    _ => {
                        let key = self.string_arg(args[1], frame, span)?;
                        match op {
                            ObjectRemove => {
                                fields.remove(&key);
                                Ready::Record(fields)
                            }
                            ObjectLookup => Ready::Record(
                                fields
                                    .get(&key)
                                    .map(|id| ("value", *id))
                                    .into_iter()
                                    .collect(),
                            ),
                            ObjectWith => {
                                let value = self.thunk(args[2], frame)?;
                                fields.insert(key, value);
                                Ready::Record(fields)
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
            StringJoin => {
                let Ready::Array(values) = self.force_expr(args[0], frame)? else {
                    return Err(Diagnostic::new(span, "expected string array").into());
                };
                let separator = self.string_arg(args[1], frame, span)?;
                let mut strings = Vec::new();
                for value in values.iter().copied() {
                    let Ready::Str(value) = self.demand(value)? else {
                        return Err(Diagnostic::new(span, "expected string element").into());
                    };
                    strings.push(value);
                }
                Ready::Str(strings.join(&separator).into())
            }
            IntString => {
                let Ready::Int(value) = self.force_expr(args[0], frame)? else {
                    return Err(Diagnostic::new(span, "expected int").into());
                };
                Ready::Str(value.to_string().into())
            }
            BoolString => {
                let Ready::Bool(value) = self.force_expr(args[0], frame)? else {
                    return Err(Diagnostic::new(span, "expected bool").into());
                };
                Ready::Str(value.to_string().into())
            }
            _ => {
                let value = self.string_arg(args[0], frame, span)?;
                if matches!(op, StringLen) {
                    return Ok(Ready::Int(i64::try_from(value.chars().count()).map_err(
                        |_| Diagnostic::new(span, "string length exceeds int"),
                    )?));
                }
                if matches!(op, StringTrim) {
                    return Ok(Ready::Str(
                        value.trim_matches([' ', '\t', '\r', '\n']).into(),
                    ));
                }
                let needle = self.string_arg(args[1], frame, span)?;
                match op {
                    StringSplit if needle.is_empty() => Ready::Array(
                        value
                            .chars()
                            .map(|c| self.ready_value(Ready::Str(c.to_string().into()), span))
                            .collect(),
                    ),
                    StringSplit => Ready::Array(
                        value
                            .split(needle.as_ref())
                            .map(|s| self.ready_value(Ready::Str(s.into()), span))
                            .collect(),
                    ),
                    StringContains => Ready::Bool(value.contains(needle.as_ref())),
                    StringStartsWith => Ready::Bool(value.starts_with(needle.as_ref())),
                    StringEndsWith => Ready::Bool(value.ends_with(needle.as_ref())),
                    StringReplace => Ready::Str(
                        value
                            .replace(needle.as_ref(), &self.string_arg(args[2], frame, span)?)
                            .into(),
                    ),
                    _ => unreachable!(),
                }
            }
        })
    }

    pub(super) fn slice(
        &mut self,
        base: Id,
        start: Option<Id>,
        end: Option<Id>,
        frame: FrameId,
        span: Span,
    ) -> Result<Ready, DemandError> {
        let value = self.force_expr(base, frame)?;
        let length = match &value {
            Ready::Array(values) => values.len(),
            Ready::Str(s) => s.chars().count(),
            _ => return Err(Diagnostic::new(span, "expected array or str for slice").into()),
        };
        let mut bound = |id: Option<Id>, default| -> Result<usize, DemandError> {
            let Some(id) = id else { return Ok(default) };
            let Ready::Int(value) = self.force_expr(id, frame)? else {
                return Err(Diagnostic::new(span, "slice bound must be int").into());
            };
            usize::try_from(value)
                .map_err(|_| Diagnostic::new(span, "slice bound must be nonnegative").into())
        };
        let start = bound(start, 0)?;
        let end = bound(end, length)?;
        if start > end || end > length {
            return Err(Diagnostic::new(span, "slice requires 0 <= start <= end <= length").into());
        }
        Ok(match value {
            Ready::Array(values) => Ready::Array(values.slice(start, end)),
            Ready::Str(value) => Ready::Str(
                value
                    .chars()
                    .skip(start)
                    .take(end - start)
                    .collect::<String>()
                    .into(),
            ),
            _ => unreachable!(),
        })
    }
}
