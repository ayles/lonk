//! One AST printer for source formatting and readable syntax dumps. No layout
//! settings or dependence on the original whitespace are involved.
use crate::syntax::{
    Binary, Block, Comment, Entry, Expr, ExprKind, Statement, TypeExpr, TypeKind, Unary,
};
use crate::{Diagnostic, Syntax};

/// Parse and print canonical source without evaluating or type checking it.
pub fn format(source: &str) -> Result<String, Diagnostic> {
    Ok(Syntax::parse(source)?.format())
}

impl Syntax {
    pub fn format(&self) -> String {
        let mut printer = Printer {
            output: String::new(),
            indent: 0,
            comments: &self.comments,
            next_comment: 0,
        };
        printer.block(&self.body, false);
        printer.comments_before(usize::MAX);
        printer.newline();
        printer.output
    }
}

struct Printer<'a> {
    output: String,
    indent: usize,
    comments: &'a [Comment],
    next_comment: usize,
}

impl Printer<'_> {
    fn write(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.output.is_empty() || self.output.ends_with('\n') {
            for _ in 0..self.indent {
                self.output.push_str("    ");
            }
        }
        self.output.push_str(text);
    }

    fn newline(&mut self) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn comments_before(&mut self, position: usize) {
        while let Some(comment) = self.comments.get(self.next_comment) {
            if comment.span.start >= position {
                break;
            }
            self.newline();
            self.write(&comment.text);
            // Comment text is content, including any trailing spaces.
            self.output.push('\n');
            self.next_comment += 1;
        }
    }

    fn block(&mut self, block: &Block, braced: bool) {
        self.comments_before(block.span.start);
        if braced {
            self.write("{");
            self.newline();
            self.indent += 1;
        }
        for statement in &block.statements {
            match statement {
                Statement::Return(value, span) => {
                    self.comments_before(span.start);
                    self.write("return");
                    if let Some(value) = value {
                        self.write(" ");
                        self.expr(value, 0);
                    }
                }
                Statement::Let(name, ty, value, span) => {
                    self.comments_before(span.start);
                    self.write("let ");
                    self.write(name);
                    if let Some(ty) = ty {
                        self.write(": ");
                        self.ty(ty);
                    }
                    self.write(" = ");
                    self.expr(value, 0);
                }
                Statement::Assign(place, op, value, span) => {
                    self.comments_before(span.start);
                    self.expr(place, 0);
                    self.write(" ");
                    if let Some(op) = op {
                        self.write(binary(*op));
                    }
                    self.write("= ");
                    self.expr(value, 0);
                }
                Statement::Discard(value) => self.expr(value, 0),
            }
            self.write(";");
            self.newline();
        }
        if let Some(value) = &block.result {
            self.expr(value, 0);
            self.newline();
        }
        if braced {
            self.comments_before(block.span.end.saturating_sub(1));
            self.indent -= 1;
            self.write("}");
        }
    }

    fn expr(&mut self, expr: &Expr, minimum: u8) {
        self.comments_before(expr.span.start);
        let precedence = precedence(&expr.kind);
        let parens = precedence < minimum;
        if parens {
            self.write("(");
        }
        match &expr.kind {
            ExprKind::Int(n) => self.write(&n.to_string()),
            ExprKind::Bool(b) => self.write(if *b { "true" } else { "false" }),
            ExprKind::Str(s) => self.write(&quoted(s)),
            ExprKind::Name(name) => self.write(name),
            ExprKind::Borrow(place) => {
                self.write("&");
                self.expr(place, 7);
            }
            ExprKind::Type(generics, ty) => {
                self.write("type");
                self.generics(generics);
                self.write(" ");
                self.ty(ty);
            }
            ExprKind::Record(entries) => {
                self.write("{");
                if !entries.is_empty() {
                    self.newline();
                }
                self.indent += 1;
                for entry in entries {
                    match entry {
                        Entry::Field(name, span, value) => {
                            self.comments_before(span.start);
                            self.field_name(name);
                            self.write(" = ");
                            self.expr(value, 0);
                        }
                        Entry::Spread(span, value) => {
                            self.comments_before(span.start);
                            self.write("...");
                            self.expr(value, 0);
                        }
                    }
                    self.write(";");
                    self.newline();
                }
                self.comments_before(expr.span.end.saturating_sub(1));
                self.indent -= 1;
                self.write("}");
            }
            ExprKind::Array(elements) => {
                self.write("[");
                if !elements.is_empty() {
                    self.newline();
                }
                self.indent += 1;
                for value in elements {
                    self.comments_before(value.span.start);
                    if value.spread {
                        self.write("...");
                    }
                    self.expr(&value.value, 0);
                    self.write(",");
                    self.newline();
                }
                self.comments_before(expr.span.end.saturating_sub(1));
                self.indent -= 1;
                self.write("]");
            }
            ExprKind::Function(generics, parameters, result, body) => {
                self.write("fn");
                self.generics(generics);
                self.write("(");
                for (i, parameter) in parameters.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.comments_before(parameter.span.start);
                    if parameter.borrowed_self {
                        self.write("&");
                    }
                    self.write(&parameter.name);
                    if let Some(ty) = &parameter.ty {
                        self.write(": ");
                        self.ty(ty);
                    }
                    if let Some(value) = &parameter.default {
                        self.write(" = ");
                        self.expr(value, 0);
                    }
                }
                self.write(")");
                if let Some(ty) = result {
                    self.write(" -> ");
                    self.ty(ty);
                }
                self.write(" ");
                if brace_led(&body.kind) {
                    self.write("(");
                    self.expr(body, 0);
                    self.write(")");
                } else {
                    self.expr(body, 0);
                }
            }
            ExprKind::Call(function, types, args) => {
                let unbound = matches!(function.kind, ExprKind::Field(..));
                if unbound {
                    self.write("(");
                }
                self.expr(function, 8);
                if unbound {
                    self.write(")");
                }
                self.type_arguments(types);
                self.write("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.expr(arg, 0);
                }
                self.comments_before(expr.span.end.saturating_sub(1));
                self.write(")");
            }
            ExprKind::MemberCall(base, name, types, args) => {
                self.expr(base, 8);
                self.write(".");
                self.write(name);
                self.type_arguments(types);
                self.write("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.expr(arg, 0);
                }
                self.comments_before(expr.span.end.saturating_sub(1));
                self.write(")");
            }
            ExprKind::Field(base, name) => {
                self.expr(base, 8);
                self.comments_before(expr.span.end);
                self.write(".");
                self.write(name);
            }
            ExprKind::Slice(base, start, end) => {
                self.expr(base, 8);
                self.write("[");
                if let Some(start) = start {
                    self.expr(start, 0);
                }
                self.write(":");
                if let Some(end) = end {
                    self.expr(end, 0);
                }
                self.comments_before(expr.span.end.saturating_sub(1));
                self.write("]");
            }
            ExprKind::Index(base, key) => {
                self.expr(base, 8);
                self.write("[");
                self.expr(key, 0);
                self.comments_before(expr.span.end.saturating_sub(1));
                self.write("]");
            }
            ExprKind::Unary(op, value) => {
                self.write(if *op == Unary::Not { "!" } else { "-" });
                self.expr(value, 7);
            }
            ExprKind::Binary(op, left, right) => {
                self.expr(left, precedence);
                self.write(" ");
                self.write(binary(*op));
                self.write(" ");
                self.expr(right, precedence + 1);
            }
            ExprKind::Match(value, arms) => {
                self.write("match ");
                self.expr(value, 0);
                self.write(" {");
                self.newline();
                self.indent += 1;
                for arm in arms {
                    self.comments_before(arm.span.start);
                    match &arm.pattern {
                        crate::syntax::Pattern::Type(ty) => self.ty(ty),
                        crate::syntax::Pattern::Literal(crate::syntax::Literal::Int(n)) => {
                            self.write(&n.to_string())
                        }
                        crate::syntax::Pattern::Literal(crate::syntax::Literal::Bool(b)) => {
                            self.write(if *b { "true" } else { "false" })
                        }
                        crate::syntax::Pattern::Literal(crate::syntax::Literal::Str(s)) => {
                            self.write(&quoted(s))
                        }
                        crate::syntax::Pattern::Else => self.write("else"),
                    }
                    self.write(" => ");
                    if brace_led(&arm.body.kind) {
                        self.write("(");
                        self.expr(&arm.body, 0);
                        self.write(")");
                    } else {
                        self.expr(&arm.body, 0);
                    }
                    self.write(";");
                    self.newline();
                }
                self.comments_before(expr.span.end.saturating_sub(1));
                self.indent -= 1;
                self.write("}");
            }
            ExprKind::Block(block) => self.block(block, true),
            ExprKind::If(condition, yes, no) => {
                self.write("if ");
                self.expr(condition, 0);
                self.write(" ");
                self.block(yes, true);
                if let Some(no) = no {
                    self.write(" else ");
                    if no.statements.is_empty()
                        && no
                            .result
                            .as_ref()
                            .is_some_and(|e| matches!(e.kind, ExprKind::If(..)))
                    {
                        if let Some(expression) = &no.result {
                            self.expr(expression, 0);
                        }
                        self.comments_before(no.span.end.saturating_sub(1));
                    } else {
                        self.block(no, true);
                    }
                }
            }
            ExprKind::For(name, name_span, source, body) => {
                self.write("for ");
                self.comments_before(name_span.start);
                self.write(name);
                self.write(" in ");
                self.expr(source, 0);
                self.write(" ");
                self.block(body, true);
            }
        }
        if parens {
            self.write(")");
        }
    }

    fn ty(&mut self, ty: &TypeExpr) {
        self.comments_before(ty.span.start);
        match &ty.kind {
            TypeKind::Literal(literal) => match literal {
                crate::syntax::Literal::Str(s) => self.write(&quoted(s)),
                crate::syntax::Literal::Int(n) => self.write(&n.to_string()),
                crate::syntax::Literal::Bool(b) => self.write(if *b { "true" } else { "false" }),
            },
            TypeKind::Union(cases) => {
                self.write("(");
                for (i, case) in cases.iter().enumerate() {
                    if i > 0 {
                        self.write(" | ");
                    }
                    self.ty(case);
                }
                self.write(")");
            }
            TypeKind::NoResult => (),
            TypeKind::Borrow(t) => {
                self.write("&");
                self.ty(t);
            }
            TypeKind::Default(t) => {
                self.ty(t);
                self.write(" = ...");
            }
            TypeKind::Name(name, _) => self.write(name),
            TypeKind::Apply(name, _, arguments) => {
                self.write(name);
                self.type_arguments(arguments);
            }
            TypeKind::Path(segments, arguments) => {
                for (index, (name, _)) in segments.iter().enumerate() {
                    if index > 0 {
                        self.write(".");
                    }
                    self.write(name);
                }
                if !arguments.is_empty() {
                    self.type_arguments(arguments);
                }
            }
            TypeKind::Array(element) => {
                self.write("[");
                self.ty(element);
                self.comments_before(ty.span.end.saturating_sub(1));
                self.write("]");
            }
            TypeKind::Record(fields, open, row) => {
                self.write("{");
                if !fields.is_empty() || *open || row.is_some() {
                    self.newline();
                }
                self.indent += 1;
                for (name, span, optional, ty) in fields {
                    self.comments_before(span.start);
                    self.field_name(name);
                    if *optional {
                        self.write("?");
                    }
                    self.write(": ");
                    self.ty(ty);
                    self.write(";");
                    self.newline();
                }
                if *open || row.is_some() {
                    self.write("...");
                    if let Some((name, span)) = row {
                        self.comments_before(span.start);
                        self.write(name);
                    }
                    self.write(";");
                    self.newline();
                }
                self.comments_before(ty.span.end.saturating_sub(1));
                self.indent -= 1;
                self.write("}");
            }
            TypeKind::Function(generics, receiver, parameters, result) => {
                self.write("fn");
                self.generics(generics);
                self.write("(");
                for (i, parameter) in parameters.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    if i == 0 && *receiver {
                        self.write("self: ");
                    }
                    self.ty(parameter);
                }
                self.write(")");
                if !matches!(result.kind, TypeKind::NoResult) {
                    self.write(" -> ");
                    self.ty(result);
                }
            }
        }
    }

    fn generics(&mut self, parameters: &[(String, crate::Span)]) {
        if parameters.is_empty() {
            return;
        }
        self.write("<");
        for (i, (name, span)) in parameters.iter().enumerate() {
            if i != 0 {
                self.write(", ");
            }
            self.comments_before(span.start);
            self.write(name);
        }
        self.write(">");
    }

    fn type_arguments(&mut self, arguments: &[TypeExpr]) {
        if arguments.is_empty() {
            return;
        }
        self.write("<");
        for (i, argument) in arguments.iter().enumerate() {
            if i != 0 {
                self.write(", ");
            }
            self.ty(argument);
        }
        self.write(">");
    }

    fn field_name(&mut self, name: &str) {
        let identifier = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && ![
                "let", "fn", "if", "else", "type", "true", "false", "in", "is", "for", "while",
                "match", "return",
            ]
            .contains(&name);
        if identifier {
            self.write(name);
        } else {
            self.write(&quoted(name));
        }
    }
}

pub(crate) fn quoted(text: &str) -> String {
    let mut result = String::from("\"");
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '%' if chars.peek() == Some(&'{') => result.push_str("%%"),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

fn binary(op: Binary) -> &'static str {
    match op {
        Binary::Add => "+",
        Binary::Subtract => "-",
        Binary::Multiply => "*",
        Binary::Divide => "/",
        Binary::Remainder => "%",
        Binary::Equal => "==",
        Binary::NotEqual => "!=",
        Binary::Less => "<",
        Binary::LessEqual => "<=",
        Binary::Greater => ">",
        Binary::GreaterEqual => ">=",
        Binary::And => "&&",
        Binary::Or => "||",
        Binary::In => "in",
    }
}

fn precedence(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::Function(..) | ExprKind::If(..) | ExprKind::Match(..) => 0,
        ExprKind::Binary(Binary::Or, ..) => 1,
        ExprKind::Binary(Binary::And, ..) => 2,
        ExprKind::Binary(Binary::Equal | Binary::NotEqual | Binary::In, ..) => 3,
        ExprKind::Binary(
            Binary::Less | Binary::LessEqual | Binary::Greater | Binary::GreaterEqual,
            ..,
        ) => 4,
        ExprKind::Binary(Binary::Add | Binary::Subtract, ..) => 5,
        ExprKind::Binary(..) => 6,
        ExprKind::Unary(..) | ExprKind::Borrow(..) => 7,
        ExprKind::Call(..)
        | ExprKind::MemberCall(..)
        | ExprKind::Field(..)
        | ExprKind::Index(..)
        | ExprKind::Slice(..) => 8,
        _ => 9,
    }
}

// In function/match-arm body position, a leading brace selects a statement
// block. Protect any expression whose printed left edge begins with a record.
fn brace_led(kind: &ExprKind) -> bool {
    brace_led_at(kind, 0)
}

fn brace_led_at(kind: &ExprKind, minimum: u8) -> bool {
    if precedence(kind) < minimum {
        return false;
    }
    match kind {
        ExprKind::Record(_) => true,
        ExprKind::Field(base, _)
        | ExprKind::Index(base, _)
        | ExprKind::Slice(base, _, _)
        | ExprKind::Call(base, _, _)
        | ExprKind::MemberCall(base, _, _, _) => brace_led_at(&base.kind, 8),
        ExprKind::Binary(_, base, _) => brace_led_at(&base.kind, precedence(kind)),
        _ => false,
    }
}
