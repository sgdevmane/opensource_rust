// =============================================================================
// DataForge Core — CSV Sniffer
// =============================================================================
// Heuristic-based CSV dialect auto-detection.
// Analyzes samples to detect delimiter, quote character, and header row.
// =============================================================================

use crate::error::{DataForgeError, Result};

/// Auto-detected CSV dialect properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SniffedDialect {
    /// Delimiter character (e.g. `,`, `;`, `\t`)
    pub delimiter: u8,
    /// Quote character (usually `"`)
    pub quote_char: u8,
    /// Whether the file has a header row
    pub has_header: bool,
}

/// Sniffer utility to auto-detect CSV properties from a slice of bytes.
pub struct CsvSniffer;

impl CsvSniffer {
    /// Sniff the CSV dialect properties from a raw byte sample.
    pub fn sniff(data: &[u8]) -> Result<SniffedDialect> {
        if data.is_empty() {
            return Err(DataForgeError::config("Cannot sniff empty CSV data"));
        }

        // 1. Detect delimiter
        let candidates = [b',', b';', b'\t', b'|'];
        let mut best_delim = b',';
        let mut max_consistency = 0.0;

        let content = String::from_utf8_lossy(data);
        let lines: Vec<&str> = content.lines().take(10).collect();

        for &delim in &candidates {
            let delim_char = delim as char;
            let counts: Vec<usize> = lines.iter()
                .map(|line| line.chars().filter(|&c| c == delim_char).count())
                .collect();

            if counts.is_empty() || counts.iter().all(|&c| c == 0) {
                continue;
            }

            let avg = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
            let variance: f64 = counts.iter()
                .map(|&c| (c as f64 - avg).powi(2))
                .sum::<f64>() / counts.len() as f64;

            let score = if avg > 0.0 {
                avg / (1.0 + variance)
            } else {
                0.0
            };

            if score > max_consistency {
                max_consistency = score;
                best_delim = delim;
            }
        }

        // 2. Detect quote character (default to `"` or `'` if prevalent)
        let mut quote_char = b'"';
        let single_quotes = content.chars().filter(|&c| c == '\'').count();
        let double_quotes = content.chars().filter(|&c| c == '"').count();
        if single_quotes > double_quotes {
            quote_char = b'\'';
        }

        // 3. Detect header by comparing type votes
        let has_header = detect_header(&lines, best_delim);

        Ok(SniffedDialect {
            delimiter: best_delim,
            quote_char,
            has_header,
        })
    }
}

fn detect_header(lines: &[&str], delimiter: u8) -> bool {
    if lines.len() < 2 {
        return false;
    }

    let first_fields: Vec<&str> = lines[0]
        .split(delimiter as char)
        .map(|s| s.trim())
        .collect();
    
    let mut num_numeric_first = 0;
    for field in &first_fields {
        if field.parse::<i64>().is_ok() || field.parse::<f64>().is_ok() {
            num_numeric_first += 1;
        }
    }

    let mut num_numeric_others = 0;
    let mut other_fields_count = 0;
    for &line in lines.iter().skip(1) {
        let fields: Vec<&str> = line
            .split(delimiter as char)
            .map(|s| s.trim())
            .collect();
        for field in &fields {
            if field.parse::<i64>().is_ok() || field.parse::<f64>().is_ok() {
                num_numeric_others += 1;
            }
        }
        other_fields_count += fields.len();
    }

    if num_numeric_first == 0 && other_fields_count > 0 && num_numeric_others > 0 {
        true
    } else {
        let all_strings = first_fields.iter().all(|s| s.chars().all(|c| !c.is_numeric()));
        all_strings && lines.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sniff_comma_separated() {
        let csv = b"name,age,city\nAlice,30,New York\nBob,25,Los Angeles\n";
        let dialect = CsvSniffer::sniff(csv).unwrap();
        assert_eq!(dialect.delimiter, b',');
        assert_eq!(dialect.quote_char, b'"');
        assert!(dialect.has_header);
    }

    #[test]
    fn test_sniff_semicolon_separated() {
        let csv = b"name;age;city\nAlice;30;New York\nBob;25;Los Angeles\n";
        let dialect = CsvSniffer::sniff(csv).unwrap();
        assert_eq!(dialect.delimiter, b';');
        assert!(dialect.has_header);
    }

    #[test]
    fn test_sniff_tabs() {
        let csv = b"name\tage\tcity\nAlice\t30\tNew York\nBob\t25\tLos Angeles\n";
        let dialect = CsvSniffer::sniff(csv).unwrap();
        assert_eq!(dialect.delimiter, b'\t');
        assert!(dialect.has_header);
    }
}
