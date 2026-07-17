// =============================================================================
// DataForge Core — PII Masking and Anonymization Transformation Stage
// =============================================================================
// Allows masking or redacting personal/sensitive information on row streams.
// =============================================================================

use sha2::{Digest, Sha256};
use crate::types::{CellValue, RowBatch};

/// Masking Strategy to apply to sensitive columns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MaskingStrategy {
    /// Completely redact character strings with standard asterisks.
    Redact,
    /// Anonymize data by replacing it with a SHA-256 hash string.
    Hash,
    /// Keep prefix and suffix characters visible, masking the middle.
    Partial {
        /// Number of characters to preserve at the start of the string.
        keep_left: usize,
        /// Number of characters to preserve at the end of the string.
        keep_right: usize,
        /// Character used for masking (e.g. '*').
        mask_char: char,
    },
}

/// Applies a masking strategy to a target column inside a RowBatch.
pub fn mask_column(batch: &mut RowBatch, col_idx: usize, strategy: &MaskingStrategy) {
    for row in &mut batch.rows {
        if let Some(cell) = row.cells.get_mut(col_idx) {
            match cell {
                CellValue::String(ref s) => {
                    let masked_str = apply_mask(s, strategy);
                    *cell = CellValue::String(masked_str.into());
                }
                CellValue::Int(val) => {
                    match strategy {
                        MaskingStrategy::Hash => {
                            let masked_str = apply_mask(&val.to_string(), strategy);
                            *cell = CellValue::String(masked_str.into());
                        }
                        _ => {
                            *cell = CellValue::Int(0);
                        }
                    }
                }
                CellValue::Float(val) => {
                    match strategy {
                        MaskingStrategy::Hash => {
                            let masked_str = apply_mask(&val.to_string(), strategy);
                            *cell = CellValue::String(masked_str.into());
                        }
                        _ => {
                            *cell = CellValue::Float(0.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn apply_mask(input: &str, strategy: &MaskingStrategy) -> String {
    match strategy {
        MaskingStrategy::Redact => {
            "*".repeat(input.len())
        }
        MaskingStrategy::Hash => {
            let mut hasher = Sha256::new();
            hasher.update(input.as_bytes());
            let result = hasher.finalize();
            result.iter().map(|b| format!("{b:02x}")).collect::<String>()
        }
        MaskingStrategy::Partial { keep_left, keep_right, mask_char } => {
            let chars: Vec<char> = input.chars().collect();
            if chars.len() <= keep_left + keep_right {
                return input.to_string();
            }
            let mut out = String::with_capacity(input.len());
            for (idx, &c) in chars.iter().enumerate() {
                if idx < *keep_left || idx >= chars.len() - *keep_right {
                    out.push(c);
                } else {
                    out.push(*mask_char);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_mask() {
        let redact = MaskingStrategy::Redact;
        assert_eq!(apply_mask("secret", &redact), "******");

        let hash = MaskingStrategy::Hash;
        assert_eq!(
            apply_mask("hello", &hash),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        let partial = MaskingStrategy::Partial {
            keep_left: 2,
            keep_right: 2,
            mask_char: 'X',
        };
        assert_eq!(apply_mask("sensitive", &partial), "seXXXXXve");
    }
}
