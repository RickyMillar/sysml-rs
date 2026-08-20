//! Standard library functions for expression evaluation.
//!
//! This module implements the KerML standard library functions from:
//! - `BaseFunctions`: `==`, `!=`, `===`, `!==`, `ToString`, `size`
//! - `ScalarFunctions`: `+`, `-`, `*`, `/`, `%`, `**`, `<`, `>`, `<=`, `>=`, `max`, `min`
//! - `BooleanFunctions`: `not`, `xor`, `|`, `&`, `implies`
//! - `SequenceFunctions`: `size`, `isEmpty`, `notEmpty`, `includes`, `excludes`,
//!   `head`, `tail`, `last`, `union`, `intersection`
//! - `StringFunctions`: `length`, `substring`, `concat`, `matches`
//! - `RealFunctions`: `floor`, `ceil`, `round`, `abs`, `sum`
//! - `MathFunctions` (Phase 15): `exp`, `sqrt`, `ln`, `sin`, `cos`

use super::{EvalContext, EvalResult, EvaluationError};
use regex::Regex;
use sysml_core::Value;

/// Evaluate a standard library function call.
#[allow(clippy::indexing_slicing)] // Indices are safe: check_arity() validates length before access
pub(crate) fn eval_function(name: &str, args: &[Value], ctx: &EvalContext) -> EvalResult {
    match name {
        "size" | "array#" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => Ok(Value::Int(items.len() as i64)),
                Value::String(s) => Ok(Value::Int(s.len() as i64)),
                _ => Ok(Value::Int(1)),
            }
        }
        "isEmpty" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => Ok(Value::Bool(items.is_empty())),
                Value::Null => Ok(Value::Bool(true)),
                _ => Ok(Value::Bool(false)),
            }
        }
        "notEmpty" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => Ok(Value::Bool(!items.is_empty())),
                Value::Null => Ok(Value::Bool(false)),
                _ => Ok(Value::Bool(true)),
            }
        }
        "abs" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(
                    n.checked_abs().ok_or(EvaluationError::Overflow)?,
                )),
                Value::Float(f) => Ok(Value::Float(f.abs())),
                Value::Complex { re, im } => Ok(Value::Float((re * re + im * im).sqrt())),
                _ => Err(EvaluationError::TypeError("abs requires numeric".into())),
            }
        }
        "max" => {
            check_arity(name, args, 2)?;
            let (a, b) = promote_to_float(&args[0], &args[1])?;
            Ok(Value::Float(a.max(b)))
        }
        "min" => {
            check_arity(name, args, 2)?;
            let (a, b) = promote_to_float(&args[0], &args[1])?;
            Ok(Value::Float(a.min(b)))
        }
        "clamp" | "clip" => {
            check_arity(name, args, 3)?;
            let val = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => return Err(EvaluationError::TypeError("clamp requires numeric".into())),
            };
            let lo = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => return Err(EvaluationError::TypeError("clamp requires numeric".into())),
            };
            let hi = match &args[2] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => return Err(EvaluationError::TypeError("clamp requires numeric".into())),
            };
            Ok(Value::Float(val.clamp(lo, hi)))
        }
        "ToString" => {
            check_arity(name, args, 1)?;
            Ok(Value::String(format!("{:?}", args[0])))
        }
        "head" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => Ok(items.first().cloned().unwrap_or(Value::Null)),
                _ => Err(EvaluationError::TypeError("head requires sequence".into())),
            }
        }
        "tail" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => {
                    if items.len() <= 1 {
                        Ok(Value::List(Vec::new()))
                    } else {
                        Ok(Value::List(items[1..].to_vec()))
                    }
                }
                _ => Err(EvaluationError::TypeError("tail requires sequence".into())),
            }
        }
        "last" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => Ok(items.last().cloned().unwrap_or(Value::Null)),
                _ => Err(EvaluationError::TypeError("last requires sequence".into())),
            }
        }
        "includes" => {
            check_arity(name, args, 2)?;
            match &args[0] {
                Value::List(items) => Ok(Value::Bool(items.contains(&args[1]))),
                _ => Err(EvaluationError::TypeError(
                    "includes requires sequence".into(),
                )),
            }
        }

        // -- String functions (from StringFunctions.kerml) -----------------
        "length" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::String(s) => Ok(Value::Int(s.chars().count() as i64)),
                _ => Err(EvaluationError::TypeError(
                    "length requires a string argument".into(),
                )),
            }
        }
        "substring" => {
            check_arity(name, args, 3)?;
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(lower), Value::Int(upper)) => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len() as i64;
                    // SysML uses 1-based indexing; clamp to valid range
                    let start = (*lower).max(1) as usize - 1;
                    let end = (*upper).min(len) as usize;
                    if start >= end || start >= chars.len() {
                        Ok(Value::String(String::new()))
                    } else {
                        Ok(Value::String(chars[start..end].iter().collect()))
                    }
                }
                _ => Err(EvaluationError::TypeError(
                    "substring requires (string, integer, integer)".into(),
                )),
            }
        }
        "concat" => {
            check_arity(name, args, 2)?;
            match (&args[0], &args[1]) {
                (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
                _ => Err(EvaluationError::TypeError(
                    "concat requires two string arguments".into(),
                )),
            }
        }
        "matches" => {
            check_arity(name, args, 2)?;
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match Regex::new(pattern) {
                    Ok(re) => Ok(Value::Bool(re.is_match(s))),
                    Err(e) => Err(EvaluationError::Runtime(format!(
                        "invalid regex pattern `{}`: {}",
                        pattern, e
                    ))),
                },
                _ => Err(EvaluationError::TypeError(
                    "matches requires (string, string) arguments".into(),
                )),
            }
        }

        // -- Numerical functions (from RealFunctions.kerml) ----------------
        "floor" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Int(f.floor() as i64)),
                Value::Int(n) => Ok(Value::Int(*n)),
                _ => Err(EvaluationError::TypeError(
                    "floor requires a numeric argument".into(),
                )),
            }
        }
        "ceil" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Int(f.ceil() as i64)),
                Value::Int(n) => Ok(Value::Int(*n)),
                _ => Err(EvaluationError::TypeError(
                    "ceil requires a numeric argument".into(),
                )),
            }
        }
        "round" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Int(f.round() as i64)),
                Value::Int(n) => Ok(Value::Int(*n)),
                _ => Err(EvaluationError::TypeError(
                    "round requires a numeric argument".into(),
                )),
            }
        }
        "product" | "product1" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => {
                    if items.is_empty() {
                        return Ok(Value::Int(1));
                    }
                    let mut has_float = false;
                    let mut float_prod: f64 = 1.0;
                    let mut int_prod: i64 = 1;
                    for item in items {
                        match item {
                            Value::Int(n) => {
                                int_prod =
                                    int_prod.checked_mul(*n).ok_or(EvaluationError::Overflow)?;
                                float_prod *= *n as f64;
                            }
                            Value::Float(f) => {
                                has_float = true;
                                float_prod *= f;
                            }
                            _ => {
                                return Err(EvaluationError::TypeError(
                                    "product: non-numeric element".into(),
                                ))
                            }
                        }
                    }
                    if has_float {
                        Ok(Value::Float(float_prod))
                    } else {
                        Ok(Value::Int(int_prod))
                    }
                }
                _ => Err(EvaluationError::TypeError(
                    "product requires a list argument".into(),
                )),
            }
        }
        "sum" | "sum0" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => {
                    if items.is_empty() {
                        return Ok(Value::Int(0));
                    }
                    let mut has_float = false;
                    let mut float_sum: f64 = 0.0;
                    let mut int_sum: i64 = 0;
                    for item in items {
                        match item {
                            Value::Int(n) => {
                                int_sum =
                                    int_sum.checked_add(*n).ok_or(EvaluationError::Overflow)?;
                                float_sum += *n as f64;
                            }
                            Value::Float(f) => {
                                has_float = true;
                                float_sum += f;
                            }
                            _ => {
                                return Err(EvaluationError::TypeError(
                                    "sum requires a list of numeric values".into(),
                                ));
                            }
                        }
                    }
                    if has_float {
                        Ok(Value::Float(float_sum))
                    } else {
                        Ok(Value::Int(int_sum))
                    }
                }
                _ => Err(EvaluationError::TypeError(
                    "sum requires a list argument".into(),
                )),
            }
        }

        // -- Collection functions (from SequenceFunctions.kerml) -----------
        "excludes" => {
            check_arity(name, args, 2)?;
            match &args[0] {
                Value::List(items) => Ok(Value::Bool(!items.contains(&args[1]))),
                _ => Err(EvaluationError::TypeError(
                    "excludes requires a sequence as the first argument".into(),
                )),
            }
        }
        "union" => {
            check_arity(name, args, 2)?;
            match (&args[0], &args[1]) {
                (Value::List(a), Value::List(b)) => {
                    let mut result = a.clone();
                    result.extend(b.iter().cloned());
                    Ok(Value::List(result))
                }
                _ => Err(EvaluationError::TypeError(
                    "union requires two sequence arguments".into(),
                )),
            }
        }
        "intersection" => {
            check_arity(name, args, 2)?;
            match (&args[0], &args[1]) {
                (Value::List(a), Value::List(b)) => {
                    let result: Vec<Value> =
                        a.iter().filter(|item| b.contains(item)).cloned().collect();
                    Ok(Value::List(result))
                }
                _ => Err(EvaluationError::TypeError(
                    "intersection requires two sequence arguments".into(),
                )),
            }
        }

        // -- Sequence functions (from SequenceFunctions.kerml) -------------
        "subsequence" => {
            check_arity(name, args, 3)?;
            match (&args[0], &args[1], &args[2]) {
                (Value::List(items), Value::Int(start), Value::Int(end)) => {
                    let len = items.len() as i64;
                    // SysML uses 1-based indexing; clamp to valid range
                    let s = (*start).max(1) as usize - 1;
                    let e = (*end).min(len) as usize;
                    if s >= e || s >= items.len() {
                        Ok(Value::List(Vec::new()))
                    } else {
                        Ok(Value::List(items[s..e].to_vec()))
                    }
                }
                _ => Err(EvaluationError::TypeError(
                    "subsequence requires (sequence, integer, integer)".into(),
                )),
            }
        }
        "indexOf" => {
            check_arity(name, args, 2)?;
            match &args[0] {
                Value::List(items) => {
                    // SysML 1-based index; 0 if not found
                    let pos = items.iter().position(|v| v == &args[1]);
                    Ok(Value::Int(pos.map(|p| p as i64 + 1).unwrap_or(0)))
                }
                _ => Err(EvaluationError::TypeError(
                    "indexOf requires a sequence as the first argument".into(),
                )),
            }
        }

        // ── Temporal query functions (require ctx.trace) ──
        "was_in_state" => {
            // was_in_state("subsystem_name", "state_name") -> bool
            check_arity(name, args, 2)?;
            let subsystem = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "was_in_state: first arg (subsystem) must be a string".into(),
                    ))
                }
            };
            let state_name = match &args[1] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "was_in_state: second arg (state) must be a string".into(),
                    ))
                }
            };
            let trace = ctx.trace.as_ref().ok_or_else(|| {
                EvaluationError::Runtime("was_in_state: no execution trace available".into())
            })?;
            let found = trace.iter().any(|snap| {
                snap.subsystem_states
                    .get(&subsystem)
                    .map(|ss| ss.current_state == state_name)
                    .unwrap_or(false)
            });
            Ok(Value::Bool(found))
        }

        "state_at" => {
            // state_at("subsystem_name", tick) -> string
            check_arity(name, args, 2)?;
            let subsystem = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "state_at: first arg (subsystem) must be a string".into(),
                    ))
                }
            };
            let tick = match &args[1] {
                Value::Int(t) => *t as u64,
                Value::Float(f) => *f as u64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "state_at: second arg (tick) must be a number".into(),
                    ))
                }
            };
            let trace = ctx.trace.as_ref().ok_or_else(|| {
                EvaluationError::Runtime("state_at: no execution trace available".into())
            })?;
            let state = trace
                .iter()
                .find(|snap| snap.tick == tick)
                .and_then(|snap| snap.subsystem_states.get(&subsystem))
                .map(|ss| ss.current_state.clone())
                .unwrap_or_default();
            Ok(Value::String(state))
        }

        "ticks_in_state" => {
            // ticks_in_state("subsystem_name", "state_name") -> int
            check_arity(name, args, 2)?;
            let subsystem = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "ticks_in_state: first arg (subsystem) must be a string".into(),
                    ))
                }
            };
            let state_name = match &args[1] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "ticks_in_state: second arg (state) must be a string".into(),
                    ))
                }
            };
            let trace = ctx.trace.as_ref().ok_or_else(|| {
                EvaluationError::Runtime("ticks_in_state: no execution trace available".into())
            })?;
            let count = trace
                .iter()
                .filter(|snap| {
                    snap.subsystem_states
                        .get(&subsystem)
                        .map(|ss| ss.current_state == state_name)
                        .unwrap_or(false)
                })
                .count();
            Ok(Value::Int(count as i64))
        }

        "variable_at" => {
            // variable_at("var_name", tick) -> value
            check_arity(name, args, 2)?;
            let var_name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "variable_at: first arg (name) must be a string".into(),
                    ))
                }
            };
            let tick = match &args[1] {
                Value::Int(t) => *t as u64,
                Value::Float(f) => *f as u64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "variable_at: second arg (tick) must be a number".into(),
                    ))
                }
            };
            let trace = ctx.trace.as_ref().ok_or_else(|| {
                EvaluationError::Runtime("variable_at: no execution trace available".into())
            })?;
            let value = trace
                .iter()
                .find(|snap| snap.tick == tick)
                .and_then(|snap| snap.variables.get(&var_name).cloned())
                .unwrap_or(Value::Null);
            Ok(value)
        }

        "held_for" => {
            // held_for("expression_string", n_ticks) -> bool
            // Evaluates the expression against each of the last N ticks' context.
            // Returns true only if the expression was true for ALL of those ticks.
            check_arity(name, args, 2)?;
            let expr_str = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "held_for: first arg (expression) must be a string".into(),
                    ))
                }
            };
            let n = match &args[1] {
                Value::Int(n) => *n as usize,
                Value::Float(f) => *f as usize,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "held_for: second arg (n_ticks) must be a number".into(),
                    ))
                }
            };
            let trace = ctx.trace.as_ref().ok_or_else(|| {
                EvaluationError::Runtime("held_for: no execution trace available".into())
            })?;

            // Compile the expression
            let compiled = crate::expressions::compile_simple_expression(&expr_str).map_err(
                |_compile_err| {
                    EvaluationError::Runtime(format!(
                        "held_for: failed to compile expression '{}'",
                        expr_str
                    ))
                },
            )?;
            let evaluator = crate::expressions::ExpressionEvaluator::new();

            // Check last N ticks
            if trace.len() < n {
                return Ok(Value::Bool(false)); // Not enough history
            }
            let start = trace.len() - n;
            for snap in &trace[start..] {
                // Build temporary EvalContext from snapshot variables for expression evaluation
                let tmp_ctx = EvalContext {
                    variables: std::sync::Arc::new(snap.variables.clone()),
                    trace: None,
                    graph: ctx.graph.clone(),
                    occurrence_registry: ctx.occurrence_registry.clone(),
                    frame_registry: ctx.frame_registry.clone(),
                    calculations: ctx.calculations.clone(),
                    // Historical-snapshot scratch context — never routes to
                    // the live slot store (RSC-2.2), and must not serve
                    // by-SlotId reads of LIVE values while evaluating
                    // against historical variables (RSC-2.3).
                    slots: None,
                    slot_reader: None,
                    // OPT #1: no slot-fast lane in a historical-snapshot
                    // scratch — SlotRef reads fall through to the variables map.
                    fast_slots: Vec::new(),
                };
                match evaluator.eval(&compiled, &tmp_ctx) {
                    Ok(Value::Bool(true)) => continue,
                    _ => return Ok(Value::Bool(false)),
                }
            }
            Ok(Value::Bool(true))
        }

        // ── Math functions (Phase 15: ODE support) ──────────────────────
        "exp" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.exp())),
                Value::Int(n) => Ok(Value::Float((*n as f64).exp())),
                _ => Err(EvaluationError::TypeError("exp requires numeric".into())),
            }
        }
        "sqrt" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) if *f < 0.0 => {
                    Err(EvaluationError::TypeError("sqrt of negative number".into()))
                }
                Value::Float(f) => Ok(Value::Float(f.sqrt())),
                Value::Int(n) if *n < 0 => {
                    Err(EvaluationError::TypeError("sqrt of negative number".into()))
                }
                Value::Int(n) => Ok(Value::Float((*n as f64).sqrt())),
                _ => Err(EvaluationError::TypeError("sqrt requires numeric".into())),
            }
        }
        "ln" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) if *f <= 0.0 => Err(EvaluationError::TypeError(
                    "ln requires positive argument".into(),
                )),
                Value::Float(f) => Ok(Value::Float(f.ln())),
                Value::Int(n) if *n <= 0 => Err(EvaluationError::TypeError(
                    "ln requires positive argument".into(),
                )),
                Value::Int(n) => Ok(Value::Float((*n as f64).ln())),
                _ => Err(EvaluationError::TypeError("ln requires numeric".into())),
            }
        }
        "sin" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.sin())),
                Value::Int(n) => Ok(Value::Float((*n as f64).sin())),
                _ => Err(EvaluationError::TypeError("sin requires numeric".into())),
            }
        }
        "cos" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.cos())),
                Value::Int(n) => Ok(Value::Float((*n as f64).cos())),
                _ => Err(EvaluationError::TypeError("cos requires numeric".into())),
            }
        }
        "tan" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.tan())),
                Value::Int(n) => Ok(Value::Float((*n as f64).tan())),
                _ => Err(EvaluationError::TypeError("tan requires numeric".into())),
            }
        }
        "tanh" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.tanh())),
                Value::Int(n) => Ok(Value::Float((*n as f64).tanh())),
                _ => Err(EvaluationError::TypeError("tanh requires numeric".into())),
            }
        }
        "cot" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => {
                    let t = f.tan();
                    if t == 0.0 {
                        Ok(Value::Float(f64::INFINITY))
                    } else {
                        Ok(Value::Float(1.0 / t))
                    }
                }
                Value::Int(n) => {
                    let t = (*n as f64).tan();
                    if t == 0.0 {
                        Ok(Value::Float(f64::INFINITY))
                    } else {
                        Ok(Value::Float(1.0 / t))
                    }
                }
                _ => Err(EvaluationError::TypeError("cot requires numeric".into())),
            }
        }
        "asin" | "arcsin" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => {
                    if *f < -1.0 || *f > 1.0 {
                        Err(EvaluationError::TypeError(
                            "asin domain: -1.0 <= x <= 1.0".into(),
                        ))
                    } else {
                        Ok(Value::Float(f.asin()))
                    }
                }
                Value::Int(n) => {
                    let f = *n as f64;
                    if f < -1.0 || f > 1.0 {
                        Err(EvaluationError::TypeError(
                            "asin domain: -1.0 <= x <= 1.0".into(),
                        ))
                    } else {
                        Ok(Value::Float(f.asin()))
                    }
                }
                _ => Err(EvaluationError::TypeError("asin requires numeric".into())),
            }
        }
        "acos" | "arccos" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => {
                    if *f < -1.0 || *f > 1.0 {
                        Err(EvaluationError::TypeError(
                            "acos domain: -1.0 <= x <= 1.0".into(),
                        ))
                    } else {
                        Ok(Value::Float(f.acos()))
                    }
                }
                Value::Int(n) => {
                    let f = *n as f64;
                    if f < -1.0 || f > 1.0 {
                        Err(EvaluationError::TypeError(
                            "acos domain: -1.0 <= x <= 1.0".into(),
                        ))
                    } else {
                        Ok(Value::Float(f.acos()))
                    }
                }
                _ => Err(EvaluationError::TypeError("acos requires numeric".into())),
            }
        }
        "atan" | "arctan" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.atan())),
                Value::Int(n) => Ok(Value::Float((*n as f64).atan())),
                _ => Err(EvaluationError::TypeError("atan requires numeric".into())),
            }
        }
        "atan2" => {
            check_arity(name, args, 2)?;
            let (y, x) = promote_to_float(&args[0], &args[1])?;
            Ok(Value::Float(y.atan2(x)))
        }
        "deg" | "toDegrees" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.to_degrees())),
                Value::Int(n) => Ok(Value::Float((*n as f64).to_degrees())),
                _ => Err(EvaluationError::TypeError("deg requires numeric".into())),
            }
        }
        "rad" | "toRadians" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(f.to_radians())),
                Value::Int(n) => Ok(Value::Float((*n as f64).to_radians())),
                _ => Err(EvaluationError::TypeError("rad requires numeric".into())),
            }
        }
        "pi" => {
            check_arity(name, args, 0)?;
            Ok(Value::Float(std::f64::consts::PI))
        }

        // ── Complex number functions ─────────────────────────────────────
        "rect" => {
            // rect(re, im) → Complex
            check_arity(name, args, 2)?;
            let (re, im) = promote_to_float(&args[0], &args[1])?;
            Ok(Value::Complex { re, im })
        }
        "polar" => {
            // polar(magnitude, angle_radians) → Complex
            check_arity(name, args, 2)?;
            let (mag, angle) = promote_to_float(&args[0], &args[1])?;
            Ok(Value::Complex {
                re: mag * angle.cos(),
                im: mag * angle.sin(),
            })
        }
        "re" => {
            // re(z) → real part
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Complex { re, .. } => Ok(Value::Float(*re)),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                _ => Err(EvaluationError::TypeError(
                    "re requires numeric or complex".into(),
                )),
            }
        }
        "im" => {
            // im(z) → imaginary part
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Complex { im, .. } => Ok(Value::Float(*im)),
                Value::Float(_) | Value::Int(_) => Ok(Value::Float(0.0)),
                _ => Err(EvaluationError::TypeError(
                    "im requires numeric or complex".into(),
                )),
            }
        }
        "arg" | "phase" => {
            // arg(z) → phase angle in radians
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Complex { re, im } => Ok(Value::Float(im.atan2(*re))),
                Value::Float(f) => Ok(Value::Float(if *f >= 0.0 {
                    0.0
                } else {
                    std::f64::consts::PI
                })),
                Value::Int(n) => Ok(Value::Float(if *n >= 0 {
                    0.0
                } else {
                    std::f64::consts::PI
                })),
                _ => Err(EvaluationError::TypeError(
                    "arg requires numeric or complex".into(),
                )),
            }
        }
        "conj" => {
            // conj(z) → complex conjugate
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Complex { re, im } => Ok(Value::Complex { re: *re, im: -im }),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Int(n) => Ok(Value::Int(*n)),
                _ => Err(EvaluationError::TypeError(
                    "conj requires numeric or complex".into(),
                )),
            }
        }

        // ── Type conversion functions ────────────────────────────────────
        "ToInteger" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Int(*f as i64)),
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
                Value::String(s) => s.parse::<i64>().map(Value::Int).map_err(|_| {
                    EvaluationError::TypeError(format!(
                        "ToInteger: cannot parse '{}' as integer",
                        s
                    ))
                }),
                _ => Err(EvaluationError::TypeError(
                    "ToInteger requires Float, Int, Bool, or String".into(),
                )),
            }
        }
        "ToReal" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Bool(b) => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
                Value::String(s) => s.parse::<f64>().map(Value::Float).map_err(|_| {
                    EvaluationError::TypeError(format!("ToReal: cannot parse '{}' as real", s))
                }),
                _ => Err(EvaluationError::TypeError(
                    "ToReal requires Int, Float, Bool, or String".into(),
                )),
            }
        }

        // ── Boolean collection functions ─────────────────────────────────
        // PassIf(isPassing: Boolean): VerdictKind — the normative verification
        // helper (VerificationCases.sysml:70-79): `return verdict : VerdictKind =
        // if isPassing? VerdictKind::pass else VerdictKind::fail`. VerdictKind is
        // rendered as its lowercase literal string ("pass"/"fail"), which
        // `cases::verdict_from_value` reads back into a `VerdictKind`. Implemented
        // natively for the same reason the trig/sampled-function library calcs are
        // (a small, signature-exact library primitive), so a modeled `return
        // verdict = PassIf(...)` evaluates without importing the library.
        "PassIf" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Bool(b) => Ok(Value::String(
                    if *b { "pass" } else { "fail" }.to_owned(),
                )),
                _ => Err(EvaluationError::TypeError(
                    "PassIf requires a Boolean argument".into(),
                )),
            }
        }
        "allTrue" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => {
                    for item in items {
                        match item {
                            Value::Bool(b) => {
                                if !b {
                                    return Ok(Value::Bool(false));
                                }
                            }
                            _ => {
                                return Err(EvaluationError::TypeError(
                                    "allTrue: list must contain only Bools".into(),
                                ))
                            }
                        }
                    }
                    Ok(Value::Bool(true))
                }
                _ => Err(EvaluationError::TypeError("allTrue requires a List".into())),
            }
        }
        "anyTrue" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::List(items) => {
                    for item in items {
                        match item {
                            Value::Bool(b) => {
                                if *b {
                                    return Ok(Value::Bool(true));
                                }
                            }
                            _ => {
                                return Err(EvaluationError::TypeError(
                                    "anyTrue: list must contain only Bools".into(),
                                ))
                            }
                        }
                    }
                    Ok(Value::Bool(false))
                }
                _ => Err(EvaluationError::TypeError("anyTrue requires a List".into())),
            }
        }

        // ── SampledFunction accessor functions ───────────────────────────
        "Domain" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Map(map) => map.get("domain").cloned().ok_or_else(|| {
                    EvaluationError::TypeError("Domain: map has no 'domain' key".into())
                }),
                _ => Err(EvaluationError::TypeError(
                    "Domain requires a SampledFunction (Map)".into(),
                )),
            }
        }
        "Range" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Map(map) => map.get("range").cloned().ok_or_else(|| {
                    EvaluationError::TypeError("Range: map has no 'range' key".into())
                }),
                _ => Err(EvaluationError::TypeError(
                    "Range requires a SampledFunction (Map)".into(),
                )),
            }
        }

        // ── SampledFunction operations (from SampledFunctions.sysml) ────

        // SamplePair(domainValue, rangeValue) → Value::Map with __type, domainValue, rangeValue
        "SamplePair" => {
            check_arity(name, args, 2)?;
            let d = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "SamplePair: domainValue must be numeric".into(),
                    ))
                }
            };
            let r = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "SamplePair: rangeValue must be numeric".into(),
                    ))
                }
            };
            let mut map = std::collections::BTreeMap::new();
            map.insert(
                "__type".to_string(),
                Value::String("SamplePair".to_string()),
            );
            map.insert("domainValue".to_string(), Value::Float(d));
            map.insert("rangeValue".to_string(), Value::Float(r));
            Ok(Value::Map(map))
        }

        "SampledFunction" => {
            // Two construction forms:
            // 1. SampledFunction(domain_list, range_list) — legacy flat format
            // 2. SampledFunction(sample_pair_list) — spec format (list of SamplePair maps)
            match args.len() {
                1 => {
                    // Spec format: SampledFunction(samples) where samples is a list of SamplePairs
                    let pairs = extract_sample_pairs_from_list(&args[0])?;
                    build_sampled_function_from_pairs(pairs)
                }
                2 => {
                    // Legacy flat format: SampledFunction(domain_list, range_list)
                    let domain = value_to_f64_list(&args[0])?;
                    let range = value_to_f64_list(&args[1])?;
                    if domain.len() != range.len() {
                        return Err(EvaluationError::TypeError(format!(
                            "SampledFunction: domain length ({}) != range length ({})",
                            domain.len(),
                            range.len()
                        )));
                    }
                    let pairs: Vec<(f64, f64)> = domain.into_iter().zip(range).collect();
                    build_sampled_function_from_pairs(pairs)
                }
                n => Err(EvaluationError::ArityMismatch {
                    name: "SampledFunction".to_string(),
                    expected: 2,
                    got: n,
                }),
            }
        }
        "Interpolate" | "interpolateLinear" => {
            // Interpolate(sampled_function, domain_value) → range value.
            check_arity(name, args, 2)?;
            let (domain, range) = extract_sampled_function_data(&args[0])?;
            let x = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "Interpolate: second arg must be numeric".into(),
                    ))
                }
            };
            // SysML §9.4.3.2.2 / SampledFunctions.sysml:80-84: `Interpolate`
            // returns `null` when the domain value is outside the
            // SampledFunction's domain bounds (no extrapolation). The bounds are
            // direction-agnostic (min/max of the endpoints) so a strictly-
            // decreasing domain is handled too. `interpolateLinear` is the
            // internal ODE/simulation helper that instead CLAMPS to the nearest
            // edge value for integration edge-continuity — an intentional tool
            // divergence from the spec's null contract (note `interpolateLinear :
            // Interpolate`); flagged for review, not used by the spec gate.
            if name == "Interpolate" && !domain.is_empty() {
                let lo = domain[0].min(domain[domain.len() - 1]);
                let hi = domain[0].max(domain[domain.len() - 1]);
                if x < lo || x > hi {
                    return Ok(Value::Null);
                }
            }
            Ok(Value::Float(interpolate_linear_impl(&domain, &range, x)?))
        }
        // Saturating lookup: like `interpolateLinear` but EXTRAPOLATES past the
        // domain along the end-segment slope instead of clamping flat. For an
        // ODE-coupled constitutive curve (e.g. inverse B-H `H(B)`) this models
        // the saturation divergence — see [`interpolate_linear_extrap_impl`].
        "interpolateSaturating" => {
            check_arity(name, args, 2)?;
            let (domain, range) = extract_sampled_function_data(&args[0])?;
            let x = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "interpolateSaturating: second arg must be numeric".into(),
                    ))
                }
            };
            Ok(Value::Float(interpolate_linear_extrap_impl(
                &domain, &range, x,
            )?))
        }
        "Sample" => {
            // Sample(sampled_function, domain_values_list) → new SampledFunction
            check_arity(name, args, 2)?;
            let (domain, range) = extract_sampled_function_data(&args[0])?;
            let sample_points = value_to_f64_list(&args[1])?;
            let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(sample_points.len());
            for x in &sample_points {
                pairs.push((*x, interpolate_linear_impl(&domain, &range, *x)?));
            }
            build_sampled_function_from_pairs(pairs)
        }

        // ── Clock functions (from Clocks.kerml) ──────────────────────────
        "TimeOf" => {
            // TimeOf() with no args: return __clock_time from context.
            // TimeOf(t) with 1 arg: return the argument unchanged (occurrence start time).
            match args.len() {
                0 => ctx.get("__clock_time").cloned().ok_or_else(|| {
                    EvaluationError::Runtime(
                        "TimeOf: no clock time available (orchestrator not running?)".into(),
                    )
                }),
                1 => match &args[0] {
                    Value::Float(_) | Value::Int(_) => Ok(args[0].clone()),
                    _ => Err(EvaluationError::TypeError(
                        "TimeOf requires a numeric argument".into(),
                    )),
                },
                _ => Err(EvaluationError::ArityMismatch {
                    name: "TimeOf".to_owned(),
                    expected: 0,
                    got: args.len(),
                }),
            }
        }
        "DurationOf" => {
            // DurationOf(start, end) -> end - start
            check_arity(name, args, 2)?;
            let (a, b) = promote_to_float(&args[0], &args[1])?;
            Ok(Value::Float(b - a))
        }

        // -- Vector operations (VectorCalculations.sysml) ----------------------
        "inner" => {
            // Dot product: inner(u, v) → scalar
            check_arity(name, args, 2)?;
            let u = value_to_f64_vec(&args[0])?;
            let v = value_to_f64_vec(&args[1])?;
            if u.len() != v.len() {
                return Err(EvaluationError::Runtime(format!(
                    "inner: vectors must have same length ({} vs {})",
                    u.len(),
                    v.len()
                )));
            }
            Ok(Value::Float(
                u.iter().zip(v.iter()).map(|(a, b)| a * b).sum(),
            ))
        }

        "outer" => {
            // Cross product (3D only): outer(u, v) → vector
            check_arity(name, args, 2)?;
            let u = value_to_f64_vec(&args[0])?;
            let v = value_to_f64_vec(&args[1])?;
            if u.len() != 3 || v.len() != 3 {
                return Err(EvaluationError::Runtime(format!(
                    "outer: requires 3D vectors ({} and {} given)",
                    u.len(),
                    v.len()
                )));
            }
            Ok(Value::List(vec![
                Value::Float(u[1] * v[2] - u[2] * v[1]),
                Value::Float(u[2] * v[0] - u[0] * v[2]),
                Value::Float(u[0] * v[1] - u[1] * v[0]),
            ]))
        }

        "norm" => {
            // L2 norm: norm(v) → scalar
            check_arity(name, args, 1)?;
            let v = value_to_f64_vec(&args[0])?;
            Ok(Value::Float(v.iter().map(|x| x * x).sum::<f64>().sqrt()))
        }

        "angle" => {
            // Angle between vectors in radians: angle(u, v) → scalar
            check_arity(name, args, 2)?;
            let u = value_to_f64_vec(&args[0])?;
            let v = value_to_f64_vec(&args[1])?;
            if u.len() != v.len() {
                return Err(EvaluationError::Runtime(format!(
                    "angle: vectors must have same length ({} vs {})",
                    u.len(),
                    v.len()
                )));
            }
            let dot: f64 = u.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            let norm_u = u.iter().map(|x| x * x).sum::<f64>().sqrt();
            let norm_v = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            let denom = norm_u * norm_v;
            if denom == 0.0 {
                return Err(EvaluationError::Runtime("angle: zero-length vector".into()));
            }
            // Clamp to [-1, 1] to avoid NaN from floating-point imprecision
            let cos_theta = (dot / denom).clamp(-1.0, 1.0);
            Ok(Value::Float(cos_theta.acos()))
        }

        "scalarVectorMult" | "vectorScalarMult" => {
            // Scalar-vector multiplication (element-wise)
            check_arity(name, args, 2)?;
            let (scalar, vec_val) = if matches!(&args[0], Value::List(_)) {
                (&args[1], &args[0])
            } else {
                (&args[0], &args[1])
            };
            let k = match scalar {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(format!(
                        "{name}: expected scalar, got {:?}",
                        scalar
                    )))
                }
            };
            let v = value_to_f64_vec(vec_val)?;
            Ok(Value::List(v.iter().map(|x| Value::Float(k * x)).collect()))
        }

        "vectorScalarDiv" => {
            // Vector / scalar (element-wise)
            check_arity(name, args, 2)?;
            let v = value_to_f64_vec(&args[0])?;
            let k = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(format!(
                        "vectorScalarDiv: expected scalar divisor, got {:?}",
                        args[1]
                    )))
                }
            };
            if k == 0.0 {
                return Err(EvaluationError::DivisionByZero);
            }
            Ok(Value::List(v.iter().map(|x| Value::Float(x / k)).collect()))
        }

        "isZeroVectorQuantity" | "isZeroVector" => {
            check_arity(name, args, 1)?;
            let v = value_to_f64_vec(&args[0])?;
            Ok(Value::Bool(v.iter().all(|x| *x == 0.0)))
        }

        "isUnitVectorQuantity" | "isUnitVector" => {
            check_arity(name, args, 1)?;
            let v = value_to_f64_vec(&args[0])?;
            let mag = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            Ok(Value::Bool((mag - 1.0).abs() < 1e-12))
        }

        // ----- Quantity & Unit functions -----

        // ConvertQuantity(value, targetUnit) — spec-standard unit conversion
        "ConvertQuantity" | "convertQuantity" => {
            check_arity(name, args, 2)?;
            let target_unit = match &args[1] {
                Value::String(s) => s.as_str(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "ConvertQuantity: second argument must be a unit name string".into(),
                    ))
                }
            };
            match &args[0] {
                Value::Quantity {
                    value,
                    dimension,
                    unit,
                } => {
                    let (new_val, new_dim, new_unit) = super::units::convert_quantity(
                        *value,
                        dimension,
                        unit.as_deref(),
                        target_unit,
                    )
                    .map_err(EvaluationError::Runtime)?;
                    Ok(Value::Quantity {
                        value: new_val,
                        dimension: new_dim,
                        unit: Some(new_unit),
                    })
                }
                // Allow converting plain numeric values (assumed SI base)
                _ => {
                    let v = match &args[0] {
                        Value::Float(f) => *f,
                        Value::Int(n) => *n as f64,
                        _ => {
                            return Err(EvaluationError::TypeError(
                                "ConvertQuantity: first argument must be numeric or quantity"
                                    .into(),
                            ))
                        }
                    };
                    let dim = sysml_core::physics::DimensionVector::default();
                    let (new_val, new_dim, new_unit) =
                        super::units::convert_quantity(v, &dim, None, target_unit)
                            .map_err(EvaluationError::Runtime)?;
                    Ok(Value::Quantity {
                        value: new_val,
                        dimension: new_dim,
                        unit: Some(new_unit),
                    })
                }
            }
        }

        // quantity(value, unitName) — construct a Quantity from a numeric value and unit name
        "quantity" => {
            check_arity(name, args, 2)?;
            let v = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                Value::Quantity { value, .. } => *value,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "quantity(): first argument must be numeric".into(),
                    ))
                }
            };
            let unit_name = match &args[1] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "quantity(): second argument must be a unit name string".into(),
                    ))
                }
            };
            let entry = super::units::lookup_unit(&unit_name).ok_or_else(|| {
                EvaluationError::Runtime(format!("quantity(): unknown unit '{unit_name}'"))
            })?;
            Ok(Value::Quantity {
                value: v,
                dimension: entry.dimension,
                unit: Some(unit_name),
            })
        }

        // unitOf(quantity) — extract the unit name from a quantity
        "unitOf" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Quantity { unit, .. } => Ok(Value::String(
                    unit.clone().unwrap_or_else(|| "dimensionless".to_string()),
                )),
                _ => Ok(Value::String("dimensionless".to_string())),
            }
        }

        // dimensionOf(quantity) — extract dimension string
        "dimensionOf" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Quantity { dimension, .. } => Ok(Value::String(dimension.to_string())),
                _ => Ok(Value::String("1".to_string())),
            }
        }

        // numericValue(quantity) — extract the raw f64 value
        "numericValue" | "num" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Quantity { value, .. } => Ok(Value::Float(*value)),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                _ => Err(EvaluationError::TypeError(
                    "numericValue: expected numeric or quantity".into(),
                )),
            }
        }

        // ----- Collection operations (SequenceFunctions/CollectionFunctions) -----

        // contains — alias for includes
        "contains" => {
            check_arity(name, args, 2)?;
            match &args[0] {
                Value::List(items) => Ok(Value::Bool(items.contains(&args[1]))),
                _ => Err(EvaluationError::TypeError(
                    "contains requires sequence".into(),
                )),
            }
        }

        // containsAll — check all elements of second list are in first
        "containsAll" => {
            check_arity(name, args, 2)?;
            let list = value_to_list(&args[0])?;
            let targets = value_to_list(&args[1])?;
            Ok(Value::Bool(targets.iter().all(|t| list.contains(t))))
        }

        // equals — structural equality
        "equals" => {
            check_arity(name, args, 2)?;
            Ok(Value::Bool(values_equal(&args[0], &args[1])))
        }

        // same — reference identity (same as ===, uses structural equality)
        "same" => {
            check_arity(name, args, 2)?;
            Ok(Value::Bool(values_equal(&args[0], &args[1])))
        }

        // including — return new list with element appended
        "including" => {
            check_arity(name, args, 2)?;
            let list = value_to_list(&args[0])?;
            let mut result = list.to_vec();
            result.push(args[1].clone());
            Ok(Value::List(result))
        }

        // includingAt — return new list with element inserted at index
        "includingAt" => {
            check_arity(name, args, 3)?;
            let list = value_to_list(&args[0])?;
            let idx = match &args[1] {
                Value::Int(i) => *i as usize,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "includingAt: index must be integer".into(),
                    ))
                }
            };
            let mut result = list.to_vec();
            let pos = idx.min(result.len());
            result.insert(pos, args[2].clone());
            Ok(Value::List(result))
        }

        // excluding — return new list with first occurrence of element removed
        "excluding" => {
            check_arity(name, args, 2)?;
            let list = value_to_list(&args[0])?;
            let mut result = list.to_vec();
            if let Some(pos) = result.iter().position(|v| v == &args[1]) {
                result.remove(pos);
            }
            Ok(Value::List(result))
        }

        // excludingAt — return new list with element at index removed
        "excludingAt" => {
            check_arity(name, args, 2)?;
            let list = value_to_list(&args[0])?;
            let idx = match &args[1] {
                Value::Int(i) => *i as usize,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "excludingAt: index must be integer".into(),
                    ))
                }
            };
            let mut result = list.to_vec();
            if idx < result.len() {
                result.remove(idx);
            }
            Ok(Value::List(result))
        }

        // includesOnly — all elements of first list are in second list
        "includesOnly" => {
            check_arity(name, args, 2)?;
            let list = value_to_list(&args[0])?;
            let allowed = value_to_list(&args[1])?;
            Ok(Value::Bool(list.iter().all(|v| allowed.contains(v))))
        }

        // selectOne — return first element (deterministic; spec allows non-deterministic)
        "selectOne" => {
            check_arity(name, args, 1)?;
            let list = value_to_list(&args[0])?;
            Ok(list.first().cloned().unwrap_or(Value::Null))
        }

        // reduce — fold: reduce(list, init, fn_name)
        // Simplified: reduces with binary function applied left-to-right
        // reduce([1,2,3], 0, "+") = ((0+1)+2)+3 = 6
        "reduce" => {
            if args.len() < 2 {
                return Err(EvaluationError::ArityMismatch {
                    name: name.to_owned(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let list = value_to_list(&args[0])?;
            if list.is_empty() {
                return Ok(if args.len() > 2 {
                    args[2].clone()
                } else {
                    Value::Null
                });
            }
            // reduce(list, fn_name) — first element is initial accumulator
            // reduce(list, init, fn_name) — explicit initial value
            let (mut acc, fn_name, start_idx) = if args.len() >= 3 {
                let fname = args[2].as_str().unwrap_or("+").to_string();
                (args[1].clone(), fname, 0)
            } else {
                let fname = args[1].as_str().unwrap_or("+").to_string();
                (list[0].clone(), fname, 1)
            };
            for item in &list[start_idx..] {
                acc = eval_function(&fn_name, &[acc, item.clone()], ctx)?;
            }
            Ok(acc)
        }

        // minimize — return minimum element by numeric value
        "minimize" => {
            check_arity(name, args, 1)?;
            let list = value_to_list(&args[0])?;
            let mut best: Option<&Value> = None;
            let mut best_f = f64::INFINITY;
            for item in list {
                if let Some(f) = item.as_float() {
                    if f < best_f {
                        best_f = f;
                        best = Some(item);
                    }
                }
            }
            Ok(best.cloned().unwrap_or(Value::Null))
        }

        // maximize — return maximum element by numeric value
        "maximize" => {
            check_arity(name, args, 1)?;
            let list = value_to_list(&args[0])?;
            let mut best: Option<&Value> = None;
            let mut best_f = f64::NEG_INFINITY;
            for item in list {
                if let Some(f) = item.as_float() {
                    if f > best_f {
                        best_f = f;
                        best = Some(item);
                    }
                }
            }
            Ok(best.cloned().unwrap_or(Value::Null))
        }

        // ----- Numeric utilities -----

        // gcd — greatest common divisor (Euclidean algorithm)
        "gcd" => {
            check_arity(name, args, 2)?;
            let mut a = match &args[0] {
                Value::Int(n) => n.unsigned_abs(),
                _ => return Err(EvaluationError::TypeError("gcd requires integers".into())),
            };
            let mut b = match &args[1] {
                Value::Int(n) => n.unsigned_abs(),
                _ => return Err(EvaluationError::TypeError("gcd requires integers".into())),
            };
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            Ok(Value::Int(a as i64))
        }

        // ----- Type conversion functions -----

        // ToNatural — convert to non-negative integer
        "ToNatural" => {
            check_arity(name, args, 1)?;
            let n = match &args[0] {
                Value::Int(i) => *i,
                Value::Float(f) => *f as i64,
                Value::Quantity { value, .. } => *value as i64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "ToNatural: expected numeric".into(),
                    ))
                }
            };
            if n < 0 {
                Err(EvaluationError::Runtime(
                    "ToNatural: value is negative".into(),
                ))
            } else {
                Ok(Value::Int(n))
            }
        }

        // ToComplex — convert numeric to complex
        "ToComplex" => {
            check_arity(name, args, 1)?;
            let f = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                Value::Quantity { value, .. } => *value,
                Value::Complex { .. } => return Ok(args[0].clone()),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "ToComplex: expected numeric".into(),
                    ))
                }
            };
            Ok(Value::Complex { re: f, im: 0.0 })
        }

        // ToRational — approximate: returns Float (no Rational type yet)
        "ToRational" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                Value::Quantity { value, .. } => Ok(Value::Float(*value)),
                _ => Err(EvaluationError::TypeError(
                    "ToRational: expected numeric".into(),
                )),
            }
        }

        // numer — numerator (approximate: round Float to Int)
        "numer" => {
            check_arity(name, args, 1)?;
            let f = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => return Err(EvaluationError::TypeError("numer: expected numeric".into())),
            };
            Ok(Value::Int(f.round() as i64))
        }

        // denom — denominator (approximate: return 1 for all)
        "denom" => {
            check_arity(name, args, 1)?;
            Ok(Value::Int(1))
        }

        // rat — construct rational from numerator/denominator (approximate: Float division)
        "rat" => {
            check_arity(name, args, 2)?;
            let (n, d) = promote_to_float(&args[0], &args[1])?;
            if d == 0.0 {
                Err(EvaluationError::DivisionByZero)
            } else {
                Ok(Value::Float(n / d))
            }
        }

        // ── Cartesian vector delegations (no coordinate frames — identical to base ops) ──
        "cartesianInner" => eval_function("inner", args, ctx),
        "cartesianNorm" => eval_function("norm", args, ctx),
        "cartesianAngle" => eval_function("angle", args, ctx),
        "cartesianScalarVectorMult" => eval_function("scalarVectorMult", args, ctx),
        "cartesianVectorScalarMult" => eval_function("vectorScalarMult", args, ctx),
        "isCartesianZeroVector" => eval_function("isZeroVector", args, ctx),
        "cartesian+" => {
            // Vector addition (element-wise)
            check_arity(name, args, 2)?;
            let a = value_to_f64_vec(&args[0])?;
            let b = value_to_f64_vec(&args[1])?;
            if a.len() != b.len() {
                return Err(EvaluationError::TypeError(
                    "cartesian+: vectors must have same length".into(),
                ));
            }
            Ok(Value::List(
                a.iter()
                    .zip(b.iter())
                    .map(|(x, y)| Value::Float(x + y))
                    .collect(),
            ))
        }
        "cartesian-" => {
            check_arity(name, args, 2)?;
            let a = value_to_f64_vec(&args[0])?;
            let b = value_to_f64_vec(&args[1])?;
            if a.len() != b.len() {
                return Err(EvaluationError::TypeError(
                    "cartesian-: vectors must have same length".into(),
                ));
            }
            Ok(Value::List(
                a.iter()
                    .zip(b.iter())
                    .map(|(x, y)| Value::Float(x - y))
                    .collect(),
            ))
        }

        // ── Quantity-aware vector aliases (delegate to existing) ──
        "scalarQuantityVectorMult" => eval_function("scalarVectorMult", args, ctx),
        "vectorScalarQuantityMult" => eval_function("vectorScalarMult", args, ctx),
        "vectorScalarQuantityDiv" => eval_function("vectorScalarDiv", args, ctx),

        // ── Tensor operations (nested lists as matrices) ──
        "scalarTensorMult" | "scalarQuantityTensorMult" => {
            // scalar × tensor → element-wise multiply
            check_arity(name, args, 2)?;
            let s = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                Value::Quantity { value, .. } => *value,
                _ => return Err(EvaluationError::TypeError("scalar expected".into())),
            };
            scalar_tensor_mult(s, &args[1])
        }
        "TensorScalarMult" | "TensorScalarQuantityMult" => {
            // tensor × scalar → element-wise multiply (arg order swapped)
            check_arity(name, args, 2)?;
            let s = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                Value::Quantity { value, .. } => *value,
                _ => return Err(EvaluationError::TypeError("scalar expected".into())),
            };
            scalar_tensor_mult(s, &args[0])
        }
        "tensorVectorMult" => {
            // matrix × vector → vector
            check_arity(name, args, 2)?;
            let mat = value_to_matrix(&args[0])?;
            let vec = value_to_f64_vec(&args[1])?;
            let result = matrix_vector_mult(&mat, &vec)?;
            Ok(Value::List(result.into_iter().map(Value::Float).collect()))
        }
        "vectorTensorMult" => {
            // vector × matrix → vector (v^T * M)
            check_arity(name, args, 2)?;
            let vec = value_to_f64_vec(&args[0])?;
            let mat = value_to_matrix(&args[1])?;
            let result = vector_matrix_mult(&vec, &mat)?;
            Ok(Value::List(result.into_iter().map(Value::Float).collect()))
        }
        "tensorTensorMult" => {
            // matrix × matrix → matrix
            check_arity(name, args, 2)?;
            let a = value_to_matrix(&args[0])?;
            let b = value_to_matrix(&args[1])?;
            let result = matrix_matrix_mult(&a, &b)?;
            Ok(Value::List(
                result
                    .into_iter()
                    .map(|row| Value::List(row.into_iter().map(Value::Float).collect()))
                    .collect(),
            ))
        }
        "isZeroTensorQuantity" => {
            check_arity(name, args, 1)?;
            let mat = value_to_matrix(&args[0])?;
            Ok(Value::Bool(
                mat.iter()
                    .all(|row| row.iter().all(|&v| v.abs() < f64::EPSILON)),
            ))
        }
        "isUnitTensorQuantity" => {
            // Identity matrix check
            check_arity(name, args, 1)?;
            let mat = value_to_matrix(&args[0])?;
            let n = mat.len();
            let is_identity = mat.iter().enumerate().all(|(i, row)| {
                row.len() == n
                    && row.iter().enumerate().all(|(j, &v)| {
                        if i == j {
                            (v - 1.0).abs() < f64::EPSILON
                        } else {
                            v.abs() < f64::EPSILON
                        }
                    })
            });
            Ok(Value::Bool(is_identity))
        }

        // ── Clock functions ──
        "BasicTimeOf" => eval_function("TimeOf", args, ctx),
        "BasicDurationOf" | "DurationOf" => {
            // DurationOf(t1, t2) → |t2 - t1|
            match args.len() {
                0 => ctx.get("__clock_duration").cloned().ok_or_else(|| {
                    EvaluationError::Runtime("DurationOf: no clock duration available".into())
                }),
                2 => {
                    let t1 = match &args[0] {
                        Value::Float(f) => *f,
                        Value::Int(n) => *n as f64,
                        _ => {
                            return Err(EvaluationError::TypeError(
                                "DurationOf: numeric args required".into(),
                            ))
                        }
                    };
                    let t2 = match &args[1] {
                        Value::Float(f) => *f,
                        Value::Int(n) => *n as f64,
                        _ => {
                            return Err(EvaluationError::TypeError(
                                "DurationOf: numeric args required".into(),
                            ))
                        }
                    };
                    Ok(Value::Float((t2 - t1).abs()))
                }
                _ => Err(EvaluationError::ArityMismatch {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                }),
            }
        }

        // ── Quantity conversion ──
        "ToDimensionOneValue" => {
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Quantity { value, .. } => Ok(Value::Float(*value)),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                _ => Err(EvaluationError::TypeError(
                    "ToDimensionOneValue: numeric or quantity required".into(),
                )),
            }
        }

        // ── Spatial / Coordinate frame constructors ──
        // From VectorFunctions.kerml and SpatialFrames.kerml
        "VectorOf" => {
            // VectorOf(component1, component2, ...) → List of floats
            if args.is_empty() {
                return Ok(Value::List(vec![]));
            }
            let components: Vec<Value> = args
                .iter()
                .map(|a| match a {
                    Value::Float(f) => Ok(Value::Float(*f)),
                    Value::Int(n) => Ok(Value::Float(*n as f64)),
                    Value::Quantity { value, .. } => Ok(Value::Float(*value)),
                    _ => Err(EvaluationError::TypeError(format!(
                        "VectorOf: expected numeric component, got {:?}",
                        a
                    ))),
                })
                .collect::<Result<_, _>>()?;
            Ok(Value::List(components))
        }

        "CartesianVectorOf" => {
            // CartesianVectorOf(component1, ...) → List with Cartesian semantics
            // Same as VectorOf but typed as CartesianVectorValue
            if args.is_empty() {
                return Ok(Value::List(vec![]));
            }
            let components: Vec<Value> = args
                .iter()
                .map(|a| match a {
                    Value::Float(f) => Ok(Value::Float(*f)),
                    Value::Int(n) => Ok(Value::Float(*n as f64)),
                    Value::Quantity { value, .. } => Ok(Value::Float(*value)),
                    _ => Err(EvaluationError::TypeError(format!(
                        "CartesianVectorOf: expected numeric, got {:?}",
                        a
                    ))),
                })
                .collect::<Result<_, _>>()?;
            Ok(Value::List(components))
        }

        "CartesianThreeVectorOf" => {
            // CartesianThreeVectorOf(x, y, z) → 3-element List
            check_arity(name, args, 3)?;
            let components: Vec<Value> = args
                .iter()
                .enumerate()
                .map(|(i, a)| match a {
                    Value::Float(f) => Ok(Value::Float(*f)),
                    Value::Int(n) => Ok(Value::Float(*n as f64)),
                    Value::Quantity { value, .. } => Ok(Value::Float(*value)),
                    _ => Err(EvaluationError::TypeError(format!(
                        "CartesianThreeVectorOf: component {} must be numeric",
                        i
                    ))),
                })
                .collect::<Result<_, _>>()?;
            Ok(Value::List(components))
        }

        "PositionOf" | "CartesianPositionOf" => {
            // PositionOf(point, time?, frame?, clock?) → ThreeVectorValue
            // Returns the position of a point element. When we have a frame registry
            // and the model stores position attributes, resolve them. Otherwise return
            // the arguments as-is (passthrough for expression-level usage).
            if args.is_empty() {
                return Err(EvaluationError::ArityMismatch {
                    name: name.to_string(),
                    expected: 1,
                    got: 0,
                });
            }
            // Try to resolve as a 3-vector from the point's attributes
            match &args[0] {
                Value::List(l) if l.len() == 3 => Ok(args[0].clone()),
                _ => {
                    // Return a zero vector as default position (spec default frame)
                    Ok(Value::List(vec![
                        Value::Float(0.0),
                        Value::Float(0.0),
                        Value::Float(0.0),
                    ]))
                }
            }
        }

        "CurrentPositionOf" | "CartesianCurrentPositionOf" => {
            // CurrentPositionOf(point, frame?, clock?) → ThreeVectorValue
            // Same as PositionOf but implicitly uses current time.
            if args.is_empty() {
                return Err(EvaluationError::ArityMismatch {
                    name: name.to_string(),
                    expected: 1,
                    got: 0,
                });
            }
            match &args[0] {
                Value::List(l) if l.len() == 3 => Ok(args[0].clone()),
                _ => Ok(Value::List(vec![
                    Value::Float(0.0),
                    Value::Float(0.0),
                    Value::Float(0.0),
                ])),
            }
        }

        "DisplacementOf" | "CartesianDisplacementOf" => {
            // DisplacementOf(p1, p2, time?, frame?, clock?) → ThreeVectorValue
            // Returns p2 - p1 as displacement vector.
            if args.len() < 2 {
                return Err(EvaluationError::ArityMismatch {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let p1 = value_to_f64_vec(&args[0])?;
            let p2 = value_to_f64_vec(&args[1])?;
            if p1.len() != p2.len() {
                return Err(EvaluationError::TypeError(format!(
                    "{}: vectors must have same dimension ({} vs {})",
                    name,
                    p1.len(),
                    p2.len()
                )));
            }
            let disp: Vec<Value> = p1
                .iter()
                .zip(p2.iter())
                .map(|(a, b)| Value::Float(b - a))
                .collect();
            Ok(Value::List(disp))
        }

        "CurrentDisplacementOf" | "CartesianCurrentDisplacementOf" => {
            // CurrentDisplacementOf(p1, p2, frame?, clock?) → ThreeVectorValue
            // Same as DisplacementOf with implicit current time.
            if args.len() < 2 {
                return Err(EvaluationError::ArityMismatch {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len(),
                });
            }
            let p1 = value_to_f64_vec(&args[0])?;
            let p2 = value_to_f64_vec(&args[1])?;
            if p1.len() != p2.len() {
                return Err(EvaluationError::TypeError(format!(
                    "{}: vectors must have same dimension ({} vs {})",
                    name,
                    p1.len(),
                    p2.len()
                )));
            }
            let disp: Vec<Value> = p1
                .iter()
                .zip(p2.iter())
                .map(|(a, b)| Value::Float(b - a))
                .collect();
            Ok(Value::List(disp))
        }

        // ── Type system functions ──
        "all" => {
            // all(type_name) → returns all elements of the given type from the graph
            check_arity(name, args, 1)?;
            let type_name = match &args[0] {
                Value::String(s) => s.clone(),
                Value::Ref(id) => ctx
                    .graph
                    .as_ref()
                    .and_then(|g| g.get_element(id))
                    .and_then(|e| e.name.clone())
                    .unwrap_or_else(|| format!("{}", id)),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "all: argument must be a type name".into(),
                    ))
                }
            };
            let graph = ctx.graph.as_ref().ok_or_else(|| {
                EvaluationError::Runtime("all: requires model graph in context".into())
            })?;
            let mut results = Vec::new();
            for element in graph.elements.values() {
                if element.name.is_some() {
                    // Check if element's kind matches or if it has a FeatureTyping to the target
                    let kind_name = format!("{:?}", element.kind);
                    if kind_name == type_name
                        || element
                            .get_prop("unresolvedTypeName")
                            .and_then(|v| v.as_str())
                            .map(|s| s.rsplit("::").next().unwrap_or(s) == type_name)
                            .unwrap_or(false)
                    {
                        results.push(Value::Ref(element.id.clone()));
                    }
                }
            }
            Ok(Value::List(results))
        }
        "meta" => {
            // meta(value) → returns type name(s) of the value
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Ref(id) => {
                    let graph = ctx.graph.as_ref().ok_or_else(|| {
                        EvaluationError::Runtime("meta: requires model graph in context".into())
                    })?;
                    let mut names = Vec::new();
                    if let Some(element) = graph.get_element(id) {
                        if let Some(Value::String(s)) = element.get_prop("unresolvedTypeName") {
                            let n = s.rsplit("::").next().unwrap_or(s);
                            names.push(Value::String(n.to_string()));
                        }
                        // Also check resolved FeatureTyping
                        for type_id in
                            sysml_core::resolution::scoping::chaining::find_feature_types(graph, id)
                        {
                            if let Some(type_elem) = graph.get_element(&type_id) {
                                if let Some(ref tn) = type_elem.name {
                                    if !names.iter().any(|v| v.as_str() == Some(tn)) {
                                        names.push(Value::String(tn.clone()));
                                    }
                                }
                            }
                        }
                    }
                    if names.is_empty() {
                        Ok(Value::Null)
                    } else if names.len() == 1 {
                        Ok(names.into_iter().next().unwrap())
                    } else {
                        Ok(Value::List(names))
                    }
                }
                other => Ok(Value::String(other.type_name().to_string())),
            }
        }

        // ── Occurrence model (from OccurrenceFunctions.kerml) ──
        "create" => {
            // create(occurrences...) — establishes startShot for each occurrence.
            // In expression context: create a new occurrence in the registry, return its ID.
            if let Some(reg) = &ctx.occurrence_registry {
                let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let life_id = reg.create_life(None);
                let time = ctx
                    .get("__time")
                    .and_then(|v| match v {
                        Value::Float(f) => Some(*f),
                        Value::Int(n) => Some(*n as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                let features = std::collections::HashMap::new();
                let clock = ctx
                    .get("__clock")
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "default".into());
                let occ_id = reg.create(life_id, time, features, clock);
                Ok(Value::String(occ_id.to_string()))
            } else {
                Err(EvaluationError::NotYetImplemented {
                    name: name.to_string(),
                    reason: "no occurrence registry in context".into(),
                })
            }
        }
        "destroy" => {
            // destroy(occurrence_ref) — finalizes endShot for the occurrence.
            check_arity(name, args, 1)?;
            if let Some(reg) = &ctx.occurrence_registry {
                let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let occ_id_str = match &args[0] {
                    Value::String(s) => s.clone(),
                    Value::Ref(id) => id.to_string(),
                    _ => {
                        return Err(EvaluationError::TypeError(
                            "destroy expects an occurrence reference".into(),
                        ))
                    }
                };
                let occ_id = sysml_core::ElementId::from_string(&occ_id_str);
                let time = ctx
                    .get("__time")
                    .and_then(|v| match v {
                        Value::Float(f) => Some(*f),
                        Value::Int(n) => Some(*n as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                let destroyed = reg.destroy(&occ_id, time, std::collections::HashMap::new());
                Ok(Value::Bool(destroyed))
            } else {
                Err(EvaluationError::NotYetImplemented {
                    name: name.to_string(),
                    reason: "no occurrence registry in context".into(),
                })
            }
        }
        "isDuring" => {
            // isDuring(occurrence_ref) — returns true if current time is during the occurrence.
            check_arity(name, args, 1)?;
            if let Some(reg) = &ctx.occurrence_registry {
                let reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let occ_id_str = match &args[0] {
                    Value::String(s) => s.clone(),
                    Value::Ref(id) => id.to_string(),
                    _ => {
                        return Err(EvaluationError::TypeError(
                            "isDuring expects an occurrence reference".into(),
                        ))
                    }
                };
                let occ_id = sysml_core::ElementId::from_string(&occ_id_str);
                let time = ctx
                    .get("__time")
                    .and_then(|v| match v {
                        Value::Float(f) => Some(*f),
                        Value::Int(n) => Some(*n as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                Ok(Value::Bool(reg.is_during(&occ_id, time)))
            } else {
                Err(EvaluationError::NotYetImplemented {
                    name: name.to_string(),
                    reason: "no occurrence registry in context".into(),
                })
            }
        }
        "addNew" => {
            // addNew(collection, value) — create occurrence + append to collection.
            check_arity(name, args, 2)?;
            let mut list = match &args[0] {
                Value::List(l) => l.clone(),
                _ => vec![args[0].clone()],
            };
            // Create occurrence in registry if available
            if let Some(reg) = &ctx.occurrence_registry {
                let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let life_id = reg.create_life(None);
                let time = ctx
                    .get("__time")
                    .and_then(|v| match v {
                        Value::Float(f) => Some(*f),
                        Value::Int(n) => Some(*n as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                let clock = ctx
                    .get("__clock")
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "default".into());
                let _occ_id = reg.create(life_id, time, std::collections::HashMap::new(), clock);
            }
            list.push(args[1].clone());
            Ok(Value::List(list))
        }
        "addNewAt" => {
            // addNewAt(collection, value, index) — create occurrence + insert at index.
            check_arity(name, args, 3)?;
            let mut list = match &args[0] {
                Value::List(l) => l.clone(),
                _ => vec![args[0].clone()],
            };
            let index = match &args[2] {
                Value::Int(n) => *n as usize,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "addNewAt index must be an integer".into(),
                    ))
                }
            };
            // Create occurrence in registry if available
            if let Some(reg) = &ctx.occurrence_registry {
                let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
                let life_id = reg.create_life(None);
                let time = ctx
                    .get("__time")
                    .and_then(|v| match v {
                        Value::Float(f) => Some(*f),
                        Value::Int(n) => Some(*n as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0);
                let clock = ctx
                    .get("__clock")
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "default".into());
                let _occ_id = reg.create(life_id, time, std::collections::HashMap::new(), clock);
            }
            let clamped_index = index.min(list.len());
            list.insert(clamped_index, args[1].clone());
            Ok(Value::List(list))
        }

        // ── Trigger functions (from Triggers.kerml) ──
        // These create trigger signal descriptors. In the expression context,
        // they evaluate the trigger condition against the current clock time.
        "TriggerWhen" => {
            // TriggerWhen(condition) → evaluate condition, return Bool
            // In the spec, this monitors for false→true transition. In expression
            // context, we simply evaluate the condition as a Bool.
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Bool(b) => Ok(Value::Bool(*b)),
                _ => Ok(Value::Bool(false)),
            }
        }
        "TriggerAt" => {
            // TriggerAt(timeInstant) → true if clock.currentTime >= timeInstant
            check_arity(name, args, 1)?;
            let target_time = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "TriggerAt: time must be numeric".into(),
                    ))
                }
            };
            let current_time = ctx
                .get("__clock_time")
                .and_then(|v| v.as_float())
                .unwrap_or(0.0);
            Ok(Value::Bool(current_time >= target_time))
        }
        "TriggerAfter" => {
            // TriggerAfter(delay) → true if delay has elapsed since reference time
            // In expression context: clock.currentTime >= delay (relative to t=0)
            check_arity(name, args, 1)?;
            let delay = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(n) => *n as f64,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "TriggerAfter: delay must be numeric".into(),
                    ))
                }
            };
            let current_time = ctx
                .get("__clock_time")
                .and_then(|v| v.as_float())
                .unwrap_or(0.0);
            Ok(Value::Bool(current_time >= delay))
        }

        // ── Coordinate frame arithmetic ──
        // From MeasurementRefCalculations.sysml
        "transform" => {
            // transform(vector, sourceFrame, targetFrame) → vector in target frame
            check_arity(name, args, 3)?;
            let vec_vals = value_to_f64_vec(&args[0])?;
            let source_name = match &args[1] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "transform: source frame must be a string".into(),
                    ))
                }
            };
            let target_name = match &args[2] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "transform: target frame must be a string".into(),
                    ))
                }
            };
            if let Some(registry) = &ctx.frame_registry {
                if let Some(xform) = registry.find_transform(&source_name, &target_name) {
                    if vec_vals.len() == 3 {
                        let result =
                            xform.transform_vector([vec_vals[0], vec_vals[1], vec_vals[2]]);
                        Ok(Value::List(
                            result.iter().map(|f| Value::Float(*f)).collect(),
                        ))
                    } else {
                        Err(EvaluationError::TypeError(format!(
                            "transform: expected 3D vector, got {} components",
                            vec_vals.len()
                        )))
                    }
                } else {
                    Err(EvaluationError::Runtime(format!(
                        "no transform registered from '{}' to '{}'",
                        source_name, target_name
                    )))
                }
            } else {
                // Without a registry, return the vector unchanged (identity transform)
                Ok(args[0].clone())
            }
        }

        "CoordinateFrame*" => {
            // CoordinateFrame * vector → transformed vector (forward transform)
            check_arity(name, args, 2)?;
            let frame_name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "CoordinateFrame*: first arg must be frame name".into(),
                    ))
                }
            };
            let vec_vals = value_to_f64_vec(&args[1])?;
            if let Some(registry) = &ctx.frame_registry {
                let default_name = registry
                    .default_frame()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "default".into());
                if let Some(xform) = registry.find_transform(&default_name, &frame_name) {
                    if vec_vals.len() == 3 {
                        let result =
                            xform.transform_vector([vec_vals[0], vec_vals[1], vec_vals[2]]);
                        Ok(Value::List(
                            result.iter().map(|f| Value::Float(*f)).collect(),
                        ))
                    } else {
                        Err(EvaluationError::TypeError(format!(
                            "CoordinateFrame*: expected 3D vector, got {} components",
                            vec_vals.len()
                        )))
                    }
                } else {
                    Ok(args[1].clone()) // No transform found, pass through
                }
            } else {
                Ok(args[1].clone()) // No registry, pass through
            }
        }

        "CoordinateFrame/" => {
            // CoordinateFrame / vector → inverse-transformed vector
            check_arity(name, args, 2)?;
            let frame_name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "CoordinateFrame/: first arg must be frame name".into(),
                    ))
                }
            };
            let vec_vals = value_to_f64_vec(&args[1])?;
            if let Some(registry) = &ctx.frame_registry {
                let default_name = registry
                    .default_frame()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "default".into());
                // Inverse: from frame back to default
                if let Some(xform) = registry.find_transform(&frame_name, &default_name) {
                    if vec_vals.len() == 3 {
                        let result =
                            xform.transform_vector([vec_vals[0], vec_vals[1], vec_vals[2]]);
                        Ok(Value::List(
                            result.iter().map(|f| Value::Float(*f)).collect(),
                        ))
                    } else {
                        Err(EvaluationError::TypeError(format!(
                            "CoordinateFrame/: expected 3D vector, got {} components",
                            vec_vals.len()
                        )))
                    }
                } else {
                    Ok(args[1].clone()) // No transform found, pass through
                }
            } else {
                Ok(args[1].clone()) // No registry, pass through
            }
        }

        // ── Performance / Evaluation model (from Performances.kerml) ──
        // These are metamodel type constructors — they describe how the runtime
        // evaluates expressions. They return the evaluation result directly.
        "Evaluation" => {
            // Abstract base — return input value as-is, or Null if no args
            if args.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(args[0].clone())
            }
        }

        "LiteralEvaluation" => {
            // LiteralEvaluation(value) → the literal ScalarValue
            check_arity(name, args, 1)?;
            Ok(args[0].clone())
        }

        "LiteralIntegerEvaluation" => {
            // LiteralIntegerEvaluation(n) → Integer
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Int(_) => Ok(args[0].clone()),
                Value::Float(f) => Ok(Value::Int(*f as i64)),
                _ => Err(EvaluationError::TypeError(
                    "LiteralIntegerEvaluation: expected integer".into(),
                )),
            }
        }

        "LiteralRationalEvaluation" => {
            // LiteralRationalEvaluation(r) → Rational (Float)
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::Float(_) => Ok(args[0].clone()),
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                _ => Err(EvaluationError::TypeError(
                    "LiteralRationalEvaluation: expected rational".into(),
                )),
            }
        }

        "LiteralStringEvaluation" => {
            // LiteralStringEvaluation(s) → String
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::String(_) => Ok(args[0].clone()),
                other => Ok(Value::String(format!("{:?}", other))),
            }
        }

        "MetadataAccessEvaluation" => {
            // MetadataAccessEvaluation(element_ref) → Metaobject (type names list)
            // Returns metadata about the referenced element. In expression context,
            // returns the element's type names if a graph is available.
            check_arity(name, args, 1)?;
            if let Some(graph) = &ctx.graph {
                if let Value::Ref(eid) = &args[0] {
                    if let Some(elem) = graph.get_element(eid) {
                        let kind_name = Value::String(elem.kind.as_str().to_string());
                        return Ok(Value::List(vec![kind_name]));
                    }
                }
            }
            // Fallback: return the argument wrapped in a list
            Ok(Value::List(vec![args[0].clone()]))
        }

        "NullEvaluation" => {
            // NullEvaluation() → Null (empty result, Anything[0..0])
            Ok(Value::Null)
        }

        "FeatureReadEvaluation" => {
            // FeatureReadEvaluation(feature_ref) → feature's value
            // Reads a feature's current value from context.
            check_arity(name, args, 1)?;
            match &args[0] {
                Value::String(name) => {
                    // Look up in context variables
                    Ok(ctx.get(name).cloned().unwrap_or(Value::Null))
                }
                Value::Ref(eid) => {
                    // Try to find the element's value in context by ID string
                    let id_str = eid.to_string();
                    Ok(ctx.get(&id_str).cloned().unwrap_or(Value::Null))
                }
                other => Ok(other.clone()),
            }
        }

        // ── State performance introspection (from StatePerformances.kerml) ──
        "allSubstatePerformances" => {
            // Returns all sub-state performances (nested state machine executions).
            // Queries __active_substates from the orchestrator-populated context.
            let states = ctx
                .get("__active_substates")
                .cloned()
                .unwrap_or(Value::List(Vec::new()));
            match states {
                Value::List(_) => Ok(states),
                other => Ok(Value::List(vec![other])),
            }
        }

        "allSubtransitionPerformances" => {
            // Returns all transition performances within the current state.
            // Queries __recent_transitions from the orchestrator-populated context.
            let transitions = ctx
                .get("__recent_transitions")
                .cloned()
                .unwrap_or(Value::List(Vec::new()));
            match transitions {
                Value::List(_) => Ok(transitions),
                other => Ok(Value::List(vec![other])),
            }
        }

        // ── Collection internals ──
        "index" => {
            // index(collection, i) → element at position i
            // Private helper in CollectionFunctions, equivalent to collection[i]
            check_arity(name, args, 2)?;
            let list = match &args[0] {
                Value::List(l) => l,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "index: first argument must be a list".into(),
                    ))
                }
            };
            let idx = match &args[1] {
                Value::Int(n) => *n as usize,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "index: second argument must be an integer".into(),
                    ))
                }
            };
            if idx < list.len() {
                Ok(list[idx].clone())
            } else {
                Err(EvaluationError::IndexOutOfBounds {
                    index: idx,
                    size: list.len(),
                })
            }
        }

        _ => Err(EvaluationError::UnknownFunction(name.to_owned())),
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub(crate) fn check_arity(
    name: &str,
    args: &[Value],
    expected: usize,
) -> Result<(), EvaluationError> {
    if args.len() != expected {
        Err(EvaluationError::ArityMismatch {
            name: name.to_owned(),
            expected,
            got: args.len(),
        })
    } else {
        Ok(())
    }
}

pub(crate) fn value_to_bool(v: &Value) -> Result<bool, EvaluationError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Null => Ok(false),
        _ => Err(EvaluationError::TypeError(format!(
            "expected boolean, got {:?}",
            v
        ))),
    }
}

/// Check if a non-Ref Value matches a SysML type name by its variant.
///
/// Used by istype/hastype when the LHS is a concrete value (not a graph element reference).
pub(crate) fn value_matches_type_name(v: &Value, type_name: &str) -> bool {
    match type_name {
        "Boolean" | "Bool" => matches!(v, Value::Bool(_)),
        "Integer" | "Int" | "Natural" | "Positive" => matches!(v, Value::Int(_)),
        "Real" | "Float" | "Number" | "Rational" => matches!(v, Value::Float(_) | Value::Int(_)),
        "Complex" => matches!(v, Value::Complex { .. }),
        "String" => matches!(v, Value::String(_)),
        "Null" | "Anything" => true,
        _ if type_name.ends_with("Value") => {
            // ISQ quantity types: LengthValue, MassValue, etc.
            matches!(v, Value::Quantity { .. })
        }
        _ => false,
    }
}

pub(crate) fn value_to_list(v: &Value) -> Result<&[Value], EvaluationError> {
    match v {
        Value::List(items) => Ok(items),
        _ => Err(EvaluationError::TypeError(format!(
            "expected sequence, got {:?}",
            v
        ))),
    }
}

/// Convert a `Value::List` to a `Vec<f64>`, promoting Int/Quantity → Float.
pub(crate) fn value_to_f64_vec(v: &Value) -> Result<Vec<f64>, EvaluationError> {
    let items = value_to_list(v)?;
    items
        .iter()
        .map(|item| match item {
            Value::Float(f) => Ok(*f),
            Value::Int(n) => Ok(*n as f64),
            Value::Quantity { value, .. } => Ok(*value),
            _ => Err(EvaluationError::TypeError(format!(
                "expected numeric in vector, got {:?}",
                item
            ))),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tensor (matrix) helpers
// ---------------------------------------------------------------------------

/// Convert a nested List of Lists to a matrix (Vec<Vec<f64>>).
fn value_to_matrix(v: &Value) -> Result<Vec<Vec<f64>>, EvaluationError> {
    match v {
        Value::List(rows) => rows.iter().map(|row| value_to_f64_vec(row)).collect(),
        _ => Err(EvaluationError::TypeError(
            "expected matrix (list of lists)".into(),
        )),
    }
}

/// Element-wise scalar × tensor.
fn scalar_tensor_mult(s: f64, tensor: &Value) -> Result<Value, EvaluationError> {
    match tensor {
        Value::List(items) => {
            let result: Result<Vec<Value>, _> = items
                .iter()
                .map(|item| {
                    match item {
                        Value::Float(f) => Ok(Value::Float(s * f)),
                        Value::Int(n) => Ok(Value::Float(s * *n as f64)),
                        Value::Quantity { value, .. } => Ok(Value::Float(s * value)),
                        Value::List(_) => scalar_tensor_mult(s, item), // recurse for nested
                        _ => Err(EvaluationError::TypeError(
                            "tensor element must be numeric or list".into(),
                        )),
                    }
                })
                .collect();
            Ok(Value::List(result?))
        }
        _ => Err(EvaluationError::TypeError("tensor must be a list".into())),
    }
}

/// Matrix × vector multiplication.
fn matrix_vector_mult(mat: &[Vec<f64>], vec: &[f64]) -> Result<Vec<f64>, EvaluationError> {
    let mut result = Vec::with_capacity(mat.len());
    for row in mat {
        if row.len() != vec.len() {
            return Err(EvaluationError::TypeError(format!(
                "tensorVectorMult: row length {} != vector length {}",
                row.len(),
                vec.len()
            )));
        }
        result.push(row.iter().zip(vec.iter()).map(|(a, b)| a * b).sum());
    }
    Ok(result)
}

/// Vector × matrix multiplication (v^T * M).
fn vector_matrix_mult(vec: &[f64], mat: &[Vec<f64>]) -> Result<Vec<f64>, EvaluationError> {
    if mat.is_empty() {
        return Ok(Vec::new());
    }
    let cols = mat[0].len();
    if vec.len() != mat.len() {
        return Err(EvaluationError::TypeError(format!(
            "vectorTensorMult: vector length {} != matrix rows {}",
            vec.len(),
            mat.len()
        )));
    }
    let mut result = vec![0.0; cols];
    for (i, &v) in vec.iter().enumerate() {
        for (j, &m) in mat[i].iter().enumerate() {
            result[j] += v * m;
        }
    }
    Ok(result)
}

/// Matrix × matrix multiplication.
fn matrix_matrix_mult(a: &[Vec<f64>], b: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, EvaluationError> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    let a_cols = a[0].len();
    if a_cols != b.len() {
        return Err(EvaluationError::TypeError(format!(
            "tensorTensorMult: A cols {} != B rows {}",
            a_cols,
            b.len()
        )));
    }
    let b_cols = b[0].len();
    let mut result = vec![vec![0.0; b_cols]; a.len()];
    for i in 0..a.len() {
        for j in 0..b_cols {
            for k in 0..a_cols {
                result[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    Ok(result)
}

pub(crate) fn values_equal(a: &Value, b: &Value) -> bool {
    a == b
}

pub(crate) fn promote_to_float(a: &Value, b: &Value) -> Result<(f64, f64), EvaluationError> {
    let af = match a {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        Value::Quantity { value, .. } => *value,
        _ => {
            return Err(EvaluationError::TypeError(format!(
                "expected numeric, got {:?}",
                a
            )))
        }
    };
    let bf = match b {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        Value::Quantity { value, .. } => *value,
        _ => {
            return Err(EvaluationError::TypeError(format!(
                "expected numeric, got {:?}",
                b
            )))
        }
    };
    Ok((af, bf))
}

pub(crate) fn numeric_binop(
    left: &Value,
    right: &Value,
    int_op: impl Fn(i64, i64) -> Option<i64>,
    float_op: impl Fn(f64, f64) -> f64,
) -> EvalResult {
    // Quantity operands: extract the scalar value, treat as Float.
    // Dimensional analysis is handled by the evaluator (eval_quantity_binary).
    // This path is reached when Quantity values flow through non-dimension-aware
    // code paths (e.g., solver, constraint evaluation).
    let left_ref: Value;
    let right_ref: Value;
    let left = if let Value::Quantity { value, .. } = left {
        left_ref = Value::Float(*value);
        &left_ref
    } else {
        left
    };
    let right = if let Value::Quantity { value, .. } = right {
        right_ref = Value::Float(*value);
        &right_ref
    } else {
        right
    };

    // Vector arithmetic: scalar ⊕ vector or vector ⊕ scalar (element-wise)
    let left_is_list = matches!(left, Value::List(_));
    let right_is_list = matches!(right, Value::List(_));

    if left_is_list && right_is_list {
        // Vector ⊕ vector: element-wise (e.g., [1,2] + [3,4] = [4,6])
        let u = value_to_f64_vec(left)?;
        let v = value_to_f64_vec(right)?;
        if u.len() != v.len() {
            return Err(EvaluationError::Runtime(format!(
                "vector arithmetic: length mismatch ({} vs {})",
                u.len(),
                v.len()
            )));
        }
        return Ok(Value::List(
            u.iter()
                .zip(v.iter())
                .map(|(a, b)| Value::Float(float_op(*a, *b)))
                .collect(),
        ));
    }
    if left_is_list || right_is_list {
        // Scalar ⊕ vector or vector ⊕ scalar: broadcast scalar
        let (vec_val, scalar_val) = if left_is_list {
            (left, right)
        } else {
            (right, left)
        };
        let v = value_to_f64_vec(vec_val)?;
        let k = match scalar_val {
            Value::Float(f) => *f,
            Value::Int(n) => *n as f64,
            _ => {
                return Err(EvaluationError::TypeError(format!(
                    "expected numeric scalar for vector arithmetic, got {:?}",
                    scalar_val
                )))
            }
        };
        return Ok(Value::List(if left_is_list {
            v.iter().map(|x| Value::Float(float_op(*x, k))).collect()
        } else {
            v.iter().map(|x| Value::Float(float_op(k, *x))).collect()
        }));
    }

    // If either operand is Complex, promote both to complex and delegate
    if matches!(left, Value::Complex { .. }) || matches!(right, Value::Complex { .. }) {
        let (a_re, a_im) = promote_to_complex(left)?;
        let (b_re, b_im) = promote_to_complex(right)?;
        // Use float_op on real parts, assume it's component-wise for +/-
        // For *, / see eval_binary which handles those specially.
        Ok(Value::Complex {
            re: float_op(a_re, b_re),
            im: float_op(a_im, b_im),
        })
    } else {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
                .map(Value::Int)
                .ok_or(EvaluationError::Overflow),
            _ => {
                let (a, b) = promote_to_float(left, right)?;
                Ok(Value::Float(float_op(a, b)))
            }
        }
    }
}

/// Promote a value to complex (re, im) pair.
pub(crate) fn promote_to_complex(v: &Value) -> Result<(f64, f64), EvaluationError> {
    match v {
        Value::Complex { re, im } => Ok((*re, *im)),
        Value::Float(f) => Ok((*f, 0.0)),
        Value::Int(n) => Ok((*n as f64, 0.0)),
        Value::Quantity { value, .. } => Ok((*value, 0.0)),
        _ => Err(EvaluationError::TypeError(format!(
            "expected numeric or complex, got {:?}",
            v
        ))),
    }
}

pub(crate) fn numeric_cmp(
    left: &Value,
    right: &Value,
    int_cmp: impl Fn(i64, i64) -> bool,
    float_cmp: impl Fn(f64, f64) -> bool,
) -> EvalResult {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(int_cmp(*a, *b))),
        _ => {
            // promote_to_float handles Quantity (extracts numeric value)
            let (a, b) = promote_to_float(left, right)?;
            Ok(Value::Bool(float_cmp(a, b)))
        }
    }
}

// ---------------------------------------------------------------------------
// SampledFunction helpers
// ---------------------------------------------------------------------------

/// Extract (domain, range) pairs from a list that may contain SamplePair maps or
/// nested [d, r] lists.
fn extract_sample_pairs_from_list(v: &Value) -> Result<Vec<(f64, f64)>, EvaluationError> {
    match v {
        Value::List(items) => {
            let mut pairs = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    // Spec format: SamplePair map with domainValue/rangeValue
                    Value::Map(map)
                        if map.get("__type").and_then(|v| v.as_str()) == Some("SamplePair") =>
                    {
                        let d = map
                            .get("domainValue")
                            .and_then(|v| v.as_float())
                            .ok_or_else(|| {
                                EvaluationError::TypeError(
                                    "SamplePair missing numeric domainValue".into(),
                                )
                            })?;
                        let r = map
                            .get("rangeValue")
                            .and_then(|v| v.as_float())
                            .ok_or_else(|| {
                                EvaluationError::TypeError(
                                    "SamplePair missing numeric rangeValue".into(),
                                )
                            })?;
                        pairs.push((d, r));
                    }
                    // Tuple format: [d, r]
                    Value::List(inner) if inner.len() == 2 => {
                        let d = inner[0].as_float().ok_or_else(|| {
                            EvaluationError::TypeError(
                                "SamplePair tuple: domainValue must be numeric".into(),
                            )
                        })?;
                        let r = inner[1].as_float().ok_or_else(|| {
                            EvaluationError::TypeError(
                                "SamplePair tuple: rangeValue must be numeric".into(),
                            )
                        })?;
                        pairs.push((d, r));
                    }
                    _ => {
                        return Err(EvaluationError::TypeError(format!(
                            "SampledFunction: expected SamplePair or [d, r] tuple, got {:?}",
                            item
                        )))
                    }
                }
            }
            Ok(pairs)
        }
        _ => Err(EvaluationError::TypeError(
            "SampledFunction: expected a list of SamplePairs".into(),
        )),
    }
}

/// Build a SampledFunction Value::Map from (domain, range) pairs.
///
/// Sorts by domain value and validates monotonicity per spec:
/// the domain must be strictly increasing or strictly decreasing.
fn build_sampled_function_from_pairs(mut pairs: Vec<(f64, f64)>) -> Result<Value, EvaluationError> {
    if pairs.is_empty() {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "__type".to_string(),
            Value::String("SampledFunction".to_string()),
        );
        map.insert("domain".to_string(), Value::List(Vec::new()));
        map.insert("range".to_string(), Value::List(Vec::new()));
        map.insert("samples".to_string(), Value::List(Vec::new()));
        return Ok(Value::Map(map));
    }

    // SampledFunctions.sysml:30-43 (§9.4.3.2.6): the domain must be strictly
    // monotonic — either strictly increasing OR strictly decreasing — and the
    // caller's order is PRESERVED. A strictly-decreasing domain is valid and is
    // NOT re-sorted ascending. Reject anything else (unsorted, duplicate, or
    // otherwise non-monotonic). NaN compares false either way, so it is rejected.
    if pairs.len() >= 2 {
        let increasing = pairs.windows(2).all(|w| w[0].0 < w[1].0);
        let decreasing = pairs.windows(2).all(|w| w[0].0 > w[1].0);
        if !increasing && !decreasing {
            return Err(EvaluationError::TypeError(format!(
                "SampledFunction: domain is not strictly monotonic (must be strictly increasing \
                 or strictly decreasing): {:?}",
                pairs.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            )));
        }
    }

    let sorted_domain: Vec<Value> = pairs.iter().map(|(d, _)| Value::Float(*d)).collect();
    let sorted_range: Vec<Value> = pairs.iter().map(|(_, r)| Value::Float(*r)).collect();

    // Build spec-format `samples` list: SamplePair[0..*] ordered
    let samples: Vec<Value> = pairs
        .iter()
        .map(|(d, r)| {
            let mut sp = std::collections::BTreeMap::new();
            sp.insert(
                "__type".to_string(),
                Value::String("SamplePair".to_string()),
            );
            sp.insert("domainValue".to_string(), Value::Float(*d));
            sp.insert("rangeValue".to_string(), Value::Float(*r));
            Value::Map(sp)
        })
        .collect();

    let mut map = std::collections::BTreeMap::new();
    map.insert(
        "__type".to_string(),
        Value::String("SampledFunction".to_string()),
    );
    // Parallel domain/range lists (efficient for interpolation binary search)
    map.insert("domain".to_string(), Value::List(sorted_domain));
    map.insert("range".to_string(), Value::List(sorted_range));
    // Spec-standard `samples` field: SamplePair[0..*] ordered
    map.insert("samples".to_string(), Value::List(samples));
    Ok(Value::Map(map))
}

/// Convert a Value to a Vec<f64>. Accepts List of numerics.
fn value_to_f64_list(v: &Value) -> Result<Vec<f64>, EvaluationError> {
    match v {
        Value::List(items) => items
            .iter()
            .map(|item| match item {
                Value::Float(f) => Ok(*f),
                Value::Int(n) => Ok(*n as f64),
                _ => Err(EvaluationError::TypeError(format!(
                    "SampledFunction: expected numeric in list, got {:?}",
                    item
                ))),
            })
            .collect(),
        _ => Err(EvaluationError::TypeError(format!(
            "SampledFunction: expected list, got {:?}",
            v
        ))),
    }
}

/// Extract (domain, range) f64 vectors from a SampledFunction Value::Map.
fn extract_sampled_function_data(v: &Value) -> Result<(Vec<f64>, Vec<f64>), EvaluationError> {
    match v {
        Value::Map(map) => {
            match map.get("__type") {
                Some(Value::String(t)) if t == "SampledFunction" => {}
                _ => {
                    return Err(EvaluationError::TypeError(
                        "Interpolate: first arg must be a SampledFunction".into(),
                    ))
                }
            }
            let domain = map.get("domain").ok_or_else(|| {
                EvaluationError::TypeError("SampledFunction missing domain".into())
            })?;
            let range = map.get("range").ok_or_else(|| {
                EvaluationError::TypeError("SampledFunction missing range".into())
            })?;
            Ok((value_to_f64_list(domain)?, value_to_f64_list(range)?))
        }
        _ => Err(EvaluationError::TypeError(
            "Interpolate: first arg must be a SampledFunction (Value::Map)".into(),
        )),
    }
}

/// Linear interpolation with clamped extrapolation.
///
/// Direction-aware: the domain may be strictly increasing OR strictly
/// decreasing (SampledFunctions.sysml:30-43, §9.4.3.2.6). For an out-of-bounds
/// `x`, clamps to the nearest edge range value (used by the internal
/// `interpolateLinear` ODE helper; the spec `Interpolate` returns null OOB —
/// that bounds check lives in the dispatch arm). If domain is empty, errors.
fn interpolate_linear_impl(domain: &[f64], range: &[f64], x: f64) -> Result<f64, EvaluationError> {
    if domain.is_empty() {
        return Err(EvaluationError::TypeError(
            "Interpolate: empty SampledFunction".into(),
        ));
    }
    if domain.len() == 1 {
        return Ok(range[0]);
    }
    let last = domain.len() - 1;
    if domain[0] <= domain[last] {
        // Ascending domain (the common case; byte-identical to the prior impl).
        if x <= domain[0] {
            return Ok(range[0]);
        }
        if x >= domain[last] {
            return Ok(range[range.len() - 1]);
        }
        // Binary search for bracketing interval
        let idx = match domain
            .binary_search_by(|d| d.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => return Ok(range[i]), // Exact match
            Err(i) => i,                  // Insert position
        };
        // Linear interpolation between domain[idx-1] and domain[idx]
        let x0 = domain[idx - 1];
        let x1 = domain[idx];
        let y0 = range[idx - 1];
        let y1 = range[idx];
        let t = (x - x0) / (x1 - x0);
        Ok(y0 + t * (y1 - y0))
    } else {
        // Descending domain: domain[0] is the max, domain[last] the min.
        if x >= domain[0] {
            return Ok(range[0]);
        }
        if x <= domain[last] {
            return Ok(range[range.len() - 1]);
        }
        // Scan for the bracketing interval domain[i] >= x >= domain[i+1]
        // (small lookup tables; no binary search on a descending array).
        for i in 0..last {
            let x0 = domain[i]; // larger endpoint
            let x1 = domain[i + 1]; // smaller endpoint
            if x <= x0 && x >= x1 {
                let y0 = range[i];
                let y1 = range[i + 1];
                let t = (x - x0) / (x1 - x0);
                return Ok(y0 + t * (y1 - y0));
            }
        }
        // Unreachable for a strictly-monotonic domain with x in-bounds; clamp.
        Ok(range[range.len() - 1])
    }
}

/// Linear lookup that **extrapolates** past the domain using the slope of the
/// nearest end segment, instead of clamping to the edge value.
///
/// This is the physically-correct edge behavior for a constitutive lookup
/// coupled to an ODE state — e.g. an inverse B-H curve `H(B)`. A real
/// ferromagnetic core saturates: as `|B|` approaches `Bs` the required field
/// `H` diverges (the core "becomes resistive", the drive current explodes).
/// A precomputed inverse-B-H table is necessarily truncated near `Bs`, so
/// clamping `H` flat past the edge removes that divergence — the current can no
/// longer spike to trip the comparator, and nothing opposes `dB/dt`, so the
/// state runs away unbounded. Continuing the (already steep) end-segment slope
/// preserves the divergence: the lookup explodes past the table, the current
/// spikes, and the state self-limits near saturation. No clamp is required and
/// no slope constant is invented — the table's own saturation slope is used.
///
/// Within the domain this is identical to [`interpolate_linear_impl`].
fn interpolate_linear_extrap_impl(
    domain: &[f64],
    range: &[f64],
    x: f64,
) -> Result<f64, EvaluationError> {
    if domain.is_empty() {
        return Err(EvaluationError::TypeError(
            "interpolateSaturating: empty SampledFunction".into(),
        ));
    }
    if domain.len() == 1 {
        return Ok(range[0]);
    }
    let last = domain.len() - 1;
    let extrap = |x0: f64, x1: f64, y0: f64, y1: f64| {
        // Guard a degenerate (zero-width) end segment: fall back to the edge
        // value rather than dividing by zero.
        if (x1 - x0).abs() < f64::EPSILON {
            y0
        } else {
            y0 + (x - x0) / (x1 - x0) * (y1 - y0)
        }
    };
    if domain[0] <= domain[last] {
        // Ascending domain.
        if x < domain[0] {
            return Ok(extrap(domain[0], domain[1], range[0], range[1]));
        }
        if x > domain[last] {
            return Ok(extrap(
                domain[last - 1],
                domain[last],
                range[last - 1],
                range[last],
            ));
        }
    } else {
        // Descending domain: domain[0] is the max, domain[last] the min.
        if x > domain[0] {
            return Ok(extrap(domain[0], domain[1], range[0], range[1]));
        }
        if x < domain[last] {
            return Ok(extrap(
                domain[last - 1],
                domain[last],
                range[last - 1],
                range[last],
            ));
        }
    }
    // In-domain: identical to the clamping variant.
    interpolate_linear_impl(domain, range, x)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::str_to_string
)]
mod tests {
    use super::*;
    use crate::expressions::EvalContext;
    use crate::expressions::{SubsystemState, TickSnapshot};
    use std::collections::HashMap;
    use std::sync::Arc;
    use sysml_core::Value;

    fn make_snapshot(
        tick: u64,
        subsystem: &str,
        state: &str,
        vars: Vec<(&str, Value)>,
    ) -> TickSnapshot {
        let mut subsystem_states = HashMap::new();
        subsystem_states.insert(
            subsystem.to_string(),
            SubsystemState {
                name: subsystem.to_string(),
                kind: "stateMachine",
                current_state: state.to_string(),
                completed: false,
                available_transitions: vec![],
                outputs: vec![],
                sends: vec![],
                active_modes: vec![],
                variables: HashMap::new(),
                deferred_event_count: 0,
                source_element_id: None,
            },
        );
        let mut variables = HashMap::new();
        for (k, v) in vars {
            variables.insert(k.to_string(), v);
        }
        TickSnapshot {
            tick,
            time_ms: tick as f64,
            variables,
            subsystem_states,
        }
    }

    #[test]
    fn test_was_in_state() {
        let mut ctx = EvalContext::new();
        ctx.trace = Some(Arc::new(vec![
            make_snapshot(0, "boiler", "cold", vec![]),
            make_snapshot(1, "boiler", "heating", vec![]),
            make_snapshot(2, "boiler", "ready", vec![]),
        ]));
        let result = eval_function(
            "was_in_state",
            &[
                Value::String("boiler".into()),
                Value::String("ready".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Bool(true));

        let result2 = eval_function(
            "was_in_state",
            &[
                Value::String("boiler".into()),
                Value::String("overheated".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result2, Value::Bool(false));
    }

    #[test]
    fn test_state_at() {
        let mut ctx = EvalContext::new();
        ctx.trace = Some(Arc::new(vec![
            make_snapshot(0, "boiler", "cold", vec![]),
            make_snapshot(1, "boiler", "heating", vec![]),
            make_snapshot(2, "boiler", "ready", vec![]),
        ]));
        let result = eval_function(
            "state_at",
            &[Value::String("boiler".into()), Value::Int(1)],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::String("heating".into()));
    }

    #[test]
    fn test_ticks_in_state() {
        let mut ctx = EvalContext::new();
        ctx.trace = Some(Arc::new(vec![
            make_snapshot(0, "boiler", "heating", vec![]),
            make_snapshot(1, "boiler", "heating", vec![]),
            make_snapshot(2, "boiler", "ready", vec![]),
        ]));
        let result = eval_function(
            "ticks_in_state",
            &[
                Value::String("boiler".into()),
                Value::String("heating".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn test_variable_at() {
        let mut ctx = EvalContext::new();
        ctx.trace = Some(Arc::new(vec![
            make_snapshot(0, "boiler", "cold", vec![("temp", Value::Float(20.0))]),
            make_snapshot(1, "boiler", "heating", vec![("temp", Value::Float(50.0))]),
            make_snapshot(2, "boiler", "ready", vec![("temp", Value::Float(93.0))]),
        ]));
        let result = eval_function(
            "variable_at",
            &[Value::String("temp".into()), Value::Int(1)],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Float(50.0));
    }

    #[test]
    fn test_held_for_succeeds() {
        let mut ctx = EvalContext::new();
        let mut snapshots = Vec::new();
        for i in 0..10 {
            snapshots.push(make_snapshot(
                i,
                "boiler",
                "ready",
                vec![("pressure", Value::Float(9.5))],
            ));
        }
        ctx.trace = Some(Arc::new(snapshots));
        let result = eval_function(
            "held_for",
            &[Value::String("pressure > 8".into()), Value::Int(5)],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_held_for_fails() {
        let mut ctx = EvalContext::new();
        let mut snapshots = Vec::new();
        for i in 0..10 {
            let pressure = if i >= 8 { 5.0 } else { 9.5 }; // drops below 8 at tick 8
            snapshots.push(make_snapshot(
                i,
                "boiler",
                "ready",
                vec![("pressure", Value::Float(pressure))],
            ));
        }
        ctx.trace = Some(Arc::new(snapshots));
        let result = eval_function(
            "held_for",
            &[Value::String("pressure > 8".into()), Value::Int(5)],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_temporal_without_trace_errors() {
        let ctx = EvalContext::new(); // no trace set
        let result = eval_function(
            "was_in_state",
            &[
                Value::String("boiler".into()),
                Value::String("ready".into()),
            ],
            &ctx,
        );
        assert!(result.is_err());
    }

    // ── Phase 15: Math stdlib functions ──────────────────────────────

    #[test]
    fn test_exp() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("exp", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(1.0)
        );
        let e = match eval_function("exp", &[Value::Float(1.0)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((e - std::f64::consts::E).abs() < 1e-10);
        // Int promotion
        assert_eq!(
            eval_function("exp", &[Value::Int(0)], &ctx).unwrap(),
            Value::Float(1.0)
        );
    }

    #[test]
    fn test_sqrt() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("sqrt", &[Value::Float(4.0)], &ctx).unwrap(),
            Value::Float(2.0)
        );
        assert_eq!(
            eval_function("sqrt", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        assert_eq!(
            eval_function("sqrt", &[Value::Int(9)], &ctx).unwrap(),
            Value::Float(3.0)
        );
        // Negative → error
        assert!(eval_function("sqrt", &[Value::Float(-1.0)], &ctx).is_err());
        assert!(eval_function("sqrt", &[Value::Int(-4)], &ctx).is_err());
    }

    #[test]
    fn test_ln() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("ln", &[Value::Float(1.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        let ln_e = match eval_function("ln", &[Value::Float(std::f64::consts::E)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((ln_e - 1.0).abs() < 1e-10);
        // Non-positive → error
        assert!(eval_function("ln", &[Value::Float(0.0)], &ctx).is_err());
        assert!(eval_function("ln", &[Value::Float(-1.0)], &ctx).is_err());
        assert!(eval_function("ln", &[Value::Int(0)], &ctx).is_err());
    }

    #[test]
    fn test_sin() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("sin", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        let sin_half_pi =
            match eval_function("sin", &[Value::Float(std::f64::consts::FRAC_PI_2)], &ctx).unwrap()
            {
                Value::Float(f) => f,
                _ => panic!("expected float"),
            };
        assert!((sin_half_pi - 1.0).abs() < 1e-10);
        assert_eq!(
            eval_function("sin", &[Value::Int(0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
    }

    #[test]
    fn test_cos() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("cos", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(1.0)
        );
        let cos_pi =
            match eval_function("cos", &[Value::Float(std::f64::consts::PI)], &ctx).unwrap() {
                Value::Float(f) => f,
                _ => panic!("expected float"),
            };
        assert!((cos_pi + 1.0).abs() < 1e-10);
        assert_eq!(
            eval_function("cos", &[Value::Int(0)], &ctx).unwrap(),
            Value::Float(1.0)
        );
    }

    // ── Phase 0: Extended trig functions ─────────────────────────────

    #[test]
    fn test_tan() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("tan", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        let tan_pi4 = match eval_function("tan", &[Value::Float(std::f64::consts::FRAC_PI_4)], &ctx)
            .unwrap()
        {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((tan_pi4 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_cot() {
        let ctx = EvalContext::new();
        let cot_pi4 = match eval_function("cot", &[Value::Float(std::f64::consts::FRAC_PI_4)], &ctx)
            .unwrap()
        {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((cot_pi4 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_asin() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("asin", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        let asin_1 = match eval_function("asin", &[Value::Float(1.0)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((asin_1 - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        // Domain violation
        assert!(eval_function("asin", &[Value::Float(2.0)], &ctx).is_err());
    }

    #[test]
    fn test_acos() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("acos", &[Value::Float(1.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        let acos_0 = match eval_function("acos", &[Value::Float(0.0)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((acos_0 - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        // Domain violation
        assert!(eval_function("acos", &[Value::Float(-2.0)], &ctx).is_err());
    }

    #[test]
    fn test_atan() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("atan", &[Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        let atan_1 = match eval_function("atan", &[Value::Float(1.0)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((atan_1 - std::f64::consts::FRAC_PI_4).abs() < 1e-10);
    }

    #[test]
    fn test_atan2() {
        let ctx = EvalContext::new();
        let atan2_result =
            match eval_function("atan2", &[Value::Float(1.0), Value::Float(1.0)], &ctx).unwrap() {
                Value::Float(f) => f,
                _ => panic!("expected float"),
            };
        assert!((atan2_result - std::f64::consts::FRAC_PI_4).abs() < 1e-10);
    }

    #[test]
    fn test_deg_rad() {
        let ctx = EvalContext::new();
        let deg_pi =
            match eval_function("deg", &[Value::Float(std::f64::consts::PI)], &ctx).unwrap() {
                Value::Float(f) => f,
                _ => panic!("expected float"),
            };
        assert!((deg_pi - 180.0).abs() < 1e-10);
        let rad_180 = match eval_function("rad", &[Value::Float(180.0)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((rad_180 - std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn test_pi() {
        let ctx = EvalContext::new();
        let pi = match eval_function("pi", &[], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((pi - std::f64::consts::PI).abs() < 1e-15);
    }

    // ── Phase 1: SampledFunction tests ──────────────────────────────

    fn make_sf(domain: &[f64], range: &[f64]) -> Value {
        let d: Vec<Value> = domain.iter().map(|x| Value::Float(*x)).collect();
        let r: Vec<Value> = range.iter().map(|x| Value::Float(*x)).collect();
        eval_function(
            "SampledFunction",
            &[Value::List(d), Value::List(r)],
            &EvalContext::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_sampled_function_construction() {
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        match &sf {
            Value::Map(m) => {
                assert_eq!(
                    m.get("__type"),
                    Some(&Value::String("SampledFunction".to_string()))
                );
                assert!(m.contains_key("domain"));
                assert!(m.contains_key("range"));
            }
            _ => panic!("expected Map"),
        }
    }

    #[test]
    fn test_sampled_function_mismatched_lengths() {
        let d = vec![Value::Float(0.0), Value::Float(1.0)];
        let r = vec![Value::Float(0.0)];
        assert!(eval_function(
            "SampledFunction",
            &[Value::List(d), Value::List(r)],
            &EvalContext::new()
        )
        .is_err());
    }

    #[test]
    fn test_interpolate_at_sample_point() {
        let ctx = EvalContext::new();
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        let result = eval_function("Interpolate", &[sf, Value::Float(1.0)], &ctx).unwrap();
        assert_eq!(result, Value::Float(10.0));
    }

    #[test]
    fn test_interpolate_between_points() {
        let ctx = EvalContext::new();
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        let result = match eval_function("Interpolate", &[sf, Value::Float(0.5)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_interpolate_linear_extrapolate_clamp() {
        // `interpolateLinear` is the internal ODE helper that CLAMPS out of
        // bounds (edge-continuity). The spec function `Interpolate` instead
        // returns null OOB — see q5 in quantity_spec_conformance.rs.
        let ctx = EvalContext::new();
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        // Below domain
        assert_eq!(
            eval_function("interpolateLinear", &[sf.clone(), Value::Float(-1.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        // Above domain
        assert_eq!(
            eval_function("interpolateLinear", &[sf, Value::Float(5.0)], &ctx).unwrap(),
            Value::Float(20.0)
        );
    }

    #[test]
    fn test_interpolate_out_of_bounds_is_null() {
        // SampledFunctions.sysml:80-84: `Interpolate` returns null out of bounds.
        let ctx = EvalContext::new();
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        assert_eq!(
            eval_function("Interpolate", &[sf.clone(), Value::Float(-1.0)], &ctx).unwrap(),
            Value::Null
        );
        assert_eq!(
            eval_function("Interpolate", &[sf, Value::Float(5.0)], &ctx).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn test_interpolate_linear_single_point() {
        let ctx = EvalContext::new();
        let sf = make_sf(&[1.0], &[42.0]);
        // The clamping helper returns the single range value for any input.
        assert_eq!(
            eval_function("interpolateLinear", &[sf.clone(), Value::Float(0.0)], &ctx).unwrap(),
            Value::Float(42.0)
        );
        assert_eq!(
            eval_function("interpolateLinear", &[sf, Value::Float(99.0)], &ctx).unwrap(),
            Value::Float(42.0)
        );
    }

    #[test]
    fn test_interpolate_descending_domain() {
        // GAP-PHYS Q4: a strictly-decreasing domain is preserved (not re-sorted)
        // and interpolated direction-aware. SampledFunctions.sysml:30-43.
        let ctx = EvalContext::new();
        let sf = make_sf(&[2.0, 1.0, 0.0], &[20.0, 10.0, 0.0]);
        // In-bounds midpoint (between domain 2.0→1.0): x=1.5 → 15.0.
        assert_eq!(
            eval_function("interpolateLinear", &[sf.clone(), Value::Float(1.5)], &ctx).unwrap(),
            Value::Float(15.0)
        );
        // Exact sample point.
        assert_eq!(
            eval_function("interpolateLinear", &[sf.clone(), Value::Float(1.0)], &ctx).unwrap(),
            Value::Float(10.0)
        );
        // Above the max (domain[0]=2.0) clamps to range[0]=20.0.
        assert_eq!(
            eval_function("interpolateLinear", &[sf.clone(), Value::Float(5.0)], &ctx).unwrap(),
            Value::Float(20.0)
        );
        // Below the min (domain[last]=0.0) clamps to range[last]=0.0.
        assert_eq!(
            eval_function("interpolateLinear", &[sf, Value::Float(-1.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
    }

    #[test]
    fn test_interpolate_empty_sf() {
        let ctx = EvalContext::new();
        let sf = make_sf(&[], &[]);
        assert!(eval_function("Interpolate", &[sf, Value::Float(1.0)], &ctx).is_err());
    }

    #[test]
    fn test_interpolate_saturating_extrapolates_past_domain() {
        let ctx = EvalContext::new();
        // domain 0,1,2 → range 0,10,20 (slope 10). End segments both slope 10.
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        let f = |x: f64| match eval_function(
            "interpolateSaturating",
            &[sf.clone(), Value::Float(x)],
            &ctx,
        )
        .unwrap()
        {
            Value::Float(v) => v,
            _ => panic!("expected float"),
        };
        // In-domain: identical to linear interpolation.
        assert!((f(0.5) - 5.0).abs() < 1e-9);
        assert!((f(1.5) - 15.0).abs() < 1e-9);
        // Past the upper edge: EXTRAPOLATES along the last-segment slope (10),
        // not clamped to 20 (which is what interpolateLinear would return).
        assert!(
            (f(3.0) - 30.0).abs() < 1e-9,
            "should extrapolate, got {}",
            f(3.0)
        );
        // Past the lower edge: extrapolates along the first-segment slope.
        assert!(
            (f(-1.0) - (-10.0)).abs() < 1e-9,
            "should extrapolate, got {}",
            f(-1.0)
        );
        // Contrast: interpolateLinear CLAMPS at the edge.
        let clamped =
            match eval_function("interpolateLinear", &[sf, Value::Float(3.0)], &ctx).unwrap() {
                Value::Float(v) => v,
                _ => panic!("expected float"),
            };
        assert!(
            (clamped - 20.0).abs() < 1e-9,
            "interpolateLinear should clamp to 20"
        );
    }

    #[test]
    fn test_interpolate_nonuniform_spacing() {
        let ctx = EvalContext::new();
        // domain: 0, 1, 10; range: 0, 100, 200
        let sf = make_sf(&[0.0, 1.0, 10.0], &[0.0, 100.0, 200.0]);
        // At x=0.5 (between 0 and 1): lerp → 50.0
        let result =
            match eval_function("Interpolate", &[sf.clone(), Value::Float(0.5)], &ctx).unwrap() {
                Value::Float(f) => f,
                _ => panic!("expected float"),
            };
        assert!((result - 50.0).abs() < 1e-10);
        // At x=5.5 (between 1 and 10): lerp → 100 + (200-100)*(5.5-1)/(10-1) = 150.0
        let result2 = match eval_function("Interpolate", &[sf, Value::Float(5.5)], &ctx).unwrap() {
            Value::Float(f) => f,
            _ => panic!("expected float"),
        };
        assert!((result2 - 150.0).abs() < 1e-10);
    }

    #[test]
    fn test_interpolate_linear_alias() {
        let ctx = EvalContext::new();
        let sf = make_sf(&[0.0, 1.0], &[0.0, 10.0]);
        // interpolateLinear should work as alias
        let result = eval_function("interpolateLinear", &[sf, Value::Float(0.5)], &ctx).unwrap();
        assert_eq!(result, Value::Float(5.0));
    }

    #[test]
    fn test_sample_function() {
        let ctx = EvalContext::new();
        let sf = make_sf(&[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
        let sample_points = Value::List(vec![Value::Float(0.5), Value::Float(1.5)]);
        let result = eval_function("Sample", &[sf, sample_points], &ctx).unwrap();
        match &result {
            Value::Map(m) => {
                assert_eq!(
                    m.get("__type"),
                    Some(&Value::String("SampledFunction".to_string()))
                );
                // Check interpolated range values
                let range = m.get("range").unwrap();
                match range {
                    Value::List(items) => {
                        assert_eq!(items.len(), 2);
                        assert_eq!(items[0], Value::Float(5.0));
                        assert_eq!(items[1], Value::Float(15.0));
                    }
                    _ => panic!("expected list"),
                }
            }
            _ => panic!("expected Map"),
        }
    }

    #[test]
    fn test_sampled_function_unsorted_domain_rejected() {
        // GAP-PHYS Q4: an unsorted (non-monotonic) domain is rejected, NOT
        // silently re-sorted — the caller must supply a strictly increasing or
        // strictly decreasing domain (SampledFunctions.sysml:30-43, §9.4.3.2.6).
        let ctx = EvalContext::new();
        let d = vec![Value::Float(2.0), Value::Float(0.0), Value::Float(1.0)];
        let r = vec![Value::Float(20.0), Value::Float(0.0), Value::Float(10.0)];
        let result = eval_function("SampledFunction", &[Value::List(d), Value::List(r)], &ctx);
        assert!(
            matches!(result, Err(EvaluationError::TypeError(ref m)) if m.contains("monotonic")),
            "unsorted domain must be rejected as non-monotonic, got {result:?}"
        );
    }

    // ── SampledFunction spec alignment tests ──────────────────────────

    #[test]
    fn test_sampled_function_has_samples_field() {
        let sf = make_sf(&[0.0, 1.0, 2.0], &[10.0, 20.0, 30.0]);
        match &sf {
            Value::Map(m) => {
                // Must have spec-standard `samples` field
                let samples = m.get("samples").expect("should have samples field");
                match samples {
                    Value::List(items) => {
                        assert_eq!(items.len(), 3, "should have 3 SamplePairs");
                        // Each item should be a SamplePair map
                        for (i, item) in items.iter().enumerate() {
                            match item {
                                Value::Map(sp) => {
                                    assert_eq!(
                                        sp.get("__type"),
                                        Some(&Value::String("SamplePair".to_string())),
                                        "item {} should be SamplePair",
                                        i
                                    );
                                    assert!(
                                        sp.contains_key("domainValue"),
                                        "item {} missing domainValue",
                                        i
                                    );
                                    assert!(
                                        sp.contains_key("rangeValue"),
                                        "item {} missing rangeValue",
                                        i
                                    );
                                }
                                _ => panic!("expected SamplePair map at index {}", i),
                            }
                        }
                        // Verify ordered domain values
                        let first = items[0]
                            .as_map()
                            .unwrap()
                            .get("domainValue")
                            .unwrap()
                            .as_float()
                            .unwrap();
                        let last = items[2]
                            .as_map()
                            .unwrap()
                            .get("domainValue")
                            .unwrap()
                            .as_float()
                            .unwrap();
                        assert!((first - 0.0).abs() < 1e-10);
                        assert!((last - 2.0).abs() < 1e-10);
                    }
                    _ => panic!("samples should be a List"),
                }
            }
            _ => panic!("expected Map"),
        }
    }

    #[test]
    fn test_sampled_function_spec_format_construction() {
        // Build using spec format: SampledFunction(list_of_sample_pairs)
        let ctx = EvalContext::new();
        let sp1 = eval_function(
            "SamplePair",
            &[Value::Float(0.0), Value::Float(100.0)],
            &ctx,
        )
        .unwrap();
        let sp2 = eval_function(
            "SamplePair",
            &[Value::Float(1.0), Value::Float(200.0)],
            &ctx,
        )
        .unwrap();
        let sp3 = eval_function(
            "SamplePair",
            &[Value::Float(2.0), Value::Float(300.0)],
            &ctx,
        )
        .unwrap();

        let sf =
            eval_function("SampledFunction", &[Value::List(vec![sp1, sp2, sp3])], &ctx).unwrap();

        // Should work with Interpolate
        let result = eval_function("Interpolate", &[sf.clone(), Value::Float(0.5)], &ctx).unwrap();
        match result {
            Value::Float(f) => assert!((f - 150.0).abs() < 1e-10),
            _ => panic!("expected float"),
        }

        // Should have samples field
        match &sf {
            Value::Map(m) => {
                assert!(
                    m.contains_key("samples"),
                    "should have spec-standard samples field"
                );
                let samples = m.get("samples").unwrap();
                match samples {
                    Value::List(items) => assert_eq!(items.len(), 3),
                    _ => panic!("samples should be a List"),
                }
            }
            _ => panic!("expected Map"),
        }
    }

    #[test]
    fn test_sampled_function_monotonicity_rejection() {
        // Duplicate domain values should be rejected (spec: strictly monotonic)
        let ctx = EvalContext::new();
        let d = vec![Value::Float(0.0), Value::Float(1.0), Value::Float(1.0)];
        let r = vec![Value::Float(0.0), Value::Float(10.0), Value::Float(20.0)];
        let result = eval_function("SampledFunction", &[Value::List(d), Value::List(r)], &ctx);
        assert!(
            result.is_err(),
            "duplicate domain values should be rejected"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("monotonic") || err_msg.contains("duplicate"),
            "error should mention monotonicity: {}",
            err_msg
        );
    }

    #[test]
    fn test_sampled_function_empty_has_samples() {
        let sf = make_sf(&[], &[]);
        match sf {
            Value::Map(m) => {
                assert!(
                    m.contains_key("samples"),
                    "empty SF should have samples field"
                );
                match m.get("samples") {
                    Some(Value::List(items)) => assert!(items.is_empty()),
                    _ => panic!("samples should be empty list"),
                }
            }
            _ => panic!("expected Map"),
        }
    }

    // ── Type conversion tests ────────────────────────────────────────

    #[test]
    fn test_to_integer() {
        let ctx = EvalContext::new();
        // Float truncation
        assert_eq!(
            eval_function("ToInteger", &[Value::Float(3.7)], &ctx).unwrap(),
            Value::Int(3)
        );
        assert_eq!(
            eval_function("ToInteger", &[Value::Float(-2.9)], &ctx).unwrap(),
            Value::Int(-2)
        );
        // Int identity
        assert_eq!(
            eval_function("ToInteger", &[Value::Int(5)], &ctx).unwrap(),
            Value::Int(5)
        );
        // Bool → Int
        assert_eq!(
            eval_function("ToInteger", &[Value::Bool(true)], &ctx).unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            eval_function("ToInteger", &[Value::Bool(false)], &ctx).unwrap(),
            Value::Int(0)
        );
        // String parse
        assert_eq!(
            eval_function("ToInteger", &[Value::String("42".into())], &ctx).unwrap(),
            Value::Int(42)
        );
        // String parse failure
        assert!(eval_function("ToInteger", &[Value::String("abc".into())], &ctx).is_err());
    }

    #[test]
    fn test_to_real() {
        let ctx = EvalContext::new();
        // Int → Float
        assert_eq!(
            eval_function("ToReal", &[Value::Int(3)], &ctx).unwrap(),
            Value::Float(3.0)
        );
        // Float identity
        assert_eq!(
            eval_function("ToReal", &[Value::Float(3.5)], &ctx).unwrap(),
            Value::Float(3.5)
        );
        // Bool → Float
        assert_eq!(
            eval_function("ToReal", &[Value::Bool(true)], &ctx).unwrap(),
            Value::Float(1.0)
        );
        assert_eq!(
            eval_function("ToReal", &[Value::Bool(false)], &ctx).unwrap(),
            Value::Float(0.0)
        );
        // String parse
        assert_eq!(
            eval_function("ToReal", &[Value::String("2.5".into())], &ctx).unwrap(),
            Value::Float(2.5)
        );
        // String parse failure
        assert!(eval_function("ToReal", &[Value::String("nope".into())], &ctx).is_err());
    }

    // ── Boolean collection tests ─────────────────────────────────────

    #[test]
    fn test_all_true() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function(
                "allTrue",
                &[Value::List(vec![Value::Bool(true), Value::Bool(true)])],
                &ctx
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            eval_function(
                "allTrue",
                &[Value::List(vec![Value::Bool(true), Value::Bool(false)])],
                &ctx
            )
            .unwrap(),
            Value::Bool(false)
        );
        // Empty list → true
        assert_eq!(
            eval_function("allTrue", &[Value::List(vec![])], &ctx).unwrap(),
            Value::Bool(true)
        );
        // Non-bool in list → error
        assert!(eval_function("allTrue", &[Value::List(vec![Value::Int(1)])], &ctx).is_err());
    }

    #[test]
    fn test_any_true() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function(
                "anyTrue",
                &[Value::List(vec![Value::Bool(false), Value::Bool(true)])],
                &ctx
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            eval_function(
                "anyTrue",
                &[Value::List(vec![Value::Bool(false), Value::Bool(false)])],
                &ctx
            )
            .unwrap(),
            Value::Bool(false)
        );
        // Empty list → false
        assert_eq!(
            eval_function("anyTrue", &[Value::List(vec![])], &ctx).unwrap(),
            Value::Bool(false)
        );
    }

    // ── SampledFunction accessor tests ───────────────────────────────

    #[test]
    fn test_domain_and_range() {
        let ctx = EvalContext::new();
        let d = vec![Value::Float(1.0), Value::Float(2.0)];
        let r = vec![Value::Float(10.0), Value::Float(20.0)];
        let sf = eval_function(
            "SampledFunction",
            &[Value::List(d.clone()), Value::List(r.clone())],
            &ctx,
        )
        .unwrap();
        // Domain
        let domain = eval_function("Domain", &[sf.clone()], &ctx).unwrap();
        assert_eq!(domain, Value::List(d));
        // Range
        let range = eval_function("Range", &[sf], &ctx).unwrap();
        assert_eq!(range, Value::List(r));
    }

    #[test]
    fn test_domain_range_errors() {
        let ctx = EvalContext::new();
        // Non-map → error
        assert!(eval_function("Domain", &[Value::Int(1)], &ctx).is_err());
        assert!(eval_function("Range", &[Value::Int(1)], &ctx).is_err());
        // Map without domain/range key → error
        let mut map = std::collections::BTreeMap::new();
        map.insert("other".to_string(), Value::Int(1));
        assert!(eval_function("Domain", &[Value::Map(map.clone())], &ctx).is_err());
        assert!(eval_function("Range", &[Value::Map(map)], &ctx).is_err());
    }

    // ── SamplePair / spec-format SampledFunction tests ─────────────

    #[test]
    fn test_sample_pair_construction() {
        let ctx = EvalContext::new();
        let result =
            eval_function("SamplePair", &[Value::Float(1.0), Value::Float(10.0)], &ctx).unwrap();
        match result {
            Value::Map(ref map) => {
                assert_eq!(
                    map.get("__type"),
                    Some(&Value::String("SamplePair".to_string()))
                );
                assert_eq!(map.get("domainValue"), Some(&Value::Float(1.0)));
                assert_eq!(map.get("rangeValue"), Some(&Value::Float(10.0)));
            }
            _ => panic!("expected Map"),
        }
    }

    #[test]
    fn test_sampled_function_from_sample_pairs() {
        let ctx = EvalContext::new();
        // Build SamplePairs in a strictly-increasing domain order (the spec
        // requires a monotonic domain; the constructor preserves the given order).
        let sp1 =
            eval_function("SamplePair", &[Value::Float(1.0), Value::Float(10.0)], &ctx).unwrap();
        let sp2 =
            eval_function("SamplePair", &[Value::Float(2.0), Value::Float(20.0)], &ctx).unwrap();
        let sp3 =
            eval_function("SamplePair", &[Value::Float(3.0), Value::Float(30.0)], &ctx).unwrap();
        let sf =
            eval_function("SampledFunction", &[Value::List(vec![sp1, sp2, sp3])], &ctx).unwrap();
        match &sf {
            Value::Map(map) => {
                assert_eq!(
                    map.get("__type"),
                    Some(&Value::String("SampledFunction".to_string()))
                );
                // Order preserved: 1, 2, 3
                let domain = map.get("domain").unwrap();
                assert_eq!(
                    domain,
                    &Value::List(vec![
                        Value::Float(1.0),
                        Value::Float(2.0),
                        Value::Float(3.0)
                    ])
                );
                let range = map.get("range").unwrap();
                assert_eq!(
                    range,
                    &Value::List(vec![
                        Value::Float(10.0),
                        Value::Float(20.0),
                        Value::Float(30.0)
                    ])
                );
            }
            _ => panic!("expected Map"),
        }
        // Interpolation should work on spec-format SampledFunction
        let v = eval_function("interpolateLinear", &[sf, Value::Float(1.5)], &ctx).unwrap();
        assert_eq!(v, Value::Float(15.0));
    }

    #[test]
    fn test_sampled_function_monotonicity_rejects_duplicates() {
        let ctx = EvalContext::new();
        // Duplicate domain values should be rejected
        let d = vec![Value::Float(1.0), Value::Float(1.0), Value::Float(2.0)];
        let r = vec![Value::Float(10.0), Value::Float(20.0), Value::Float(30.0)];
        let result = eval_function("SampledFunction", &[Value::List(d), Value::List(r)], &ctx);
        assert!(
            result.is_err(),
            "duplicate domain values should be rejected"
        );
        let err_msg = format!("{}", result.unwrap_err());
        // A duplicate makes the domain neither strictly increasing nor
        // decreasing → rejected as non-monotonic (§9.4.3.2.6).
        assert!(err_msg.contains("monotonic"), "error: {err_msg}");
    }

    #[test]
    fn test_sampled_function_from_tuple_list() {
        let ctx = EvalContext::new();
        // Construct from list of [d, r] tuples
        let pairs = Value::List(vec![
            Value::List(vec![Value::Float(0.0), Value::Float(0.0)]),
            Value::List(vec![Value::Float(1.0), Value::Float(1.0)]),
            Value::List(vec![Value::Float(2.0), Value::Float(4.0)]),
        ]);
        let sf = eval_function("SampledFunction", &[pairs], &ctx).unwrap();
        // Interpolate at 1.5 → 2.5 (linear between (1,1) and (2,4))
        let v = eval_function("Interpolate", &[sf, Value::Float(1.5)], &ctx).unwrap();
        assert_eq!(v, Value::Float(2.5));
    }

    // ── Complex number tests ─────────────────────────────────────────

    #[test]
    fn test_rect() {
        let ctx = EvalContext::new();
        let result = eval_function("rect", &[Value::Float(3.0), Value::Float(4.0)], &ctx).unwrap();
        assert_eq!(result, Value::Complex { re: 3.0, im: 4.0 });
    }

    #[test]
    fn test_polar() {
        let ctx = EvalContext::new();
        // polar(1, 0) → 1+0i
        let result = eval_function("polar", &[Value::Float(1.0), Value::Float(0.0)], &ctx).unwrap();
        match result {
            Value::Complex { re, im } => {
                assert!((re - 1.0).abs() < 1e-10);
                assert!(im.abs() < 1e-10);
            }
            _ => panic!("expected Complex"),
        }
        // polar(1, pi/2) → 0+1i
        let result = eval_function(
            "polar",
            &[Value::Float(1.0), Value::Float(std::f64::consts::FRAC_PI_2)],
            &ctx,
        )
        .unwrap();
        match result {
            Value::Complex { re, im } => {
                assert!(re.abs() < 1e-10);
                assert!((im - 1.0).abs() < 1e-10);
            }
            _ => panic!("expected Complex"),
        }
    }

    #[test]
    fn test_re_im() {
        let ctx = EvalContext::new();
        let z = Value::Complex { re: 3.0, im: 4.0 };
        assert_eq!(
            eval_function("re", &[z.clone()], &ctx).unwrap(),
            Value::Float(3.0)
        );
        assert_eq!(eval_function("im", &[z], &ctx).unwrap(), Value::Float(4.0));
        // Real numbers: re returns the value, im returns 0
        assert_eq!(
            eval_function("re", &[Value::Float(5.0)], &ctx).unwrap(),
            Value::Float(5.0)
        );
        assert_eq!(
            eval_function("im", &[Value::Float(5.0)], &ctx).unwrap(),
            Value::Float(0.0)
        );
    }

    #[test]
    fn test_abs_complex() {
        let ctx = EvalContext::new();
        let z = Value::Complex { re: 3.0, im: 4.0 };
        let result = eval_function("abs", &[z], &ctx).unwrap();
        assert_eq!(result, Value::Float(5.0)); // |3+4i| = 5
    }

    #[test]
    fn test_arg_conj() {
        let ctx = EvalContext::new();
        let z = Value::Complex { re: 0.0, im: 1.0 };
        // arg(i) = pi/2
        let arg = eval_function("arg", &[z.clone()], &ctx).unwrap();
        match arg {
            Value::Float(f) => assert!((f - std::f64::consts::FRAC_PI_2).abs() < 1e-10),
            _ => panic!("expected Float"),
        }
        // conj(0+1i) = 0-1i
        let conj = eval_function("conj", &[z], &ctx).unwrap();
        assert_eq!(conj, Value::Complex { re: 0.0, im: -1.0 });
    }

    #[test]
    fn test_complex_arithmetic() {
        // Test complex addition and multiplication through the evaluator
        let a = Value::Complex { re: 1.0, im: 2.0 };
        let b = Value::Complex { re: 3.0, im: 4.0 };
        // (1+2i) + (3+4i) = 4+6i
        let sum = numeric_binop(&a, &b, |x, y| x.checked_add(y), |x, y| x + y).unwrap();
        assert_eq!(sum, Value::Complex { re: 4.0, im: 6.0 });
        // (1+2i) - (3+4i) = -2-2i
        let diff = numeric_binop(&a, &b, |x, y| x.checked_sub(y), |x, y| x - y).unwrap();
        assert_eq!(diff, Value::Complex { re: -2.0, im: -2.0 });
        // Complex + real: (1+2i) + 5 = 6+2i
        let c = Value::Float(5.0);
        let mixed = numeric_binop(&a, &c, |x, y| x.checked_add(y), |x, y| x + y).unwrap();
        assert_eq!(mixed, Value::Complex { re: 6.0, im: 2.0 });
    }

    // -- Vector operation tests -----------------------------------------------

    fn vec3(x: f64, y: f64, z: f64) -> Value {
        Value::List(vec![Value::Float(x), Value::Float(y), Value::Float(z)])
    }

    fn as_f64_list(v: &Value) -> Vec<f64> {
        match v {
            Value::List(items) => items
                .iter()
                .map(|i| match i {
                    Value::Float(f) => *f,
                    Value::Int(n) => *n as f64,
                    _ => panic!("expected numeric in list"),
                })
                .collect(),
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn test_inner_product() {
        let ctx = EvalContext::new();
        // [1,2,3] · [4,5,6] = 4 + 10 + 18 = 32
        let result =
            eval_function("inner", &[vec3(1.0, 2.0, 3.0), vec3(4.0, 5.0, 6.0)], &ctx).unwrap();
        assert_eq!(result, Value::Float(32.0));
    }

    #[test]
    fn test_inner_product_orthogonal() {
        let ctx = EvalContext::new();
        // [1,0,0] · [0,1,0] = 0
        let result =
            eval_function("inner", &[vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], &ctx).unwrap();
        assert_eq!(result, Value::Float(0.0));
    }

    #[test]
    fn test_inner_product_length_mismatch() {
        let ctx = EvalContext::new();
        let u = Value::List(vec![Value::Float(1.0), Value::Float(2.0)]);
        let v = vec3(1.0, 2.0, 3.0);
        assert!(eval_function("inner", &[u, v], &ctx).is_err());
    }

    #[test]
    fn test_outer_product() {
        let ctx = EvalContext::new();
        // [1,0,0] × [0,1,0] = [0,0,1]
        let result =
            eval_function("outer", &[vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], &ctx).unwrap();
        let v = as_f64_list(&result);
        assert_eq!(v, vec![0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_outer_product_general() {
        let ctx = EvalContext::new();
        // [1,2,3] × [4,5,6] = [2*6-3*5, 3*4-1*6, 1*5-2*4] = [-3, 6, -3]
        let result =
            eval_function("outer", &[vec3(1.0, 2.0, 3.0), vec3(4.0, 5.0, 6.0)], &ctx).unwrap();
        let v = as_f64_list(&result);
        assert_eq!(v, vec![-3.0, 6.0, -3.0]);
    }

    #[test]
    fn test_norm() {
        let ctx = EvalContext::new();
        // norm([3,4]) = 5
        let v = Value::List(vec![Value::Float(3.0), Value::Float(4.0)]);
        let result = eval_function("norm", &[v], &ctx).unwrap();
        assert_eq!(result, Value::Float(5.0));
    }

    #[test]
    fn test_norm_3d() {
        let ctx = EvalContext::new();
        // norm([1,2,2]) = 3
        let result = eval_function("norm", &[vec3(1.0, 2.0, 2.0)], &ctx).unwrap();
        assert_eq!(result, Value::Float(3.0));
    }

    #[test]
    fn test_angle_perpendicular() {
        let ctx = EvalContext::new();
        // angle between [1,0,0] and [0,1,0] = π/2
        let result =
            eval_function("angle", &[vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], &ctx).unwrap();
        match result {
            Value::Float(f) => assert!((f - std::f64::consts::FRAC_PI_2).abs() < 1e-10),
            _ => panic!("expected float"),
        }
    }

    #[test]
    fn test_angle_parallel() {
        let ctx = EvalContext::new();
        // angle between [1,0,0] and [2,0,0] = 0
        let result =
            eval_function("angle", &[vec3(1.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0)], &ctx).unwrap();
        match result {
            Value::Float(f) => assert!(f.abs() < 1e-10),
            _ => panic!("expected float"),
        }
    }

    #[test]
    fn test_scalar_vector_mult() {
        let ctx = EvalContext::new();
        // 3 * [1,2,3] = [3,6,9]
        let result = eval_function(
            "scalarVectorMult",
            &[Value::Float(3.0), vec3(1.0, 2.0, 3.0)],
            &ctx,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![3.0, 6.0, 9.0]);
    }

    #[test]
    fn test_vector_scalar_div() {
        let ctx = EvalContext::new();
        // [6,9,12] / 3 = [2,3,4]
        let result = eval_function(
            "vectorScalarDiv",
            &[vec3(6.0, 9.0, 12.0), Value::Float(3.0)],
            &ctx,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_vector_scalar_div_by_zero() {
        let ctx = EvalContext::new();
        assert!(eval_function(
            "vectorScalarDiv",
            &[vec3(1.0, 2.0, 3.0), Value::Float(0.0)],
            &ctx
        )
        .is_err());
    }

    #[test]
    fn test_is_zero_vector() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("isZeroVector", &[vec3(0.0, 0.0, 0.0)], &ctx).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            eval_function("isZeroVector", &[vec3(0.0, 0.001, 0.0)], &ctx).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_is_unit_vector() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("isUnitVector", &[vec3(1.0, 0.0, 0.0)], &ctx).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            eval_function("isUnitVector", &[vec3(2.0, 0.0, 0.0)], &ctx).unwrap(),
            Value::Bool(false)
        );
    }

    // -- Scalar-vector arithmetic via numeric_binop ---------------------------

    #[test]
    fn test_vector_add() {
        // [1,2,3] + [4,5,6] = [5,7,9]
        let result = numeric_binop(
            &vec3(1.0, 2.0, 3.0),
            &vec3(4.0, 5.0, 6.0),
            |a, b| a.checked_add(b),
            |a, b| a + b,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![5.0, 7.0, 9.0]);
    }

    #[test]
    fn test_vector_subtract() {
        // [5,7,9] - [1,2,3] = [4,5,6]
        let result = numeric_binop(
            &vec3(5.0, 7.0, 9.0),
            &vec3(1.0, 2.0, 3.0),
            |a, b| a.checked_sub(b),
            |a, b| a - b,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_scalar_times_vector() {
        // 2 * [1,2,3] = [2,4,6]
        let result = numeric_binop(
            &Value::Float(2.0),
            &vec3(1.0, 2.0, 3.0),
            |a, b| a.checked_mul(b),
            |a, b| a * b,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn test_vector_times_scalar() {
        // [1,2,3] * 2 = [2,4,6]
        let result = numeric_binop(
            &vec3(1.0, 2.0, 3.0),
            &Value::Float(2.0),
            |a, b| a.checked_mul(b),
            |a, b| a * b,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn test_vector_div_scalar() {
        // [6,8,10] / 2 = [3,4,5]
        let result = numeric_binop(
            &vec3(6.0, 8.0, 10.0),
            &Value::Float(2.0),
            |a, b| a.checked_div(b),
            |a, b| a / b,
        )
        .unwrap();
        assert_eq!(as_f64_list(&result), vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn test_vector_length_mismatch_arithmetic() {
        let u = Value::List(vec![Value::Float(1.0), Value::Float(2.0)]);
        let v = vec3(1.0, 2.0, 3.0);
        assert!(numeric_binop(&u, &v, |a, b| a.checked_add(b), |a, b| a + b).is_err());
    }

    // -----------------------------------------------------------------------
    // Quantity & Unit tests
    // -----------------------------------------------------------------------

    use sysml_core::physics::DimensionVector;

    fn length_dim() -> DimensionVector {
        DimensionVector::new(1, 0, 0, 0, 0, 0, 0)
    }
    fn time_dim() -> DimensionVector {
        DimensionVector::new(0, 0, 1, 0, 0, 0, 0)
    }
    fn velocity_dim() -> DimensionVector {
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0)
    }

    fn qty(v: f64, dim: DimensionVector, unit: &str) -> Value {
        Value::Quantity {
            value: v,
            dimension: dim,
            unit: Some(unit.to_string()),
        }
    }

    #[test]
    fn test_quantity_construct_via_function() {
        let ctx = EvalContext::new();
        let result = eval_function(
            "quantity",
            &[Value::Float(5.0), Value::String("m".into())],
            &ctx,
        )
        .unwrap();
        match &result {
            Value::Quantity {
                value,
                dimension,
                unit,
            } => {
                assert_eq!(*value, 5.0);
                assert_eq!(*dimension, length_dim());
                assert_eq!(unit.as_deref(), Some("m"));
            }
            _ => panic!("expected Quantity, got {:?}", result),
        }
    }

    #[test]
    fn test_quantity_construct_unknown_unit() {
        let ctx = EvalContext::new();
        let result = eval_function(
            "quantity",
            &[Value::Float(5.0), Value::String("furlongs".into())],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_quantity_km_to_m() {
        let ctx = EvalContext::new();
        let input = qty(5.0, length_dim(), "km");
        let result =
            eval_function("ConvertQuantity", &[input, Value::String("m".into())], &ctx).unwrap();
        match &result {
            Value::Quantity { value, unit, .. } => {
                assert!((value - 5000.0).abs() < 1e-10);
                assert_eq!(unit.as_deref(), Some("m"));
            }
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn test_convert_quantity_celsius_to_kelvin() {
        let ctx = EvalContext::new();
        let temp_dim = DimensionVector::new(0, 0, 0, 0, 1, 0, 0);
        let input = qty(100.0, temp_dim, "degC");
        let result =
            eval_function("ConvertQuantity", &[input, Value::String("K".into())], &ctx).unwrap();
        match &result {
            Value::Quantity { value, .. } => {
                assert!((value - 373.15).abs() < 1e-10);
            }
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn test_convert_quantity_dimension_mismatch() {
        let ctx = EvalContext::new();
        let input = qty(5.0, length_dim(), "m");
        let result = eval_function(
            "ConvertQuantity",
            &[input, Value::String("kg".into())],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_unit_of() {
        let ctx = EvalContext::new();
        let input = qty(5.0, length_dim(), "m");
        let result = eval_function("unitOf", &[input], &ctx).unwrap();
        assert_eq!(result, Value::String("m".to_string()));
    }

    #[test]
    fn test_dimension_of() {
        let ctx = EvalContext::new();
        let input = qty(5.0, length_dim(), "m");
        let result = eval_function("dimensionOf", &[input], &ctx).unwrap();
        assert_eq!(result, Value::String("L".to_string()));
    }

    #[test]
    fn test_numeric_value() {
        let ctx = EvalContext::new();
        let input = qty(3.14, length_dim(), "m");
        let result = eval_function("numericValue", &[input], &ctx).unwrap();
        assert_eq!(result, Value::Float(3.14));
    }

    #[test]
    fn test_quantity_promote_to_float() {
        let q = qty(5.0, length_dim(), "m");
        let f = Value::Float(3.0);
        let (a, b) = promote_to_float(&q, &f).unwrap();
        assert_eq!(a, 5.0);
        assert_eq!(b, 3.0);
    }

    #[test]
    fn test_quantity_in_numeric_binop() {
        // Quantity + Float — numeric_binop strips dimension, returns Float
        let q = qty(5.0, length_dim(), "m");
        let f = Value::Float(3.0);
        let result = numeric_binop(&q, &f, |a, b| a.checked_add(b), |a, b| a + b).unwrap();
        assert_eq!(result, Value::Float(8.0));
    }

    // ── Occurrence model tests ──

    fn ctx_with_registry() -> EvalContext {
        let mut ctx = EvalContext::new();
        let reg = std::sync::Arc::new(std::sync::Mutex::new(
            sysml_core::occurrence::OccurrenceRegistry::new(),
        ));
        ctx.occurrence_registry = Some(reg);
        ctx.set("__time", Value::Float(1.0));
        ctx.set("__clock", Value::String("default".into()));
        ctx
    }

    #[test]
    fn test_occurrence_create() {
        let ctx = ctx_with_registry();
        let result = eval_function("create", &[], &ctx).unwrap();
        // Should return an ID string
        assert!(matches!(result, Value::String(_)));

        // Registry should have one instance
        let reg = ctx.occurrence_registry.as_ref().unwrap().lock().unwrap();
        assert_eq!(reg.instance_count(), 1);
        assert_eq!(reg.life_count(), 1);
    }

    #[test]
    fn test_occurrence_create_destroy_is_during() {
        let mut ctx = ctx_with_registry();
        ctx.set("__time", Value::Float(2.0));

        // Create
        let occ_id = eval_function("create", &[], &ctx).unwrap();
        let occ_id_str = match &occ_id {
            Value::String(s) => s.clone(),
            _ => panic!("expected string ID"),
        };

        // isDuring at t=5 (after creation, not yet destroyed) → true
        ctx.set("__time", Value::Float(5.0));
        let during = eval_function("isDuring", &[occ_id.clone()], &ctx).unwrap();
        assert_eq!(during, Value::Bool(true));

        // Destroy at t=8
        ctx.set("__time", Value::Float(8.0));
        let destroyed = eval_function("destroy", &[occ_id.clone()], &ctx).unwrap();
        assert_eq!(destroyed, Value::Bool(true));

        // isDuring at t=5 → still true (within [2, 8])
        ctx.set("__time", Value::Float(5.0));
        let during = eval_function("isDuring", &[occ_id.clone()], &ctx).unwrap();
        assert_eq!(during, Value::Bool(true));

        // isDuring at t=10 → false (after end)
        ctx.set("__time", Value::Float(10.0));
        let during = eval_function("isDuring", &[occ_id.clone()], &ctx).unwrap();
        assert_eq!(during, Value::Bool(false));

        // isDuring at t=1 → false (before start)
        ctx.set("__time", Value::Float(1.0));
        let during = eval_function("isDuring", &[Value::String(occ_id_str)], &ctx).unwrap();
        assert_eq!(during, Value::Bool(false));
    }

    #[test]
    fn test_occurrence_add_new() {
        let ctx = ctx_with_registry();
        let list = Value::List(vec![Value::Int(1), Value::Int(2)]);
        let result = eval_function("addNew", &[list, Value::Int(3)], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );

        // Registry should have one occurrence from the addNew call
        let reg = ctx.occurrence_registry.as_ref().unwrap().lock().unwrap();
        assert_eq!(reg.instance_count(), 1);
    }

    #[test]
    fn test_occurrence_add_new_at() {
        let ctx = ctx_with_registry();
        let list = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let result =
            eval_function("addNewAt", &[list, Value::Int(99), Value::Int(1)], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Int(1),
                Value::Int(99),
                Value::Int(2),
                Value::Int(3)
            ])
        );
    }

    #[test]
    fn test_occurrence_add_new_at_clamps_index() {
        let ctx = ctx_with_registry();
        let list = Value::List(vec![Value::Int(1)]);
        // Index 100 exceeds list size — should clamp to end
        let result =
            eval_function("addNewAt", &[list, Value::Int(99), Value::Int(100)], &ctx).unwrap();
        assert_eq!(result, Value::List(vec![Value::Int(1), Value::Int(99)]));
    }

    #[test]
    fn test_occurrence_without_registry() {
        // Without a registry, functions should return NotYetImplemented
        let ctx = EvalContext::new();
        let result = eval_function("create", &[], &ctx);
        assert!(matches!(
            result,
            Err(EvaluationError::NotYetImplemented { .. })
        ));
    }

    #[test]
    fn test_occurrence_destroy_nonexistent() {
        let ctx = ctx_with_registry();
        // Destroy an ID that was never created → false
        let result =
            eval_function("destroy", &[Value::String("nonexistent".into())], &ctx).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_occurrence_is_during_nonexistent() {
        let ctx = ctx_with_registry();
        // isDuring for nonexistent occurrence → false
        let result =
            eval_function("isDuring", &[Value::String("nonexistent".into())], &ctx).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    // ── Spatial / Coordinate frame tests ──

    #[test]
    fn test_vector_of() {
        let ctx = EvalContext::new();
        let result = eval_function(
            "VectorOf",
            &[Value::Float(1.0), Value::Float(2.0), Value::Float(3.0)],
            &ctx,
        )
        .unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0)
            ])
        );
    }

    #[test]
    fn test_vector_of_mixed_types() {
        let ctx = EvalContext::new();
        let result = eval_function("VectorOf", &[Value::Int(1), Value::Float(2.5)], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![Value::Float(1.0), Value::Float(2.5)])
        );
    }

    #[test]
    fn test_vector_of_empty() {
        let ctx = EvalContext::new();
        let result = eval_function("VectorOf", &[], &ctx).unwrap();
        assert_eq!(result, Value::List(vec![]));
    }

    #[test]
    fn test_cartesian_vector_of() {
        let ctx = EvalContext::new();
        let result = eval_function(
            "CartesianVectorOf",
            &[Value::Float(4.0), Value::Float(5.0)],
            &ctx,
        )
        .unwrap();
        assert_eq!(
            result,
            Value::List(vec![Value::Float(4.0), Value::Float(5.0)])
        );
    }

    #[test]
    fn test_cartesian_three_vector_of() {
        let ctx = EvalContext::new();
        let result = eval_function(
            "CartesianThreeVectorOf",
            &[Value::Float(1.0), Value::Int(2), Value::Float(3.0)],
            &ctx,
        )
        .unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0)
            ])
        );
    }

    #[test]
    fn test_cartesian_three_vector_of_arity() {
        let ctx = EvalContext::new();
        let result = eval_function("CartesianThreeVectorOf", &[Value::Float(1.0)], &ctx);
        assert!(matches!(result, Err(EvaluationError::ArityMismatch { .. })));
    }

    #[test]
    fn test_displacement_of() {
        let ctx = EvalContext::new();
        let p1 = Value::List(vec![
            Value::Float(1.0),
            Value::Float(2.0),
            Value::Float(3.0),
        ]);
        let p2 = Value::List(vec![
            Value::Float(4.0),
            Value::Float(6.0),
            Value::Float(8.0),
        ]);
        let result = eval_function("DisplacementOf", &[p1, p2], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Float(3.0),
                Value::Float(4.0),
                Value::Float(5.0)
            ])
        );
    }

    #[test]
    fn test_cartesian_displacement_of() {
        let ctx = EvalContext::new();
        let p1 = Value::List(vec![
            Value::Float(0.0),
            Value::Float(0.0),
            Value::Float(0.0),
        ]);
        let p2 = Value::List(vec![
            Value::Float(1.0),
            Value::Float(1.0),
            Value::Float(1.0),
        ]);
        let result = eval_function("CartesianDisplacementOf", &[p1, p2], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Float(1.0),
                Value::Float(1.0),
                Value::Float(1.0)
            ])
        );
    }

    #[test]
    fn test_position_of_passthrough() {
        let ctx = EvalContext::new();
        let pos = Value::List(vec![
            Value::Float(1.0),
            Value::Float(2.0),
            Value::Float(3.0),
        ]);
        let result = eval_function("PositionOf", &[pos.clone()], &ctx).unwrap();
        assert_eq!(result, pos);
    }

    #[test]
    fn test_position_of_default() {
        let ctx = EvalContext::new();
        // Non-vector arg → default zero vector
        let result = eval_function("PositionOf", &[Value::String("point1".into())], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Float(0.0),
                Value::Float(0.0),
                Value::Float(0.0)
            ])
        );
    }

    #[test]
    fn test_transform_with_registry() {
        use sysml_core::spatial::*;
        let mut reg = FrameRegistry::new();
        reg.register_frame(SpatialFrame {
            id: sysml_core::ElementId::from_string("world"),
            name: "world".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        reg.register_frame(SpatialFrame {
            id: sysml_core::ElementId::from_string("body"),
            name: "body".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        // 90° rotation around Z
        let t = CoordinateTransformation::from_origin_and_basis(
            sysml_core::ElementId::from_string("world"),
            sysml_core::ElementId::from_string("body"),
            [0.0, 0.0, 0.0],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        );
        reg.register_transform_named("world", "body", t);

        let mut ctx = EvalContext::new();
        ctx.frame_registry = Some(Arc::new(reg));

        let vec = Value::List(vec![
            Value::Float(1.0),
            Value::Float(0.0),
            Value::Float(0.0),
        ]);
        let result = eval_function(
            "transform",
            &[
                vec,
                Value::String("world".into()),
                Value::String("body".into()),
            ],
            &ctx,
        )
        .unwrap();

        match result {
            Value::List(vals) => {
                let floats: Vec<f64> = vals
                    .iter()
                    .map(|v| match v {
                        Value::Float(f) => *f,
                        _ => panic!("expected float"),
                    })
                    .collect();
                assert!((floats[0] - 0.0).abs() < 1e-10);
                assert!((floats[1] - 1.0).abs() < 1e-10);
                assert!((floats[2] - 0.0).abs() < 1e-10);
            }
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn test_transform_without_registry() {
        let ctx = EvalContext::new();
        let vec = Value::List(vec![
            Value::Float(1.0),
            Value::Float(2.0),
            Value::Float(3.0),
        ]);
        // Without registry, transform returns vector unchanged
        let result = eval_function(
            "transform",
            &[
                vec.clone(),
                Value::String("a".into()),
                Value::String("b".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, vec);
    }

    #[test]
    fn test_coordinate_frame_mult() {
        use sysml_core::spatial::*;
        let mut reg = FrameRegistry::new();
        reg.register_frame(SpatialFrame {
            id: sysml_core::ElementId::from_string("default"),
            name: "default".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        reg.set_default_frame("default");
        reg.register_frame(SpatialFrame {
            id: sysml_core::ElementId::from_string("rotated"),
            name: "rotated".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        // Identity transform for simplicity — just check the path works
        let t = CoordinateTransformation::identity(
            sysml_core::ElementId::from_string("default"),
            sysml_core::ElementId::from_string("rotated"),
        );
        reg.register_transform_named("default", "rotated", t);

        let mut ctx = EvalContext::new();
        ctx.frame_registry = Some(Arc::new(reg));

        let vec = Value::List(vec![
            Value::Float(1.0),
            Value::Float(2.0),
            Value::Float(3.0),
        ]);
        let result = eval_function(
            "CoordinateFrame*",
            &[Value::String("rotated".into()), vec.clone()],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, vec);
    }

    // ── Performance / Evaluation model tests ──

    #[test]
    fn test_evaluation_abstract() {
        let ctx = EvalContext::new();
        assert_eq!(eval_function("Evaluation", &[], &ctx).unwrap(), Value::Null);
        assert_eq!(
            eval_function("Evaluation", &[Value::Int(42)], &ctx).unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn test_literal_evaluation() {
        let ctx = EvalContext::new();
        let result = eval_function("LiteralEvaluation", &[Value::Float(3.14)], &ctx).unwrap();
        assert_eq!(result, Value::Float(3.14));
    }

    #[test]
    fn test_literal_integer_evaluation() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("LiteralIntegerEvaluation", &[Value::Int(42)], &ctx).unwrap(),
            Value::Int(42)
        );
        // Float → Int coercion
        assert_eq!(
            eval_function("LiteralIntegerEvaluation", &[Value::Float(7.9)], &ctx).unwrap(),
            Value::Int(7)
        );
    }

    #[test]
    fn test_literal_rational_evaluation() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("LiteralRationalEvaluation", &[Value::Float(2.5)], &ctx).unwrap(),
            Value::Float(2.5)
        );
        // Int → Float coercion
        assert_eq!(
            eval_function("LiteralRationalEvaluation", &[Value::Int(3)], &ctx).unwrap(),
            Value::Float(3.0)
        );
    }

    #[test]
    fn test_literal_string_evaluation() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function(
                "LiteralStringEvaluation",
                &[Value::String("hello".into())],
                &ctx
            )
            .unwrap(),
            Value::String("hello".into())
        );
    }

    #[test]
    fn test_null_evaluation() {
        let ctx = EvalContext::new();
        assert_eq!(
            eval_function("NullEvaluation", &[], &ctx).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn test_feature_read_evaluation() {
        let mut ctx = EvalContext::new();
        ctx.set("temperature", Value::Float(98.6));
        let result = eval_function(
            "FeatureReadEvaluation",
            &[Value::String("temperature".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Float(98.6));
    }

    #[test]
    fn test_feature_read_evaluation_missing() {
        let ctx = EvalContext::new();
        let result = eval_function(
            "FeatureReadEvaluation",
            &[Value::String("missing".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_all_substate_performances() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "__active_substates",
            Value::List(vec![
                Value::String("heating".into()),
                Value::String("pumping".into()),
            ]),
        );
        let result = eval_function("allSubstatePerformances", &[], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::String("heating".into()),
                Value::String("pumping".into()),
            ])
        );
    }

    #[test]
    fn test_all_substate_performances_empty() {
        let ctx = EvalContext::new();
        let result = eval_function("allSubstatePerformances", &[], &ctx).unwrap();
        assert_eq!(result, Value::List(vec![]));
    }

    #[test]
    fn test_all_subtransition_performances() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "__recent_transitions",
            Value::List(vec![Value::String("idle_to_heating".into())]),
        );
        let result = eval_function("allSubtransitionPerformances", &[], &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![Value::String("idle_to_heating".into()),])
        );
    }

    #[test]
    fn test_index_function() {
        let ctx = EvalContext::new();
        let list = Value::List(vec![Value::Int(10), Value::Int(20), Value::Int(30)]);
        assert_eq!(
            eval_function("index", &[list.clone(), Value::Int(0)], &ctx).unwrap(),
            Value::Int(10)
        );
        assert_eq!(
            eval_function("index", &[list.clone(), Value::Int(2)], &ctx).unwrap(),
            Value::Int(30)
        );
    }

    #[test]
    fn test_index_out_of_bounds() {
        let ctx = EvalContext::new();
        let list = Value::List(vec![Value::Int(1)]);
        let result = eval_function("index", &[list, Value::Int(5)], &ctx);
        assert!(matches!(
            result,
            Err(EvaluationError::IndexOutOfBounds { index: 5, size: 1 })
        ));
    }
}
