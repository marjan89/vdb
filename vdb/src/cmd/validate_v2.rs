use clap::Args;
use image::{GenericImageView, Rgb, RgbImage};

use crate::schema::SemanticSchema;

#[derive(Args)]
pub struct ValidateV2Args {
    /// Device screenshot with agent validate overlay (white canvas, colored strokes)
    pub device_overlay: String,
    /// Semantic schema YAML
    pub schema: String,

    /// Output composite image
    #[arg(short, long, default_value = "/tmp/vdb-validate-v2.png")]
    pub output: String,

    /// Real device screenshot (no overlay) for visual context
    #[arg(long)]
    pub screenshot: Option<String>,

    /// Stroke width (must match agent, default 4)
    #[arg(long, default_value = "4")]
    pub stroke_width: u32,

    /// Validation pass: "stroke" (position) or "fill" (dimensions)
    #[arg(long, default_value = "stroke")]
    pub pass: String,

    /// Pass threshold per element (overlap %, default 70)
    #[arg(long, default_value = "70")]
    pub threshold: f64,

    /// Color matching tolerance (0-255, default 80, use 120+ for OLED displays)
    #[arg(long, default_value = "120")]
    pub color_tolerance: u8,

    /// Device density (pt/dp to px)
    #[arg(long)]
    pub density: Option<f64>,
}

pub fn run(args: ValidateV2Args) -> Result<(), String> {
    let device = image::open(&args.device_overlay)
        .map_err(|e| format!("read device: {e}"))?
        .to_rgb8();
    let schema_str = std::fs::read_to_string(&args.schema)
        .map_err(|e| format!("read schema: {e}"))?;
    let schema: SemanticSchema =
        serde_yaml::from_str(&schema_str).map_err(|e| format!("parse schema: {e}"))?;

    let (dw, dh) = device.dimensions();

    // Warn if image looks downscaled
    let expected_min_w = if schema.platform == "ios" { 1170 } else { 1080 };
    if dw < expected_min_w {
        eprintln!("WARNING: screenshot {}x{} looks downscaled (expected >= {}px wide). use full-res screencap for accurate validation.", dw, dh, expected_min_w);
    }

    let density = args.density.unwrap_or_else(|| {
        let max_x = schema.elements.iter()
            .filter(|e| !matches!(e.elem_type.as_str(), "container" | "list" | "scroll" | "pager" | "view"))
            .map(|e| (e.bounds.x + e.bounds.w) as f64)
            .fold(0.0f64, f64::max);
        if max_x > 100.0 { dw as f64 / max_x } else { 3.0 }
    });

    // Filter to content elements (same as agent)
    let viewport_w = if schema.platform == "ios" { 390 } else { 384 };
    let viewport_h = if schema.platform == "ios" { 844 } else { 832 };

    let elements: Vec<&crate::schema::SemanticElement> = schema.elements.iter()
        .filter(|e| !matches!(e.elem_type.as_str(), "container" | "list" | "scroll" | "pager" | "view"))
        .filter(|e| e.bounds.w > 0 && e.bounds.h > 0)
        .filter(|e| e.bounds.x >= 0 && e.bounds.y >= 0)
        .filter(|e| e.bounds.x < viewport_w && e.bounds.y < viewport_h)
        .filter(|e| e.children.is_none()) // skip composite parents
        .collect();

    let total = elements.len();
    let sw = args.stroke_width;
    let mut results = Vec::new();
    let mut pass_count = 0;

    // Sort by z_index (walk order = z-order, bottom to top)
    let mut sorted_indices: Vec<usize> = (0..elements.len()).collect();
    sorted_indices.sort_by_key(|&i| elements[i].z_index.unwrap_or(i as u64));

    // Render complementary colors with z-order aware drawing
    let mut render = RgbImage::from_pixel(dw, dh, Rgb([255, 255, 255]));
    let white = Rgb([255, 255, 255]);

    for &idx in &sorted_indices {
        let elem = elements[idx];
        let elem_id = if !elem.id.is_empty() { &elem.id } else { elem.content.as_deref().unwrap_or("") };
        let (_, render_color) = element_colors_by_id(elem_id);
        let x = (elem.bounds.x as f64 * density) as i32;
        let y = (elem.bounds.y as f64 * density) as i32;
        let w = (elem.bounds.w as f64 * density) as u32;
        let h = (elem.bounds.h as f64 * density) as u32;

        if args.pass == "fill" {
            // Fills naturally occlude — just draw in z-order
            draw_fill_rect(&mut render, x, y, w, h, render_color);
        } else {
            // White fill THEN colored stroke — higher-z white clips lower-z strokes
            draw_fill_rect(&mut render, x, y, w, h, white);
            draw_stroke_rect(&mut render, x, y, w, h, sw, render_color);
        }
    }


    // Composite via Difference (shows misalignment as bright pixels)
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

    // Per-element verification
    println!("vdb validate v2 results:\n");

    for (i, elem) in elements.iter().enumerate() {
        let elem_id = if !elem.id.is_empty() { &elem.id } else { elem.content.as_deref().unwrap_or("") };
        let (device_color, render_color) = element_colors_by_id(elem_id);
        let x = (elem.bounds.x as f64 * density) as i32;
        let y = (elem.bounds.y as f64 * density) as i32;
        let w = (elem.bounds.w as f64 * density) as u32;
        let h = (elem.bounds.h as f64 * density) as u32;

        let mut match_count = 0u32;
        let mut total_samples = 0u32;
        let tol = args.color_tolerance;

        let elem_id_display = if !elem.id.is_empty() {
            &elem.id
        } else {
            elem.content.as_deref().unwrap_or("(unnamed)")
        };

        // Skip fill validation for external render surfaces
        let is_external = elem.render.as_deref() == Some("external");
        if args.pass == "fill" && is_external {
            println!("  {:30} SKIP (external render)", elem_id_display);
            pass_count += 1;
            continue;
        }

        if args.pass == "fill" {
            let step = 2u32;
            for dy in (0..h).step_by(step as usize) {
                for dx in (0..w).step_by(step as usize) {
                    let px = (x + dx as i32) as u32;
                    let py = (y + dy as i32) as u32;
                    if px < dw && py < dh {
                        total_samples += 1;
                        let dp = device.get_pixel(px, py);
                        let rp = render.get_pixel(px, py);
                        let device_has = color_close(dp, &device_color, tol);
                        let render_has = color_close(rp, &render_color, tol);
                        if device_has && render_has {
                            match_count += 1;
                        }
                    }
                }
            }
        } else {
            // Sample stroke edges
            for dx in 0..w.min(dw) {
                for dy in 0..sw.min(h) {
                    let px = (x + dx as i32) as u32;
                    let py = (y + dy as i32) as u32;
                    if px < dw && py < dh {
                        total_samples += 1;
                        let dp = device.get_pixel(px, py);
                        let rp = render.get_pixel(px, py);
                        if color_close(dp, &device_color, tol) && color_close(rp, &render_color, tol) {
                            match_count += 1;
                        }
                    }
                }
            }
            for dy in sw..h.min(dh) {
                for dx in 0..sw.min(w) {
                    let px = (x + dx as i32) as u32;
                    let py = (y + dy as i32) as u32;
                    if px < dw && py < dh {
                        total_samples += 1;
                        let dp = device.get_pixel(px, py);
                        let rp = render.get_pixel(px, py);
                        if color_close(dp, &device_color, tol) && color_close(rp, &render_color, tol) {
                            match_count += 1;
                        }
                    }
                }
            }
        }

        let overlap = if total_samples > 0 {
            match_count as f64 / total_samples as f64 * 100.0
        } else {
            0.0
        };

        // Post-scoring occlusion adjustment:
        // if element is partially occluded by higher-z siblings,
        // score against visible area, not total area
        let adjusted_overlap = if overlap < args.threshold {
            let visible = compute_visible_fraction(elem, i, &elements, density);
            if visible < 0.99 {
                // Rescale: actual overlap relative to visible portion
                let rescaled = overlap / (visible * 100.0) * 100.0;
                rescaled
            } else {
                overlap
            }
        } else {
            overlap
        };
        let status = if adjusted_overlap >= args.threshold { "PASS" } else { "FAIL" };
        if status == "PASS" { pass_count += 1; }

        println!("  {:30} {} (stroke overlap {:.0}%)", elem_id_display, status, overlap);
        results.push((elem_id_display.to_string(), status.to_string(), overlap));
    }

    println!("\n  TOTAL: {}/{} PASS", pass_count, total);

    composite
        .save(&args.output)
        .map_err(|e| format!("save error: {e}"))?;
    eprintln!("composite: {}", args.output);

    // Screen blend on real screenshot for visual context
    if let Some(ref screenshot_path) = args.screenshot {
        if let Ok(screenshot) = image::open(screenshot_path) {
            let ss = screenshot.resize_exact(dw, dh, image::imageops::FilterType::Lanczos3).to_rgb8();
            let mut visual = RgbImage::new(dw, dh);
            for y in 0..dh {
                for x in 0..dw {
                    let sp = ss.get_pixel(x, y);
                    let cp = composite.get_pixel(x, y);
                    // Screen blend: 1 - (1-a)(1-b)
                    let r = 255 - ((255 - sp.0[0] as u16) * (255 - cp.0[0] as u16) / 255) as u8;
                    let g = 255 - ((255 - sp.0[1] as u16) * (255 - cp.0[1] as u16) / 255) as u8;
                    let b = 255 - ((255 - sp.0[2] as u16) * (255 - cp.0[2] as u16) / 255) as u8;
                    visual.put_pixel(x, y, Rgb([r, g, b]));
                }
            }
            let visual_path = args.output.replace(".png", "-visual.png");
            visual.save(&visual_path).map_err(|e| format!("save visual: {e}"))?;
            eprintln!("visual overlay: {}", visual_path);
        }
    }

    if pass_count < total {
        std::process::exit(1);
    }

    Ok(())
}

fn element_colors_by_id(id: &str) -> (Rgb<u8>, Rgb<u8>) {
    let hue = id_to_hue(id);
    let device = hsl_to_rgb(hue, 1.0, 0.5);
    let render = hsl_to_rgb((hue + 180.0) % 360.0, 1.0, 0.5);
    (device, render)
}

fn id_to_hue(id: &str) -> f32 {
    let mut hash: u32 = 5381;
    for b in id.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add((b & 0xFF) as u32);
    }
    (hash % 360) as f32
}

fn multiply_colors(a: Rgb<u8>, b: Rgb<u8>) -> Rgb<u8> {
    Rgb([
        (a.0[0] as u16 * b.0[0] as u16 / 255) as u8,
        (a.0[1] as u16 * b.0[1] as u16 / 255) as u8,
        (a.0[2] as u16 * b.0[2] as u16 / 255) as u8,
    ])
}

fn color_close(a: &Rgb<u8>, b: &Rgb<u8>, threshold: u8) -> bool {
    let dr = (a.0[0] as i16 - b.0[0] as i16).unsigned_abs() as u8;
    let dg = (a.0[1] as i16 - b.0[1] as i16).unsigned_abs() as u8;
    let db = (a.0[2] as i16 - b.0[2] as i16).unsigned_abs() as u8;
    dr <= threshold && dg <= threshold && db <= threshold
}

fn compute_visible_fraction(
    elem: &crate::schema::SemanticElement,
    elem_idx: usize,
    all_elements: &[&crate::schema::SemanticElement],
    density: f64,
) -> f64 {
    let ex = elem.bounds.x;
    let ey = elem.bounds.y;
    let ew = elem.bounds.w;
    let eh = elem.bounds.h;
    let total_area = (ew * eh) as f64;
    if total_area <= 0.0 {
        return 1.0;
    }

    let mut occluded = 0.0;

    // Elements after this one in the list are higher z-order
    for other in all_elements.iter().skip(elem_idx + 1) {
        let ox = other.bounds.x;
        let oy = other.bounds.y;
        let ow = other.bounds.w;
        let oh = other.bounds.h;

        // Compute intersection
        let ix1 = ex.max(ox);
        let iy1 = ey.max(oy);
        let ix2 = (ex + ew).min(ox + ow);
        let iy2 = (ey + eh).min(oy + oh);

        if ix2 > ix1 && iy2 > iy1 {
            let intersection = ((ix2 - ix1) * (iy2 - iy1)) as f64;
            occluded += intersection;
        }
    }

    let visible = (total_area - occluded).max(0.0) / total_area;
    visible.max(0.1) // at least 10% — fully occluded elements should be filtered elsewhere
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
    // Top edge
    for dy in 0..sw.min(h) {
        for dx in 0..w {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
    // Bottom edge
    for dy in h.saturating_sub(sw)..h {
        for dx in 0..w {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
    // Left edge
    for dy in sw..h.saturating_sub(sw) {
        for dx in 0..sw.min(w) {
            let px = x + dx as i32;
            let py = y + dy as i32;
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
    // Right edge
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
