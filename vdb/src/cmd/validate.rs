use clap::Args;
use image::{Rgb, RgbImage};

use crate::schema::SemanticSchema;

#[derive(Args)]
pub struct ValidateArgs {
    /// Device screenshot with agent overlay (white canvas, djb2-colored strokes/fills)
    pub device_overlay: String,
    /// Semantic schema YAML
    pub schema: String,

    /// Output composite image
    #[arg(short, long, default_value = "/tmp/vdb-validate.png")]
    pub output: String,

    /// Real device screenshot (no overlay) for visual context
    #[arg(long)]
    pub screenshot: Option<String>,

    /// Stroke width in px (must match agent, default 4)
    #[arg(long, default_value = "4")]
    pub stroke_width: u32,

    /// Validation pass: "stroke" (position) or "fill" (dimensions)
    #[arg(long, default_value = "stroke")]
    pub pass: String,

    /// Pass threshold per element (overlap %, default 70)
    #[arg(long, default_value = "70")]
    pub threshold: f64,

    /// Color matching tolerance (0-255, default 120 for OLED displays)
    #[arg(long, default_value = "120")]
    pub color_tolerance: u8,

    /// Device density (dp to px). Auto-detected from schema bounds vs screenshot width if omitted
    #[arg(long)]
    pub density: Option<f64>,

    /// Viewport width in dp (auto-detected from screenshot/density if omitted)
    #[arg(long)]
    pub viewport_width: Option<i32>,

    /// Viewport height in dp (auto-detected from screenshot/density if omitted)
    #[arg(long)]
    pub viewport_height: Option<i32>,
}

pub fn run(args: ValidateArgs) -> Result<(), String> {
    let device = image::open(&args.device_overlay)
        .map_err(|e| format!("read device: {e}"))?
        .to_rgb8();
    let schema_str =
        std::fs::read_to_string(&args.schema).map_err(|e| format!("read schema: {e}"))?;
    let schema: SemanticSchema =
        serde_yaml::from_str(&schema_str).map_err(|e| format!("parse schema: {e}"))?;

    let (dw, dh) = device.dimensions();

    let density = args.density.unwrap_or_else(|| {
        schema.viewport.as_ref()
            .filter(|v| v.density > 0.0)
            .map(|v| { eprintln!("density: {:.3} (from YAML viewport)", v.density); v.density })
            .unwrap_or_else(|| auto_density(&schema, dw))
    });

    let viewport_w = args.viewport_width.unwrap_or_else(|| {
        schema.viewport.as_ref()
            .filter(|v| v.width > 0)
            .map(|v| v.width)
            .unwrap_or_else(|| (dw as f64 / density).round() as i32)
    });
    let viewport_h = args.viewport_height.unwrap_or_else(|| {
        schema.viewport.as_ref()
            .filter(|v| v.height > 0)
            .map(|v| v.height)
            .unwrap_or_else(|| (dh as f64 / density).round() as i32)
    });

    // All elements sorted by z_index — used for rendering (must match agent)
    let mut all_elements: Vec<(usize, &crate::schema::SemanticElement)> = schema
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| e.bounds.w > 0 && e.bounds.h > 0)
        .filter(|(_, e)| e.bounds.x >= 0 && e.bounds.y >= 0)
        .collect();
    all_elements.sort_by_key(|(orig_idx, e)| e.z_index.unwrap_or(*orig_idx as u64));

    // Content elements only — used for scoring
    let elements: Vec<&crate::schema::SemanticElement> = all_elements
        .iter()
        .filter(|(_, e)| {
            !matches!(
                e.elem_type.as_str(),
                "container" | "list" | "scroll" | "pager" | "view"
            )
        })
        .filter(|(_, e)| e.bounds.x < viewport_w && e.bounds.y < viewport_h)
        .filter(|(_, e)| e.children.is_none())
        .map(|(_, e)| *e)
        .collect();

    let total = elements.len();
    let sw = args.stroke_width;
    let mut pass_count = 0;

    // Render reference overlay matching agent (same djb2 colors, same z-order)
    let mut render = RgbImage::from_pixel(dw, dh, Rgb([255, 255, 255]));
    let white = Rgb([255, 255, 255]);

    for (_, elem) in &all_elements {
        let color = djb2_color(effective_id(elem));
        let x = (elem.bounds.x as f64 * density) as i32;
        let y = (elem.bounds.y as f64 * density) as i32;
        let w = (elem.bounds.w as f64 * density) as u32;
        let h = (elem.bounds.h as f64 * density) as u32;

        if args.pass == "fill" {
            draw_fill_rect(&mut render, x, y, w, h, color);
        } else {
            draw_fill_rect(&mut render, x, y, w, h, white);
            draw_stroke_rect(&mut render, x, y, w, h, sw, color);
        }
    }

    // Difference composite for visual debugging
    let mut composite = RgbImage::new(dw, dh);
    for y in 0..dh {
        for x in 0..dw {
            let dp = device.get_pixel(x, y);
            let rp = render.get_pixel(x, y);
            let dr = (dp.0[0] as i16 - rp.0[0] as i16).unsigned_abs() as u8;
            let dg = (dp.0[1] as i16 - rp.0[1] as i16).unsigned_abs() as u8;
            let db = (dp.0[2] as i16 - rp.0[2] as i16).unsigned_abs() as u8;
            composite.put_pixel(x, y, Rgb([dr, dg, db]));
        }
    }

    println!("vdb validate results:\n");

    for (i, elem) in elements.iter().enumerate() {
        let expected = djb2_color(effective_id(elem));
        let x = (elem.bounds.x as f64 * density) as i32;
        let y = (elem.bounds.y as f64 * density) as i32;
        let w = (elem.bounds.w as f64 * density) as u32;
        let h = (elem.bounds.h as f64 * density) as u32;
        let tol = args.color_tolerance;

        let display_id = if !elem.id.is_empty() {
            &elem.id
        } else {
            elem.content.as_deref().unwrap_or("(unnamed)")
        };

        if args.pass == "fill" && elem.render.as_deref() == Some("external") {
            println!("  {:30} SKIP (external render)", display_id);
            pass_count += 1;
            continue;
        }

        // Compute occlusion from ALL higher-z elements (including containers)
        let visible = visible_fraction_all(elem, &all_elements);

        if visible < 0.05 {
            println!(
                "  {:30} SKIP (occluded {:.0}% visible)",
                display_id,
                visible * 100.0
            );
            pass_count += 1;
            continue;
        }

        let (match_count, total_samples) = if args.pass == "fill" {
            sample_fill(&device, x, y, w, h, dw, dh, &expected, tol)
        } else {
            sample_stroke(&device, x, y, w, h, sw, dw, dh, &expected, tol)
        };

        let overlap = if total_samples > 0 {
            match_count as f64 / total_samples as f64 * 100.0
        } else {
            0.0
        };

        let adjusted = if visible < 0.95 {
            (overlap / visible).min(100.0)
        } else {
            overlap
        };

        let status = if adjusted >= args.threshold {
            "PASS"
        } else {
            "FAIL"
        };
        if status == "PASS" {
            pass_count += 1;
        }

        if visible < 0.95 {
            println!(
                "  {:30} {} ({} overlap {:.0}%, {:.0}% visible, adj {:.0}%)",
                display_id, status, args.pass, overlap, visible * 100.0, adjusted
            );
        } else {
            println!(
                "  {:30} {} ({} overlap {:.0}%)",
                display_id, status, args.pass, overlap
            );
        }
    }

    println!("\n  TOTAL: {}/{} PASS", pass_count, total);

    composite
        .save(&args.output)
        .map_err(|e| format!("save error: {e}"))?;
    eprintln!("composite: {}", args.output);

    if let Some(ref screenshot_path) = args.screenshot {
        save_visual_overlay(&composite, screenshot_path, dw, dh, &args.output)?;
    }

    if pass_count < total {
        std::process::exit(1);
    }

    Ok(())
}

fn auto_density(schema: &SemanticSchema, screenshot_width: u32) -> f64 {
    // Use the root container's width as the viewport (most reliable)
    if let Some(root) = schema.elements.first() {
        if root.bounds.w > 100 {
            let d = screenshot_width as f64 / root.bounds.w as f64;
            eprintln!("auto-density: {:.3} (from root element w={}dp)", d, root.bounds.w);
            return d;
        }
    }
    // Fallback: use 90th percentile of right-edges to exclude off-screen elements
    let mut right_edges: Vec<i32> = schema
        .elements
        .iter()
        .filter(|e| e.bounds.w > 0 && e.bounds.x >= 0)
        .map(|e| e.bounds.x + e.bounds.w)
        .collect();
    right_edges.sort();
    let idx = (right_edges.len() as f64 * 0.9) as usize;
    let viewport_w = right_edges.get(idx.min(right_edges.len().saturating_sub(1)))
        .copied()
        .unwrap_or(390) as f64;
    if viewport_w > 100.0 {
        let d = screenshot_width as f64 / viewport_w;
        eprintln!("auto-density: {:.3} (from 90th percentile w={}dp)", d, viewport_w);
        d
    } else {
        3.0
    }
}

fn effective_id(elem: &crate::schema::SemanticElement) -> &str {
    if !elem.id.is_empty() {
        &elem.id
    } else {
        elem.content.as_deref().unwrap_or("")
    }
}

fn djb2_color(id: &str) -> Rgb<u8> {
    let hue = djb2_hue(id);
    hsl_to_rgb(hue, 1.0, 0.5)
}

fn djb2_hue(id: &str) -> f32 {
    let mut hash: u32 = 5381;
    for b in id.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    (hash % 360) as f32
}

fn color_close(a: &Rgb<u8>, b: &Rgb<u8>, threshold: u8) -> bool {
    let dr = (a.0[0] as i16 - b.0[0] as i16).unsigned_abs() as u8;
    let dg = (a.0[1] as i16 - b.0[1] as i16).unsigned_abs() as u8;
    let db = (a.0[2] as i16 - b.0[2] as i16).unsigned_abs() as u8;
    dr <= threshold && dg <= threshold && db <= threshold
}

fn sample_fill(
    device: &RgbImage,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    dw: u32,
    dh: u32,
    expected: &Rgb<u8>,
    tol: u8,
) -> (u32, u32) {
    let mut match_count = 0u32;
    let mut total_samples = 0u32;
    let step = 2u32;
    for dy in (0..h).step_by(step as usize) {
        for dx in (0..w).step_by(step as usize) {
            let px = (x + dx as i32) as u32;
            let py = (y + dy as i32) as u32;
            if px < dw && py < dh {
                total_samples += 1;
                if color_close(device.get_pixel(px, py), expected, tol) {
                    match_count += 1;
                }
            }
        }
    }
    (match_count, total_samples)
}

fn sample_stroke(
    device: &RgbImage,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    sw: u32,
    dw: u32,
    dh: u32,
    expected: &Rgb<u8>,
    tol: u8,
) -> (u32, u32) {
    let mut match_count = 0u32;
    let mut total_samples = 0u32;

    let sample = |px: u32, py: u32, mc: &mut u32, ts: &mut u32| {
        if px < dw && py < dh {
            *ts += 1;
            if color_close(device.get_pixel(px, py), expected, tol) {
                *mc += 1;
            }
        }
    };

    // Top edge
    for dx in 0..w.min(dw) {
        for dy in 0..sw.min(h) {
            sample((x + dx as i32) as u32, (y + dy as i32) as u32, &mut match_count, &mut total_samples);
        }
    }
    // Bottom edge
    for dx in 0..w.min(dw) {
        for dy in h.saturating_sub(sw)..h {
            sample((x + dx as i32) as u32, (y + dy as i32) as u32, &mut match_count, &mut total_samples);
        }
    }
    // Left edge
    for dy in sw..h.saturating_sub(sw) {
        for dx in 0..sw.min(w) {
            sample((x + dx as i32) as u32, (y + dy as i32) as u32, &mut match_count, &mut total_samples);
        }
    }
    // Right edge
    for dy in sw..h.saturating_sub(sw) {
        for dx in w.saturating_sub(sw)..w {
            sample((x + dx as i32) as u32, (y + dy as i32) as u32, &mut match_count, &mut total_samples);
        }
    }

    (match_count, total_samples)
}

fn visible_fraction_all(
    elem: &crate::schema::SemanticElement,
    all: &[(usize, &crate::schema::SemanticElement)],
) -> f64 {
    let total_area = (elem.bounds.w * elem.bounds.h) as f64;
    if total_area <= 0.0 {
        return 1.0;
    }

    let elem_z = elem.z_index.unwrap_or(0);
    let mut occluded = 0.0;

    for (_, other) in all {
        let other_z = other.z_index.unwrap_or(0);
        if other_z <= elem_z {
            continue;
        }
        let ix1 = elem.bounds.x.max(other.bounds.x);
        let iy1 = elem.bounds.y.max(other.bounds.y);
        let ix2 = (elem.bounds.x + elem.bounds.w).min(other.bounds.x + other.bounds.w);
        let iy2 = (elem.bounds.y + elem.bounds.h).min(other.bounds.y + other.bounds.h);
        if ix2 > ix1 && iy2 > iy1 {
            occluded += ((ix2 - ix1) * (iy2 - iy1)) as f64;
        }
    }

    ((total_area - occluded).max(0.0) / total_area).max(0.0)
}

fn save_visual_overlay(
    composite: &RgbImage,
    screenshot_path: &str,
    dw: u32,
    dh: u32,
    output: &str,
) -> Result<(), String> {
    let screenshot = image::open(screenshot_path).map_err(|e| format!("read screenshot: {e}"))?;
    let ss = screenshot
        .resize_exact(dw, dh, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let mut visual = RgbImage::new(dw, dh);
    for y in 0..dh {
        for x in 0..dw {
            let sp = ss.get_pixel(x, y);
            let cp = composite.get_pixel(x, y);
            let r = 255 - ((255 - sp.0[0] as u16) * (255 - cp.0[0] as u16) / 255) as u8;
            let g = 255 - ((255 - sp.0[1] as u16) * (255 - cp.0[1] as u16) / 255) as u8;
            let b = 255 - ((255 - sp.0[2] as u16) * (255 - cp.0[2] as u16) / 255) as u8;
            visual.put_pixel(x, y, Rgb([r, g, b]));
        }
    }
    let visual_path = output.replace(".png", "-visual.png");
    visual
        .save(&visual_path)
        .map_err(|e| format!("save visual: {e}"))?;
    eprintln!("visual overlay: {}", visual_path);
    Ok(())
}

fn draw_fill_rect(img: &mut RgbImage, x: i32, y: i32, w: u32, h: u32, color: Rgb<u8>) {
    let (iw, ih) = img.dimensions();
    for dy in 0..h {
        for dx in 0..w {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

fn draw_stroke_rect(img: &mut RgbImage, x: i32, y: i32, w: u32, h: u32, sw: u32, color: Rgb<u8>) {
    let (iw, ih) = img.dimensions();
    for dy in 0..sw.min(h) {
        for dx in 0..w {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
    for dy in h.saturating_sub(sw)..h {
        for dx in 0..w {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
    for dy in sw..h.saturating_sub(sw) {
        for dx in 0..sw.min(w) {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
    for dy in sw..h.saturating_sub(sw) {
        for dx in w.saturating_sub(sw)..w {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Rgb<u8> {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h2 = h / 60.0;
    let x = c * (1.0 - (h2 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h2 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Rgb([
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    ])
}
