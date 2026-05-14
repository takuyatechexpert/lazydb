#[cfg(test)]
mod tests;

use crate::db::adapter::QueryResult;
use anyhow::Result;
use std::path::Path;
use unicode_width::UnicodeWidthStr;

/// テーブル形式出力での NULL 表示。TUI の結果表示と揃える。
const TABLE_NULL_DISPLAY: &str = "NULL";

pub fn to_csv(result: &QueryResult) -> Result<String> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(&result.columns)?;
    for row in &result.rows {
        // CSV には NULL リテラルが無いため、NULL は空フィールドとして書き出す
        let cells: Vec<&str> = row.iter().map(|c| c.as_deref().unwrap_or("")).collect();
        wtr.write_record(&cells)?;
    }
    let bytes = wtr.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

pub fn to_json(result: &QueryResult) -> Result<String> {
    let records: Vec<serde_json::Value> = result
        .rows
        .iter()
        .map(|row| {
            let obj: serde_json::Map<String, serde_json::Value> = result
                .columns
                .iter()
                .zip(row.iter())
                .map(|(k, v)| {
                    // SQL の NULL は JSON の null として出力する。
                    // 値があれば従来通り bool/数値/文字列を推測する。
                    let val = match v {
                        None => serde_json::Value::Null,
                        Some(s) => parse_json_value(s),
                    };
                    (k.clone(), val)
                })
                .collect();
            serde_json::Value::Object(obj)
        })
        .collect();

    Ok(serde_json::to_string_pretty(&records)?)
}

/// TUI の結果テーブル表示と同じレイアウトの文字列を生成する。
///
/// クリップボード貼り付け時に「ターミナルで見えていた通り」を再現するため、
/// 列幅は `unicode-width` ベースで計算し、区切り文字（` │ ` / `─` / `┼`）も
/// 結果ペインの `render_table` と揃える。NULL は `NULL` リテラルで出力する。
pub fn to_table(result: &QueryResult) -> String {
    if result.columns.is_empty() {
        return String::new();
    }

    let col_widths = compute_col_widths(result);

    let mut out = String::new();

    // ヘッダー行
    let header_cells: Vec<String> = result
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| pad_right(col, col_widths[i]))
        .collect();
    out.push(' ');
    out.push_str(&header_cells.join(" │ "));
    out.push('\n');

    // 区切り線（各セルの両側 1 文字分のスペースぶんを足して `─` を伸ばす）
    let sep_cells: Vec<String> = col_widths.iter().map(|w| "─".repeat(w + 2)).collect();
    out.push_str(&sep_cells.join("┼"));
    out.push('\n');

    // データ行
    for row in &result.rows {
        out.push(' ');
        let cells: Vec<String> = (0..result.columns.len())
            .map(|j| {
                let width = col_widths[j];
                match row.get(j).and_then(|c| c.as_deref()) {
                    Some(s) => pad_right(s, width),
                    None => pad_right(TABLE_NULL_DISPLAY, width),
                }
            })
            .collect();
        out.push_str(&cells.join(" │ "));
        out.push('\n');
    }

    out
}

fn compute_col_widths(result: &QueryResult) -> Vec<usize> {
    let mut widths: Vec<usize> = result
        .columns
        .iter()
        .map(|c| UnicodeWidthStr::width(c.as_str()))
        .collect();
    for row in &result.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                let s = cell.as_deref().unwrap_or(TABLE_NULL_DISPLAY);
                widths[i] = widths[i].max(UnicodeWidthStr::width(s));
            }
        }
    }
    widths
}

fn pad_right(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

pub fn export_to_file(result: &QueryResult, path: &Path, format: ExportFormat) -> Result<()> {
    let content = match format {
        ExportFormat::Csv => to_csv(result)?,
        ExportFormat::Json => to_json(result)?,
        ExportFormat::Clipboard => {
            // Clipboard はファイル出力ではないため、ここでは到達しないことを期待。
            // 呼び出し側で分岐すべきだが、保険として Table 文字列を書き出す。
            to_table(result)
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportFormat {
    Csv,
    Json,
    /// OS クリップボードへ、TUI 表示と同じ表形式でコピーする出力先。
    Clipboard,
}

impl ExportFormat {
    pub fn extension(&self) -> &str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            // ファイル拡張子としては使われないが、フォールバックとして txt を返す
            ExportFormat::Clipboard => "txt",
        }
    }

    #[allow(dead_code)]
    pub fn label(&self) -> &str {
        match self {
            ExportFormat::Csv => "CSV",
            ExportFormat::Json => "JSON",
            ExportFormat::Clipboard => "Clipboard",
        }
    }
}

fn parse_json_value(v: &str) -> serde_json::Value {
    match v {
        "t" | "true" => serde_json::Value::Bool(true),
        "f" | "false" => serde_json::Value::Bool(false),
        "" => serde_json::Value::Null,
        _ => {
            if let Ok(n) = v.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(n) = v.parse::<f64>() {
                serde_json::json!(n)
            } else {
                serde_json::Value::String(v.to_string())
            }
        }
    }
}
