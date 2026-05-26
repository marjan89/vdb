use clap::Args;

use crate::schema::{SemanticElement, SemanticSchema};

use super::tolerance::delta_e_cie2000;

#[derive(Args)]
pub struct TolerancesInferArgs {
    /// Source YAML files (2 or 3)
    #[arg(required = true)]
    pub sources: Vec<String>,

    /// Output as YAML (paste into manifest)
    #[arg(long)]
    pub yaml: bool,
}

struct MatchedPair {
    id: String,
    font_size_pct: Option<f64>,
    color_delta_e: Option<f64>,
    bg_delta_e: Option<f64>,
    x_offset_dp: Option<f64>,
    width_pct: Option<f64>,
    height_pct: Option<f64>,
    corner_radius_dp: Option<f64>,
    border_width_dp: Option<f64>,
}

pub fn run(args: TolerancesInferArgs) -> Result<(), String> {
    if args.sources.len() < 2 {
        return Err("need at least 2 YAML sources".into());
    }

    let schemas: Vec<SemanticSchema> = args
        .sources
        .iter()
        .map(|path| {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("read {path}: {e}"))
                .unwrap();
            serde_yaml::from_str(&content)
                .map_err(|e| format!("parse {path}: {e}"))
                .unwrap()
        })
        .collect();

    let mut all_pairs: Vec<MatchedPair> = Vec::new();

    for i in 0..schemas.len() {
        for j in (i + 1)..schemas.len() {
            let pairs = find_matched_pairs(&schemas[i], &schemas[j]);
            all_pairs.extend(pairs);
        }
    }

    if all_pairs.is_empty() {
        return Err("no matched element pairs found".into());
    }

    let font_sizes: Vec<f64> = all_pairs.iter().filter_map(|p| p.font_size_pct).collect();
    let colors: Vec<f64> = all_pairs.iter().filter_map(|p| p.color_delta_e).collect();
    let bgs: Vec<f64> = all_pairs.iter().filter_map(|p| p.bg_delta_e).collect();
    let x_offsets: Vec<f64> = all_pairs.iter().filter_map(|p| p.x_offset_dp).collect();
    let widths: Vec<f64> = all_pairs.iter().filter_map(|p| p.width_pct).collect();
    let heights: Vec<f64> = all_pairs.iter().filter_map(|p| p.height_pct).collect();
    let corners: Vec<f64> = all_pairs.iter().filter_map(|p| p.corner_radius_dp).collect();
    let borders: Vec<f64> = all_pairs.iter().filter_map(|p| p.border_width_dp).collect();

    let all_colors: Vec<f64> = colors.iter().chain(bgs.iter()).copied().collect();

    let spatial_pct = p90(&widths).max(p90(&heights)).max(p90_pct(&x_offsets, 400.0));
    let text_size_pct = p90(&font_sizes);
    let color_de = p90(&all_colors);
    let corner_dp = p90(&corners);
    let border_dp = p90(&borders);

    if args.yaml {
        println!("tolerances:");
        println!("  spatial: {:.0}%", spatial_pct.ceil());
        println!("  text_size: {:.0}%", text_size_pct.ceil());
        println!("  color: {:.1}", color_de.max(1.0));
        println!("  text_weight: exact");
        println!("  corner_radius: {:.0}px", corner_dp.ceil());
        println!("  border_width: {:.0}px", border_dp.ceil().max(1.0));
        return Ok(());
    }

    println!("Tolerance inference from {} pairs across {} sources:\n", all_pairs.len(), schemas.len());

    print_stat("font_size", &font_sizes, "%");
    print_stat("color (ΔE)", &all_colors, "");
    print_stat("x_offset", &x_offsets, "dp");
    print_stat("width", &widths, "%");
    print_stat("height", &heights, "%");
    print_stat("corner_radius", &corners, "dp");
    print_stat("border_width", &borders, "dp");

    println!("\nSUGGESTED TOLERANCES:");
    println!("  spatial: {:.0}%", spatial_pct.ceil());
    println!("  text_size: {:.0}%", text_size_pct.ceil());
    println!("  color: {:.1} (Delta E)", color_de.max(1.0));
    println!("  text_weight: exact");
    println!("  corner_radius: {:.0}px", corner_dp.ceil());
    println!("  border_width: {:.0}px", border_dp.ceil().max(1.0));

    Ok(())
}

fn find_matched_pairs(a: &SemanticSchema, b: &SemanticSchema) -> Vec<MatchedPair> {
    let mut pairs = Vec::new();
    let mut b_used = vec![false; b.elements.len()];

    for ae in &a.elements {
        let ae_content = ae.content.as_deref().unwrap_or("").to_lowercase();
        if ae_content.is_empty() && ae.id.is_empty() {
            continue;
        }

        for (bi, be) in b.elements.iter().enumerate() {
            if b_used[bi] {
                continue;
            }
            let be_content = be.content.as_deref().unwrap_or("").to_lowercase();

            let id_match = !ae.id.is_empty() && ae.id == be.id;
            let content_match = !ae_content.is_empty() && ae_content == be_content;

            if !id_match && !content_match {
                continue;
            }

            b_used[bi] = true;

            let id = if !ae.id.is_empty() {
                ae.id.clone()
            } else {
                ae_content.clone()
            };

            let font_size_pct = match (&ae.font, &be.font) {
                (Some(af), Some(bf)) if af.size > 0.0 && bf.size > 0.0 => {
                    Some((af.size - bf.size).abs() / af.size.max(bf.size) * 100.0)
                }
                _ => None,
            };

            let color_delta_e = match (
                ae.foreground.as_ref().or(ae.color.as_ref()),
                be.foreground.as_ref().or(be.color.as_ref()),
            ) {
                (Some(ac), Some(bc)) if ac.starts_with('#') && bc.starts_with('#') => {
                    Some(delta_e_cie2000(ac, bc))
                }
                _ => None,
            };

            let bg_delta_e = match (&ae.background, &be.background) {
                (Some(ab), Some(bb)) if ab.starts_with('#') && bb.starts_with('#') => {
                    Some(delta_e_cie2000(ab, bb))
                }
                _ => None,
            };

            let x_offset_dp = {
                let dx = (ae.bounds.x - be.bounds.x).abs() as f64;
                if dx > 0.0 { Some(dx) } else { None }
            };

            let width_pct = {
                let max_w = ae.bounds.w.max(be.bounds.w).max(1) as f64;
                let dw = (ae.bounds.w - be.bounds.w).abs() as f64;
                if dw > 0.0 { Some(dw / max_w * 100.0) } else { None }
            };

            let height_pct = {
                let max_h = ae.bounds.h.max(be.bounds.h).max(1) as f64;
                let dh = (ae.bounds.h - be.bounds.h).abs() as f64;
                if dh > 0.0 { Some(dh / max_h * 100.0) } else { None }
            };

            let corner_radius_dp = match (ae.corner_radius, be.corner_radius) {
                (Some(ar), Some(br)) => {
                    let d = (ar - br).abs();
                    if d > 0.0 { Some(d) } else { None }
                }
                _ => None,
            };

            let border_width_dp = match (&ae.border, &be.border) {
                (Some(ab), Some(bb)) => {
                    let d = (ab.width - bb.width).abs();
                    if d > 0.0 { Some(d) } else { None }
                }
                _ => None,
            };

            pairs.push(MatchedPair {
                id,
                font_size_pct,
                color_delta_e,
                bg_delta_e,
                x_offset_dp,
                width_pct,
                height_pct,
                corner_radius_dp,
                border_width_dp,
            });
            break;
        }
    }
    pairs
}

fn median(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut sorted = vals.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

fn p90(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut sorted = vals.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = (sorted.len() as f64 * 0.9).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn p90_pct(vals: &[f64], viewport_w: f64) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let pcts: Vec<f64> = vals.iter().map(|v| v / viewport_w * 100.0).collect();
    p90(&pcts)
}

fn print_stat(name: &str, vals: &[f64], unit: &str) {
    if vals.is_empty() {
        println!("  {name}: no data");
        return;
    }
    println!(
        "  {name}: median={:.1}{unit} p90={:.1}{unit} max={:.1}{unit} (n={})",
        median(vals),
        p90(vals),
        vals.iter().cloned().fold(0.0_f64, f64::max),
        vals.len()
    );
}
