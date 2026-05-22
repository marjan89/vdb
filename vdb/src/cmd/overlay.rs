use clap::Args;
use ab_glyph::FontRef;
use image::{Rgba, GenericImageView};
use imageproc::drawing::{draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;

use crate::schema::SemanticSchema;

#[derive(Args)]
pub struct OverlayArgs {
    /// Device screenshot PNG
    pub screenshot: String,
    /// Semantic schema YAML
    pub schema: String,

    /// Output PNG path
    #[arg(short, long, default_value = "/tmp/vdb-overlay.png")]
    pub output: String,

    /// Device density (dp to px conversion, default auto-detect from image/schema)
    #[arg(long)]
    pub density: Option<f64>,

    /// Safe area top inset in pt (iOS notch/status bar offset, default 47 for modern iPhones)
    #[arg(long)]
    pub safe_area_top: Option<f64>,
}

static FONT_BYTES: &[u8] = include_bytes!("/System/Library/Fonts/Helvetica.ttc");

pub fn run(args: OverlayArgs) -> Result<(), String> {
    let screenshot = image::open(&args.screenshot)
        .map_err(|e| format!("read screenshot: {e}"))?;
    let schema_str = std::fs::read_to_string(&args.schema)
        .map_err(|e| format!("read schema: {e}"))?;
    let schema: SemanticSchema = serde_yaml::from_str(&schema_str)
        .map_err(|e| format!("parse schema: {e}"))?;

    let (img_w, img_h) = screenshot.dimensions();
    let mut overlay = screenshot.to_rgba8();

    let font = FontRef::try_from_slice(FONT_BYTES)
        .map_err(|e| format!("font error: {e}"))?;

    // Auto-detect density from screenshot width vs schema viewport
    let density = args.density.unwrap_or_else(|| {
        let max_x = schema.elements.iter()
            .map(|e| (e.bounds.x + e.bounds.w) as f64)
            .fold(0.0f64, f64::max);
        if max_x > 0.0 {
            img_w as f64 / max_x
        } else {
            2.8 // fallback
        }
    });

    // iOS safe area: YAML coords start below notch, screenshot includes full screen
    let safe_top = args.safe_area_top.unwrap_or_else(|| {
        if schema.platform == "ios" { 47.0 } else { 0.0 }
    });

    for elem in &schema.elements {
        let x = (elem.bounds.x as f64 * density) as i32;
        let y = ((elem.bounds.y as f64 + safe_top) * density) as i32;
        let w = (elem.bounds.w as f64 * density) as u32;
        let h = (elem.bounds.h as f64 * density) as u32;

        if x < 0 || y < 0 || w == 0 || h == 0 {
            continue;
        }
        let x = x as u32;
        let y = y as u32;
        if x >= img_w || y >= img_h {
            continue;
        }
        let w = w.min(img_w - x);
        let h = h.min(img_h - y);

        // Semi-transparent fill based on type
        let fill = type_fill(&elem.elem_type);
        for dy in 0..h {
            for dx in 0..w {
                let px = x + dx;
                let py = y + dy;
                if px < img_w && py < img_h {
                    let existing = overlay.get_pixel(px, py);
                    let blended = blend(existing, &fill);
                    overlay.put_pixel(px, py, blended);
                }
            }
        }

        // Border
        let border = type_border(&elem.elem_type);
        let rect = Rect::at(x as i32, y as i32).of_size(w, h);
        draw_hollow_rect_mut(&mut overlay, rect, border);
        if w > 2 && h > 2 {
            let inner = Rect::at(x as i32 + 1, y as i32 + 1).of_size(w - 2, h - 2);
            draw_hollow_rect_mut(&mut overlay, inner, border);
        }

        // ID label
        let label = if !elem.id.is_empty() {
            &elem.id
        } else {
            elem.content.as_deref().unwrap_or("")
        };
        if !label.is_empty() {
            let label_size = 10.0;
            // Background for label readability
            let lw = (label.chars().count() as u32 * 6).min(w);
            let lh = 14u32;
            for dy in 0..lh.min(img_h.saturating_sub(y)) {
                for dx in 0..lw.min(img_w.saturating_sub(x)) {
                    overlay.put_pixel(x + dx, y + dy, Rgba([0, 0, 0, 180]));
                }
            }
            draw_text_mut(
                &mut overlay,
                Rgba([255, 255, 255, 255]),
                x as i32 + 2,
                y as i32 + 1,
                label_size,
                &font,
                label,
            );
        }
    }

    overlay.save(&args.output)
        .map_err(|e| format!("save error: {e}"))?;
    eprintln!("overlay: {} ({} elements, density {:.2})", args.output, schema.elements.len(), density);
    Ok(())
}

fn type_fill(elem_type: &str) -> Rgba<u8> {
    match elem_type {
        "button" => Rgba([0, 100, 255, 40]),
        "text" => Rgba([255, 200, 0, 30]),
        "image" => Rgba([0, 200, 0, 30]),
        "input" => Rgba([255, 100, 0, 40]),
        "container" => Rgba([150, 150, 150, 15]),
        "list" | "scroll" | "pager" => Rgba([100, 100, 200, 15]),
        _ => Rgba([100, 100, 100, 20]),
    }
}

fn type_border(elem_type: &str) -> Rgba<u8> {
    match elem_type {
        "button" => Rgba([0, 100, 255, 200]),
        "text" => Rgba([200, 150, 0, 200]),
        "image" => Rgba([0, 180, 0, 200]),
        "input" => Rgba([255, 100, 0, 200]),
        "container" => Rgba([150, 150, 150, 120]),
        "list" | "scroll" | "pager" => Rgba([100, 100, 200, 120]),
        _ => Rgba([100, 100, 100, 150]),
    }
}

fn blend(bg: &Rgba<u8>, fg: &Rgba<u8>) -> Rgba<u8> {
    let a = fg.0[3] as f32 / 255.0;
    let inv = 1.0 - a;
    Rgba([
        (fg.0[0] as f32 * a + bg.0[0] as f32 * inv) as u8,
        (fg.0[1] as f32 * a + bg.0[1] as f32 * inv) as u8,
        (fg.0[2] as f32 * a + bg.0[2] as f32 * inv) as u8,
        255,
    ])
}
