mod draw;
mod icons;
mod text;

use ab_glyph::{FontRef, FontVec};
use clap::Args;
use image::{Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;

use crate::schema::{SemanticElement, SemanticSchema};

#[derive(Args)]
pub struct RenderArgs {
    /// Input YAML schema file(s)
    pub input: Vec<String>,

    /// Output PNG path
    #[arg(short, long, default_value = "/tmp/vdb-render.png")]
    pub output: String,

    /// Scale factor (default 1.0, use 2.0 for retina)
    #[arg(long, default_value = "1.0")]
    pub scale: f32,

    /// Font directory (loads .ttf files for accurate text metrics)
    #[arg(long)]
    pub font_dir: Option<String>,

    /// Android source root for raster icon loading (res/ directory)
    #[arg(long)]
    pub source_root: Option<String>,

    /// Validation mode: filled blue rectangles only
    #[arg(long)]
    pub validate: bool,

    /// Viewport width in dp (auto-detected if omitted)
    #[arg(long)]
    pub viewport_width: Option<u32>,

    /// Viewport height in dp (auto-detected if omitted)
    #[arg(long)]
    pub viewport_height: Option<u32>,
}

static FONT_REGULAR: &[u8] = include_bytes!("/System/Library/Fonts/Helvetica.ttc");

pub fn run(args: RenderArgs) -> Result<(), String> {
    if args.input.is_empty() {
        return Err("no input files".to_string());
    }

    let schemas: Vec<SemanticSchema> = args
        .input
        .iter()
        .map(|path| {
            let content =
                std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
            serde_yaml::from_str(&content).map_err(|e| format!("parse {path}: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let custom_font = args.font_dir.as_ref().and_then(|dir| text::load_font_dir(dir));
    let icon_cache = args
        .source_root
        .as_ref()
        .map(|root| icons::IconCache::scan(root));

    let s = args.scale;

    if args.validate {
        let img = render_validate(&schemas[0], s, &args)?;
        img.save(&args.output)
            .map_err(|e| format!("save error: {e}"))?;
        eprintln!("rendered (validate mode) {}", args.output);
        return Ok(());
    }

    if schemas.len() == 1 {
        let img = render_schema(
            &schemas[0],
            s,
            custom_font.as_ref(),
            icon_cache.as_ref(),
            &args,
        )?;
        img.save(&args.output)
            .map_err(|e| format!("save error: {e}"))?;
    } else {
        let images: Vec<RgbImage> = schemas
            .iter()
            .map(|schema| {
                render_schema(schema, s, custom_font.as_ref(), icon_cache.as_ref(), &args)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let gap = (4.0 * s) as u32;
        let total_w: u32 =
            images.iter().map(|i| i.width()).sum::<u32>() + (images.len() as u32 - 1) * gap;
        let max_h = images.iter().map(|i| i.height()).max().unwrap_or(100);
        let mut combined = RgbImage::from_pixel(total_w, max_h, Rgb([240, 240, 240]));
        let mut x_offset = 0u32;
        for img in &images {
            for y in 0..img.height() {
                for x in 0..img.width() {
                    combined.put_pixel(x_offset + x, y, *img.get_pixel(x, y));
                }
            }
            x_offset += img.width() + gap;
        }
        combined
            .save(&args.output)
            .map_err(|e| format!("save error: {e}"))?;
    }

    eprintln!("rendered {}", args.output);
    Ok(())
}

fn render_validate(
    schema: &SemanticSchema,
    scale: f32,
    args: &RenderArgs,
) -> Result<RgbImage, String> {
    let default_w = args.viewport_width.unwrap_or(if schema.platform == "ios" {
        390
    } else {
        384
    });
    let default_h = args.viewport_height.unwrap_or(if schema.platform == "ios" {
        844
    } else {
        832
    });
    let w = (default_w as f32 * scale) as u32;
    let h = (default_h as f32 * scale) as u32;

    let mut img = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
    let blue = Rgb([0, 0, 200]);
    let blue_fill = Rgb([80, 80, 255]);

    for elem in &schema.elements {
        if matches!(
            elem.elem_type.as_str(),
            "container" | "list" | "scroll" | "pager" | "view"
        ) {
            continue;
        }
        if elem.bounds.y as u32 >= default_h {
            continue;
        }

        let x = (elem.bounds.x.max(0) as f32 * scale) as u32;
        let y = (elem.bounds.y.max(0) as f32 * scale) as u32;
        let ew = (elem.bounds.w as f32 * scale) as u32;
        let eh = (elem.bounds.h as f32 * scale) as u32;

        if x >= w || y >= h || ew == 0 || eh == 0 {
            continue;
        }
        let ew = ew.min(w - x);
        let eh = eh.min(h - y);

        let rect = Rect::at(x as i32, y as i32).of_size(ew, eh);
        draw_filled_rect_mut(&mut img, rect, blue_fill);
        imageproc::drawing::draw_hollow_rect_mut(&mut img, rect, blue);
    }

    Ok(img)
}

fn render_schema(
    schema: &SemanticSchema,
    scale: f32,
    custom_font: Option<&FontVec>,
    icon_cache: Option<&icons::IconCache>,
    args: &RenderArgs,
) -> Result<RgbImage, String> {
    let default_w = args.viewport_width.unwrap_or(if schema.platform == "ios" {
        390
    } else {
        384
    });
    let default_h = args.viewport_height.unwrap_or_else(|| {
        viewport_height_from_elements(schema, if schema.platform == "ios" { 844 } else { 832 })
    });

    let w = (default_w as f32 * scale) as u32;
    let h = (default_h as f32 * scale) as u32;

    let mut img = RgbImage::from_pixel(w, h, Rgb([250, 250, 250]));

    let fallback =
        FontRef::try_from_slice(FONT_REGULAR).map_err(|e| format!("font error: {e}"))?;

    for elem in &schema.elements {
        if let Some(cf) = custom_font {
            render_element(&mut img, elem, cf, scale, w, icon_cache);
        } else {
            render_element(&mut img, elem, &fallback, scale, w, icon_cache);
        }
    }

    if schema.platform == "android" {
        let sb_h = (25.0 * scale) as u32;
        let rect = Rect::at(0, 0).of_size(w, sb_h);
        draw_filled_rect_mut(&mut img, rect, Rgb([230, 230, 230]));
    }

    let title = format!(
        "{} — {} ({})",
        schema.screen, schema.device, schema.platform
    );
    draw_text_mut(
        &mut img,
        Rgb([80, 80, 80]),
        4,
        2,
        12.0 * scale,
        &fallback,
        &title,
    );

    Ok(img)
}

fn viewport_height_from_elements(schema: &SemanticSchema, default: u32) -> u32 {
    schema
        .elements
        .iter()
        .filter(|e| e.clickable)
        .map(|e| (e.bounds.y + e.bounds.h) as u32 + 10)
        .max()
        .unwrap_or(default)
        .min(default + 100)
}

fn render_element(
    img: &mut RgbImage,
    elem: &SemanticElement,
    font: &impl ab_glyph::Font,
    scale: f32,
    _canvas_w: u32,
    icon_cache: Option<&icons::IconCache>,
) {
    let b = &elem.bounds;
    if b.w <= 0 || b.h <= 0 {
        return;
    }

    let x = (b.x.max(0) as f32 * scale) as u32;
    let y = (b.y.max(0) as f32 * scale) as u32;
    let w = (b.w as f32 * scale) as u32;
    let h = (b.h as f32 * scale) as u32;

    let img_w = img.width();
    let img_h = img.height();
    if x >= img_w || y >= img_h {
        return;
    }
    let w = w.min(img_w.saturating_sub(x));
    let h = h.min(img_h.saturating_sub(y));
    if w == 0 || h == 0 {
        return;
    }

    if let Some(ref bg) = elem.background {
        if let Some(color) = draw::parse_hex(bg) {
            draw::fill_bg(img, x, y, w, h, color);
        }
    }

    if elem.elem_type == "button" && elem.background.is_none() && elem.children.is_none() {
        if w < (200.0 * scale) as u32 && h < (80.0 * scale) as u32 {
            draw::fill_bg(img, x, y, w, h, Rgb([235, 243, 255]));
        }
    }

    draw::draw_border(
        img,
        x,
        y,
        w,
        h,
        draw::type_color(&elem.elem_type),
        elem.clickable,
    );

    let content = if elem.children.is_some() {
        None
    } else {
        elem.content.as_ref().map(|c| draw::decode_entities(c))
    };

    if let Some(ref icon) = elem.icon {
        if icon.format == "vector" && !icon.paths.is_empty() {
            let icon_color = elem
                .color
                .as_ref()
                .and_then(|c| draw::parse_hex(c))
                .unwrap_or(Rgb([60, 60, 60]));
            icons::render_svg_paths(img, &icon.paths, x, y, w, h, icon_color);
        } else if matches!(icon.format.as_str(), "raster" | "xcassets" | "unknown") {
            let loaded = icon_cache.and_then(|ic| ic.get(&icon.name));
            if let Some(src_img) = loaded {
                icons::render_raster_icon(img, src_img, x, y, w, h);
            } else {
                draw::fill_bg(img, x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2), Rgb([220, 220, 220]));
            }
        }
    }

    if let Some(ref txt) = content {
        let raw_color = elem
            .color
            .as_ref()
            .and_then(|c| draw::parse_hex(c))
            .unwrap_or(Rgb([20, 20, 20]));

        let luminance = raw_color.0[0] as u32 + raw_color.0[1] as u32 + raw_color.0[2] as u32;
        let text_color = if luminance > 680 {
            Rgb([40, 40, 40])
        } else {
            raw_color
        };

        let base_size = elem
            .font
            .as_ref()
            .map(|f| if f.size > 0.0 { f.size as f32 } else { 12.0 })
            .unwrap_or(12.0)
            .max(8.0)
            * scale;

        let pad = (3.0 * scale) as i32;
        let max_w = w as i32 - pad * 2;
        let max_h = h as i32 - pad * 2;

        let font_size = text::shrink_to_fit(base_size, txt, max_w, max_h, scale);

        let text_x = x as i32 + pad;
        let text_y = y as i32 + pad;

        if text_x >= 0 && text_y >= 0 && max_w > 0 {
            text::draw_wrapped_clipped(img, text_color, text_x, text_y, font_size, max_w, max_h, font, txt);
        }

        if elem.color.is_some() {
            let ss = (3.0 * scale) as u32;
            if x + ss < img_w && y + ss < img_h {
                let sr = Rect::at(x as i32, y as i32).of_size(ss, ss);
                draw_filled_rect_mut(img, sr, text_color);
            }
        }
    }

    if !elem.id.is_empty() && content.is_none() && elem.icon.is_none() {
        let label_size = 7.0 * scale;
        let label_y = (y + h / 2) as i32;
        draw_text_mut(
            img,
            Rgb([160, 160, 160]),
            x as i32 + 2,
            label_y,
            label_size,
            font,
            &elem.id,
        );
    }
}
