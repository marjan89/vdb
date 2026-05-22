use clap::Args;

use crate::schema::{SemanticElement, SemanticSchema};

#[derive(Args)]
pub struct DiffArgs {
    /// Source schema YAML (left / reference)
    pub source: String,
    /// Target schema YAML (right / comparison)
    pub target: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Only compare accessible elements (accessible: true)
    #[arg(long)]
    pub accessible_only: bool,
}

struct Match<'a> {
    src: &'a SemanticElement,
    tgt: &'a SemanticElement,
    #[allow(dead_code)]
    method: &'static str,
}

pub fn run(args: DiffArgs) -> Result<(), String> {
    let src_content =
        std::fs::read_to_string(&args.source).map_err(|e| format!("read {}: {e}", args.source))?;
    let tgt_content =
        std::fs::read_to_string(&args.target).map_err(|e| format!("read {}: {e}", args.target))?;

    let mut src: SemanticSchema =
        serde_yaml::from_str(&src_content).map_err(|e| format!("parse source: {e}"))?;
    let mut tgt: SemanticSchema =
        serde_yaml::from_str(&tgt_content).map_err(|e| format!("parse target: {e}"))?;

    if args.accessible_only {
        src.elements.retain(|e| e.accessible == Some(true));
        tgt.elements.retain(|e| e.accessible == Some(true));
        eprintln!("accessible-only: {} src, {} tgt elements", src.elements.len(), tgt.elements.len());
    }

    let (matches, unmatched_src, unmatched_tgt) = match_elements(&src.elements, &tgt.elements);

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut infos: Vec<String> = Vec::new();

    // Missing elements (in source, not in target)
    for elem in &unmatched_src {
        let label = elem
            .content
            .as_deref()
            .unwrap_or(if elem.id.is_empty() { "(unnamed)" } else { &elem.id });
        let id_part = if elem.id.is_empty() {
            String::new()
        } else {
            format!(" ({}:{})", src.platform, elem.id)
        };
        errors.push(format!(
            "MISSING: \"{}\" {}{} — not found in {}",
            decode(label),
            elem.elem_type,
            id_part,
            tgt.platform
        ));
    }

    // Extra elements (in target, not in source)
    for elem in &unmatched_tgt {
        let label = elem
            .content
            .as_deref()
            .unwrap_or(if elem.id.is_empty() { "(unnamed)" } else { &elem.id });
        let id_part = if elem.id.is_empty() {
            String::new()
        } else {
            format!(" ({}:{})", tgt.platform, elem.id)
        };
        infos.push(format!(
            "EXTRA: \"{}\" {} in {}{} — not in {}",
            decode(label),
            elem.elem_type,
            tgt.platform,
            id_part,
            src.platform
        ));
    }

    // Compare matched pairs
    for m in &matches {
        let id = if !m.src.id.is_empty() {
            &m.src.id
        } else {
            m.src.content.as_deref().unwrap_or("(unnamed)")
        };

        // Text content
        let sc = m.src.content.as_deref().map(decode);
        let tc = m.tgt.content.as_deref().map(decode);
        if sc != tc {
            errors.push(format!(
                "WRONG_TEXT: {} — {}:\"{}\" {}:\"{}\"",
                id,
                src.platform,
                sc.as_deref().unwrap_or("(none)"),
                tgt.platform,
                tc.as_deref().unwrap_or("(none)")
            ));
        }

        // Element type
        if m.src.elem_type != m.tgt.elem_type {
            warnings.push(format!(
                "WRONG_TYPE: {} — {}:{} {}:{}",
                id, src.platform, m.src.elem_type, tgt.platform, m.tgt.elem_type
            ));
        }

        // Color / foreground
        let src_fg = m.src.foreground.as_ref().or(m.src.color.as_ref());
        let tgt_fg = m.tgt.foreground.as_ref().or(m.tgt.color.as_ref());
        if let (Some(sc), Some(tc)) = (src_fg, tgt_fg) {
            if !colors_equal(sc, tc) {
                warnings.push(format!(
                    "WRONG_COLOR: {} — {}:{} {}:{}",
                    id, src.platform, sc, tgt.platform, tc
                ));
            }
        }

        // Font
        if let (Some(sf), Some(tf)) = (&m.src.font, &m.tgt.font) {
            if sf.family != tf.family || sf.weight != tf.weight {
                warnings.push(format!(
                    "WRONG_FONT: {} — {}:{}-{} {}:{}-{}",
                    id, src.platform, sf.family, sf.weight, tgt.platform, tf.family, tf.weight
                ));
            }
            if (sf.size - tf.size).abs() > 1.0 && sf.size > 0.0 && tf.size > 0.0 {
                warnings.push(format!(
                    "WRONG_FONT_SIZE: {} — {}:{}sp {}:{}sp",
                    id, src.platform, sf.size, tgt.platform, tf.size
                ));
            }
        }

        // Background
        if let (Some(sb), Some(tb)) = (&m.src.background, &m.tgt.background) {
            if !colors_equal(sb, tb) {
                warnings.push(format!(
                    "WRONG_BACKGROUND: {} — {}:{} {}:{}",
                    id, src.platform, sb, tgt.platform, tb
                ));
            }
        }

        // Icon
        if let (Some(si), Some(ti)) = (&m.src.icon, &m.tgt.icon) {
            if si.paths != ti.paths && !si.paths.is_empty() && !ti.paths.is_empty() {
                warnings.push(format!(
                    "WRONG_ICON: {} — pathData differs ({} vs {})",
                    id, si.name, ti.name
                ));
            }
        }

        // Line count
        if let (Some(sl), Some(tl)) = (m.src.line_count, m.tgt.line_count) {
            if sl != tl {
                warnings.push(format!(
                    "WRONG_LINE_COUNT: {} — {}:{} {}:{}",
                    id, src.platform, sl, tgt.platform, tl
                ));
            }
        }

        // Truncated
        if let (Some(st), Some(tt)) = (m.src.truncated, m.tgt.truncated) {
            if st != tt {
                warnings.push(format!(
                    "WRONG_TRUNCATED: {} — {}:{} {}:{}",
                    id, src.platform, st, tgt.platform, tt
                ));
            }
        }

        // Corner radius
        if let (Some(sr), Some(tr)) = (m.src.corner_radius, m.tgt.corner_radius) {
            if (sr - tr).abs() > 1.0 {
                warnings.push(format!(
                    "WRONG_RADIUS: {} — {}:{}dp {}:{}dp",
                    id, src.platform, sr, tgt.platform, tr
                ));
            }
        }

        // Bounds / spacing
        let dx = (m.src.bounds.x - m.tgt.bounds.x).abs();
        let dy = (m.src.bounds.y - m.tgt.bounds.y).abs();
        let dw = (m.src.bounds.w - m.tgt.bounds.w).abs();
        let dh = (m.src.bounds.h - m.tgt.bounds.h).abs();
        if dx > 4 || dy > 4 {
            infos.push(format!(
                "SPACING: {} — {}:({},{}) {}:({},{}) ({}dp drift)",
                id,
                src.platform,
                m.src.bounds.x,
                m.src.bounds.y,
                tgt.platform,
                m.tgt.bounds.x,
                m.tgt.bounds.y,
                dx.max(dy)
            ));
        }
        if dw > 8 || dh > 8 {
            infos.push(format!(
                "SIZE: {} — {}:{}x{} {}:{}x{} ({}dp delta)",
                id,
                src.platform,
                m.src.bounds.w,
                m.src.bounds.h,
                tgt.platform,
                m.tgt.bounds.w,
                m.tgt.bounds.h,
                dw.max(dh)
            ));
        }
    }

    // Output
    if args.json {
        let report = serde_json::json!({
            "source": { "platform": src.platform, "screen": src.screen, "device": src.device },
            "target": { "platform": tgt.platform, "screen": tgt.screen, "device": tgt.device },
            "matched": matches.len(),
            "unmatched_source": unmatched_src.len(),
            "unmatched_target": unmatched_tgt.len(),
            "errors": errors,
            "warnings": warnings,
            "info": infos,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
        return Ok(());
    }

    println!(
        "--- {} ({}) vs {} ({}) ---",
        src.platform, src.device, tgt.platform, tgt.device
    );
    println!(
        "matched: {}  unmatched: {} src, {} tgt\n",
        matches.len(),
        unmatched_src.len(),
        unmatched_tgt.len()
    );

    if !errors.is_empty() {
        println!("ERRORS ({}):", errors.len());
        for e in &errors {
            println!("  {e}");
        }
        println!();
    }
    if !warnings.is_empty() {
        println!("WARNINGS ({}):", warnings.len());
        for w in &warnings {
            println!("  {w}");
        }
        println!();
    }
    if !infos.is_empty() {
        println!("INFO ({}):", infos.len());
        for i in &infos {
            println!("  {i}");
        }
        println!();
    }

    if errors.is_empty() && warnings.is_empty() && infos.is_empty() {
        println!("MATCH: no differences found");
    }

    let total = errors.len() + warnings.len();
    if total > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn match_elements<'a>(
    src: &'a [SemanticElement],
    tgt: &'a [SemanticElement],
) -> (
    Vec<Match<'a>>,
    Vec<&'a SemanticElement>,
    Vec<&'a SemanticElement>,
) {
    let mut matches = Vec::new();
    let mut tgt_matched = vec![false; tgt.len()];
    let mut src_matched = vec![false; src.len()];

    // Pass 1: match by ID (exact)
    for (si, se) in src.iter().enumerate() {
        if se.id.is_empty() || src_matched[si] {
            continue;
        }
        for (ti, te) in tgt.iter().enumerate() {
            if tgt_matched[ti] || te.id.is_empty() {
                continue;
            }
            if se.id == te.id {
                matches.push(Match {
                    src: se,
                    tgt: te,
                    method: "id",
                });
                src_matched[si] = true;
                tgt_matched[ti] = true;
                break;
            }
        }
    }

    // Pass 2: match by content text (exact, decoded)
    for (si, se) in src.iter().enumerate() {
        if src_matched[si] {
            continue;
        }
        let sc = match &se.content {
            Some(c) => decode(c),
            None => continue,
        };
        if sc.is_empty() {
            continue;
        }
        for (ti, te) in tgt.iter().enumerate() {
            if tgt_matched[ti] {
                continue;
            }
            let tc = match &te.content {
                Some(c) => decode(c),
                None => continue,
            };
            if sc == tc && se.elem_type == te.elem_type {
                matches.push(Match {
                    src: se,
                    tgt: te,
                    method: "content",
                });
                src_matched[si] = true;
                tgt_matched[ti] = true;
                break;
            }
        }
    }

    // Pass 3: match by type + position proximity (within 20dp)
    for (si, se) in src.iter().enumerate() {
        if src_matched[si] {
            continue;
        }
        let mut best: Option<(usize, i32)> = None;
        for (ti, te) in tgt.iter().enumerate() {
            if tgt_matched[ti] {
                continue;
            }
            if se.elem_type != te.elem_type {
                continue;
            }
            let dist = (se.bounds.x - te.bounds.x).abs() + (se.bounds.y - te.bounds.y).abs();
            if dist <= 40 {
                if best.map_or(true, |(_, bd)| dist < bd) {
                    best = Some((ti, dist));
                }
            }
        }
        if let Some((ti, _)) = best {
            matches.push(Match {
                src: se,
                tgt: &tgt[ti],
                method: "proximity",
            });
            src_matched[si] = true;
            tgt_matched[ti] = true;
        }
    }

    let unmatched_src: Vec<&SemanticElement> = src
        .iter()
        .enumerate()
        .filter(|(i, _)| !src_matched[*i])
        .map(|(_, e)| e)
        .collect();

    let unmatched_tgt: Vec<&SemanticElement> = tgt
        .iter()
        .enumerate()
        .filter(|(i, _)| !tgt_matched[*i])
        .map(|(_, e)| e)
        .collect();

    (matches, unmatched_src, unmatched_tgt)
}

fn colors_equal(a: &str, b: &str) -> bool {
    normalize_color(a) == normalize_color(b)
}

fn normalize_color(hex: &str) -> String {
    let hex = hex.trim_start_matches('#').to_uppercase();
    // Strip alpha from AARRGGBB
    if hex.len() == 8 {
        hex[2..].to_string()
    } else {
        hex
    }
}

fn decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}
