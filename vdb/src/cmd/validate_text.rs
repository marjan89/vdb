use clap::Args;
use std::process::Command;

use crate::schema::SemanticSchema;

#[derive(Args)]
pub struct ValidateTextArgs {
    /// Device screenshot PNG
    pub screenshot: String,
    /// Semantic schema YAML
    pub schema: String,

    /// Path to ocr-helper.swift
    #[arg(long, default_value = "ocr-helper.swift")]
    pub ocr_helper: String,

    /// Device density (from YAML viewport if available)
    #[arg(long)]
    pub density: Option<f64>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(serde::Deserialize)]
struct OCRResult {
    id: String,
    text: String,
    confidence: f32,
}

pub fn run(args: ValidateTextArgs) -> Result<(), String> {
    let schema_str =
        std::fs::read_to_string(&args.schema).map_err(|e| format!("read schema: {e}"))?;
    let schema: SemanticSchema =
        serde_yaml::from_str(&schema_str).map_err(|e| format!("parse schema: {e}"))?;

    let density = args.density.unwrap_or_else(|| {
        schema
            .viewport
            .as_ref()
            .filter(|v| v.density > 0.0)
            .map(|v| v.density)
            .unwrap_or_else(|| {
                schema
                    .elements
                    .first()
                    .map(|e| e.bounds.w as f64)
                    .filter(|w| *w > 100.0)
                    .map(|_| 3.0)
                    .unwrap_or(3.0)
            })
    });

    let text_elements: Vec<_> = schema
        .elements
        .iter()
        .filter(|e| e.content.is_some())
        .filter(|e| {
            let c = e.content.as_deref().unwrap_or("");
            !c.is_empty() && c.len() >= 2
        })
        .filter(|e| e.bounds.w >= 10 && e.bounds.h >= 8)
        .filter(|e| e.bounds.x >= 0 && e.bounds.y >= 0)
        .collect();

    if text_elements.is_empty() {
        println!("no text elements to validate");
        return Ok(());
    }

    #[derive(serde::Serialize)]
    struct Region {
        id: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    let regions: Vec<Region> = text_elements
        .iter()
        .map(|e| {
            let pad = 2;
            Region {
                id: e.id.clone(),
                x: ((e.bounds.x as f64 * density) as i32 - pad).max(0),
                y: ((e.bounds.y as f64 * density) as i32 - pad).max(0),
                w: (e.bounds.w as f64 * density) as i32 + pad * 2,
                h: (e.bounds.h as f64 * density) as i32 + pad * 2,
            }
        })
        .collect();

    let regions_json =
        serde_json::to_string(&regions).map_err(|e| format!("serialize regions: {e}"))?;

    let output = Command::new("swift")
        .arg(&args.ocr_helper)
        .arg(&args.screenshot)
        .arg("--regions")
        .arg(&regions_json)
        .output()
        .map_err(|e| format!("run ocr-helper: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ocr-helper failed: {stderr}"));
    }

    let ocr_results: Vec<OCRResult> = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("parse ocr results: {e}"))?;

    let mut pass_count = 0;
    let total = text_elements.len();

    println!("vdb validate-text results:\n");

    for elem in &text_elements {
        let expected = elem.content.as_deref().unwrap_or("");
        let expected_norm = normalize(expected);

        let ocr = ocr_results.iter().find(|r| r.id == elem.id);
        let ocr_text = ocr.map(|r| r.text.as_str()).unwrap_or("");
        let ocr_norm = normalize(ocr_text);
        let confidence = ocr.map(|r| r.confidence).unwrap_or(0.0);

        let matches = if expected_norm == ocr_norm {
            true
        } else if expected_norm.len() >= 3
            && (ocr_norm.contains(&expected_norm) || expected_norm.contains(&ocr_norm))
        {
            true
        } else {
            false
        };

        let display_id = if !elem.id.is_empty() {
            &elem.id
        } else {
            expected
        };

        if matches {
            pass_count += 1;
            println!("  {:30} PASS (conf {:.0}%)", display_id, confidence * 100.0);
        } else {
            println!(
                "  {:30} FAIL expected=\"{}\" ocr=\"{}\" (conf {:.0}%)",
                display_id,
                truncate(expected, 30),
                truncate(ocr_text, 30),
                confidence * 100.0
            );
        }
    }

    println!("\n  TOTAL: {}/{} PASS", pass_count, total);

    if pass_count < total {
        std::process::exit(1);
    }

    Ok(())
}

fn normalize(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}
