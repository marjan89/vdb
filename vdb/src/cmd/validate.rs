use clap::Args;
use image::{GenericImageView, Rgb, RgbImage};

#[derive(Args)]
pub struct ValidateArgs {
    /// Device screenshot with agent overlay (red bounds)
    pub device_overlay: String,
    /// Schema-rendered image (blue bounds from vdb render)
    pub schema_render: String,

    /// Output composite image
    #[arg(short, long, default_value = "/tmp/vdb-validate.png")]
    pub output: String,

    /// Color channel threshold (0-255, how different R/G/B must be to detect overlay)
    #[arg(long, default_value = "30")]
    pub threshold: u8,

    /// Position tolerance in pixels (dilate comparison to handle slight offsets)
    #[arg(long, default_value = "6")]
    pub tolerance: u32,
}

pub fn run(args: ValidateArgs) -> Result<(), String> {
    let device = image::open(&args.device_overlay)
        .map_err(|e| format!("read device overlay: {e}"))?
        .to_rgb8();
    let render = image::open(&args.schema_render)
        .map_err(|e| format!("read schema render: {e}"))?;

    // Resize render to match device dimensions
    let (dw, dh) = device.dimensions();
    let render = render
        .resize_exact(dw, dh, image::imageops::FilterType::Lanczos3)
        .to_rgb8();

    let mut composite = RgbImage::new(dw, dh);
    let mut both = 0u64;
    let mut agent_only = 0u64;
    let mut yaml_only = 0u64;
    let mut neither = 0u64;
    let t = args.threshold;
    let tol = args.tolerance;

    // Pre-compute agent and yaml masks
    let is_agent_px = |x: u32, y: u32| -> bool {
        let dp = device.get_pixel(x, y);
        dp.0[0] > dp.0[1].saturating_add(t) && dp.0[0] > dp.0[2].saturating_add(t)
    };
    let is_yaml_px = |x: u32, y: u32| -> bool {
        let rp = render.get_pixel(x, y);
        rp.0[2] > rp.0[0].saturating_add(t) && rp.0[2] > rp.0[1].saturating_add(t)
    };

    // Check within tolerance radius
    let has_nearby_agent = |cx: u32, cy: u32| -> bool {
        let x0 = cx.saturating_sub(tol);
        let y0 = cy.saturating_sub(tol);
        let x1 = (cx + tol).min(dw - 1);
        let y1 = (cy + tol).min(dh - 1);
        for sy in y0..=y1 {
            for sx in x0..=x1 {
                if is_agent_px(sx, sy) { return true; }
            }
        }
        false
    };
    let has_nearby_yaml = |cx: u32, cy: u32| -> bool {
        let x0 = cx.saturating_sub(tol);
        let y0 = cy.saturating_sub(tol);
        let x1 = (cx + tol).min(dw - 1);
        let y1 = (cy + tol).min(dh - 1);
        for sy in y0..=y1 {
            for sx in x0..=x1 {
                if is_yaml_px(sx, sy) { return true; }
            }
        }
        false
    };

    for y in 0..dh {
        for x in 0..dw {
            let is_agent = is_agent_px(x, y);
            let is_yaml = is_yaml_px(x, y);

            // With tolerance: agent pixel matches if yaml nearby, and vice versa
            let agent_matched = is_agent && has_nearby_yaml(x, y);
            let yaml_matched = is_yaml && has_nearby_agent(x, y);

            let color = if agent_matched || yaml_matched || (is_agent && is_yaml) {
                both += 1;
                Rgb([128, 0, 128])
            } else if is_agent {
                agent_only += 1;
                Rgb([255, 0, 0])
            } else if is_yaml {
                yaml_only += 1;
                Rgb([0, 0, 255])
            } else {
                neither += 1;
                *device.get_pixel(x, y)
            };
            composite.put_pixel(x, y, color);
        }
    }

    composite
        .save(&args.output)
        .map_err(|e| format!("save error: {e}"))?;

    let total_marked = both + agent_only + yaml_only;
    let match_pct = if total_marked > 0 {
        both as f64 / total_marked as f64 * 100.0
    } else {
        100.0
    };

    println!("BOUNDS VALIDATION:");
    println!("  match (purple): {} px ({:.1}%)", both, match_pct);
    println!("  agent-only (red): {} px — YAML missed these", agent_only);
    println!("  yaml-only (blue): {} px — phantom/wrong bounds", yaml_only);
    println!("  verdict: {}", if match_pct > 80.0 { "PASS" } else { "FAIL" });
    println!("  composite: {}", args.output);

    if match_pct < 80.0 {
        std::process::exit(1);
    }

    Ok(())
}
