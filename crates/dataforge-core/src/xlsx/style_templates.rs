// =============================================================================
// DataForge Core — XLSX Styling Templates
// =============================================================================
// Provides pre-built styling templates for common spreadsheet output formats.
//
// Usage:
//   let config = WriterConfig::default()
//       .with_style_template(StyleTemplate::Professional);
//
// Templates control:
//   - Header row background color and font weight
//   - Alternating row shading for readability
//   - Column auto-width hints
//   - Number/date format strings
// =============================================================================

use serde::{Deserialize, Serialize};

/// Pre-built styling template for XLSX output.
///
/// These templates define visual formatting applied during XLSX writing.
/// Each template produces professional-looking spreadsheets without
/// requiring manual cell-by-cell styling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StyleTemplate {
    /// No special styling (raw data output).
    None,

    /// Clean professional look: bold white-on-navy header, alternating light-grey rows.
    Professional,

    /// Financial report style: right-aligned numbers, currency formatting,
    /// dark header with gold accents.
    Financial,

    /// Dashboard/analytics style: compact fonts, color-coded header bands,
    /// subtle gridlines.
    Dashboard,

    /// Custom template with user-specified colors and formats.
    Custom(CustomStyle),
}

impl Default for StyleTemplate {
    fn default() -> Self {
        StyleTemplate::None
    }
}

/// User-customizable styling parameters for XLSX output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomStyle {
    /// Header row background color as hex (e.g., "4472C4").
    pub header_bg_color: String,

    /// Header row font color as hex (e.g., "FFFFFF").
    pub header_font_color: String,

    /// Whether header text is bold.
    pub header_bold: bool,

    /// Alternating row background color (even rows). Empty = no shading.
    pub alt_row_bg_color: String,

    /// Default number format string (e.g., "#,##0.00").
    pub number_format: String,

    /// Default date format string (e.g., "yyyy-mm-dd").
    pub date_format: String,

    /// Font family name (e.g., "Calibri").
    pub font_family: String,

    /// Font size in points (e.g., 11).
    pub font_size: u8,

    /// Whether to freeze the header row (first row stays visible on scroll).
    pub freeze_header: bool,

    /// Whether to enable auto-filter on the header row.
    pub auto_filter: bool,
}

impl Default for CustomStyle {
    fn default() -> Self {
        CustomStyle {
            header_bg_color: "4472C4".to_string(),
            header_font_color: "FFFFFF".to_string(),
            header_bold: true,
            alt_row_bg_color: "D9E2F3".to_string(),
            number_format: "#,##0.00".to_string(),
            date_format: "yyyy-mm-dd".to_string(),
            font_family: "Calibri".to_string(),
            font_size: 11,
            freeze_header: true,
            auto_filter: true,
        }
    }
}

impl StyleTemplate {
    /// Resolve this template into a concrete `CustomStyle`.
    ///
    /// Built-in templates return pre-defined color/format values.
    /// `Custom(s)` returns the user-provided style directly.
    pub fn resolve(&self) -> CustomStyle {
        match self {
            StyleTemplate::None => CustomStyle {
                header_bg_color: String::new(),
                header_font_color: String::new(),
                header_bold: false,
                alt_row_bg_color: String::new(),
                number_format: String::new(),
                date_format: String::new(),
                font_family: "Calibri".to_string(),
                font_size: 11,
                freeze_header: false,
                auto_filter: false,
            },
            StyleTemplate::Professional => CustomStyle {
                header_bg_color: "1F4E79".to_string(),
                header_font_color: "FFFFFF".to_string(),
                header_bold: true,
                alt_row_bg_color: "DAEEF3".to_string(),
                number_format: "#,##0.00".to_string(),
                date_format: "yyyy-mm-dd".to_string(),
                font_family: "Calibri".to_string(),
                font_size: 11,
                freeze_header: true,
                auto_filter: true,
            },
            StyleTemplate::Financial => CustomStyle {
                header_bg_color: "2C3E50".to_string(),
                header_font_color: "F1C40F".to_string(),
                header_bold: true,
                alt_row_bg_color: "ECF0F1".to_string(),
                number_format: "#,##0.00".to_string(),
                date_format: "yyyy-mm-dd".to_string(),
                font_family: "Arial".to_string(),
                font_size: 10,
                freeze_header: true,
                auto_filter: true,
            },
            StyleTemplate::Dashboard => CustomStyle {
                header_bg_color: "34495E".to_string(),
                header_font_color: "ECDBBA".to_string(),
                header_bold: true,
                alt_row_bg_color: "F7F9FC".to_string(),
                number_format: "#,##0".to_string(),
                date_format: "dd/mm/yyyy".to_string(),
                font_family: "Segoe UI".to_string(),
                font_size: 9,
                freeze_header: true,
                auto_filter: false,
            },
            StyleTemplate::Custom(s) => s.clone(),
        }
    }

    /// Generate the `<styleSheet>` XML snippet for this template's resolved style.
    ///
    /// This produces the minimal `xl/styles.xml` content needed to apply
    /// the header and alternating-row fills/fonts.
    pub fn to_styles_xml(&self) -> String {
        let style = self.resolve();

        if style.header_bg_color.is_empty() {
            // No styling — return a minimal styles.xml with date format support
            return r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="2">
    <numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm:ss"/>
    <numFmt numFmtId="165" formatCode="hh:mm:ss"/>
  </numFmts>
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="3">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>
    <xf numFmtId="165" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>
  </cellXfs>
</styleSheet>"#.to_string();
        }

        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="2">
    <numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm:ss"/>
    <numFmt numFmtId="165" formatCode="hh:mm:ss"/>
  </numFmts>
  <fonts count="2">
    <font><sz val="{font_size}"/><name val="{font_family}"/></font>
    <font><b/><sz val="{font_size}"/><color rgb="FF{header_font}"/><name val="{font_family}"/></font>
  </fonts>
  <fills count="4">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FF{header_bg}"/></patternFill></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FF{alt_row_bg}"/></patternFill></fill>
  </fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="5">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>
    <xf numFmtId="165" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>
    <xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFont="1" applyFill="1"/>
    <xf numFmtId="0" fontId="0" fillId="3" borderId="0" xfId="0" applyFill="1"/>
  </cellXfs>
</styleSheet>"#,
            font_size = style.font_size,
            font_family = style.font_family,
            header_font = style.header_font_color,
            header_bg = style.header_bg_color,
            alt_row_bg = style.alt_row_bg_color,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_none() {
        let s = StyleTemplate::None.resolve();
        assert!(s.header_bg_color.is_empty());
        assert!(!s.header_bold);
    }

    #[test]
    fn test_resolve_professional() {
        let s = StyleTemplate::Professional.resolve();
        assert_eq!(s.header_bg_color, "1F4E79");
        assert!(s.header_bold);
        assert!(s.freeze_header);
    }

    #[test]
    fn test_styles_xml_none() {
        let xml = StyleTemplate::None.to_styles_xml();
        assert!(xml.contains("<fonts count=\"1\">"));
    }

    #[test]
    fn test_styles_xml_professional() {
        let xml = StyleTemplate::Professional.to_styles_xml();
        assert!(xml.contains("FF1F4E79"));
        assert!(xml.contains("FFFFFFFF"));
        assert!(xml.contains("<b/>"));
    }

    #[test]
    fn test_custom_style() {
        let custom = CustomStyle {
            header_bg_color: "FF0000".to_string(),
            ..CustomStyle::default()
        };
        let template = StyleTemplate::Custom(custom);
        let s = template.resolve();
        assert_eq!(s.header_bg_color, "FF0000");
    }
}
