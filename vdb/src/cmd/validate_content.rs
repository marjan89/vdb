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

    /// Exclude a11y elements inside external render surfaces (maps, webviews)
    #[arg(long)]
    pub exclude_external: bool,

    /// Exclude a11y elements beyond viewport bounds
    #[arg(long)]
    pub exclude_offscreen: bool,
}

struct A11yElement {
    text: String,
    class: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

pub fn run(args: ValidateContentArgs) -> Result<(), String> {
    let schema_str =
        std::fs::read_to_string(&args.schema).map_err(|e| format!("read schema: {e}"))?;
    let schema: SemanticSchema =
        serde_yaml::from_str(&schema_str).map_err(|e| format!("parse schema: {e}"))?;

    let dump_str =
        std::fs::read_to_string(&args.a11y_dump).map_err(|e| format!("read a11y dump: {e}"))?;

    let mut a11y_elements = if dump_str.trim_start().starts_with('<') {
        parse_uiautomator_xml(&dump_str)?
    } else {
        parse_wda_json(&dump_str)?
    };

    let viewport_w = schema.viewport.as_ref().map(|v| v.width).unwrap_or(
        schema.elements.first().map(|e| e.bounds.w).unwrap_or(400),
    );
    let viewport_h = schema.viewport.as_ref().map(|v| v.height).unwrap_or(
        schema.elements.first().map(|e| e.bounds.h).unwrap_or(900),
    );
    let density = schema
        .viewport
        .as_ref()
        .filter(|v| v.density > 0.0)
        .map(|v| v.density)
        .unwrap_or(3.0);

    if args.exclude_offscreen {
        let max_px_x = (viewport_w as f64 * density) as i32;
        let max_px_y = (viewport_h as f64 * density) as i32;
        a11y_elements.retain(|e| e.x >= 0 && e.y >= 0 && e.x < max_px_x && e.y < max_px_y);
    }

    if args.exclude_external {
        let external_rects: Vec<_> = schema
            .elements
            .iter()
            .filter(|e| e.render.as_deref() == Some("external"))
            .map(|e| {
                let b = &e.bounds;
                (
                    (b.x as f64 * density) as i32,
                    (b.y as f64 * density) as i32,
                    ((b.x + b.w) as f64 * density) as i32,
                    ((b.y + b.h) as f64 * density) as i32,
                )
            })
            .collect();

        let external_classes = [
            "Map", "MapView", "MKMapView", "XCUIElementTypeMap",
            "WebView", "WKWebView", "XCUIElementTypeWebView",
        ];
        a11y_elements.retain(|e| {
            let is_external_class = external_classes.iter().any(|c| e.class.contains(c));
            let is_map_child = e.text == "Map pin" || e.text == "Legal"
                || e.text == "Map Marker" || e.text == "Google Map";
            let in_external_rect = external_rects.iter().any(|(rx, ry, rr, rb)| {
                e.x >= *rx && e.y >= *ry && e.x + e.w <= *rr && e.y + e.h <= *rb
            });
            !is_external_class && !is_map_child && !in_external_rect
        });
    }

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
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim()
        .to_lowercase()
}

fn parse_uiautomator_xml(xml: &str) -> Result<Vec<A11yElement>, String> {
    let mut elements = Vec::new();
    let mut pos = 0;
    let bytes = xml.as_bytes();

    while pos < bytes.len() {
        let next_node = xml[pos..].find("<node ");
        let next_xcui = xml[pos..].find("<XCUIElementType");
        let start = match (next_node, next_xcui) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        if let Some(start) = start {
            let abs_start = pos + start;
            let node_end = xml[abs_start..].find('>').map(|e| abs_start + e);
            let Some(end) = node_end else {
                pos = abs_start + 1;
                continue;
            };
            let node_str = &xml[abs_start..=end];

            let text = extract_attr(node_str, "text").unwrap_or_default();
            let content_desc = extract_attr(node_str, "content-desc").unwrap_or_default();
            let label_attr = extract_attr(node_str, "label").unwrap_or_default();
            let value_attr = extract_attr(node_str, "value").unwrap_or_default();
            let class = extract_attr(node_str, "class")
                .or_else(|| extract_attr(node_str, "type"))
                .unwrap_or_default();
            let bounds = extract_attr(node_str, "bounds").unwrap_or_default();

            let label = if !text.is_empty() {
                text
            } else if !label_attr.is_empty() {
                label_attr
            } else if !content_desc.is_empty() {
                content_desc
            } else if !value_attr.is_empty() {
                value_attr
            } else {
                pos = end + 1;
                continue;
            };

            let (x, y, w, h) = parse_bounds_from_node(node_str);
            elements.push(A11yElement {
                text: label,
                class,
                x,
                y,
                w,
                h,
            });
            pos = end + 1;
        } else {
            break;
        }
    }

    Ok(elements)
}

fn extract_attr(node: &str, attr: &str) -> Option<String> {
    let pattern = format!(" {}=\"", attr);
    let start = node.find(&pattern)? + pattern.len();
    let rest = &node[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_bounds_from_node(node: &str) -> (i32, i32, i32, i32) {
    // WDA format: x="10" y="20" width="100" height="50"
    if let (Some(x), Some(y), Some(w), Some(h)) = (
        extract_attr(node, "x").and_then(|v| v.parse::<i32>().ok()),
        extract_attr(node, "y").and_then(|v| v.parse::<i32>().ok()),
        extract_attr(node, "width").and_then(|v| v.parse::<i32>().ok()),
        extract_attr(node, "height").and_then(|v| v.parse::<i32>().ok()),
    ) {
        return (x, y, w, h);
    }
    // uiautomator format: bounds="[x1,y1][x2,y2]"
    if let Some(bounds) = extract_attr(node, "bounds") {
        let nums: Vec<i32> = bounds
            .split(|c: char| !c.is_ascii_digit() && c != '-')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() >= 4 {
            return (nums[0], nums[1], nums[2] - nums[0], nums[3] - nums[1]);
        }
    }
    (0, 0, 0, 0)
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
            let x = obj.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = obj.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let w = obj.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let h = obj.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            out.push(A11yElement {
                text: label,
                class: elem_type,
                x,
                y,
                w,
                h,
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
