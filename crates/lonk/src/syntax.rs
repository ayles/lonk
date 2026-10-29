use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    fn through(self, other: Self) -> Self {
        Self {
            start: self.start,
            end: other.end,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub(crate) origin: Option<usize>,
}

impl Diagnostic {
    pub(crate) fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            origin: None,
        }
    }

    pub(crate) fn with_origin(mut self, origin: usize) -> Self {
        self.origin.get_or_insert(origin);
        self
    }

    pub fn render(&self, source: &str) -> String {
        let start = self.span.start.min(source.len());
        let prefix = source.get(..start).unwrap_or("");
        let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!("{line}:{column}: {}", self.message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}
impl std::error::Error for Diagnostic {}

#[derive(Clone, Debug)]
pub(crate) struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum ExprKind {
    Int(i64),
    Bool(bool),
    Str(String),
    Name(String),
    Borrow(Box<Expr>),
    Record(Vec<Entry>),
    Array(Vec<ArrayElement>),
    Function(
        Vec<(String, Span)>,
        Vec<Parameter>,
        Option<TypeExpr>,
        Box<Expr>,
    ),
    Call(Box<Expr>, Vec<TypeExpr>, Vec<Expr>),
    MemberCall(Box<Expr>, String, Vec<TypeExpr>, Vec<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    Block(Block),
    If(Box<Expr>, Block, Option<Block>),
    For(String, Span, Box<Expr>, Block),
    Match(Box<Expr>, Vec<Arm>),
    Type(Vec<(String, Span)>, TypeExpr),
}

#[derive(Clone, Debug)]
pub(crate) struct Arm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum Pattern {
    Type(TypeExpr),
    Literal(Literal),
    Else,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Literal {
    Int(i64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug)]
pub(crate) enum Entry {
    Field(String, Span, Expr),
    Spread(Span, Expr),
}

#[derive(Clone, Debug)]
pub(crate) struct ArrayElement {
    pub value: Expr,
    pub spread: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct Parameter {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub default: Option<Expr>,
    pub borrowed_self: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum TypeKind {
    Literal(Literal),
    Union(Vec<TypeExpr>),
    NoResult,
    Borrow(Box<TypeExpr>),
    Default(Box<TypeExpr>),
    Name(String, Span),
    Apply(String, Span, Vec<TypeExpr>),
    /// `module.Name` or `module.Name<args>`: a type value reached through object fields.
    Path(Vec<(String, Span)>, Vec<TypeExpr>),
    Array(Box<TypeExpr>),
    Record(
        Vec<(String, Span, bool, TypeExpr)>,
        bool,
        Option<(String, Span)>,
    ),
    Function(Vec<(String, Span)>, bool, Vec<TypeExpr>, Box<TypeExpr>),
}

/// Comments are part of the parsed source, retained in source order with exact
/// text (including delimiters). Whitespace outside comments is not syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub span: Span,
    pub text: String,
}

/// Comment-preserving syntax, usable before name resolution or type checking.
#[derive(Clone, Debug)]
pub struct Syntax {
    pub(crate) body: Block,
    pub(crate) comments: Vec<Comment>,
}

impl Syntax {
    pub fn parse(source: &str) -> Result<Self, Diagnostic> {
        parse(source)
    }

    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Unary {
    Negate,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    In,
}

#[derive(Clone, Debug)]
pub(crate) struct Block {
    pub statements: Vec<Statement>,
    pub result: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum Statement {
    Return(Option<Expr>, Span),
    Let(String, Option<TypeExpr>, Expr, Span),
    Assign(Expr, Option<Binary>, Expr, Span),
    Discard(Expr),
}

#[derive(Clone, Debug, PartialEq)]
enum TokenKind<'a> {
    Ident(&'a str),
    Number(&'a str),
    Text(String),
    Symbol(&'static str),
    Eof,
}
#[derive(Clone, Debug)]
struct Token<'a> {
    kind: TokenKind<'a>,
    span: Span,
}

fn lex(source: &str) -> Result<(Vec<Token<'_>>, Vec<Comment>), Diagnostic> {
    let mut tokens = Vec::new();
    let mut comments = Vec::new();
    let mut pos = 0;
    while pos < source.len() {
        let tail = &source[pos..];
        let c = tail.chars().next().ok_or_else(|| {
            Diagnostic::new(
                Span {
                    start: pos,
                    end: pos,
                },
                "invalid source boundary",
            )
        })?;
        if c.is_whitespace() {
            pos += c.len_utf8();
            continue;
        }
        if tail.starts_with("//") {
            let start = pos;
            pos += tail.find('\n').unwrap_or(tail.len());
            comments.push(Comment {
                span: Span { start, end: pos },
                text: source[start..pos].to_owned(),
            });
            continue;
        }
        if tail.starts_with("/*") {
            let start = pos;
            pos += 2;
            let mut depth = 1usize;
            while pos < source.len() && depth > 0 {
                if source[pos..].starts_with("/*") {
                    depth += 1;
                    pos += 2;
                } else if source[pos..].starts_with("*/") {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += source[pos..].chars().next().map_or(1, char::len_utf8);
                }
            }
            if depth != 0 {
                return Err(Diagnostic::new(
                    Span { start, end: pos },
                    "unterminated comment",
                ));
            }
            comments.push(Comment {
                span: Span { start, end: pos },
                text: source[start..pos].to_owned(),
            });
            continue;
        }
        let start = pos;
        let kind = if c == '"' {
            pos += 1;
            let mut value = String::new();
            let mut closed = false;
            while pos < source.len() {
                let ch = source[pos..].chars().next().ok_or_else(|| {
                    Diagnostic::new(Span { start, end: pos }, "unterminated string")
                })?;
                pos += ch.len_utf8();
                if ch == '"' {
                    closed = true;
                    break;
                }
                if ch == '%' && source[pos..].starts_with("%{") {
                    value.push('%');
                    value.push('{');
                    pos += 2;
                    continue;
                }
                if ch == '%' && source[pos..].starts_with('{') {
                    return Err(Diagnostic::new(
                        Span {
                            start,
                            end: pos + 1,
                        },
                        "string interpolation is not supported in this initial slice",
                    ));
                }
                if ch == '\\' {
                    let escaped = source[pos..].chars().next().ok_or_else(|| {
                        Diagnostic::new(Span { start, end: pos }, "unterminated string escape")
                    })?;
                    pos += escaped.len_utf8();
                    value.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        '%' => '%',
                        _ => {
                            return Err(Diagnostic::new(
                                Span {
                                    start: pos - escaped.len_utf8(),
                                    end: pos,
                                },
                                "unsupported string escape",
                            ));
                        }
                    });
                } else {
                    value.push(ch);
                }
            }
            if !closed {
                return Err(Diagnostic::new(
                    Span { start, end: pos },
                    "unterminated string",
                ));
            }
            TokenKind::Text(value)
        } else if c.is_ascii_digit() {
            pos += tail.bytes().take_while(u8::is_ascii_digit).count();
            TokenKind::Number(&source[start..pos])
        } else if c.is_ascii_alphabetic() || c == '_' {
            pos += tail
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            TokenKind::Ident(&source[start..pos])
        } else {
            let symbol = match c {
                '.' if tail.starts_with("...") => "...",
                '-' if tail.starts_with("->") => "->",
                '=' if tail.starts_with("=>") => "=>",
                '+' if tail.starts_with("+=") => "+=",
                '-' if tail.starts_with("-=") => "-=",
                '*' if tail.starts_with("*=") => "*=",
                '/' if tail.starts_with("/=") => "/=",
                '=' if tail.starts_with("==") => "==",
                '!' if tail.starts_with("!=") => "!=",
                '<' if tail.starts_with("<=") => "<=",
                '&' if tail.starts_with("&&") => "&&",
                '|' if tail.starts_with("||") => "||",
                '{' => "{",
                '}' => "}",
                '[' => "[",
                ']' => "]",
                '(' => "(",
                ')' => ")",
                ':' => ":",
                ';' => ";",
                ',' => ",",
                '.' => ".",
                '?' => "?",
                '=' => "=",
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '%' => "%",
                '!' => "!",
                '<' => "<",
                '>' => ">",
                '&' => "&",
                '|' => "|",
                _ => {
                    return Err(Diagnostic::new(
                        Span {
                            start,
                            end: start + c.len_utf8(),
                        },
                        format!("unsupported character {c:?}"),
                    ));
                }
            };
            pos += symbol.len();
            TokenKind::Symbol(symbol)
        };
        tokens.push(Token {
            kind,
            span: Span { start, end: pos },
        });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: pos,
            end: pos,
        },
    });
    Ok((tokens, comments))
}

pub(crate) fn parse(source: &str) -> Result<Syntax, Diagnostic> {
    let (tokens, comments) = lex(source)?;
    let body = Parser {
        source,
        tokens,
        at: 0,
        depth: 0,
    }
    .block(false)?;
    Ok(Syntax { body, comments })
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token<'a>>,
    at: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn token(&self) -> &Token<'a> {
        &self.tokens[self.at]
    }
    fn is(&self, s: &str) -> bool {
        matches!(&self.token().kind, TokenKind::Symbol(t) if *t == s)
            || matches!(&self.token().kind, TokenKind::Ident(t) if *t == s)
    }
    fn bump(&mut self) -> Token<'a> {
        let token = self.token().clone();
        if token.kind != TokenKind::Eof {
            self.at += 1;
        }
        token
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn need(&mut self, s: &str) -> Result<Token<'a>, Diagnostic> {
        if self.is(s) {
            Ok(self.bump())
        } else {
            Err(Diagnostic::new(
                self.token().span,
                format!("expected `{s}`"),
            ))
        }
    }
    fn name(&mut self) -> Result<(String, Span), Diagnostic> {
        let token = self.bump();
        match token.kind {
            TokenKind::Ident(name)
                if ![
                    "let", "fn", "if", "else", "type", "true", "false", "in", "is", "for", "while",
                    "match", "return",
                ]
                .contains(&name) =>
            {
                Ok((name.to_owned(), token.span))
            }
            _ => Err(Diagnostic::new(token.span, "expected a name")),
        }
    }
    fn field_name(&mut self) -> Result<String, Diagnostic> {
        if let TokenKind::Text(name) = &self.token().kind {
            let name = name.clone();
            self.bump();
            Ok(name)
        } else {
            self.name().map(|(name, _)| name)
        }
    }

    fn block(&mut self, braced: bool) -> Result<Block, Diagnostic> {
        let start = if braced {
            self.need("{")?.span
        } else {
            self.token().span
        };
        let mut statements = Vec::new();
        let mut result = None;
        while self.token().kind != TokenKind::Eof && !(braced && self.is("}")) {
            if self.is("return") {
                let start = self.bump().span;
                let value = if self.is(";") {
                    None
                } else {
                    Some(self.expr(0)?)
                };
                let end = self.need(";")?.span;
                statements.push(Statement::Return(value, start.through(end)));
                continue;
            }
            if self.eat("let") {
                let (name, name_span) = self.name()?;
                let ty = if self.eat(":") {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.need("=")?;
                let value = self.expr(0)?;
                let span = name_span.through(value.span);
                self.need(";")?;
                statements.push(Statement::Let(name, ty, value, span));
                continue;
            }
            let value = self.expr(0)?;
            let assignment = match &self.token().kind {
                TokenKind::Symbol("=") => Some(None),
                TokenKind::Symbol("+=") => Some(Some(Binary::Add)),
                TokenKind::Symbol("-=") => Some(Some(Binary::Subtract)),
                TokenKind::Symbol("*=") => Some(Some(Binary::Multiply)),
                TokenKind::Symbol("/=") => Some(Some(Binary::Divide)),
                _ => None,
            };
            if let Some(op) = assignment {
                self.bump();
                let rhs = self.expr(0)?;
                let span = value.span.through(rhs.span);
                self.need(";")?;
                statements.push(Statement::Assign(value, op, rhs, span));
            } else if self.eat(";")
                || matches!(value.kind, ExprKind::For(..))
                || matches!(&value.kind, ExprKind::Block(block) if block.result.is_none())
            {
                statements.push(Statement::Discard(value));
            } else {
                result = Some(Box::new(value));
                break;
            }
        }
        let end = if braced {
            self.need("}")?.span
        } else {
            if self.token().kind != TokenKind::Eof {
                return Err(Diagnostic::new(
                    self.token().span,
                    "expected `;` or end of source",
                ));
            }
            self.token().span
        };
        Ok(Block {
            statements,
            result,
            span: start.through(end),
        })
    }

    fn ty(&mut self) -> Result<TypeExpr, Diagnostic> {
        self.enter()?;
        let start = self.token().span;
        let mut kind = self.type_kind()?;
        if self.is("|") {
            let mut cases = vec![TypeExpr {
                kind,
                span: start.through(self.tokens[self.at - 1].span),
            }];
            while self.eat("|") {
                let case_start = self.token().span;
                let case = self.type_kind()?;
                cases.push(TypeExpr {
                    kind: case,
                    span: case_start.through(self.tokens[self.at - 1].span),
                });
            }
            kind = TypeKind::Union(cases);
        }
        self.depth -= 1;
        Ok(TypeExpr {
            kind,
            span: start.through(self.tokens[self.at - 1].span),
        })
    }

    fn type_kind(&mut self) -> Result<TypeKind, Diagnostic> {
        if self.eat("(") {
            let ty = self.ty()?;
            self.need(")")?;
            return Ok(ty.kind);
        }
        if matches!(self.token().kind, TokenKind::Text(_) | TokenKind::Number(_))
            || self.is("true")
            || self.is("false")
            || self.is("-")
        {
            let value = self.atom()?;
            let literal = match value.kind {
                ExprKind::Str(s) => Literal::Str(s),
                ExprKind::Int(n) => Literal::Int(n),
                ExprKind::Bool(b) => Literal::Bool(b),
                ExprKind::Unary(Unary::Negate, inner) => match inner.kind {
                    ExprKind::Int(n) => Literal::Int(n.checked_neg().ok_or_else(|| {
                        Diagnostic::new(
                            value.span,
                            "integer literal type is outside the signed 64-bit range",
                        )
                    })?),
                    _ => return Err(Diagnostic::new(value.span, "expected a literal type")),
                },
                _ => return Err(Diagnostic::new(value.span, "expected a literal type")),
            };
            return Ok(TypeKind::Literal(literal));
        }
        if self.eat("&") {
            return Ok(TypeKind::Borrow(Box::new(self.ty()?)));
        }
        if self.eat("[") {
            let element = self.ty()?;
            self.need("]")?;
            return Ok(TypeKind::Array(Box::new(element)));
        }
        if self.eat("{") {
            let mut fields = Vec::new();
            let mut open = false;
            let mut row = None;
            while !self.is("}") {
                if self.eat("...") {
                    if matches!(self.token().kind, TokenKind::Ident(_)) {
                        row = Some(self.name()?);
                    } else {
                        open = true;
                    }
                    self.eat(";");
                    self.eat(",");
                    break;
                }
                let span = self.token().span;
                let name = self.field_name()?;
                let optional = self.eat("?");
                self.need(":")?;
                fields.push((name, span, optional, self.ty()?));
                self.separator("}")?;
            }
            self.need("}")?;
            return Ok(TypeKind::Record(fields, open, row));
        }
        if self.eat("fn") {
            let generics = self.generic_parameters()?;
            self.need("(")?;
            let mut parameters = Vec::new();
            let mut receiver = false;
            while !self.is(")") {
                if self.eat("self") {
                    if !parameters.is_empty() {
                        return Err(Diagnostic::new(
                            self.token().span,
                            "self must be the first parameter",
                        ));
                    }
                    receiver = true;
                    self.need(":")?;
                }
                let mut parameter = self.ty()?;
                if self.eat("=") {
                    if receiver && parameters.is_empty() {
                        return Err(Diagnostic::new(
                            parameter.span,
                            "a receiver cannot have a default",
                        ));
                    }
                    let end = self.need("...")?.span;
                    parameter = TypeExpr {
                        span: parameter.span.through(end),
                        kind: TypeKind::Default(Box::new(parameter)),
                    };
                }
                parameters.push(parameter);
                if !self.eat(",") {
                    break;
                }
            }
            self.need(")")?;
            let result = if self.eat("->") {
                self.ty()?
            } else {
                TypeExpr {
                    kind: TypeKind::NoResult,
                    span: self.tokens[self.at - 1].span,
                }
            };
            return Ok(TypeKind::Function(
                generics,
                receiver,
                parameters,
                Box::new(result),
            ));
        }
        let first = self.name()?;
        let mut path = vec![first.clone()];
        while self.eat(".") {
            path.push(self.name()?);
        }
        let arguments = if self.is("<") {
            self.type_arguments()?
        } else {
            Vec::new()
        };
        if path.len() > 1 {
            return Ok(TypeKind::Path(path, arguments));
        }
        let (name, span) = first;
        if arguments.is_empty() {
            Ok(TypeKind::Name(name, span))
        } else {
            Ok(TypeKind::Apply(name, span, arguments))
        }
    }

    fn generic_parameters(&mut self) -> Result<Vec<(String, Span)>, Diagnostic> {
        let mut result = Vec::new();
        if self.eat("<") {
            loop {
                result.push(self.name()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.need(">")?;
        }
        Ok(result)
    }

    fn type_arguments(&mut self) -> Result<Vec<TypeExpr>, Diagnostic> {
        self.need("<")?;
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.ty()?);
            if !self.eat(",") {
                break;
            }
        }
        self.need(">")?;
        Ok(arguments)
    }

    fn separator(&mut self, end: &str) -> Result<(), Diagnostic> {
        if self.eat(";") || self.eat(",") || self.is(end) {
            return Ok(());
        }
        let previous_end = self.tokens[self.at.saturating_sub(1)].span.end;
        if self.source[previous_end..self.token().span.start].contains('\n') {
            return Ok(());
        }
        Err(Diagnostic::new(
            self.token().span,
            "expected a field separator",
        ))
    }

    fn expr(&mut self, minimum: u8) -> Result<Expr, Diagnostic> {
        self.enter()?;
        let result = self.expression(minimum);
        self.depth -= 1;
        result
    }

    fn enter(&mut self) -> Result<(), Diagnostic> {
        if self.depth >= 64 {
            return Err(Diagnostic::new(
                self.token().span,
                "syntax nesting exceeds the initial implementation limit (64)",
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn expression(&mut self, minimum: u8) -> Result<Expr, Diagnostic> {
        let mut lhs = self.atom()?;
        // Only a field selected in this postfix chain is a direct member call.
        // A parenthesized field comes back from atom and stays an unbound value.
        let mut member = false;
        let mut chain = 0;
        loop {
            // A left-associated chain grows the AST without recursive parsing.
            chain += 1;
            if chain > 64 {
                return Err(Diagnostic::new(
                    lhs.span,
                    "expression chain exceeds the initial implementation limit (64)",
                ));
            }
            if self.eat(".") {
                let (name, end) = self.name()?;
                let span = lhs.span.through(end);
                lhs = Expr {
                    kind: ExprKind::Field(Box::new(lhs), name),
                    span,
                };
                member = true;
                continue;
            }
            if self.eat("[") {
                lhs = self.index_expression(lhs)?;
                member = false;
                continue;
            }
            let mut type_args = Vec::new();
            if self.is("<") && lhs.span.end == self.token().span.start {
                let saved = (self.at, self.depth);
                match self.type_arguments() {
                    Ok(args) if self.is("(") => type_args = args,
                    _ => {
                        self.at = saved.0;
                        self.depth = saved.1;
                    }
                }
            }
            if self.eat("(") {
                lhs = self.call_expression(lhs, member, type_args)?;
                member = false;
                continue;
            }
            if self.is("is") {
                return Err(Diagnostic::new(
                    self.token().span,
                    "type tests belong in match arms: match value { Type => ...; else => ...; }",
                ));
            }
            let op = match &self.token().kind {
                TokenKind::Symbol("||") => Some((1, Binary::Or)),
                TokenKind::Symbol("&&") => Some((2, Binary::And)),
                TokenKind::Symbol("==") => Some((3, Binary::Equal)),
                TokenKind::Symbol("!=") => Some((3, Binary::NotEqual)),
                TokenKind::Ident(s) if *s == "in" => Some((3, Binary::In)),
                TokenKind::Symbol("<") => Some((4, Binary::Less)),
                TokenKind::Symbol("<=") => Some((4, Binary::LessEqual)),
                // Keep `>` separate during lexing so `Box<int>=value` and
                // nested generic closers do not require token-stream edits.
                TokenKind::Symbol(">") => Some((
                    4,
                    if self.tokens.get(self.at + 1).is_some_and(|next| {
                        next.kind == TokenKind::Symbol("=")
                            && next.span.start == self.token().span.end
                    }) {
                        Binary::GreaterEqual
                    } else {
                        Binary::Greater
                    },
                )),
                TokenKind::Symbol("+") => Some((5, Binary::Add)),
                TokenKind::Symbol("-") => Some((5, Binary::Subtract)),
                TokenKind::Symbol("*") => Some((6, Binary::Multiply)),
                TokenKind::Symbol("/") => Some((6, Binary::Divide)),
                TokenKind::Symbol("%") => Some((6, Binary::Remainder)),
                _ => None,
            };
            let Some((precedence, op)) = op else {
                break;
            };
            if precedence < minimum {
                break;
            }
            self.bump();
            if op == Binary::GreaterEqual {
                self.need("=")?;
            }
            let rhs = self.expr(precedence + 1)?;
            let span = lhs.span.through(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
                span,
            };
            member = false;
        }
        Ok(lhs)
    }

    fn call_expression(
        &mut self,
        lhs: Expr,
        member: bool,
        type_args: Vec<TypeExpr>,
    ) -> Result<Expr, Diagnostic> {
        let mut args = Vec::new();
        while !self.is(")") {
            args.push(self.expr(0)?);
            if !self.eat(",") {
                break;
            }
        }
        let span = lhs.span.through(self.need(")")?.span);
        let kind = if member {
            let ExprKind::Field(base, name) = lhs.kind else {
                unreachable!()
            };
            ExprKind::MemberCall(base, name, type_args, args)
        } else {
            ExprKind::Call(Box::new(lhs), type_args, args)
        };
        Ok(Expr { kind, span })
    }

    fn index_expression(&mut self, lhs: Expr) -> Result<Expr, Diagnostic> {
        let index = if self.is(":") {
            None
        } else {
            Some(self.expr(0)?)
        };
        let end = if self.eat(":") {
            Some(if self.is("]") {
                None
            } else {
                Some(Box::new(self.expr(0)?))
            })
        } else {
            None
        };
        let span = lhs.span.through(self.need("]")?.span);
        Ok(Expr {
            kind: if let Some(end) = end {
                ExprKind::Slice(Box::new(lhs), index.map(Box::new), end)
            } else {
                ExprKind::Index(
                    Box::new(lhs),
                    Box::new(index.ok_or_else(|| {
                        Diagnostic::new(span, "internal invariant: index or colon")
                    })?),
                )
            },
            span,
        })
    }

    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.bump();
        let start = token.span;
        let kind = match token.kind {
            TokenKind::Number(n) => ExprKind::Int(n.parse().map_err(|_| {
                Diagnostic::new(start, "integer literal is outside the signed 64-bit range")
            })?),
            TokenKind::Text(s) => ExprKind::Str(s),
            TokenKind::Ident(s) if s == "true" || s == "false" => ExprKind::Bool(s == "true"),
            TokenKind::Ident("type") => {
                let generics = self.generic_parameters()?;
                ExprKind::Type(generics, self.ty()?)
            }
            TokenKind::Ident("fn") => self.function_expression()?,
            TokenKind::Ident("if") => self.conditional_expression()?,
            TokenKind::Ident("for") => self.loop_expression()?,
            TokenKind::Ident("match") => self.match_expression()?,
            TokenKind::Ident(s) if ["for", "while", "return", "else", "let"].contains(&s) => {
                return Err(Diagnostic::new(
                    start,
                    format!("`{s}` is not supported here in this initial slice"),
                ));
            }
            TokenKind::Ident(s) => ExprKind::Name(s.to_owned()),
            TokenKind::Symbol("(") => {
                let mut e = self.expr(0)?;
                let end = self.need(")")?.span;
                e.span = start.through(end);
                return Ok(e);
            }
            TokenKind::Symbol("-") | TokenKind::Symbol("!") => {
                let negative = token.kind == TokenKind::Symbol("-");
                if negative
                    && matches!(&self.token().kind, TokenKind::Number(n) if *n == "9223372036854775808")
                {
                    self.bump();
                    ExprKind::Int(i64::MIN)
                } else {
                    ExprKind::Unary(
                        if negative { Unary::Negate } else { Unary::Not },
                        Box::new(self.expr(7)?),
                    )
                }
            }
            TokenKind::Symbol("{") => self.brace_expression()?,
            TokenKind::Symbol("[") => self.array_expression()?,
            TokenKind::Symbol("&") => ExprKind::Borrow(Box::new(self.expr(7)?)),
            _ => return Err(Diagnostic::new(start, "expected an expression")),
        };
        Ok(Expr {
            kind,
            span: start.through(self.tokens[self.at.saturating_sub(1)].span),
        })
    }
    fn brace_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let record_start = self.is("}")
            || self.is("...")
            || (matches!(self.token().kind, TokenKind::Ident(_) | TokenKind::Text(_))
                && self
                    .tokens
                    .get(self.at + 1)
                    .is_some_and(|token| token.kind == TokenKind::Symbol("=")));
        if !record_start {
            self.at -= 1;
            let block = self.block(true)?;
            return Ok(ExprKind::Block(block));
        }
        let mut fields = Vec::new();
        while !self.is("}") {
            let entry_span = self.token().span;
            if self.eat("...") {
                fields.push(Entry::Spread(entry_span, self.expr(0)?));
            } else {
                let name = self.field_name()?;
                self.need("=")?;
                fields.push(Entry::Field(name, entry_span, self.expr(0)?));
            }
            self.separator("}")?;
        }
        self.need("}")?;
        Ok(ExprKind::Record(fields))
    }

    fn function_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let generics = self.generic_parameters()?;
        self.need("(")?;
        let mut parameters = Vec::new();
        while !self.is(")") {
            let borrowed_self = self.eat("&");
            let (name, span) = self.name()?;
            if borrowed_self && name != "self" {
                return Err(Diagnostic::new(
                    span,
                    "only &self has an inferred borrow shortcut; annotate ordinary parameters as name: &Type",
                ));
            }
            let ty = if self.eat(":") {
                Some(self.ty()?)
            } else {
                None
            };
            if borrowed_self && ty.is_some() {
                return Err(Diagnostic::new(
                    span,
                    "write self: &Type for an annotated receiver",
                ));
            }
            let default = if self.eat("=") {
                Some(self.expr(0)?)
            } else {
                None
            };
            parameters.push(Parameter {
                name,
                ty,
                default,
                borrowed_self,
                span,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.need(")")?;
        let result = if self.eat("->") {
            Some(self.ty()?)
        } else {
            None
        };
        if result.is_some() && !self.is("{") {
            return Err(Diagnostic::new(
                self.token().span,
                "a function with an explicit result type requires a block body: `fn(...) -> Type { expression }`",
            ));
        }
        let body = if self.is("{") {
            let b = self.block(true)?;
            Expr {
                span: b.span,
                kind: ExprKind::Block(b),
            }
        } else {
            self.expr(0)?
        };
        Ok(ExprKind::Function(
            generics,
            parameters,
            result,
            Box::new(body),
        ))
    }

    fn conditional_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let condition = self.expr(0)?;
        let yes = self.block(true)?;
        let no = if self.eat("else") {
            if self.is("if") {
                let expression = self.expr(0)?;
                Some(Block {
                    span: expression.span,
                    statements: Vec::new(),
                    result: Some(Box::new(expression)),
                })
            } else {
                Some(self.block(true)?)
            }
        } else {
            None
        };
        Ok(ExprKind::If(Box::new(condition), yes, no))
    }

    fn loop_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let (name, name_span) = self.name()?;
        self.need("in")?;
        let source = self.expr(0)?;
        let body = self.block(true)?;
        Ok(ExprKind::For(name, name_span, Box::new(source), body))
    }

    fn match_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let value = self.expr(0)?;
        self.need("{")?;
        let mut arms = Vec::new();
        while !self.is("}") {
            let start = self.token().span;
            let pattern = if self.eat("else") {
                Pattern::Else
            } else if matches!(self.token().kind, TokenKind::Number(_) | TokenKind::Text(_))
                || self.is("true")
                || self.is("false")
                || self.is("-")
            {
                let literal = self.atom()?;
                Pattern::Literal(match literal.kind {
                    ExprKind::Int(n) => Literal::Int(n),
                    ExprKind::Bool(b) => Literal::Bool(b),
                    ExprKind::Str(s) => Literal::Str(s),
                    ExprKind::Unary(Unary::Negate, value) => match value.kind {
                        ExprKind::Int(n) => Literal::Int(n.checked_neg().ok_or_else(|| {
                            Diagnostic::new(
                                literal.span,
                                "integer pattern is outside the signed 64-bit range",
                            )
                        })?),
                        _ => {
                            return Err(Diagnostic::new(
                                literal.span,
                                "expected a literal pattern",
                            ));
                        }
                    },
                    _ => {
                        return Err(Diagnostic::new(literal.span, "expected a literal pattern"));
                    }
                })
            } else {
                Pattern::Type(self.ty()?)
            };
            self.need("=>")?;
            let body = if self.is("{") {
                let block = self.block(true)?;
                Expr {
                    span: block.span,
                    kind: ExprKind::Block(block),
                }
            } else {
                self.expr(0)?
            };
            let span = start.through(body.span);
            arms.push(Arm {
                pattern,
                body,
                span,
            });
            self.separator("}")?;
        }
        self.need("}")?;
        Ok(ExprKind::Match(Box::new(value), arms))
    }

    fn array_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let mut elements = Vec::new();
        while !self.is("]") {
            let start = self.token().span;
            let spread = self.eat("...");
            let value = self.expr(0)?;
            elements.push(ArrayElement {
                span: start.through(value.span),
                value,
                spread,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.need("]")?;
        Ok(ExprKind::Array(elements))
    }
}
