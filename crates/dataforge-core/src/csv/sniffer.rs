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
    /// Escape character (typically `\`)
    pub escape_char: Option<u8>,
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
            let counts: Vec<usize> = lines.iter()
                .map(|line| simd_count_occ(line.as_bytes(), delim))
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

        // 2. Detect quote character (using SIMD)
        let single_quotes = simd_count_occ(data, b'\'');
        let double_quotes = simd_count_occ(data, b'"');
        let mut quote_char = b'"';
        if single_quotes > double_quotes {
            quote_char = b'\'';
        }

        // 3. Detect escape character (using SIMD)
        let backslashes = simd_count_occ(data, b'\\');
        let escape_char = if backslashes > 0 {
            Some(b'\\')
        } else {
            None
        };

        // 4. Detect header by comparing type votes
        let has_header = detect_header(&lines, best_delim);

        Ok(SniffedDialect {
            delimiter: best_delim,
            quote_char,
            escape_char,
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

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn simd_count_occ(data: &[u8], delim: u8) -> usize {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    if is_x86_feature_detected!("sse2") {
        let mut count = 0;
        unsafe {
            let delim_vec = _mm_set1_epi8(delim as i8);
            let chunks = data.chunks_exact(16);
            let rem = chunks.remainder();
            for chunk in chunks {
                let chunk_vec = _mm_loadu_si128(chunk.as_ptr() as *const __m128i);
                let eq = _mm_cmpeq_epi8(chunk_vec, delim_vec);
                let mask = _mm_movemask_epi8(eq);
                count += mask.count_ones() as usize;
            }
            count + rem.iter().filter(|&&b| b == delim).count()
        }
    } else {
        data.iter().filter(|&&b| b == delim).count()
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn simd_count_occ(data: &[u8], delim: u8) -> usize {
    use std::arch::wasm32::*;
    let mut count = 0;
    let delim_vec = unsafe { u8x16_splat(delim) };
    let chunks = data.chunks_exact(16);
    let rem = chunks.remainder();
    for chunk in chunks {
        unsafe {
            let chunk_vec = v128_load(chunk.as_ptr() as *const v128);
            let eq = u8x16_eq(chunk_vec, delim_vec);
            let bitmask = u8x16_bitmask(eq);
            count += bitmask.count_ones() as usize;
        }
    }
    count + rem.iter().filter(|&&b| b == delim).count()
}

#[cfg(not(any(
    any(target_arch = "x86", target_arch = "x86_64"),
    all(target_arch = "wasm32", target_feature = "simd128")
)))]
fn simd_count_occ(data: &[u8], delim: u8) -> usize {
    data.iter().filter(|&&b| b == delim).count()
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
        assert_eq!(dialect.escape_char, None);
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

    #[test]
    fn test_sniff_escape_char() {
        let csv = b"name,description\nAlice,likes \\, comma\nBob,no escape\n";
        let dialect = CsvSniffer::sniff(csv).unwrap();
        assert_eq!(dialect.escape_char, Some(b'\\'));
    }
}


