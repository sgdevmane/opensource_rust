// =============================================================================
// DataForge Core — ODS Module (OpenDocument Spreadsheet)
// =============================================================================
// Streaming reader and writer for LibreOffice/OpenOffice ODS files.
//
// ODS files are ZIP archives containing XML (like XLSX), but using the
// OpenDocument Format (ODF) namespace. The primary content file is
// `content.xml` which contains all sheet data.
// =============================================================================

pub mod reader;
pub mod writer;

pub use reader::OdsReader;
pub use writer::OdsWriter;
