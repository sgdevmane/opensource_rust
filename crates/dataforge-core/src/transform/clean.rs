// =============================================================================
// DataForge Core — Data Cleaning & Anonymization Pipeline
// =============================================================================
// High-performance string cleaning, normalization, and PII anonymization.
// =============================================================================

use crate::types::{CellValue, RowBatch};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Strategy for cleaning and anonymizing cell data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CleanStrategy {
    /// Trim leading and trailing whitespace.
    TrimWhitespace,
    /// Collapse multiple consecutive whitespace characters into a single space.
    CollapseWhitespace,
    /// Convert text to UPPERCASE.
    Uppercase,
    /// Convert text to lowercase.
    Lowercase,
    /// Mask email addresses (e.g. `j***n@domain.com`).
    MaskEmail,
    /// Mask phone numbers (e.g. `***-***-1234`).
    MaskPhone,
    /// Mask Social Security Numbers (e.g. `***-**-6789`).
    MaskSsn,
    /// Mask Credit Card numbers (e.g. `****-****-****-1234`).
    MaskCreditCard,
    /// Custom Regex Replacement pattern.
    RegexReplace { pattern: String, replace_with: String },
}

/// Cleaner for tabular row batches.
pub struct DataCleaner {
    column_strategies: Vec<(usize, CleanStrategy)>,
}

impl DataCleaner {
    pub fn new() -> Self {
        DataCleaner {
            column_strategies: Vec::new(),
        }
    }

    pub fn add_rule(mut self, col_idx: usize, strategy: CleanStrategy) -> Self {
        self.column_strategies.push((col_idx, strategy));
        self
    }

    pub fn clean_batch(&self, batch: &mut RowBatch) {
        let email_regex = Regex::new(r"(?i)^([a-z0-9._%+-]+)@([a-z0-9.-]+\.[a-z]{2,})$").ok();
        let phone_regex = Regex::new(r"^\+?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}$").ok();
        let ssn_regex = Regex::new(r"^\d{3}-\d{2}-\d{4}$").ok();
        let cc_regex = Regex::new(r"^\d{4}[-.\s]?\d{4}[-.\s]?\d{4}[-.\s]?\d{4}$").ok();

        for row in &mut batch.rows {
            for (col_idx, strategy) in &self.column_strategies {
                if let Some(cell) = row.cells.get_mut(*col_idx) {
                    if let CellValue::String(ref mut text) = cell {
                        let mut s = text.to_string();
                        match strategy {
                            CleanStrategy::TrimWhitespace => {
                                s = s.trim().to_string();
                            }
                            CleanStrategy::CollapseWhitespace => {
                                let ws_regex = Regex::new(r"\s+").unwrap();
                                s = ws_regex.replace_all(s.trim(), " ").to_string();
                            }
                            CleanStrategy::Uppercase => {
                                s = s.to_uppercase();
                            }
                            CleanStrategy::Lowercase => {
                                s = s.to_lowercase();
                            }
                            CleanStrategy::MaskEmail => {
                                if let Some(ref re) = email_regex {
                                    if let Some(caps) = re.captures(&s) {
                                        let user = &caps[1];
                                        let domain = &caps[2];
                                        if user.len() <= 2 {
                                            s = format!("*@{}", domain);
                                        } else {
                                            let first = &user[..1];
                                            let last = &user[user.len() - 1..];
                                            s = format!("{}***{}@{}", first, last, domain);
                                        }
                                    } else if s.contains('@') {
                                        let parts: Vec<&str> = s.split('@').collect();
                                        if parts.len() == 2 {
                                            s = format!("***@{}", parts[1]);
                                        }
                                    }
                                }
                            }
                            CleanStrategy::MaskPhone => {
                                if let Some(ref re) = phone_regex {
                                    if re.is_match(&s) {
                                        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
                                        if digits.len() >= 4 {
                                            let last4 = &digits[digits.len() - 4..];
                                            s = format!("***-***-{}", last4);
                                        }
                                    }
                                }
                            }
                            CleanStrategy::MaskSsn => {
                                if let Some(ref re) = ssn_regex {
                                    if re.is_match(&s) {
                                        let last4 = &s[s.len() - 4..];
                                        s = format!("***-**-{}", last4);
                                    }
                                }
                            }
                            CleanStrategy::MaskCreditCard => {
                                if let Some(ref re) = cc_regex {
                                    if re.is_match(&s) {
                                        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
                                        if digits.len() >= 4 {
                                            let last4 = &digits[digits.len() - 4..];
                                            s = format!("****-****-****-{}", last4);
                                        }
                                    }
                                }
                            }
                            CleanStrategy::RegexReplace { pattern, replace_with } => {
                                if let Ok(re) = Regex::new(pattern) {
                                    s = re.replace_all(&s, replace_with.as_str()).to_string();
                                }
                            }
                        }
                        *cell = CellValue::String(s.into());
                    }
                }
            }
        }
    }
}

impl Default for DataCleaner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Row;
    use compact_str::CompactString;

    #[test]
    fn test_data_cleaner() {
        let mut batch = RowBatch {
            schema: crate::types::Schema { fields: vec![] },
            rows: vec![
                Row {
                    cells: vec![
                        CellValue::String(CompactString::new("  john.doe@example.com  ")),
                        CellValue::String(CompactString::new("123-456-7890")),
                    ],
                },
            ],
        };

        let cleaner = DataCleaner::new()
            .add_rule(0, CleanStrategy::TrimWhitespace)
            .add_rule(0, CleanStrategy::MaskEmail)
            .add_rule(1, CleanStrategy::MaskPhone);

        cleaner.clean_batch(&mut batch);

        assert_eq!(
            batch.rows[0].cells[0].as_str().unwrap(),
            "j***e@example.com"
        );
        assert_eq!(
            batch.rows[0].cells[1].as_str().unwrap(),
            "***-***-7890"
        );
    }
}
