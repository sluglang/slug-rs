//! Reports `value copy { field: new }`, which `value + { field: new }`
//! replaced, with a fix that rewrites it.
//!
//! `copy` binds as tightly as `.field`, and `+` is an ordinary binary
//! operator, so the fix adds parentheses wherever the rewritten `+` would
//! group differently: before `.field`, `[i]` or a call, after a prefix
//! operator, as the right operand of `+` or `-`, and as an operand of an
//! operator that binds more tightly than `+`, such as `*` or `:+`. Any
//! position not known to be safe gets them, as they never change meaning.

use super::{
    SourceEdit, SourceError, SourceErrorCode, SourcePosition, SourceRange,
    ast::{Binary, CallArgument, Expr, ExprKind, ListElement, Token, TokenKind},
};

/// Returns an error for the first `copy` in `expressions`, outermost first,
/// whose fix rewrites it as `+`. `tokens` are the source's, which show the
/// parentheses the expressions no longer do.
pub(super) fn reject_copies(expressions: &[Expr], tokens: &[Token]) -> Result<(), SourceError> {
    expressions
        .iter()
        .try_for_each(|expression| walk(expression, true))
        .map_err(|found| {
            let parenthesize =
                found.parenthesize && !parenthesized(found.value, found.copy, tokens);
            copy_error(found.copy, found.value, parenthesize)
        })
}

/// A `copy`, its value, and whether its position needs it parenthesized.
struct Found<'a> {
    copy: &'a Expr,
    value: &'a Expr,
    parenthesize: bool,
}

/// Whether the source already wraps `value copy {...}` in parentheses.
fn parenthesized(value: &Expr, copy: &Expr, tokens: &[Token]) -> bool {
    let start = value.extent_start();
    let opening = tokens
        .iter()
        .position(|token| token.span.line == start.line && token.span.column == start.column);
    let closing = tokens.iter().position(|token| token.end == copy.end);
    match (opening, closing) {
        (Some(opening), Some(closing)) if opening > 0 => {
            matches!(tokens[opening - 1].kind, TokenKind::LParen)
                && matches!(
                    tokens.get(closing + 1).map(|token| &token.kind),
                    Some(TokenKind::RParen)
                )
        }
        _ => false,
    }
}

/// `safe`: whether `expression`'s position keeps `left + right` grouped as
/// one operand, so that it needs no parentheses there.
fn walk(expression: &Expr, safe: bool) -> Result<(), Found<'_>> {
    super::stack::grow(|| {
        if let ExprKind::StructCopy { value, .. } = &expression.kind {
            return Err(Found {
                copy: expression,
                value,
                parenthesize: !safe,
            });
        }
        match &expression.kind {
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let (left_safe, right_safe) = match operator {
                    Binary::Add | Binary::Subtract => (true, false),
                    Binary::Multiply
                    | Binary::Divide
                    | Binary::Modulo
                    | Binary::Append
                    | Binary::Prepend
                    | Binary::Pipeline => (false, false),
                    // Everything else binds less tightly than `+`.
                    _ => (true, true),
                };
                walk(left, left_safe)?;
                walk(right, right_safe)
            }
            // A prefix operator binds more tightly than `+`; a task's body and
            // a deferred value are parenthesized, to be sure.
            ExprKind::Prefix { value, .. }
            | ExprKind::Spawn { body: value, .. }
            | ExprKind::Defer { value, .. } => walk(value, false),
            ExprKind::Call { callee, arguments } => {
                walk(callee, false)?;
                arguments.iter().try_for_each(argument)
            }
            ExprKind::Recur(arguments) => arguments.iter().try_for_each(argument),
            ExprKind::TypeApply { callee, .. } => walk(callee, false),
            ExprKind::Index { collection, index } => {
                walk(collection, false)?;
                walk(index, true)
            }
            ExprKind::Slice {
                collection,
                start,
                end,
                step,
            } => {
                walk(collection, false)?;
                [start, end, step]
                    .into_iter()
                    .flatten()
                    .try_for_each(|bound| walk(bound, true))
            }
            ExprKind::StructInit { schema, fields } => {
                walk(schema, false)?;
                fields.iter().try_for_each(|(_, value)| walk(value, true))
            }
            ExprKind::List(elements) => elements.iter().try_for_each(|element| match element {
                ListElement::Value(value) => walk(value, true),
                ListElement::Spread(value) => walk(value, false),
            }),
            // A subject is followed by its cases' `{`: parenthesized, to be sure.
            ExprKind::Match { subject, .. } => {
                if let Some(subject) = subject {
                    walk(subject, false)?;
                }
                let mut result = Ok(());
                expression.for_each_child(|child| {
                    if result.is_ok() && !subject.as_deref().is_some_and(|s| std::ptr::eq(s, child))
                    {
                        result = walk(child, true);
                    }
                });
                result
            }
            // Every other child is a whole expression: a statement, an
            // element, a field's or a declaration's value, or a branch.
            _ => {
                let mut result = Ok(());
                expression.for_each_child(|child| {
                    if result.is_ok() {
                        result = walk(child, true);
                    }
                });
                result
            }
        }
    })
}

fn argument(argument: &CallArgument) -> Result<(), Found<'_>> {
    match argument {
        CallArgument::Positional(value) | CallArgument::Named { value, .. } => walk(value, true),
        CallArgument::Spread(value) => walk(value, false),
    }
}

fn copy_error(copy: &Expr, value: &Expr, parenthesize: bool) -> SourceError {
    // A copy's span is its `copy` keyword's.
    let keyword = SourcePosition {
        line: copy.span.line,
        column: copy.span.column,
    };
    let keyword_end = SourcePosition {
        line: keyword.line,
        column: keyword.column + 4,
    };
    let insert = |at: SourcePosition, text: &str| SourceEdit {
        range: SourceRange { start: at, end: at },
        text: text.into(),
    };
    let mut edits = Vec::new();
    if parenthesize {
        edits.push(insert(value.extent_start(), "("));
    }
    edits.push(SourceEdit {
        range: SourceRange {
            start: keyword,
            end: keyword_end,
        },
        text: "+".into(),
    });
    if parenthesize {
        edits.push(insert(copy.end, ")"));
    }
    SourceError::at(
        SourceErrorCode::RemovedSyntax,
        "`copy` is removed; `value + { field: new }` sets a struct's fields",
        copy.span.clone(),
    )
    .with_extent(copy)
    .with_fix_edits("write `+` in place of `copy`", edits)
}
