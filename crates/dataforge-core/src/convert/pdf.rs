// =============================================================================
// DataForge Core — HTML & PDF Report Generator
// =============================================================================
// Formats tabular data into clean, responsive HTML reports suitable for PDF printing.
// =============================================================================

use crate::error::Result;
use crate::types::RowBatch;

pub struct PdfReportGenerator {
    title: String,
    dark_mode: bool,
}

impl PdfReportGenerator {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            dark_mode: false,
        }
    }

    pub fn with_dark_mode(mut self, dark_mode: bool) -> Self {
        self.dark_mode = dark_mode;
        self
    }

    /// Render RowBatch into a complete, standalone HTML report string.
    pub fn render_html(&self, batch: &RowBatch) -> Result<String> {
        let mut html = String::with_capacity(1024 + batch.rows.len() * 128);

        let bg_color = if self.dark_mode { "#0f172a" } else { "#ffffff" };
        let text_color = if self.dark_mode { "#f8fafc" } else { "#1e293b" };
        let card_bg = if self.dark_mode { "#1e293b" } else { "#f8fafc" };
        let border_color = if self.dark_mode { "#334155" } else { "#e2e8f0" };
        let header_bg = if self.dark_mode { "#3b82f6" } else { "#2563eb" };

        html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
        html.push_str("<meta charset=\"UTF-8\">\n");
        html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
        html.push_str(&format!("<title>{}</title>\n", self.title));
        html.push_str("<style>\n");
        html.push_str(&format!(
            "body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background-color: {}; color: {}; margin: 0; padding: 24px; }}\n",
            bg_color, text_color
        ));
        html.push_str(&format!(
            ".report-container {{ max-width: 1200px; margin: 0 auto; background: {}; border-radius: 12px; padding: 24px; box-shadow: 0 4px 6px -1px rgba(0,0,0,0.1); border: 1px solid {}; }}\n",
            card_bg, border_color
        ));
        html.push_str("h1 { font-size: 24px; font-weight: 700; margin-top: 0; margin-bottom: 16px; }\n");
        html.push_str("table { width: 100%; border-collapse: collapse; margin-top: 16px; font-size: 14px; }\n");
        html.push_str(&format!(
            "th {{ background-color: {}; color: #ffffff; font-weight: 600; text-align: left; padding: 12px 16px; border: 1px solid {}; }}\n",
            header_bg, border_color
        ));
        html.push_str(&format!(
            "td {{ padding: 10px 16px; border: 1px solid {}; }}\n",
            border_color
        ));
        html.push_str(&format!(
            "tr:nth-child(even) {{ background-color: {}; }}\n",
            if self.dark_mode { "#141e33" } else { "#f1f5f9" }
        ));
        html.push_str("@media print { body { padding: 0; background: white; color: black; } .report-container { border: none; box-shadow: none; } }\n");
        html.push_str("</style>\n</head>\n<body>\n");

        html.push_str("<div class=\"report-container\">\n");
        html.push_str(&format!("<h1>{}</h1>\n", self.title));

        html.push_str("<table>\n<thead>\n<tr>\n");
        for field in &batch.schema.fields {
            html.push_str(&format!("<th>{}</th>", field.name));
        }
        html.push_str("\n</tr>\n</thead>\n<tbody>\n");

        for row in &batch.rows {
            html.push_str("<tr>\n");
            for cell in &row.cells {
                html.push_str(&format!("<td>{}</td>", cell));
            }
            html.push_str("\n</tr>\n");
        }

        html.push_str("</tbody>\n</table>\n");
        html.push_str("</div>\n</body>\n</html>");

        Ok(html)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellValue, Field, DataType, Row, Schema};
    use compact_str::CompactString;

    #[test]
    fn test_pdf_report_generator() {
        let batch = RowBatch {
            schema: Schema {
                fields: vec![
                    Field { name: "ID".into(), data_type: DataType::Int },
                    Field { name: "Name".into(), data_type: DataType::String },
                ],
            },
            rows: vec![
                Row {
                    cells: vec![
                        CellValue::Int(101),
                        CellValue::String(CompactString::new("Widget A")),
                    ],
                },
            ],
        };

        let generator = PdfReportGenerator::new("Sales Executive Summary").with_dark_mode(true);
        let html = generator.render_html(&batch).unwrap();

        assert!(html.contains("Sales Executive Summary"));
        assert!(html.contains("<th>ID</th>"));
        assert!(html.contains("<td>Widget A</td>"));
    }
}
