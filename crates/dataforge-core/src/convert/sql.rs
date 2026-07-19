// =============================================================================
// DataForge Core — Database SQL Connector (Copy/Insert Generator)
// =============================================================================
// Converts RowBatches into PostgreSQL COPY or standard SQL INSERT statements.
// =============================================================================

use crate::types::{CellValue, RowBatch};
use crate::error::{DataForgeError, Result};
use std::fmt::Write;

/// SQL generator for bulk database insertion.
pub struct SqlConnector;

impl SqlConnector {
    /// Generate a multi-row INSERT INTO statement for a given table name.
    ///
    /// Example: `INSERT INTO users (name, age) VALUES ('Alice', 30), ('Bob', 25);`
    pub fn generate_insert(table_name: &str, batch: &RowBatch) -> Result<String> {
        let headers = batch.headers.as_ref().ok_or_else(|| {
            DataForgeError::config("Cannot generate INSERT statement without headers")
        })?;

        if batch.rows.is_empty() {
            return Ok(String::new());
        }

        let mut sql = String::new();
        write!(sql, "INSERT INTO {} (", table_name).unwrap();
        for (i, h) in headers.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(h);
        }
        sql.push_str(") VALUES\n");

        for (r_idx, row) in batch.rows.iter().enumerate() {
            if r_idx > 0 {
                sql.push_str(",\n");
            }
            sql.push('(');
            for (c_idx, cell) in row.cells.iter().enumerate() {
                if c_idx > 0 {
                    sql.push_str(", ");
                }
                match cell {
                    CellValue::Null => sql.push_str("NULL"),
                    CellValue::Bool(b) => sql.push_str(if *b { "TRUE" } else { "FALSE" }),
                    CellValue::Int(i) => write!(sql, "{}", i).unwrap(),
                    CellValue::Float(f) => write!(sql, "{}", f).unwrap(),
                    CellValue::String(s) => {
                        let escaped = s.replace('\'', "''");
                        write!(sql, "'{}'", escaped).unwrap();
                    }
                    CellValue::Bytes(b) => {
                        // Hexadecimal representation for binary data
                        sql.push_str("'\\x");
                        for byte in b {
                            write!(sql, "{:02x}", byte).unwrap();
                        }
                        sql.push('\'');
                    }
                    _ => sql.push_str("NULL"),
                }
            }
            sql.push(')');
        }
        sql.push_str(";\n");

        Ok(sql)
    }

    /// Generate PostgreSQL COPY FROM STDIN command and its associated CSV-formatted stdin payload.
    /// Returns a tuple containing: `(copy_statement, payload)`.
    pub fn generate_postgres_copy(table_name: &str, batch: &RowBatch) -> Result<(String, String)> {
        let headers = batch.headers.as_ref().ok_or_else(|| {
            DataForgeError::config("Cannot generate COPY statement without headers")
        })?;

        let mut copy_stmt = String::new();
        write!(copy_stmt, "COPY {} (", table_name).unwrap();
        for (i, h) in headers.iter().enumerate() {
            if i > 0 {
                copy_stmt.push_str(", ");
            }
            copy_stmt.push_str(h);
        }
        copy_stmt.push_str(") FROM STDIN WITH (FORMAT csv, HEADER false, NULL 'NULL');");

        let mut payload = String::new();
        for row in &batch.rows {
            for (c_idx, cell) in row.cells.iter().enumerate() {
                if c_idx > 0 {
                    payload.push(',');
                }
                match cell {
                    CellValue::Null => payload.push_str("NULL"),
                    CellValue::Bool(b) => payload.push_str(if *b { "true" } else { "false" }),
                    CellValue::Int(i) => write!(payload, "{}", i).unwrap(),
                    CellValue::Float(f) => write!(payload, "{}", f).unwrap(),
                    CellValue::String(s) => {
                        // Double quotes check for CSV escaping
                        let needs_quotes = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
                        if needs_quotes {
                            payload.push('"');
                            payload.push_str(&s.replace('"', "\"\""));
                            payload.push('"');
                        } else {
                            payload.push_str(s);
                        }
                    }
                    _ => payload.push_str("NULL"),
                }
            }
            payload.push('\n');
        }

        Ok((copy_stmt, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Row;

    #[test]
    fn test_sql_connector_insert_copy() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "age".to_string()]);

        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("Bob"));
        r2.push(CellValue::from(25_i64));
        batch.push(r2);

        let insert_sql = SqlConnector::generate_insert("users", &batch).unwrap();
        assert!(insert_sql.contains("INSERT INTO users (name, age) VALUES"));
        assert!(insert_sql.contains("('Alice', 30)"));
        assert!(insert_sql.contains("('Bob', 25)"));

        let (copy_stmt, payload) = SqlConnector::generate_postgres_copy("users", &batch).unwrap();
        assert_eq!(copy_stmt, "COPY users (name, age) FROM STDIN WITH (FORMAT csv, HEADER false, NULL 'NULL');");
        assert_eq!(payload, "Alice,30\nBob,25\n");
    }
}
