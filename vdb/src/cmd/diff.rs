use clap::Args;

use crate::schema::{SemanticElement, SemanticSchema};

use super::tolerance::{delta_e_cie2000, Tolerances};

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

    /// Manifest YAML with tolerances section
    #[arg(long)]
    pub tolerances: Option<String>,
}

struct Match<'a> {
    src: &'a SemanticElement,
    tgt: &'a SemanticElement,
    #[allow(dead_code)]
    method: &'static str,
}

#[derive(Clone)]
enum Severity {
    Error,
    Warning,
    Info,
}

struct Diagnostic {
    severity: Severity,
    message: String,
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

    let tol = match &args.tolerances {
        Some(path) => {
            let t = Tolerances::from_manifest(path)?;
            eprintln!(
                "tolerances: spatial={}% text_size={}% color=ΔE{} corner={}px border={}px",
                t.spatial_pct, t.text_size_pct, t.color_delta_e, t.corner_radius_px, t.border_width_px
            );
            Some(t)
        }
        None => None,
    };

    src.elements.retain(|e| !is_generated_container(e));
    tgt.elements.retain(|e| !is_generated_container(e));

    if args.accessible_only {
        src.elements.retain(|e| e.accessible == Some(true));
        tgt.elements.retain(|e| e.accessible == Some(true));
        eprintln!(
            "accessible-only: {} src, {} tgt elements",
            src.elements.len(),
            tgt.elements.len()
        );
    }

    let viewport_w = src
        .viewport
        .as_ref()
        .map(|v| v.width as f64)
        .or_else(|| tgt.viewport.as_ref().map(|v| v.width as f64))
        .unwrap_or(400.0);

    let (matches, unmatched_src, unmatched_tgt) = match_elements(&src.elements, &tgt.elements);

    let mut diags: Vec<Diagnostic> = Vec::new();

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
        diags.push(Diagnostic {
            severity: Severity::Error,
            message: format!(
                "MISSING: \"{}\" {}{} — not found in {}",
                decode(label),
                elem.elem_type,
                id_part,
                tgt.platform
            ),
        });
    }

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
        diags.push(Diagnostic {
            severity: Severity::Info,
            message: format!(
                "EXTRA: \"{}\" {} in {}{} — not in {}",
                decode(label),
                elem.elem_type,
                tgt.platform,
                id_part,
                src.platform
            ),
        });
    }

    let mut pass_count: usize = 0;

    for m in &matches {
        let id = if !m.src.id.is_empty() {
            &m.src.id
        } else {
            m.src.content.as_deref().unwrap_or("(unnamed)")
        };

        let is_root = is_root_container(&m.src.id) || is_root_container(&m.tgt.id);
        let diag_count_before = diags.len();

        // Text content — always exact match
        let sc = m.src.content.as_deref().map(decode);
        let tc = m.tgt.content.as_deref().map(decode);
        if sc != tc {
            diags.push(Diagnostic {
                severity: Severity::Error,
                message: format!(
                    "WRONG_TEXT: {} — {}:\"{}\" {}:\"{}\"",
                    id,
                    src.platform,
                    sc.as_deref().unwrap_or("(none)"),
                    tgt.platform,
                    tc.as_deref().unwrap_or("(none)")
                ),
            });
        }

        // Element type — always exact
        if m.src.elem_type != m.tgt.elem_type {
            diags.push(Diagnostic {
                severity: Severity::Warning,
                message: format!(
                    "WRONG_TYPE: {} — {}:{} {}:{}",
                    id, src.platform, m.src.elem_type, tgt.platform, m.tgt.elem_type
                ),
            });
        }

        // Color / foreground
        let src_fg = m.src.foreground.as_ref().or(m.src.color.as_ref());
        let tgt_fg = m.tgt.foreground.as_ref().or(m.tgt.color.as_ref());
        if let (Some(sc), Some(tc)) = (src_fg, tgt_fg) {
            if !colors_equal(sc, tc) {
                let de = delta_e_cie2000(sc, tc);
                let within = tol.as_ref().map_or(false, |t| de <= t.color_delta_e);
                diags.push(Diagnostic {
                    severity: if within { Severity::Info } else { Severity::Warning },
                    message: format!(
                        "WRONG_COLOR: {} — {}:{} {}:{} (ΔE={:.1}{})",
                        id, src.platform, sc, tgt.platform, tc, de,
                        if within { " within tolerance" } else { " VIOLATION" }
                    ),
                });
            }
        }

        // Font
        if let (Some(sf), Some(tf)) = (&m.src.font, &m.tgt.font) {
            // Font family — informational (cross-platform names differ)
            if !sf.family.is_empty()
                && !tf.family.is_empty()
                && sf.family.to_lowercase() != tf.family.to_lowercase()
            {
                diags.push(Diagnostic {
                    severity: Severity::Info,
                    message: format!(
                        "DIFF_FONT_FAMILY: {} — {}:{} {}:{}",
                        id, src.platform, sf.family, tgt.platform, tf.family
                    ),
                });
            }

            // Font weight
            if !sf.weight.is_empty() && !tf.weight.is_empty() && sf.weight != tf.weight {
                diags.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "WRONG_FONT_WEIGHT: {} — {}:{} {}:{} VIOLATION",
                        id, src.platform, sf.weight, tgt.platform, tf.weight
                    ),
                });
            }

            // Font size
            if sf.size > 0.0 && tf.size > 0.0 {
                let diff = (sf.size - tf.size).abs();
                let pct = diff / sf.size.max(tf.size) * 100.0;
                let within = tol.as_ref().map_or(diff <= 1.0, |t| pct <= t.text_size_pct);
                if diff > 0.5 {
                    diags.push(Diagnostic {
                        severity: if within { Severity::Info } else { Severity::Warning },
                        message: format!(
                            "WRONG_FONT_SIZE: {} — {}:{}sp {}:{}sp ({:.1}%{})",
                            id, src.platform, sf.size, tgt.platform, tf.size, pct,
                            if within { " within tolerance" } else { " VIOLATION" }
                        ),
                    });
                }
            }
        }

        // Background
        if let (Some(sb), Some(tb)) = (&m.src.background, &m.tgt.background) {
            if !colors_equal(sb, tb) {
                let de = delta_e_cie2000(sb, tb);
                let within = tol.as_ref().map_or(false, |t| de <= t.color_delta_e);
                diags.push(Diagnostic {
                    severity: if within { Severity::Info } else { Severity::Warning },
                    message: format!(
                        "WRONG_BACKGROUND: {} — {}:{} {}:{} (ΔE={:.1}{})",
                        id, src.platform, sb, tgt.platform, tb, de,
                        if within { " within tolerance" } else { " VIOLATION" }
                    ),
                });
            }
        }

        // Icon
        if let (Some(si), Some(ti)) = (&m.src.icon, &m.tgt.icon) {
            if si.paths != ti.paths && !si.paths.is_empty() && !ti.paths.is_empty() {
                diags.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "WRONG_ICON: {} — pathData differs ({} vs {})",
                        id, si.name, ti.name
                    ),
                });
            }
        }

        // Line count
        if let (Some(sl), Some(tl)) = (m.src.line_count, m.tgt.line_count) {
            if sl != tl {
                diags.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "WRONG_LINE_COUNT: {} — {}:{} {}:{}",
                        id, src.platform, sl, tgt.platform, tl
                    ),
                });
            }
        }

        // Truncated
        if let (Some(st), Some(tt)) = (m.src.truncated, m.tgt.truncated) {
            if st != tt {
                diags.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "WRONG_TRUNCATED: {} — {}:{} {}:{}",
                        id, src.platform, st, tgt.platform, tt
                    ),
                });
            }
        }

        // Gradient
        if let (Some(sg), Some(tg)) = (&m.src.gradient, &m.tgt.gradient) {
            if sg.gradient_type != tg.gradient_type {
                diags.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "WRONG_GRADIENT_TYPE: {} — {}:{} {}:{}",
                        id, src.platform, sg.gradient_type, tgt.platform, tg.gradient_type
                    ),
                });
            }
            if sg.colors != tg.colors {
                diags.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "WRONG_GRADIENT_COLORS: {} — {}:{:?} {}:{:?}",
                        id, src.platform, sg.colors, tgt.platform, tg.colors
                    ),
                });
            }
        }

        // Border width
        if let (Some(sb), Some(tb)) = (&m.src.border, &m.tgt.border) {
            let diff = (sb.width - tb.width).abs();
            let within = tol.as_ref().map_or(diff <= 0.5, |t| diff <= t.border_width_px);
            if diff > 0.1 {
                diags.push(Diagnostic {
                    severity: if within { Severity::Info } else { Severity::Warning },
                    message: format!(
                        "WRONG_BORDER_WIDTH: {} — {}:{}dp {}:{}dp (Δ{:.1}dp{})",
                        id, src.platform, sb.width, tgt.platform, tb.width, diff,
                        if within { " within tolerance" } else { " VIOLATION" }
                    ),
                });
            }
            if let (Some(sc), Some(tc)) = (&sb.color, &tb.color) {
                if !colors_equal(sc, tc) {
                    let de = delta_e_cie2000(sc, tc);
                    let within = tol.as_ref().map_or(false, |t| de <= t.color_delta_e);
                    diags.push(Diagnostic {
                        severity: if within { Severity::Info } else { Severity::Warning },
                        message: format!(
                            "WRONG_BORDER_COLOR: {} — {}:{} {}:{} (ΔE={:.1}{})",
                            id, src.platform, sc, tgt.platform, tc, de,
                            if within { " within tolerance" } else { " VIOLATION" }
                        ),
                    });
                }
            }
        }

        // Corner radius
        if let (Some(sr), Some(tr)) = (m.src.corner_radius, m.tgt.corner_radius) {
            let diff = (sr - tr).abs();
            let within = tol.as_ref().map_or(diff <= 1.0, |t| diff <= t.corner_radius_px);
            if diff > 0.1 {
                diags.push(Diagnostic {
                    severity: if within { Severity::Info } else { Severity::Warning },
                    message: format!(
                        "WRONG_RADIUS: {} — {}:{}dp {}:{}dp (Δ{:.1}dp{})",
                        id, src.platform, sr, tgt.platform, tr, diff,
                        if within { " within tolerance" } else { " VIOLATION" }
                    ),
                });
            }
        }

        // Bounds / spacing — skip root containers (viewport wrappers)
        if !is_root {
            let dx = (m.src.bounds.x - m.tgt.bounds.x).abs() as f64;
            let dy = (m.src.bounds.y - m.tgt.bounds.y).abs() as f64;
            let dw = (m.src.bounds.w - m.tgt.bounds.w).abs() as f64;
            let dh = (m.src.bounds.h - m.tgt.bounds.h).abs() as f64;

            let pos_drift = dx.max(dy);
            let pos_pct = pos_drift / viewport_w * 100.0;
            let within_pos = tol.as_ref().map_or(pos_drift <= 4.0, |t| pos_pct <= t.spatial_pct);
            if pos_drift > 2.0 && pos_pct <= 200.0 {
                diags.push(Diagnostic {
                    severity: if within_pos { Severity::Info } else { Severity::Warning },
                    message: format!(
                        "SPACING: {} — {}:({},{}) {}:({},{}) ({:.0}dp={:.1}%vw{})",
                        id,
                        src.platform,
                        m.src.bounds.x,
                        m.src.bounds.y,
                        tgt.platform,
                        m.tgt.bounds.x,
                        m.tgt.bounds.y,
                        pos_drift,
                        pos_pct,
                        if within_pos { " within tolerance" } else { " VIOLATION" }
                    ),
                });
            }

            let size_drift = dw.max(dh);
            let size_pct = size_drift / viewport_w * 100.0;
            let ref_dim = m.src.bounds.w.max(m.src.bounds.h).max(1) as f64;
            let rel_pct = size_drift / ref_dim * 100.0;
            let within_size =
                tol.as_ref().map_or(size_drift <= 8.0, |t| size_pct <= t.spatial_pct);
            if size_drift > 2.0 && rel_pct <= 100.0 {
                diags.push(Diagnostic {
                    severity: if within_size { Severity::Info } else { Severity::Warning },
                    message: format!(
                        "SIZE: {} — {}:{}x{} {}:{}x{} ({:.0}dp={:.1}%vw{})",
                        id,
                        src.platform,
                        m.src.bounds.w,
                        m.src.bounds.h,
                        tgt.platform,
                        m.tgt.bounds.w,
                        m.tgt.bounds.h,
                        size_drift,
                        size_pct,
                        if within_size { " within tolerance" } else { " VIOLATION" }
                    ),
                });
            }
        }

        let diag_count_after = diags.len();
        if diag_count_before == diag_count_after {
            pass_count += 1;
        }
    }

    let errors: Vec<&str> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .map(|d| d.message.as_str())
        .collect();
    let warnings: Vec<&str> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Warning))
        .map(|d| d.message.as_str())
        .collect();
    let infos: Vec<&str> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Info))
        .map(|d| d.message.as_str())
        .collect();

    if args.json {
        let report = serde_json::json!({
            "source": { "platform": src.platform, "screen": src.screen, "device": src.device },
            "target": { "platform": tgt.platform, "screen": tgt.screen, "device": tgt.device },
            "matched": matches.len(),
            "unmatched_source": unmatched_src.len(),
            "unmatched_target": unmatched_tgt.len(),
            "tolerances_applied": tol.is_some(),
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
    if tol.is_some() {
        println!("tolerances: applied from manifest");
    }
    println!(
        "matched: {}  unmatched: {} src, {} tgt\n",
        matches.len(),
        unmatched_src.len(),
        unmatched_tgt.len()
    );

    let violations: usize = warnings
        .iter()
        .filter(|w| w.contains("VIOLATION"))
        .count()
        + errors.len();
    let suppressed: usize = warnings
        .iter()
        .filter(|w| w.contains("within tolerance"))
        .count()
        + infos.iter().filter(|i| i.contains("within tolerance")).count();

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

    if tol.is_some() {
        println!(
            "SUMMARY: {} pass, {} violations, {} within tolerance",
            pass_count, violations, suppressed
        );
    }

    if errors.is_empty() && warnings.is_empty() && infos.is_empty() {
        println!("MATCH: no differences found");
    }

    if violations > 0 {
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

    // Pass 2: content matching (case-insensitive exact, stripped counts, containment)
    for (si, se) in src.iter().enumerate() {
        if src_matched[si] {
            continue;
        }
        let sc = match &se.content {
            Some(c) => decode(c),
            None => continue,
        };
        if sc.len() < 2 {
            continue;
        }
        let sc_lower = sc.to_lowercase();
        let sc_stripped = strip_dynamic_counts(&sc_lower);

        let mut best: Option<(usize, u32)> = None;
        for (ti, te) in tgt.iter().enumerate() {
            if tgt_matched[ti] {
                continue;
            }
            let tc = match &te.content {
                Some(c) => decode(c),
                None => continue,
            };
            if tc.len() < 2 {
                continue;
            }
            let tc_lower = tc.to_lowercase();

            // Tier 1: exact case-insensitive
            if sc_lower == tc_lower {
                best = Some((ti, 300));
                break;
            }

            // Tier 2: stripped counts match
            let tc_stripped = strip_dynamic_counts(&tc_lower);
            if sc_stripped == tc_stripped && sc_stripped.len() >= 3 {
                best = Some((ti, 200));
                break;
            }

            // Tier 3: containment with >=70% length overlap
            let shorter = sc_lower.len().min(tc_lower.len());
            let longer = sc_lower.len().max(tc_lower.len());
            if shorter >= 3 && shorter * 100 / longer >= 60 {
                if sc_lower.contains(&tc_lower) || tc_lower.contains(&sc_lower) {
                    let score = 100 + (shorter * 100 / longer) as u32;
                    if best.map_or(true, |(_, bs)| score > bs) {
                        best = Some((ti, score));
                    }
                }
            }
        }
        if let Some((ti, _)) = best {
            matches.push(Match {
                src: se,
                tgt: &tgt[ti],
                method: "content",
            });
            src_matched[si] = true;
            tgt_matched[ti] = true;
        }
    }

    // Pass 3: type + position proximity (within 20dp, non-text only)
    for (si, se) in src.iter().enumerate() {
        if src_matched[si] {
            continue;
        }
        if se.elem_type == "text" || se.elem_type == "button" {
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

fn is_generated_container(e: &SemanticElement) -> bool {
    if e.elem_type != "container" && e.elem_type != "view" {
        return false;
    }
    if e.content.is_some() {
        return false;
    }
    let id = &e.id;
    if id.is_empty() {
        return false;
    }
    lazy_static_regex(id)
}

fn lazy_static_regex(id: &str) -> bool {
    let lower = id.to_lowercase();
    let prefixes = [
        "linearlayout", "constraintlayout", "framelayout", "relativelayout",
        "cardview", "recyclerview", "nestedscrollview", "scrollview",
        "appbarlayout", "coordinatorlayout", "collapsingtoolbar",
    ];
    if !prefixes.iter().any(|p| lower.starts_with(p)) {
        return false;
    }
    id.chars().any(|c| c == '_') && id.chars().any(|c| c.is_ascii_digit())
}

fn is_root_container(id: &str) -> bool {
    const ROOT_IDS: &[&str] = &[
        "decorview",
        "framelayout",
        "action_bar_root",
        "content",
        "coordinatorlayout",
        "linearlayout_0_0",
        "framelayout_0_0_0",
        "framelayout_0_0_1",
    ];
    if id.is_empty() {
        return false;
    }
    let lower = id.to_lowercase();
    ROOT_IDS.iter().any(|r| lower.starts_with(r))
}

fn colors_equal(a: &str, b: &str) -> bool {
    normalize_color(a) == normalize_color(b)
}

fn normalize_color(hex: &str) -> String {
    let hex = hex.trim_start_matches('#').to_uppercase();
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

fn strip_dynamic_counts(s: &str) -> String {
    let mut result = s.to_string();
    while let Some(start) = result.find('(') {
        if let Some(end) = result[start..].find(')') {
            let inner = &result[start + 1..start + end];
            if inner.trim().chars().all(|c| c.is_ascii_digit() || c == ' ') {
                result = format!("{}{}", result[..start].trim_end(), &result[start + end + 1..]);
                continue;
            }
        }
        break;
    }
    result.trim().to_string()
}
