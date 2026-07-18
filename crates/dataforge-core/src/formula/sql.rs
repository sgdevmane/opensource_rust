// =============================================================================
// DataForge Core — Inline SQL Query Engine
// =============================================================================
// Lightweight SQL parser and executor over RowBatches.
// =============================================================================

use crate::types::{CellValue, Row, RowBatch};
use crate::error::{DataForgeError, Result};

/// Lightweight parser and evaluator for inline SQL queries.
pub struct SqlEngine {
    select_cols: Vec<String>,
    where_col: Option<String>,
    where_op: Option<String>,
    where_val: Option<CellValue>,
}

impl SqlEngine {
    /// Parse a standard inline SQL query string.
    ///
    /// Supported syntax: `SELECT col1, col2 FROM [table] WHERE col3 > 100`
    pub fn parse(query: &str) -> Result<Self> {
        let tokens: Vec<&str> = query.split_whitespace().collect();
        if tokens.is_empty() {
            return Err(DataForgeError::config("Empty SQL query"));
        }

        if tokens[0].to_uppercase() != "SELECT" {
            return Err(DataForgeError::config("SQL query must start with SELECT"));
        }

        let from_pos = tokens.iter().position(|t| t.to_uppercase() == "FROM");
        let select_slice = match from_pos {
            Some(pos) => &tokens[1..pos],
            None => &tokens[1..],
        };

        let mut select_cols = Vec::new();
        for col in select_slice {
            let clean_col = col.trim_end_matches(',');
            if clean_col != "*" {
                select_cols.push(clean_col.to_string());
            }
        }

        let mut where_col = None;
        let mut where_op = None;
        let mut where_val = None;

        if let Some(pos) = tokens.iter().position(|t| t.to_uppercase() == "WHERE") {
            let where_slice = &tokens[pos + 1..];
            if where_slice.len() >= 3 {
                where_col = Some(where_slice[0].to_string());
                where_op = Some(where_slice[1].to_string());
                
                let val_str = where_slice[2..].join(" ");
                let parsed_val = if let Ok(b) = val_str.parse::<bool>() {
                    CellValue::Bool(b)
                } else if let Ok(i) = val_str.parse::<i64>() {
                    CellValue::Int(i)
                } else if let Ok(f) = val_str.parse::<f64>() {
                    CellValue::Float(f)
                } else {
                    let clean_str = val_str.trim_matches('\'').trim_matches('"');
                    CellValue::String(clean_str.to_string().into())
                };
                where_val = Some(parsed_val);
            }
        }

        Ok(SqlEngine {
            select_cols,
            where_col,
            where_op,
            where_val,
        })
    }

    /// Execute the parsed query on a target RowBatch.
    pub fn execute(&self, batch: &RowBatch) -> Result<RowBatch> {
        let mut out_batch = RowBatch::new(batch.start_index);
        
        let headers = batch.headers.as_ref().ok_or_else(|| {
            DataForgeError::config("Cannot execute SQL query on RowBatch without headers")
        })?;

        let mut select_indices = Vec::new();
        if self.select_cols.is_empty() {
            select_indices = (0..headers.len()).collect();
            out_batch.headers = Some(headers.clone());
        } else {
            let mut out_headers = Vec::new();
            for col_name in &self.select_cols {
                let idx = headers.iter().position(|h| h.eq_ignore_ascii_case(col_name))
                    .ok_or_else(|| {
                        DataForgeError::config(format!("Column not found: {col_name}"))
                    })?;
                select_indices.push(idx);
                out_headers.push(headers[idx].clone());
            }
            out_batch.headers = Some(out_headers);
        }

        let where_col_idx = if let Some(ref col_name) = self.where_col {
            let idx = headers.iter().position(|h| h.eq_ignore_ascii_case(col_name))
                .ok_or_else(|| {
                    DataForgeError::config(format!("WHERE Column not found: {col_name}"))
                })?;
            Some(idx)
        } else {
            None
        };

        for row in &batch.rows {
            if let (Some(col_idx), Some(ref op), Some(ref val)) = (where_col_idx, &self.where_op, &self.where_val) {
                let cell = row.get(col_idx).unwrap_or(&CellValue::Null);
                let matches = match op.as_str() {
                    "=" => cell == val,
                    "!=" => cell != val,
                    ">" => cell_compare(cell, val).is_some_and(|o| o == std::cmp::Ordering::Greater),
                    ">=" => cell_compare(cell, val).is_some_and(|o| o != std::cmp::Ordering::Less),
                    "<" => cell_compare(cell, val).is_some_and(|o| o == std::cmp::Ordering::Less),
                    "<=" => cell_compare(cell, val).is_some_and(|o| o != std::cmp::Ordering::Greater),
                    _ => false,
                };
                if !matches {
                    continue;
                }
            }

            let mut out_row = Row::new(row.index);
            for &idx in &select_indices {
                let cell = row.get(idx).cloned().unwrap_or(CellValue::Null);
                out_row.push(cell);
            }
            out_batch.push(out_row);
        }

        Ok(out_batch)
    }
}

fn cell_compare(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (CellValue::Int(x), CellValue::Int(y)) => x.partial_cmp(y),
        (CellValue::Float(x), CellValue::Float(y)) => x.partial_cmp(y),
        (CellValue::Int(x), CellValue::Float(y)) => (*x as f64).partial_cmp(y),
        (CellValue::Float(x), CellValue::Int(y)) => x.partial_cmp(&(*y as f64)),
        (CellValue::String(x), CellValue::String(y)) => x.partial_cmp(y),
        (CellValue::Bool(x), CellValue::Bool(y)) => x.partial_cmp(y),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_engine_parsing_and_execution() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "age".to_string(), "active".to_string()]);

        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        r1.push(CellValue::from(true));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("Bob"));
        r2.push(CellValue::from(25_i64));
        r2.push(CellValue::from(false));
        batch.push(r2);

        let mut r3 = Row::new(2);
        r3.push(CellValue::from("Charlie"));
        r3.push(CellValue::from(35_i64));
        r3.push(CellValue::from(true));
        batch.push(r3);

        // Select specific columns with condition
        let sql = SqlEngine::parse("SELECT name, age FROM users WHERE age >= 30").unwrap();
        let result = sql.execute(&batch).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.headers.as_ref().unwrap(), &["name", "age"]);
        assert_eq!(result.rows[0].get_str(0), Some("Alice"));
        assert_eq!(result.rows[0].get_int(1), Some(30));
        assert_eq!(result.rows[1].get_str(0), Some("Charlie"));
        assert_eq!(result.rows[1].get_int(1), Some(35));

        // Select all wildcard
        let sql_all = SqlEngine::parse("SELECT * FROM users WHERE active = true").unwrap();
        let result_all = sql_all.execute(&batch).unwrap();
        assert_eq!(result_all.len(), 2);
    }
}
