#[cfg(test)]
mod tests;

use crate::db::adapter::QueryResult;
use anyhow::Result;
use std::path::Path;

pub fn to_csv(result: &QueryResult) -> Result<String> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(&result.columns)?;
    for row in &result.rows {
        wtr.write_record(row)?;
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
                    let val = parse_json_value(v);
                    (k.clone(), val)
                })
                .collect();
            serde_json::Value::Object(obj)
        })
        .collect();

    Ok(serde_json::to_string_pretty(&records)?)
}

pub fn export_to_file(result: &QueryResult, path: &Path, format: ExportFormat) -> Result<()> {
    let content = match format {
        ExportFormat::Csv => to_csv(result)?,
        ExportFormat::Json => to_json(result)?,
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
}

impl ExportFormat {
    pub fn extension(&self) -> &str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
        }
    }

    #[allow(dead_code)]
    pub fn label(&self) -> &str {
        match self {
            ExportFormat::Csv => "CSV",
            ExportFormat::Json => "JSON",
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
