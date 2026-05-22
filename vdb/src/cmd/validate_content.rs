use clap::Args;
use std::collections::HashSet;

use crate::schema::SemanticSchema;

#[derive(Args)]
pub struct ValidateContentArgs {
    /// Agent semantic YAML
    pub schema: String,
    /// Accessibility dump (uiautomator XML or WDA JSON)
    pub a11y_dump: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

struct A11yElement {
    text: String,
    class: String,
    bounds: String,
}

pub fn run(args: ValidateContentArgs) -> Result<(), String> {
    let schema_str =
        std::fs::read_to_string(&args.schema).map_err(|e| format!("read schema: {e}"))?;
    let schema: SemanticSchema =
        serde_yaml::from_str(&schema_str).map_err(|e| format!("parse schema: {e}"))?;

    let dump_str =
        std::fs::read_to_string(&args.a11y_dump).map_err(|e| format!("read a11y dump: {e}"))?;

    let a11y_elements = if dump_str.trim_start().starts_with('<') {
        parse_uiautomator_xml(&dump_str)?
    } else {
        parse_wda_json(&dump_str)?
    };

    let yaml_texts: HashSet<String> = schema
        .elements
        .iter()
        .filter_map(|e| e.content.as_ref())
        .map(|c| normalize(c))
        .filter(|c| !c.is_empty())
        .collect();

    let a11y_texts: HashSet<String> = a11y_elements
        .iter()
        .map(|e| normalize(&e.text))
        .filter(|t| !t.is_empty())
        .collect();

    let mut matched = Vec::new();
    let mut missing = Vec::new();
    let mut extra = Vec::new();

    for elem in &a11y_elements {
        let norm = normalize(&elem.text);
        if norm.is_empty() {
            continue;
        }
        if yaml_texts.contains(&norm) {
            matched.push(elem);
        } else {
            missing.push(elem);
        }
    }

    for elem in &schema.elements {
        if let Some(ref content) = elem.content {
            let norm = normalize(content);
            if !norm.is_empty() && !a11y_texts.contains(&norm) {
                extra.push(content.as_str());
            }
        }
    }

    if args.json {
        let report = serde_json::json!({
            "matched": matched.len(),
            "missing": missing.iter().map(|e| &e.text).collect::<Vec<_>>(),
            "extra": extra,
            "a11y_total": a11y_elements.iter().filter(|e| !e.text.is_empty()).count(),
            "yaml_total": yaml_texts.len(),
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
        return Ok(());
    }

    println!("vdb validate-content results:\n");
    println!(
        "  a11y elements with text: {}",
        a11y_elements.iter().filter(|e| !e.text.is_empty()).count()
    );
    println!("  yaml elements with content: {}", yaml_texts.len());
    println!("  matched: {}", matched.len());
    println!("  missing from yaml: {}", missing.len());
    println!("  extra in yaml: {}", extra.len());

    if !missing.is_empty() {
        println!("\nMISSING (in a11y dump, not in YAML):");
        for elem in &missing {
            println!("  {:30} ({})", elem.text, elem.class);
        }
    }

    if !extra.is_empty() {
        println!("\nEXTRA (in YAML, not in a11y dump):");
        for text in &extra {
            println!("  {}", text);
        }
    }

    let total_a11y = a11y_elements.iter().filter(|e| !e.text.is_empty()).count();
    let coverage = if total_a11y > 0 {
        matched.len() as f64 / total_a11y as f64 * 100.0
    } else {
        100.0
    };
    println!("\n  COVERAGE: {:.0}% ({}/{})", coverage, matched.len(), total_a11y);

    if !missing.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

fn parse_uiautomator_xml(xml: &str) -> Result<Vec<A11yElement>, String> {
    let mut elements = Vec::new();

    for line in xml.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("text=\"") {
            continue;
        }

        let text = extract_attr(trimmed, "text").unwrap_or_default();
        let content_desc = extract_attr(trimmed, "content-desc").unwrap_or_default();
        let class = extract_attr(trimmed, "class").unwrap_or_default();
        let bounds = extract_attr(trimmed, "bounds").unwrap_or_default();

        let label = if !text.is_empty() {
            text
        } else if !content_desc.is_empty() {
            content_desc
        } else {
            continue;
        };

        elements.push(A11yElement {
            text: label,
            class,
            bounds,
        });
    }

    Ok(elements)
}

fn extract_attr(line: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = line.find(&pattern)? + pattern.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_wda_json(json_str: &str) -> Result<Vec<A11yElement>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("parse WDA JSON: {e}"))?;

    let mut elements = Vec::new();
    collect_wda_elements(&value, &mut elements);
    Ok(elements)
}

fn collect_wda_elements(value: &serde_json::Value, out: &mut Vec<A11yElement>) {
    if let Some(obj) = value.as_object() {
        let label = obj
            .get("label")
            .or_else(|| obj.get("name"))
            .or_else(|| obj.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let elem_type = obj
            .get("type")
            .or_else(|| obj.get("elementType"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !label.is_empty() {
            out.push(A11yElement {
                text: label,
                class: elem_type,
                bounds: String::new(),
            });
        }

        if let Some(children) = obj.get("children").and_then(|c| c.as_array()) {
            for child in children {
                collect_wda_elements(child, out);
            }
        }
    }

    if let Some(arr) = value.as_array() {
        for item in arr {
            collect_wda_elements(item, out);
        }
    }
}
