// =============================================================================
// DataForge Core — Formula Evaluator
// =============================================================================
// Lightweight formula parsing and evaluation engine.
// Supports row-relative arithmetic operations and batch aggregate functions.
// =============================================================================

use crate::types::{CellValue, Row};
use crate::error::{DataForgeError, Result};

/// Lightweight spreadsheet formula evaluator.
pub struct FormulaEvaluator;

/// Helper to normalize ODS-style formulas into standard Excel formulas.
pub fn normalize_ods_formula(formula: &str) -> String {
    let mut formula = formula.trim().to_string();
    if formula.starts_with("oooc:=") {
        formula = formula.replacen("oooc:=", "=", 1);
    }
    // Remove OpenDocument cell reference delimiters, e.g. [.A1] -> A1
    formula = formula.replace("[.", "").replace("]", "");
    formula
}

impl FormulaEvaluator {
    /// Evaluate a row-relative formula (e.g. "=A1+B1" or "=A+B*C") against a single row.
    pub fn eval_row(formula: &str, row: &Row) -> Result<CellValue> {
        let normalized = normalize_ods_formula(formula);
        let formula = normalized.trim();
        if !formula.starts_with('=') {
            return Err(DataForgeError::config("Formula must start with '='"));
        }
        let expr = &formula[1..];
        Self::eval_simple_expression(expr, row)
    }

    /// Evaluate an aggregate function (e.g. "=SUM(A)", "=AVERAGE(B)") over a slice of rows.
    pub fn eval_batch(formula: &str, rows: &[Row]) -> Result<CellValue> {
        let normalized = normalize_ods_formula(formula);
        let formula = normalized.trim();
        if !formula.starts_with('=') {
            return Err(DataForgeError::config("Formula must start with '='"));
        }
        let expr = formula[1..].trim().to_uppercase();

        if expr.starts_with("SUM(") && expr.ends_with(')') {
            let col_name = &expr[4..expr.len() - 1];
            let col_idx = col_letter_to_index(col_name)?;
            let mut sum = 0.0;
            for row in rows {
                if let Some(val) = row.get(col_idx) {
                    sum += val.as_float().unwrap_or(0.0);
                }
            }
            Ok(CellValue::Float(sum))
        } else if expr.starts_with("AVERAGE(") && expr.ends_with(')') {
            let col_name = &expr[8..expr.len() - 1];
            let col_idx = col_letter_to_index(col_name)?;
            if rows.is_empty() {
                return Ok(CellValue::Null);
            }
            let mut sum = 0.0;
            let mut count = 0;
            for row in rows {
                if let Some(val) = row.get(col_idx) {
                    sum += val.as_float().unwrap_or(0.0);
                    count += 1;
                }
            }
            if count == 0 {
                Ok(CellValue::Null)
            } else {
                Ok(CellValue::Float(sum / count as f64))
            }
        } else if expr.starts_with("MIN(") && expr.ends_with(')') {
            let col_name = &expr[4..expr.len() - 1];
            let col_idx = col_letter_to_index(col_name)?;
            let mut min_val = f64::MAX;
            let mut found = false;
            for row in rows {
                if let Some(val) = row.get(col_idx) {
                    if let Some(f) = val.as_float() {
                        if f < min_val {
                            min_val = f;
                            found = true;
                        }
                    }
                }
            }
            if found {
                Ok(CellValue::Float(min_val))
            } else {
                Ok(CellValue::Null)
            }
        } else if expr.starts_with("MAX(") && expr.ends_with(')') {
            let col_name = &expr[4..expr.len() - 1];
            let col_idx = col_letter_to_index(col_name)?;
            let mut max_val = f64::MIN;
            let mut found = false;
            for row in rows {
                if let Some(val) = row.get(col_idx) {
                    if let Some(f) = val.as_float() {
                        if f > max_val {
                            max_val = f;
                            found = true;
                        }
                    }
                }
            }
            if found {
                Ok(CellValue::Float(max_val))
            } else {
                Ok(CellValue::Null)
            }
        } else {
            Err(DataForgeError::config(format!("Unsupported batch formula: {}", formula)))
        }
    }

    fn eval_simple_expression(expr: &str, row: &Row) -> Result<CellValue> {
        let tokens = tokenize(expr)?;
        let mut values = Vec::new();
        let mut ops = Vec::new();

        for token in tokens {
            match token {
                Token::Value(val) => values.push(val),
                Token::Ref(col_idx) => {
                    let cell_val = row.get(col_idx).cloned().unwrap_or(CellValue::Null);
                    values.push(cell_val);
                }
                Token::Op(op) => ops.push(op),
            }
        }

        if values.is_empty() {
            return Ok(CellValue::Null);
        }

        // Apply * and / first
        let mut i = 0;
        while i < ops.len() {
            if ops[i] == '*' || ops[i] == '/' {
                let op = ops.remove(i);
                let left = values.remove(i);
                let right = values.remove(i);
                let res = apply_op(left, right, op)?;
                values.insert(i, res);
            } else {
                i += 1;
            }
        }

        // Apply + and -
        let mut res = values.remove(0);
        for op in ops {
            let right = values.remove(0);
            res = apply_op(res, right, op)?;
        }

        Ok(res)
    }
}

enum Token {
    Value(CellValue),
    Ref(usize),
    Op(char),
}

fn tokenize(expr: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c == '+' || c == '-' || c == '*' || c == '/' {
            tokens.push(Token::Op(c));
            i += 1;
        } else if c.is_alphabetic() {
            let mut ref_str = String::new();
            while i < chars.len() && (chars[i].is_alphabetic() || chars[i].is_numeric()) {
                ref_str.push(chars[i]);
                i += 1;
            }
            let col_idx = col_letter_to_index(&ref_str)?;
            tokens.push(Token::Ref(col_idx));
        } else if c.is_numeric() || c == '.' {
            let mut num_str = String::new();
            while i < chars.len() && (chars[i].is_numeric() || chars[i] == '.') {
                num_str.push(chars[i]);
                i += 1;
            }
            if num_str.contains('.') {
                let f = num_str.parse::<f64>().map_err(|_| DataForgeError::config("Invalid float token"))?;
                tokens.push(Token::Value(CellValue::Float(f)));
            } else {
                let val = num_str.parse::<i64>().map_err(|_| DataForgeError::config("Invalid int token"))?;
                tokens.push(Token::Value(CellValue::Int(val)));
            }
        } else {
            return Err(DataForgeError::config(format!("Unexpected character in formula: {}", c)));
        }
    }

    Ok(tokens)
}

fn col_letter_to_index(col_ref: &str) -> Result<usize> {
    let letters: String = col_ref.chars().take_while(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return Err(DataForgeError::config(format!("Invalid column reference: {}", col_ref)));
    }

    let mut index = 0;
    for c in letters.to_uppercase().chars() {
        index = index * 26 + (c as usize - 'A' as usize + 1);
    }
    Ok(index - 1)
}

fn apply_op(left: CellValue, right: CellValue, op: char) -> Result<CellValue> {
    let lf = left.as_float().unwrap_or(0.0);
    let rf = right.as_float().unwrap_or(0.0);

    match op {
        '+' => Ok(CellValue::Float(lf + rf)),
        '-' => Ok(CellValue::Float(lf - rf)),
        '*' => Ok(CellValue::Float(lf * rf)),
        '/' => {
            if rf == 0.0 {
                Ok(CellValue::Null)
            } else {
                Ok(CellValue::Float(lf / rf))
            }
        }
        _ => Err(DataForgeError::config(format!("Unknown operator: {}", op))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_row_arithmetic() {
        let mut row = Row::new(0);
        row.push(CellValue::from(10.0)); // A
        row.push(CellValue::from(5.0));  // B
        row.push(CellValue::from(2.0));  // C

        let res = FormulaEvaluator::eval_row("=A1+B1*C1", &row).unwrap();
        assert_eq!(res.as_float(), Some(20.0));

        let res2 = FormulaEvaluator::eval_row("=A1/B1-C1", &row).unwrap();
        assert_eq!(res2.as_float(), Some(0.0));
    }

    #[test]
    fn test_eval_batch_aggregates() {
        let mut r1 = Row::new(0);
        r1.push(CellValue::from(10.0));
        let mut r2 = Row::new(1);
        r2.push(CellValue::from(20.0));
        let mut r3 = Row::new(2);
        r3.push(CellValue::from(30.0));

        let rows = vec![r1, r2, r3];

        let sum = FormulaEvaluator::eval_batch("=SUM(A)", &rows).unwrap();
        assert_eq!(sum.as_float(), Some(60.0));

        let avg = FormulaEvaluator::eval_batch("=AVERAGE(A)", &rows).unwrap();
        assert_eq!(avg.as_float(), Some(20.0));

        let min = FormulaEvaluator::eval_batch("=MIN(A)", &rows).unwrap();
        assert_eq!(min.as_float(), Some(10.0));

        let max = FormulaEvaluator::eval_batch("=MAX(A)", &rows).unwrap();
        assert_eq!(max.as_float(), Some(30.0));
    }

    #[test]
    fn test_ods_formula_normalization() {
        let mut row = Row::new(0);
        row.push(CellValue::from(10.0)); // A
        row.push(CellValue::from(5.0));  // B

        // ODS style formula: oooc:=A1+B1
        let res = FormulaEvaluator::eval_row("oooc:=[.A1]+[.B1]", &row).unwrap();
        assert_eq!(res.as_float(), Some(15.0));
    }
}
