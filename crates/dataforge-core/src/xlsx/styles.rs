// =============================================================================
// DataForge Core — XLSX Styles Parser
// =============================================================================
// Parses xl/styles.xml to extract number format information.
//
// The critical use case is date detection: Excel stores dates as serial
// numbers (days since 1900-01-01), and the only way to know whether a
// numeric cell is a date or a regular number is by checking its format
// in the styles table.
//
// Built-in format IDs (no need to look them up in styles.xml):
//   14: "mm-dd-yy"
//   15: "d-mmm-yy"
//   16: "d-mmm"
//   17: "mmm-yy"
//   18: "h:mm AM/PM"
//   19: "h:mm:ss AM/PM"
//   20: "h:mm"
//   21: "h:mm:ss"
//   22: "m/d/yy h:mm"
//   45: "mm:ss"
//   46: "[h]:mm:ss"
//   47: "mm:ss.0"
// =============================================================================

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use tracing::debug;

use crate::error::{DataForgeError, Result};

/// Parsed styles information from xl/styles.xml.
///
/// We only extract what's needed for correct data type detection:
/// - Number format codes (to identify date/time formats)
/// - Cell format → number format mapping (cellXfs)
#[derive(Debug, Clone)]
pub struct Styles {
    /// Map from number format ID to format string
    /// e.g., 164 → "yyyy-mm-dd", 165 → "#,##0.00"
    number_formats: HashMap<u32, String>,

    /// For each cell format (cellXfs index), the associated number format ID.
    /// The cellXfs index is what cells reference via their `s` attribute.
    cell_format_to_num_format: Vec<u32>,
}

/// Set of built-in Excel number format IDs that represent date/time formats.
/// These are hardcoded in Excel and don't appear in xl/styles.xml.
const BUILTIN_DATE_FORMAT_IDS: &[u32] = &[
    14, 15, 16, 17, 18, 19, 20, 21, 22, 45, 46, 47,
    // East Asian date formats
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
    // Additional date/time formats
    50, 51, 52, 53, 54, 55, 56, 57, 58,
];

impl Styles {
    /// Create an empty styles table (used when styles.xml is not present).
    pub fn new() -> Self {
        Styles {
            number_formats: HashMap::new(),
            cell_format_to_num_format: Vec::new(),
        }
    }

    /// Parse styles from XML bytes.
    ///
    /// We extract:
    /// 1. `<numFmts>` → custom number format definitions
    /// 2. `<cellXfs>` → cell format entries (maps style index → number format ID)
    pub fn parse(xml_data: &[u8]) -> Result<Self> {
        let mut reader = XmlReader::from_reader(xml_data);
        reader.config_mut().trim_text(true);

        let mut number_formats: HashMap<u32, String> = HashMap::new();
        let mut cell_format_to_num_format: Vec<u32> = Vec::new();
        let mut buf = Vec::with_capacity(1024);
        let mut in_num_fmts = false;
        let mut in_cell_xfs = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    match e.local_name().as_ref() {
                        b"numFmts" => {
                            in_num_fmts = true;
                        }
                        b"cellXfs" => {
                            in_cell_xfs = true;
                        }
                        b"numFmt" if in_num_fmts => {
                            // Parse custom number format: <numFmt numFmtId="164" formatCode="yyyy-mm-dd"/>
                            let mut fmt_id: Option<u32> = None;
                            let mut fmt_code: Option<String> = None;

                            for attr in e.attributes() {
                                let attr = attr.map_err(|e| DataForgeError::XlsxParse {
                                    component: "styles".to_string(),
                                    message: format!("Failed to parse numFmt attribute: {e}"),
                                })?;
                                match attr.key.as_ref() {
                                    b"numFmtId" => {
                                        let val = attr.unescape_value().map_err(|e| {
                                            DataForgeError::XlsxParse {
                                                component: "styles".to_string(),
                                                message: format!("Invalid numFmtId: {e}"),
                                            }
                                        })?;
                                        fmt_id = val.parse().ok();
                                    }
                                    b"formatCode" => {
                                        let val = attr.unescape_value().map_err(|e| {
                                            DataForgeError::XlsxParse {
                                                component: "styles".to_string(),
                                                message: format!("Invalid formatCode: {e}"),
                                            }
                                        })?;
                                        fmt_code = Some(val.into_owned());
                                    }
                                    _ => {}
                                }
                            }

                            if let (Some(id), Some(code)) = (fmt_id, fmt_code) {
                                number_formats.insert(id, code);
                            }
                        }
                        b"xf" if in_cell_xfs => {
                            // Parse cell format entry: <xf numFmtId="14" .../>
                            let mut num_fmt_id: u32 = 0;

                            for attr in e.attributes() {
                                let attr = attr.map_err(|e| DataForgeError::XlsxParse {
                                    component: "styles".to_string(),
                                    message: format!("Failed to parse xf attribute: {e}"),
                                })?;
                                if attr.key.as_ref() == b"numFmtId" {
                                    let val = attr.unescape_value().map_err(|e| {
                                        DataForgeError::XlsxParse {
                                            component: "styles".to_string(),
                                            message: format!("Invalid numFmtId: {e}"),
                                        }
                                    })?;
                                    num_fmt_id = val.parse().unwrap_or(0);
                                }
                            }

                            cell_format_to_num_format.push(num_fmt_id);
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    match e.local_name().as_ref() {
                        b"numFmts" => in_num_fmts = false,
                        b"cellXfs" => in_cell_xfs = false,
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(DataForgeError::XlsxParse {
                        component: "styles".to_string(),
                        message: format!("XML parse error: {e}"),
                    });
                }
                _ => {}
            }
            buf.clear();
        }

        debug!(
            num_formats = number_formats.len(),
            cell_formats = cell_format_to_num_format.len(),
            "Parsed styles table"
        );

        Ok(Styles {
            number_formats,
            cell_format_to_num_format,
        })
    }

    /// Check if a cell style index corresponds to a date/time format.
    ///
    /// This is the critical function for date detection. Excel stores dates
    /// as floating-point serial numbers, and we need the style information
    /// to know whether to interpret a number as a date.
    ///
    /// # Arguments
    /// * `style_index` - The `s` attribute value from a cell element
    pub fn is_date_format(&self, style_index: u32) -> bool {
        // Look up the number format ID for this style index
        let num_fmt_id = self
            .cell_format_to_num_format
            .get(style_index as usize)
            .copied()
            .unwrap_or(0);

        // Check built-in date format IDs
        if BUILTIN_DATE_FORMAT_IDS.contains(&num_fmt_id) {
            return true;
        }

        // Check custom format codes for date-related tokens
        if let Some(format_code) = self.number_formats.get(&num_fmt_id) {
            return is_date_format_string(format_code);
        }

        false
    }

    /// Get the number format string for a style index.
    pub fn get_format_string(&self, style_index: u32) -> Option<&str> {
        let num_fmt_id = self
            .cell_format_to_num_format
            .get(style_index as usize)
            .copied()?;
        self.number_formats.get(&num_fmt_id).map(|s| s.as_str())
    }
}

impl Default for Styles {
    fn default() -> Self {
        Self::new()
    }
}

/// Determine if a custom format string represents a date/time format.
///
/// Heuristic: a format is a date if it contains date/time tokens
/// (y, m, d, h, s) that are NOT inside quoted strings and NOT part
/// of number formatting (like "0.00").
///
/// Examples of date formats: "yyyy-mm-dd", "dd/mm/yyyy hh:mm:ss"
/// Examples of non-date formats: "#,##0.00", "0%", "General"
fn is_date_format_string(format: &str) -> bool {
    let lower = format.to_lowercase();

    // Skip if it's clearly a number format
    if lower == "general" || lower == "0" || lower == "#,##0" {
        return false;
    }

    // Remove quoted strings (text in double quotes is literal)
    let mut cleaned = String::with_capacity(lower.len());
    let mut in_quotes = false;
    for ch in lower.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            cleaned.push(ch);
        }
    }

    // Remove color codes like [Red], [Green], etc.
    let cleaned = remove_bracket_contents(&cleaned);

    // Check for date/time tokens (locale-aware for English, German, French, Spanish, Italian)
    let has_date_tokens = cleaned.contains('y')  // year (English)
        || cleaned.contains('d')                   // day (English)
        || cleaned.contains('j')                   // year/day (German: Jahr/Tag, French: jour)
        || cleaned.contains('t')                   // day (German: Tag)
        || cleaned.contains('a')                   // year (French: année, Spanish: año)
        || (cleaned.contains('m') && !cleaned.contains('#')); // month (not number format)

    let has_time_tokens = cleaned.contains('h')  // hour
        || cleaned.contains('s');                  // second

    has_date_tokens || has_time_tokens
}

/// Remove content within square brackets from a format string.
/// These are typically color codes like [Red], [Blue], or conditional formatting.
fn remove_bracket_contents(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_bracket = false;
    for ch in s.chars() {
        match ch {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            _ if !in_bracket => result.push(ch),
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_date_format_string() {
        // Date formats
        assert!(is_date_format_string("yyyy-mm-dd"));
        assert!(is_date_format_string("dd/mm/yyyy"));
        assert!(is_date_format_string("m/d/yy h:mm"));
        assert!(is_date_format_string("hh:mm:ss"));
        assert!(is_date_format_string("yyyy-mm-dd hh:mm:ss"));
        assert!(is_date_format_string("jjjj-mm-tt")); // German
        assert!(is_date_format_string("aaaa-mm-jj")); // French / Spanish

        // Non-date formats
        assert!(!is_date_format_string("General"));
        assert!(!is_date_format_string("0"));
        assert!(!is_date_format_string("#,##0"));
        assert!(!is_date_format_string("#,##0.00"));
        assert!(!is_date_format_string("0%"));
    }

    #[test]
    fn test_quoted_strings_ignored() {
        // "d" inside quotes should not be treated as a date token
        assert!(!is_date_format_string("#,##0 \"days\""));
    }

    #[test]
    fn test_parse_styles() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <numFmts count="1">
                <numFmt numFmtId="164" formatCode="yyyy-mm-dd"/>
            </numFmts>
            <cellXfs count="2">
                <xf numFmtId="0"/>
                <xf numFmtId="164"/>
            </cellXfs>
        </styleSheet>"#;

        let styles = Styles::parse(xml).unwrap();

        // Style index 0 → numFmtId 0 (General) → not a date
        assert!(!styles.is_date_format(0));

        // Style index 1 → numFmtId 164 (yyyy-mm-dd) → is a date
        assert!(styles.is_date_format(1));
    }

    #[test]
    fn test_builtin_date_formats() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <cellXfs count="2">
                <xf numFmtId="0"/>
                <xf numFmtId="14"/>
            </cellXfs>
        </styleSheet>"#;

        let styles = Styles::parse(xml).unwrap();

        // numFmtId 14 is a built-in date format (mm-dd-yy)
        assert!(styles.is_date_format(1));
    }

    #[test]
    fn test_empty_styles() {
        let styles = Styles::new();
        assert!(!styles.is_date_format(0));
        assert!(styles.get_format_string(0).is_none());
    }
}
