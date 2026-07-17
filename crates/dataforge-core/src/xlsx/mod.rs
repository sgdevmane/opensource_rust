// =============================================================================
// DataForge Core — XLSX Module
// =============================================================================
// Streaming XLSX (Excel 2007+) reader and writer.
//
// XLSX files are ZIP archives containing XML files:
//   - xl/worksheets/sheet*.xml — Cell data (the big one)
//   - xl/sharedStrings.xml — Deduplicated string table
//   - xl/styles.xml — Number formats, fonts, etc.
//   - xl/workbook.xml — Sheet names and metadata
//   - [Content_Types].xml — MIME types
//   - _rels/.rels — Relationships
//
// Our streaming approach:
// 1. Open the ZIP, read shared strings and styles FIRST (they're small)
// 2. Stream the worksheet XML using SAX-style parsing (quick-xml events)
// 3. Resolve cell values lazily using the pre-loaded shared strings
// 4. Emit RowBatch values as rows are parsed — never hold full DOM
// =============================================================================

pub mod reader;
pub mod shared_strings;
pub mod styles;
pub mod writer;
pub mod decrypt;
pub mod encrypt;
pub mod style_templates;

pub use reader::XlsxReader;
pub use writer::XlsxWriter;
pub use decrypt::XlsxDecrypter;
pub use encrypt::encrypt_xlsx;
pub use style_templates::StyleTemplate;
