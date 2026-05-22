use clap::Args;
use image::{DynamicImage, GenericImageView, GrayImage, Luma, Rgb, RgbImage};

#[derive(Args)]
pub struct CompareArgs {
    /// First image (reference)
    pub source: String,
    /// Second image (comparison)
    pub target: String,

    /// Output diff image path
    #[arg(short, long, default_value = "/tmp/vdb-compare.png")]
    pub output: String,

    /// SSIM threshold (0.0-1.0, default 0.95)
    #[arg(long, default_value = "0.95")]
    pub threshold: f64,
}

pub fn run(args: CompareArgs) -> Result<(), String> {
    let src = image::open(&args.source).map_err(|e| format!("read {}: {e}", args.source))?;
    let tgt = image::open(&args.target).map_err(|e| format!("read {}: {e}", args.target))?;

    // Resize target to match source dimensions if needed
    let tgt = if src.dimensions() != tgt.dimensions() {
        let (w, h) = src.dimensions();
        eprintln!(
            "resizing target {}x{} → {}x{}",
            tgt.width(),
            tgt.height(),
            w,
            h
        );
        tgt.resize_exact(w, h, image::imageops::FilterType::Lanczos3)
    } else {
        tgt
    };

    let src_gray = src.to_luma8();
    let tgt_gray = tgt.to_luma8();

    let ssim = compute_ssim(&src_gray, &tgt_gray);
    let pixel_diff = compute_pixel_diff(&src.to_rgb8(), &tgt.to_rgb8());
    let diff_img = generate_diff_image(&src.to_rgb8(), &tgt.to_rgb8());

    diff_img
        .save(&args.output)
        .map_err(|e| format!("save error: {e}"))?;

    println!("SSIM: {:.4}", ssim);
    println!("pixel diff: {:.2}%", pixel_diff * 100.0);
    println!(
        "verdict: {}",
        if ssim >= args.threshold {
            "PASS"
        } else {
            "FAIL"
        }
    );
    println!("diff image: {}", args.output);

    if ssim < args.threshold {
        std::process::exit(1);
    }

    Ok(())
}

fn compute_ssim(src: &GrayImage, tgt: &GrayImage) -> f64 {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return 0.0;
    }

    let n = (w * h) as f64;

    let mut sum_src = 0.0f64;
    let mut sum_tgt = 0.0f64;
    let mut sum_src2 = 0.0f64;
    let mut sum_tgt2 = 0.0f64;
    let mut sum_st = 0.0f64;

    for y in 0..h {
        for x in 0..w {
            let s = src.get_pixel(x, y).0[0] as f64;
            let t = tgt.get_pixel(x, y).0[0] as f64;
            sum_src += s;
            sum_tgt += t;
            sum_src2 += s * s;
            sum_tgt2 += t * t;
            sum_st += s * t;
        }
    }

    let mu_s = sum_src / n;
    let mu_t = sum_tgt / n;
    let sigma_s2 = sum_src2 / n - mu_s * mu_s;
    let sigma_t2 = sum_tgt2 / n - mu_t * mu_t;
    let sigma_st = sum_st / n - mu_s * mu_t;

    let c1 = (0.01 * 255.0) * (0.01 * 255.0);
    let c2 = (0.03 * 255.0) * (0.03 * 255.0);

    let num = (2.0 * mu_s * mu_t + c1) * (2.0 * sigma_st + c2);
    let den = (mu_s * mu_s + mu_t * mu_t + c1) * (sigma_s2 + sigma_t2 + c2);

    num / den
}

fn compute_pixel_diff(src: &RgbImage, tgt: &RgbImage) -> f64 {
    let (w, h) = src.dimensions();
    let total = (w * h) as f64;
    if total == 0.0 {
        return 0.0;
    }

    let mut diff_count = 0u64;
    let threshold = 30u8;

    for y in 0..h {
        for x in 0..w {
            let sp = src.get_pixel(x, y);
            let tp = tgt.get_pixel(x, y);
            let dr = (sp.0[0] as i32 - tp.0[0] as i32).unsigned_abs() as u8;
            let dg = (sp.0[1] as i32 - tp.0[1] as i32).unsigned_abs() as u8;
            let db = (sp.0[2] as i32 - tp.0[2] as i32).unsigned_abs() as u8;
            if dr > threshold || dg > threshold || db > threshold {
                diff_count += 1;
            }
        }
    }

    diff_count as f64 / total
}

fn generate_diff_image(src: &RgbImage, tgt: &RgbImage) -> RgbImage {
    let (w, h) = src.dimensions();
    let mut diff = RgbImage::new(w * 3 + 4, h);

    // Left: source
    for y in 0..h {
        for x in 0..w {
            diff.put_pixel(x, y, *src.get_pixel(x, y));
        }
    }

    // Separator
    for y in 0..h {
        diff.put_pixel(w, y, Rgb([200, 200, 200]));
        diff.put_pixel(w + 1, y, Rgb([200, 200, 200]));
    }

    // Middle: target
    for y in 0..h {
        for x in 0..w {
            diff.put_pixel(w + 2 + x, y, *tgt.get_pixel(x, y));
        }
    }

    // Separator
    for y in 0..h {
        diff.put_pixel(w * 2 + 2, y, Rgb([200, 200, 200]));
        diff.put_pixel(w * 2 + 3, y, Rgb([200, 200, 200]));
    }

    // Right: diff highlight
    let threshold = 30u8;
    for y in 0..h {
        for x in 0..w {
            let sp = src.get_pixel(x, y);
            let tp = tgt.get_pixel(x, y);
            let dr = (sp.0[0] as i32 - tp.0[0] as i32).unsigned_abs() as u8;
            let dg = (sp.0[1] as i32 - tp.0[1] as i32).unsigned_abs() as u8;
            let db = (sp.0[2] as i32 - tp.0[2] as i32).unsigned_abs() as u8;
            let px = if dr > threshold || dg > threshold || db > threshold {
                Rgb([255, 0, 80])
            } else {
                // Desaturate matching pixels
                let gray = ((sp.0[0] as u16 + sp.0[1] as u16 + sp.0[2] as u16) / 3) as u8;
                Rgb([gray, gray, gray])
            };
            diff.put_pixel(w * 2 + 4 + x, y, px);
        }
    }

    diff
}
