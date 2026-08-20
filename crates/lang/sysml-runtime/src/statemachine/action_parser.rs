//! Parser for action expressions in state machine actions.
//!
//! Parses action strings like:
//! - `t += 10` -> AssignmentIR { var: "t", op: Add, value: 10.0 }
//! - `send('eventName')` -> adds "eventName" to sends list
//! - `doSomething()` -> TransitionActionIR::Simple("doSomething()")

#![allow(clippy::indexing_slicing)]
use crate::expressions::compile_simple_expression;
use crate::{AssignmentIR, AssignmentOp, TransitionActionIR};
use sysml_core::Value;

/// Parse an action string into an TransitionActionIR.
///
/// Supports:
/// - Simple actions: any string that doesn't match structured patterns
/// - Assignments: `var = value`, `var += value`, `var -= value`
/// - Send events: `send('event')` or `send("event")`
/// - Multiple statements separated by `;`
///
/// # Examples
///
/// ```
/// use sysml_runtime::statemachine::parse_action;
/// use sysml_runtime::{TransitionActionIR, AssignmentOp};
///
/// // Simple action
/// let action = parse_action("doSomething()");
/// assert!(action.is_simple());
///
/// // Assignment
/// let action = parse_action("t += 10");
/// if let TransitionActionIR::Structured { assignments, sends, .. } = action {
///     assert_eq!(assignments.len(), 1);
///     assert_eq!(assignments[0].variable, "t");
///     assert_eq!(assignments[0].operator, AssignmentOp::Add);
///     assert_eq!(assignments[0].value, sysml_core::Value::Float(10.0));
/// }
///
/// // Send event
/// let action = parse_action("send('gridFail')");
/// if let TransitionActionIR::Structured { assignments, sends, .. } = action {
///     assert_eq!(sends.len(), 1);
///     assert_eq!(sends[0], "gridFail");
/// }
/// ```
pub fn parse_action(input: &str) -> TransitionActionIR {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return TransitionActionIR::Simple(String::new());
    }

    // Try to parse as structured action
    let mut assignments = Vec::new();
    let mut sends = Vec::new();
    let mut has_structured = false;

    // Split by semicolons for multiple statements
    for statement in trimmed.split(';') {
        let stmt = statement.trim();
        if stmt.is_empty() {
            continue;
        }

        if let Some(assign) = try_parse_assignment(stmt) {
            assignments.push(assign);
            has_structured = true;
        } else if let Some(event) = try_parse_send(stmt) {
            sends.push(event);
            has_structured = true;
        }
    }

    if has_structured {
        TransitionActionIR::structured(assignments, sends)
    } else {
        TransitionActionIR::Simple(trimmed.to_owned())
    }
}

/// Try to parse an assignment statement.
///
/// Formats:
/// - `var = value`
/// - `var += value`
/// - `var -= value`
fn try_parse_assignment(input: &str) -> Option<AssignmentIR> {
    let trimmed = input.trim();

    // Try compound operators first (+=, -=)
    if let Some((var, value)) = try_split_operator(trimmed, "+=") {
        let var = var.trim();
        if !is_valid_identifier(var) {
            return None;
        }
        return parse_assignment_rhs(var, AssignmentOp::Add, value.trim());
    }

    if let Some((var, value)) = try_split_operator(trimmed, "-=") {
        let var = var.trim();
        if !is_valid_identifier(var) {
            return None;
        }
        return parse_assignment_rhs(var, AssignmentOp::Subtract, value.trim());
    }

    // Simple assignment (but not == comparison)
    if let Some(pos) = trimmed.find('=') {
        // Make sure it's not == or part of +=/-=
        if pos > 0 {
            let before = &trimmed[..pos];
            let after = &trimmed[pos + 1..];

            // Skip if it's == or if there's another = immediately after
            if !after.starts_with('=') && !before.ends_with('+') && !before.ends_with('-') {
                let var = before.trim();

                // Validate variable name (simple alphanumeric + underscore)
                if is_valid_identifier(var) {
                    return parse_assignment_rhs(var, AssignmentOp::Set, after.trim());
                }
            }
        }
    }

    None
}

/// Parse a value string as a boolean or number.
fn parse_literal_value(input: &str) -> Option<Value> {
    match input {
        "true" => Some(Value::Bool(true)),
        "false" => Some(Value::Bool(false)),
        _ => parse_number(input).map(Value::Float),
    }
}

/// Parse an assignment RHS as either a literal or a compiled expression.
fn parse_assignment_rhs(variable: &str, operator: AssignmentOp, rhs: &str) -> Option<AssignmentIR> {
    if let Some(val) = parse_literal_value(rhs) {
        return Some(AssignmentIR::new(variable, operator, val));
    }

    compile_simple_expression(rhs)
        .ok()
        .map(|expr| AssignmentIR::from_expr(variable, operator, rhs, expr))
}

/// Try to split on an operator.
fn try_split_operator<'a>(input: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let parts: Vec<&str> = input.splitn(2, op).collect();
    if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
}

/// Try to parse a send statement.
///
/// Formats:
/// - `send('eventName')`
/// - `send("eventName")`
fn try_parse_send(input: &str) -> Option<String> {
    let trimmed = input.trim();

    // Check for send( prefix
    if !trimmed.starts_with("send(") {
        return None;
    }

    // Find closing paren
    if !trimmed.ends_with(')') {
        return None;
    }

    // Extract content between send( and )
    let content = &trimmed[5..trimmed.len() - 1].trim();

    // Extract quoted string
    if (content.starts_with('\'') && content.ends_with('\''))
        || (content.starts_with('"') && content.ends_with('"'))
    {
        let event = &content[1..content.len() - 1];
        return Some(event.to_owned());
    }

    // Allow unquoted identifier
    if is_valid_identifier(content) {
        return Some(content.to_string());
    }

    None
}

/// Parse a string as a number.
fn parse_number(input: &str) -> Option<f64> {
    input.trim().parse().ok()
}

/// Check if a string is a valid identifier.
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    let first = chars
        .next()
        .expect("invariant: non-empty string checked above");

    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_action() {
        let action = parse_action("doSomething()");
        assert!(action.is_simple());
        assert_eq!(action.as_simple(), Some("doSomething()"));
    }

    #[test]
    fn parse_empty_action() {
        let action = parse_action("");
        assert!(action.is_simple());
        assert_eq!(action.as_simple(), Some(""));
    }

    #[test]
    fn parse_set_assignment() {
        let action = parse_action("x = 5");
        if let TransitionActionIR::Structured {
            assignments, sends, ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "x");
            assert_eq!(assignments[0].operator, AssignmentOp::Set);
            assert_eq!(assignments[0].value, Value::Float(5.0));
            assert!(sends.is_empty());
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_add_assignment() {
        let action = parse_action("t += 10");
        if let TransitionActionIR::Structured {
            assignments,
            sends: _,
            ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "t");
            assert_eq!(assignments[0].operator, AssignmentOp::Add);
            assert_eq!(assignments[0].value, Value::Float(10.0));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_subtract_assignment() {
        let action = parse_action("count -= 1");
        if let TransitionActionIR::Structured {
            assignments,
            sends: _,
            ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "count");
            assert_eq!(assignments[0].operator, AssignmentOp::Subtract);
            assert_eq!(assignments[0].value, Value::Float(1.0));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_send_single_quotes() {
        let action = parse_action("send('gridFail')");
        if let TransitionActionIR::Structured {
            assignments: _,
            sends,
            ..
        } = action
        {
            assert_eq!(sends.len(), 1);
            assert_eq!(sends[0], "gridFail");
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_send_double_quotes() {
        let action = parse_action("send(\"relayOpen\")");
        if let TransitionActionIR::Structured {
            assignments: _,
            sends,
            ..
        } = action
        {
            assert_eq!(sends.len(), 1);
            assert_eq!(sends[0], "relayOpen");
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_send_unquoted() {
        let action = parse_action("send(eventName)");
        if let TransitionActionIR::Structured {
            assignments: _,
            sends,
            ..
        } = action
        {
            assert_eq!(sends.len(), 1);
            assert_eq!(sends[0], "eventName");
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_multiple_statements() {
        let action = parse_action("t += 10; send('ready')");
        if let TransitionActionIR::Structured {
            assignments, sends, ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "t");
            assert_eq!(assignments[0].operator, AssignmentOp::Add);

            assert_eq!(sends.len(), 1);
            assert_eq!(sends[0], "ready");
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_complex_action() {
        let action = parse_action("t += 5; x = 100; send('a'); send('b')");
        if let TransitionActionIR::Structured {
            assignments, sends, ..
        } = action
        {
            assert_eq!(assignments.len(), 2);
            assert_eq!(sends.len(), 2);
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_float_values() {
        let action = parse_action("delay = 15.5");
        if let TransitionActionIR::Structured { assignments, .. } = action {
            assert_eq!(assignments[0].value, Value::Float(15.5));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_negative_values() {
        let action = parse_action("offset = -10");
        if let TransitionActionIR::Structured { assignments, .. } = action {
            assert_eq!(assignments[0].value, Value::Float(-10.0));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_variable_reference_assignment() {
        let action = parse_action("v = supply");
        if let TransitionActionIR::Structured { assignments, .. } = action {
            assert!(assignments[0].value_expr.is_some());
            assert_eq!(assignments[0].value_source.as_deref(), Some("supply"));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_expression_assignment() {
        let action = parse_action("v = 0 - supply");
        if let TransitionActionIR::Structured { assignments, .. } = action {
            assert!(assignments[0].value_expr.is_some());
            assert_eq!(assignments[0].value_source.as_deref(), Some("0 - supply"));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_boolean_false_assignment() {
        let action = parse_action("heaterOn = false");
        if let TransitionActionIR::Structured {
            assignments, sends, ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "heaterOn");
            assert_eq!(assignments[0].operator, AssignmentOp::Set);
            assert_eq!(assignments[0].value, Value::Bool(false));
            assert!(sends.is_empty());
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_boolean_true_assignment() {
        let action = parse_action("isActive = true");
        if let TransitionActionIR::Structured {
            assignments, sends, ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "isActive");
            assert_eq!(assignments[0].operator, AssignmentOp::Set);
            assert_eq!(assignments[0].value, Value::Bool(true));
            assert!(sends.is_empty());
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_with_whitespace() {
        let action = parse_action("  t  +=   10  ;  send( 'event' )  ");
        if let TransitionActionIR::Structured {
            assignments, sends, ..
        } = action
        {
            assert_eq!(assignments.len(), 1);
            assert_eq!(sends.len(), 1);
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn parse_underscore_variable() {
        let action = parse_action("_private_var += 1");
        if let TransitionActionIR::Structured { assignments, .. } = action {
            assert_eq!(assignments[0].variable, "_private_var");
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn dont_parse_comparison() {
        // == should not be parsed as assignment
        let action = parse_action("x == 5");
        assert!(action.is_simple());
    }

    #[test]
    fn dont_parse_invalid_identifier() {
        // Invalid variable names should fall back to simple
        let action = parse_action("123 = 5");
        assert!(action.is_simple());
    }
}
