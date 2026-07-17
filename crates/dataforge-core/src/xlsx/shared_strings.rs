// =============================================================================
// DataForge Core — XLSX Shared Strings Table
// =============================================================================
// XLSX files use a shared string table (xl/sharedStrings.xml) to deduplicate
// string values across cells. Instead of storing "Hello" in every cell that
// contains it, the cell stores an index (e.g., 0) and the shared strings
// table maps index 0 → "Hello".
//
// This module parses the shared strings table using SAX-style XML parsing
// for memory efficiency. The table is typically small relative to the
// worksheet data and is loaded fully into memory.
// =============================================================================

use compact_str::CompactString;
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use tracing::debug;

use crate::error::{DataForgeError, Result};

/// In-memory shared string table parsed from xl/sharedStrings.xml.
///
/// This table is indexed by position (0-based) and maps to string values.
/// XLSX cells of type "s" (shared string) store the index rather than
/// the actual string content.
///
/// # Memory
/// Uses `CompactString` for small-string optimization. Strings ≤ 24 bytes
/// are stored inline without heap allocation, which is effective since
/// many shared strings are short (column headers, enum values, etc.).
#[derive(Debug, Clone)]
pub struct SharedStrings {
    /// The string values, indexed by their position in the XML
    strings: Vec<CompactString>,
}

impl SharedStrings {
    /// Create an empty shared strings table.
    pub fn new() -> Self {
        SharedStrings {
            strings: Vec::new(),
        }
    }

    /// Parse the shared strings table from XML bytes.
    ///
    /// The XML structure is:
    /// ```xml
    /// <sst count="5" uniqueCount="3">
    ///   <si><t>Hello</t></si>
    ///   <si><t>World</t></si>
    ///   <si><r><t>Rich </t></r><r><t>Text</t></r></si>
    /// </sst>
    /// ```
    ///
    /// Note: Rich text elements (`<r>`) can split a single string across
    /// multiple `<t>` tags. We concatenate them.
    ///
    /// # Arguments
    /// * `xml_data` - Raw XML bytes from xl/sharedStrings.xml
    pub fn parse(xml_data: &[u8]) -> Result<Self> {
        let mut reader = XmlReader::from_reader(xml_data);
        reader.config_mut().trim_text(false);

        let mut strings = Vec::new();
        let mut buf = Vec::with_capacity(1024);
        let mut current_string = String::new();
        let mut in_si = false; // Inside <si> element
        let mut in_t = false;  // Inside <t> element

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    match e.local_name().as_ref() {
                        b"si" => {
                            // Start of a new shared string entry
                            in_si = true;
                            current_string.clear();
                        }
                        b"t" if in_si => {
                            // Start of text content
                            in_t = true;
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    match e.local_name().as_ref() {
                        b"si" => {
                            // End of shared string entry — store it
                            strings.push(CompactString::new(&current_string));
                            in_si = false;
                            in_t = false;
                        }
                        b"t" => {
                            in_t = false;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) if in_t => {
                    // Text content inside <t> element
                    let text = e.unescape().map_err(|err| DataForgeError::XlsxParse {
                        component: "shared_strings".to_string(),
                        message: format!("Failed to unescape text: {err}"),
                    })?;
                    current_string.push_str(&text);
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(DataForgeError::XlsxParse {
                        component: "shared_strings".to_string(),
                        message: format!("XML parse error: {e}"),
                    });
                }
                _ => {} // Ignore other events (comments, PI, etc.)
            }
            buf.clear();
        }

        debug!(count = strings.len(), "Parsed shared strings table");

        Ok(SharedStrings { strings })
    }

    /// Look up a shared string by its 0-based index.
    ///
    /// Returns `None` if the index is out of bounds.
    pub fn get(&self, index: usize) -> Option<&str> {
        self.strings.get(index).map(|s| s.as_str())
    }

    /// Get the total number of shared strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Check if the shared strings table is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

impl Default for SharedStrings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_shared_strings() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="3" uniqueCount="3">
            <si><t>Hello</t></si>
            <si><t>World</t></si>
            <si><t>Test</t></si>
        </sst>"#;

        let sst = SharedStrings::parse(xml).unwrap();
        assert_eq!(sst.len(), 3);
        assert_eq!(sst.get(0), Some("Hello"));
        assert_eq!(sst.get(1), Some("World"));
        assert_eq!(sst.get(2), Some("Test"));
        assert_eq!(sst.get(3), None);
    }

    #[test]
    fn test_parse_rich_text() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <si><r><t>Rich </t></r><r><t>Text</t></r></si>
        </sst>"#;

        let sst = SharedStrings::parse(xml).unwrap();
        assert_eq!(sst.len(), 1);
        assert_eq!(sst.get(0), Some("Rich Text"));
    }

    #[test]
    fn test_empty_shared_strings() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="0" uniqueCount="0">
        </sst>"#;

        let sst = SharedStrings::parse(xml).unwrap();
        assert!(sst.is_empty());
    }

    #[test]
    fn test_empty_string_value() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <si><t></t></si>
            <si><t>non-empty</t></si>
        </sst>"#;

        let sst = SharedStrings::parse(xml).unwrap();
        assert_eq!(sst.len(), 2);
        assert_eq!(sst.get(0), Some(""));
        assert_eq!(sst.get(1), Some("non-empty"));
    }
}
