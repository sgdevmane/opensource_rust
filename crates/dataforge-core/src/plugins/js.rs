// =============================================================================
// DataForge Core — WASM JavaScript Plugin Engine
// =============================================================================
// Lightweight JavaScript expression interpreter for row transformations.
// Supports ternary operators, logical checks, and variable resolution.
// =============================================================================

use crate::types::{CellValue, Row};
use crate::error::{DataForgeError, Result};

/// Lightweight JS-compatible expression interpreter.
pub struct JsEngine;

impl JsEngine {
    /// Interpret a simple JS expression, e.g. "age >= 18 ? 'Adult' : 'Minor'"
    pub fn eval_expr(expr: &str, row: &Row, headers: &[String]) -> Result<CellValue> {
        let expr = expr.trim();
        // Handle ternary operator: cond ? true_val : false_val
        if let Some(q_pos) = expr.find('?') {
            if let Some(c_pos) = expr.find(':') {
                let cond_str = &expr[..q_pos].trim();
                let true_str = &expr[q_pos+1..c_pos].trim();
                let false_str = &expr[c_pos+1..].trim();

                let cond_res = Self::eval_condition(cond_str, row, headers)?;
                if cond_res {
                    return Self::eval_val(true_str, row, headers);
                } else {
                    return Self::eval_val(false_str, row, headers);
                }
            }
        }
        Self::eval_val(expr, row, headers)
    }

    fn eval_condition(cond: &str, row: &Row, headers: &[String]) -> Result<bool> {
        let ops = [">=", "<=", ">", "<", "==", "!="];
        for op in &ops {
            if let Some(pos) = cond.find(op) {
                let field = cond[..pos].trim();
                let val_str = cond[pos + op.len()..].trim();

                let col_idx = headers.iter().position(|h| h.eq_ignore_ascii_case(field))
                    .ok_or_else(|| DataForgeError::config(format!("Field not found: {field}")))?;

                let cell_val = row.get(col_idx).cloned().unwrap_or(CellValue::Null);
                let check_val = Self::eval_val(val_str, row, headers)?;

                match *op {
                    ">=" => return Ok(cell_val.as_float().unwrap_or(0.0) >= check_val.as_float().unwrap_or(0.0)),
                    "<=" => return Ok(cell_val.as_float().unwrap_or(0.0) <= check_val.as_float().unwrap_or(0.0)),
                    ">" => return Ok(cell_val.as_float().unwrap_or(0.0) > check_val.as_float().unwrap_or(0.0)),
                    "<" => return Ok(cell_val.as_float().unwrap_or(0.0) < check_val.as_float().unwrap_or(0.0)),
                    "==" => return Ok(cell_val.to_display_string() == check_val.to_display_string()),
                    "!=" => return Ok(cell_val.to_display_string() != check_val.to_display_string()),
                    _ => {}
                }
            }
        }
        Ok(false)
    }

    fn eval_val(val_str: &str, row: &Row, headers: &[String]) -> Result<CellValue> {
        let val_str = val_str.trim();
        if (val_str.starts_with('\'') && val_str.ends_with('\'')) || (val_str.starts_with('"') && val_str.ends_with('"')) {
            return Ok(CellValue::from(&val_str[1..val_str.len()-1]));
        }
        if let Ok(i) = val_str.parse::<i64>() {
            return Ok(CellValue::Int(i));
        }
        if let Ok(f) = val_str.parse::<f64>() {
            return Ok(CellValue::Float(f));
        }
        if let Some(col_idx) = headers.iter().position(|h| h.eq_ignore_ascii_case(val_str)) {
            return Ok(row.get(col_idx).cloned().unwrap_or(CellValue::Null));
        }
        Ok(CellValue::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_js_engine_ternary() {
        let headers = vec!["name".to_string(), "age".to_string()];
        let mut row = Row::new(0);
        row.push(CellValue::from("Bob"));
        row.push(CellValue::from(20_i64));

        let res = JsEngine::eval_expr("age >= 18 ? 'Adult' : 'Minor'", &row, &headers).unwrap();
        assert_eq!(res.as_str(), Some("Adult"));

        let mut row_child = Row::new(1);
        row_child.push(CellValue::from("Alice"));
        row_child.push(CellValue::from(15_i64));
        let res2 = JsEngine::eval_expr("age >= 18 ? 'Adult' : 'Minor'", &row_child, &headers).unwrap();
        assert_eq!(res2.as_str(), Some("Minor"));
    }
}
