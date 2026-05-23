use clap::{Args, Subcommand};
use std::collections::{HashMap, HashSet};

use crate::schema::{SemanticElement, SemanticSchema};

#[derive(Args)]
pub struct RegionsArgs {
    #[command(subcommand)]
    pub command: RegionsCommand,
}

#[derive(Subcommand)]
pub enum RegionsCommand {
    /// Auto-discover regions in a semantic YAML
    Discover(DiscoverArgs),
    /// Match regions across two YAMLs (cross-platform or design-vs-device)
    Match(MatchArgs),
}

#[derive(Args)]
pub struct DiscoverArgs {
    pub schema: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct MatchArgs {
    pub source: String,
    pub target: String,
    #[arg(long)]
    pub json: bool,
}

struct Region {
    id: String,
    description: String,
    elements: Vec<String>,
    bounds: (i32, i32, i32, i32),
    confidence: f64,
}

struct RegionDef {
    id: &'static str,
    description: &'static str,
    required: Vec<TokenPattern>,
    min_required: usize,
}

struct TokenPattern {
    types: Vec<&'static str>,
    content_fuzzy: Vec<&'static str>,
    count: usize,
}

impl TokenPattern {
    fn single(types: &[&'static str], content: &[&'static str]) -> Self {
        Self {
            types: types.to_vec(),
            content_fuzzy: content.to_vec(),
            count: 1,
        }
    }
    fn counted(types: &[&'static str], content: &[&'static str], n: usize) -> Self {
        Self {
            types: types.to_vec(),
            content_fuzzy: content.to_vec(),
            count: n,
        }
    }
}

fn region_defs() -> Vec<RegionDef> {
    vec![
        RegionDef {
            id: "action_bar",
            description: "Primary site action buttons",
            required: vec![
                TokenPattern::single(&["button", "image", "text"], &["save"]),
                TokenPattern::single(&["button", "image", "text"], &["directions"]),
                TokenPattern::single(&["button", "image", "text"], &["share"]),
            ],
            min_required: 3,
        },
        RegionDef {
            id: "rating_row",
            description: "Star rating display",
            required: vec![
                TokenPattern::counted(
                    &["image", "container"],
                    &["star", "icon_solid_star", "icon_outline_star"],
                    3,
                ),
            ],
            min_required: 1,
        },
        RegionDef {
            id: "reviews_section",
            description: "Reviews header with rating",
            required: vec![
                TokenPattern::single(&["text", "button"], &["reviews"]),
                TokenPattern::single(&["text", "button"], &["rate this", "rate_this"]),
            ],
            min_required: 2,
        },
        RegionDef {
            id: "qa_section",
            description: "Questions and answers section",
            required: vec![
                TokenPattern::single(&["text", "button"], &["questions", "questions & answers"]),
                TokenPattern::single(
                    &["button", "text"],
                    &["post question", "post_question", "ask a question"],
                ),
            ],
            min_required: 2,
        },
        RegionDef {
            id: "map_cta",
            description: "See on map button",
            required: vec![TokenPattern::single(
                &["button", "text"],
                &["see on map", "see_on_map"],
            )],
            min_required: 1,
        },
        RegionDef {
            id: "social_counts",
            description: "Been here / Want to go counts",
            required: vec![
                TokenPattern::single(
                    &["text", "button"],
                    &["been here", "been_here", "i've been here"],
                ),
            ],
            min_required: 1,
        },
        RegionDef {
            id: "categories_section",
            description: "Categories list",
            required: vec![TokenPattern::single(
                &["text", "button"],
                &["categories"],
            )],
            min_required: 1,
        },
        RegionDef {
            id: "contact_section",
            description: "Contact information",
            required: vec![TokenPattern::single(
                &["text", "button"],
                &["contact us", "contact_us"],
            )],
            min_required: 1,
        },
    ]
}

fn normalize_content(s: &str) -> String {
    s.to_lowercase()
        .replace("i've ", "")
        .replace("i've ", "")
        .replace('\u{2019}', "'")
        .trim()
        .to_string()
}

fn content_matches(content: &str, patterns: &[&str]) -> bool {
    let norm = normalize_content(content);
    patterns
        .iter()
        .any(|p| norm.contains(&p.to_lowercase()) || p.to_lowercase().contains(&norm))
}

fn token_matches(elem: &SemanticElement, pattern: &TokenPattern) -> bool {
    if !pattern.types.contains(&elem.elem_type.as_str()) {
        return false;
    }
    let text = elem
        .content
        .as_deref()
        .or(elem.a11y_label.as_deref())
        .unwrap_or(&elem.id);
    content_matches(text, &pattern.content_fuzzy)
}

fn discover_regions(schema: &SemanticSchema) -> Vec<Region> {
    let defs = region_defs();
    let elements = &schema.elements;
    let mut regions = Vec::new();

    for def in &defs {
        let mut matched_elements: Vec<&SemanticElement> = Vec::new();
        let mut matched_count = 0;

        for pattern in &def.required {
            let matches: Vec<&SemanticElement> = elements
                .iter()
                .filter(|e| token_matches(e, pattern))
                .collect();
            if matches.len() >= pattern.count {
                matched_count += 1;
                matched_elements.extend(matches.iter().take(pattern.count));
            }
        }

        if matched_count >= def.min_required {
            let elem_ids: Vec<String> = matched_elements.iter().map(|e| e.id.clone()).collect();

            let min_x = matched_elements
                .iter()
                .map(|e| e.bounds.x)
                .min()
                .unwrap_or(0);
            let min_y = matched_elements
                .iter()
                .map(|e| e.bounds.y)
                .min()
                .unwrap_or(0);
            let max_r = matched_elements
                .iter()
                .map(|e| e.bounds.x + e.bounds.w)
                .max()
                .unwrap_or(0);
            let max_b = matched_elements
                .iter()
                .map(|e| e.bounds.y + e.bounds.h)
                .max()
                .unwrap_or(0);

            let confidence =
                matched_count as f64 / def.required.len().max(1) as f64;

            regions.push(Region {
                id: def.id.to_string(),
                description: def.description.to_string(),
                elements: elem_ids,
                bounds: (min_x, min_y, max_r - min_x, max_b - min_y),
                confidence,
            });
        }
    }

    regions
}

pub fn run(args: RegionsArgs) -> Result<(), String> {
    match args.command {
        RegionsCommand::Discover(d) => run_discover(d),
        RegionsCommand::Match(m) => run_match(m),
    }
}

fn run_discover(args: DiscoverArgs) -> Result<(), String> {
    let content = std::fs::read_to_string(&args.schema).map_err(|e| format!("read: {e}"))?;
    let schema: SemanticSchema =
        serde_yaml::from_str(&content).map_err(|e| format!("parse: {e}"))?;

    let regions = discover_regions(&schema);

    if args.json {
        let items: Vec<serde_json::Value> = regions
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "description": r.description,
                    "elements": r.elements,
                    "bounds": {"x": r.bounds.0, "y": r.bounds.1, "w": r.bounds.2, "h": r.bounds.3},
                    "confidence": r.confidence,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&items).unwrap_or_default()
        );
        return Ok(());
    }

    println!(
        "Discovered {} regions in {} ({}):\n",
        regions.len(),
        schema.screen,
        schema.platform
    );
    for r in &regions {
        println!(
            "  {} — {} (conf {:.0}%)",
            r.id,
            r.description,
            r.confidence * 100.0
        );
        println!(
            "    bounds: ({},{}) {}x{}",
            r.bounds.0, r.bounds.1, r.bounds.2, r.bounds.3
        );
        println!("    elements: {}", r.elements.join(", "));
        println!();
    }

    Ok(())
}

fn run_match(args: MatchArgs) -> Result<(), String> {
    let src_content =
        std::fs::read_to_string(&args.source).map_err(|e| format!("read source: {e}"))?;
    let src: SemanticSchema =
        serde_yaml::from_str(&src_content).map_err(|e| format!("parse source: {e}"))?;
    let tgt_content =
        std::fs::read_to_string(&args.target).map_err(|e| format!("read target: {e}"))?;
    let tgt: SemanticSchema =
        serde_yaml::from_str(&tgt_content).map_err(|e| format!("parse target: {e}"))?;

    let src_regions = discover_regions(&src);
    let tgt_regions = discover_regions(&tgt);

    let src_ids: HashSet<&str> = src_regions.iter().map(|r| r.id.as_str()).collect();
    let tgt_ids: HashSet<&str> = tgt_regions.iter().map(|r| r.id.as_str()).collect();

    let matched: Vec<&str> = src_ids.intersection(&tgt_ids).copied().collect();
    let src_only: Vec<&str> = src_ids.difference(&tgt_ids).copied().collect();
    let tgt_only: Vec<&str> = tgt_ids.difference(&src_ids).copied().collect();

    if args.json {
        let report = serde_json::json!({
            "source": {"platform": src.platform, "screen": src.screen, "regions": src_regions.len()},
            "target": {"platform": tgt.platform, "screen": tgt.screen, "regions": tgt_regions.len()},
            "matched": matched,
            "source_only": src_only,
            "target_only": tgt_only,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
        return Ok(());
    }

    println!(
        "Region match: {} ({}) vs {} ({})\n",
        src.platform, src.screen, tgt.platform, tgt.screen
    );
    println!(
        "  {} matched, {} source-only, {} target-only\n",
        matched.len(),
        src_only.len(),
        tgt_only.len()
    );

    if !matched.is_empty() {
        println!("MATCHED REGIONS:");
        for id in &matched {
            let sr = src_regions.iter().find(|r| r.id == *id).unwrap();
            let tr = tgt_regions.iter().find(|r| r.id == *id).unwrap();

            let dy = (sr.bounds.1 - tr.bounds.1).abs();
            let dh = (sr.bounds.3 - tr.bounds.3).abs();
            let drift = if dy > 4 || dh > 4 {
                format!(" (drift: {}dp y, {}dp h)", dy, dh)
            } else {
                String::new()
            };

            println!(
                "  {} — {} vs {} elements{}",
                id,
                sr.elements.len(),
                tr.elements.len(),
                drift
            );
        }
        println!();
    }

    if !src_only.is_empty() {
        println!(
            "SOURCE ONLY (in {} but not {}):",
            src.platform, tgt.platform
        );
        for id in &src_only {
            println!("  {}", id);
        }
        println!();
    }

    if !tgt_only.is_empty() {
        println!(
            "TARGET ONLY (in {} but not {}):",
            tgt.platform, src.platform
        );
        for id in &tgt_only {
            println!("  {}", id);
        }
    }

    Ok(())
}
