//! The terminal's flat-file layout, written chunk by chunk.
//!
//! A flat file is written the way the terminal writes it: under the column
//! names the service sends, with each cell as the service sent it, text (the
//! root, the right, and every price, which the service sends as decimal text)
//! quoted and integers bare. Each chunk is rendered as soon as it arrives, so
//! only the chunk in hand is held, except in `json_legacy`: its shape is one
//! array per column, so no column can close before the last row arrives, and
//! the file is held until then, as the terminal holds it.

use std::fmt::Write as _;

use crate::error::Error;
use crate::proto::{DataValue, DataValueList, data_value::DataType};

use super::FlatFileFormat;

/// The terminal's HTML page around a table.
pub const HTML_START: &str = "<!DOCTYPE html><html lang=\"en\">\n<head><title>Theta Data</title></head>\n<body>\n<table style=\"width:100%; text-align: center;\">";
/// The end of the terminal's HTML page.
pub const HTML_END: &str = "</tbody></table></body></html>\n";

/// HTML-escape `&`, `<`, `>`, `"` so an attacker-controlled cell (a symbol or
/// condition string) cannot inject markup into the rendered table.
#[must_use]
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// Renders one flat file in the terminal's layout, one chunk at a time.
pub struct FileRenderer {
    format: FlatFileFormat,
    wrote_header: bool,
    wrote_rows: bool,
    /// `json_legacy` only: each column so far, from its key to its last cell.
    columns: Vec<String>,
}

impl FileRenderer {
    /// A renderer for a file in `format`.
    #[must_use]
    pub fn new(format: FlatFileFormat) -> Self {
        Self {
            format,
            wrote_header: false,
            wrote_rows: false,
            columns: Vec::new(),
        }
    }

    /// The text of one chunk of the file, its column names and its rows as
    /// the service sent them. `json_legacy` returns nothing until
    /// [`finish`](Self::finish).
    ///
    /// # Errors
    ///
    /// Fails on a cell of a type the file cannot carry.
    pub fn chunk(&mut self, headers: &[String], rows: &[DataValueList]) -> Result<String, Error> {
        if self.format == FlatFileFormat::JsonLegacy {
            if !self.wrote_header {
                self.columns = headers
                    .iter()
                    .enumerate()
                    .map(|(i, header)| {
                        let mut column = String::from(if i == 0 { "{" } else { "," });
                        push_json_string(&mut column, header);
                        column.push_str(":[");
                        column
                    })
                    .collect();
                self.wrote_header = true;
            }
            for cells in rows {
                for (i, column) in self.columns.iter_mut().enumerate() {
                    if self.wrote_rows {
                        column.push(',');
                    }
                    push_json_cell(column, cells.values.get(i))?;
                }
                self.wrote_rows = true;
            }
            return Ok(String::new());
        }
        let mut out = String::with_capacity(rows.len() * 160);
        if !self.wrote_header {
            prefix(&mut out, self.format, headers);
            self.wrote_header = true;
        }
        for cells in rows {
            row(
                &mut out,
                self.format,
                !self.wrote_rows,
                headers,
                &cells.values,
            )?;
            self.wrote_rows = true;
        }
        Ok(out)
    }

    /// The rest of the file once its last chunk has been rendered, in
    /// pieces: one per column for `json_legacy`, so the file is never copied
    /// whole.
    #[must_use]
    pub fn finish(self) -> Vec<String> {
        if self.format == FlatFileFormat::JsonLegacy {
            let mut tail = self.columns;
            for column in &mut tail {
                column.push(']');
            }
            match tail.last_mut() {
                Some(last) => last.push('}'),
                None => tail.push("{}".to_owned()),
            }
            return tail;
        }
        let mut out = String::new();
        if !self.wrote_header {
            // No chunk arrived: the file is empty.
            prefix(&mut out, self.format, &[]);
        }
        suffix(&mut out, self.format);
        vec![out]
    }
}

fn prefix(out: &mut String, format: FlatFileFormat, headers: &[String]) {
    match format {
        FlatFileFormat::Csv => {
            out.push_str(&headers.join(","));
            out.push('\n');
        }
        FlatFileFormat::Json => out.push_str("{\"response\":["),
        FlatFileFormat::Jsonl | FlatFileFormat::JsonLegacy => {}
        FlatFileFormat::Html => {
            out.push_str(HTML_START);
            out.push_str("<thead><tr>");
            for header in headers {
                let _ = write!(out, "<th>{}</th>", escape_html(header));
            }
            out.push_str("</tr></thead>\n<tbody>");
        }
    }
}

/// The terminal ends a CSV file with one blank line.
fn suffix(out: &mut String, format: FlatFileFormat) {
    match format {
        FlatFileFormat::Csv => out.push('\n'),
        FlatFileFormat::Jsonl | FlatFileFormat::JsonLegacy => {}
        FlatFileFormat::Json => out.push_str("]}"),
        FlatFileFormat::Html => out.push_str(HTML_END),
    }
}

/// One cell as the service sent it. The flat files carry text and integers;
/// any other cell type ends the file in an error rather than being written in
/// a form the terminal never writes.
fn cell(value: &DataValue) -> Result<Option<Cell<'_>>, Error> {
    match value.data_type.as_ref() {
        Some(DataType::Text(text)) => Ok(Some(Cell::Text(text))),
        Some(DataType::Number(n)) => Ok(Some(Cell::Int(*n))),
        Some(DataType::NullValue(_)) => Ok(None),
        other => Err(Error::config_internal(format!(
            "flat-file cell of an unexpected type: {other:?}"
        ))),
    }
}

enum Cell<'a> {
    Text(&'a str),
    Int(i64),
}

fn row(
    out: &mut String,
    format: FlatFileFormat,
    first: bool,
    headers: &[String],
    values: &[DataValue],
) -> Result<(), Error> {
    match format {
        FlatFileFormat::Csv => {
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match cell(value)? {
                    Some(Cell::Text(text)) => {
                        out.push('"');
                        out.push_str(&text.replace('"', "\"\""));
                        out.push('"');
                    }
                    Some(Cell::Int(n)) => {
                        let _ = write!(out, "{n}");
                    }
                    None => {}
                }
            }
            out.push('\n');
        }
        FlatFileFormat::Json | FlatFileFormat::Jsonl => {
            if format == FlatFileFormat::Json && !first {
                out.push(',');
            }
            out.push('{');
            for (i, (header, value)) in headers.iter().zip(values).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_string(out, header);
                out.push(':');
                push_json_cell(out, Some(value))?;
            }
            out.push('}');
            if format == FlatFileFormat::Jsonl {
                out.push('\n');
            }
        }
        FlatFileFormat::Html => {
            out.push_str("<tr>");
            for value in values {
                out.push_str("<td>");
                match cell(value)? {
                    Some(Cell::Text(text)) => out.push_str(&escape_html(text)),
                    Some(Cell::Int(n)) => {
                        let _ = write!(out, "{n}");
                    }
                    None => out.push_str("null"),
                }
                out.push_str("</td>");
            }
            out.push_str("</tr>\n");
        }
        // Written column by column in `FileRenderer::chunk`.
        FlatFileFormat::JsonLegacy => {}
    }
    Ok(())
}

/// One cell as JSON: text as a string, an integer as a number, and a missing
/// or null cell as `null`.
fn push_json_cell(out: &mut String, value: Option<&DataValue>) -> Result<(), Error> {
    match value.map(cell).transpose()?.flatten() {
        Some(Cell::Text(text)) => push_json_string(out, text),
        Some(Cell::Int(n)) => {
            let _ = write!(out, "{n}");
        }
        None => out.push_str("null"),
    }
    Ok(())
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk shaped like the service's: the header line, then rows whose
    /// quoted cells are text (the service sends prices as decimal text) and
    /// whose bare cells are integers.
    fn chunk(file: &str) -> (Vec<String>, Vec<DataValueList>) {
        let cell = |cell: &str| DataValue {
            data_type: Some(
                match cell.strip_prefix('"').and_then(|c| c.strip_suffix('"')) {
                    Some(text) => DataType::Text(text.to_owned()),
                    None => DataType::Number(cell.parse().expect("integer cell")),
                },
            ),
        };
        let mut lines = file.lines();
        let headers = lines
            .next()
            .expect("header line")
            .split(',')
            .map(str::to_owned)
            .collect();
        let rows = lines
            .map(|line| DataValueList {
                values: line.split(',').map(cell).collect(),
            })
            .collect();
        (headers, rows)
    }

    fn written(format: FlatFileFormat, file: &str) -> String {
        let mut renderer = FileRenderer::new(format);
        let (headers, rows) = chunk(file);
        let mut out = renderer
            .chunk(&headers, &rows)
            .expect("the file is written");
        out.extend(renderer.finish());
        out
    }

    /// A flat file is written as the terminal writes it: the terminal's own
    /// rows for 2026-10-21, sent through as the service's cells, come back
    /// byte for byte, the sub-penny `"0.00010"` and the closing blank line
    /// included.
    #[test]
    fn flat_files_are_written_as_the_terminal_writes_them() {
        let stock_eod = "root,ms_of_day,ms_of_day2,open,high,low,close,volume,count,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"HCMA\",62128177,57600350,\"10.3\",\"10.3\",\"10.3\",\"10.3\",112241,101,100,7,\"10.26\",0,100,7,\"10.35\",0,20261021\n\
            \"HCMC\",62128177,57577936,\"0.00010\",\"0.00010\",\"0.0\",\"0.00010\",42627531,109,0,0,\"0.0\",0,0,0,\"0.0\",0,20261021\n";
        assert_eq!(
            written(FlatFileFormat::Csv, stock_eod),
            format!("{stock_eod}\n")
        );

        let option_quote = "root,expiration,strike,right,interval_ms_of_day,quote_ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"ABG\",20270115,\"115.0\",\"C\",34260000,34259636,1,11,\"61.0\",50,1,11,\"65.0\",50,20261021\n";
        assert_eq!(
            written(FlatFileFormat::Csv, option_quote),
            format!("{option_quote}\n")
        );

        // The JSON form carries the same cells: text as strings, integers as
        // numbers, in the `response` envelope.
        assert_eq!(
            written(
                FlatFileFormat::Json,
                "root,bid,date\n\"HCMC\",\"0.00010\",20261021\n"
            ),
            "{\"response\":[{\"root\":\"HCMC\",\"bid\":\"0.00010\",\"date\":20261021}]}"
        );

        // The legacy JSON form is one array per column, the same cells in
        // the same order, row `i` at index `i` of every array.
        assert_eq!(
            written(
                FlatFileFormat::JsonLegacy,
                "root,bid,date\n\"HCMA\",\"10.26\",20261021\n\"HCMC\",\"0.00010\",20261021\n"
            ),
            "{\"root\":[\"HCMA\",\"HCMC\"],\"bid\":[\"10.26\",\"0.00010\"],\"date\":[20261021,20261021]}"
        );

        // The HTML form is the terminal's page, as the regular routes write it.
        assert_eq!(
            written(
                FlatFileFormat::Html,
                "root,bid,date\n\"HCMC\",\"0.00010\",20261021\n"
            ),
            "<!DOCTYPE html><html lang=\"en\">\n<head><title>Theta Data</title></head>\n<body>\n\
             <table style=\"width:100%; text-align: center;\"><thead><tr><th>root</th><th>bid</th><th>date</th></tr></thead>\n\
             <tbody><tr><td>HCMC</td><td>0.00010</td><td>20261021</td></tr>\n\
             </tbody></table></body></html>\n"
        );
    }
}
