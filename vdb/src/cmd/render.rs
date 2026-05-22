use clap::Args;
use ab_glyph::{Font as AbFont, FontRef, FontVec};
use image::{GenericImageView, Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use lyon_path::Path as LyonPath;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers,
};

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

    /// Validation mode: filled blue rectangles only (for vdb validate comparison)
    #[arg(long)]
    pub validate: bool,
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

    let custom_font = args.font_dir.as_ref().and_then(|dir| load_font_dir(dir));
    let icon_cache = args.source_root.as_ref().map(|root| IconCache::scan(root));

    let s = args.scale;

    if args.validate {
        let img = render_validate(&schemas[0], s)?;
        img.save(&args.output).map_err(|e| format!("save error: {e}"))?;
        eprintln!("rendered (validate mode) {}", args.output);
        return Ok(());
    }

    if schemas.len() == 1 {
        let img = render_schema(&schemas[0], s, custom_font.as_ref(), icon_cache.as_ref())?;
        img.save(&args.output)
            .map_err(|e| format!("save error: {e}"))?;
    } else {
        let images: Vec<RgbImage> = schemas
            .iter()
            .map(|schema| render_schema(schema, s, custom_font.as_ref(), icon_cache.as_ref()))
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

fn render_validate(schema: &SemanticSchema, scale: f32) -> Result<RgbImage, String> {
    let default_w = if schema.platform == "ios" { 390u32 } else { 384u32 };
    let default_h = if schema.platform == "ios" { 844u32 } else { 832u32 };
    // Canvas matches device screen — no safe area offset (YAML coords already in screen space)
    let w = (default_w as f32 * scale) as u32;
    let h = (default_h as f32 * scale) as u32;

    let mut img = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
    let blue = Rgb([0, 0, 200]);
    let blue_fill = Rgb([80, 80, 255]);

    for elem in &schema.elements {
        if matches!(elem.elem_type.as_str(), "container" | "list" | "scroll" | "pager" | "view") {
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

        // Filled blue rectangle
        let rect = Rect::at(x as i32, y as i32).of_size(ew, eh);
        draw_filled_rect_mut(&mut img, rect, blue_fill);
        draw_hollow_rect_mut(&mut img, rect, blue);

        if let Some(ref children) = elem.children {
            for child in children {
                let cx = (child.bounds.x.max(0) as f32 * scale) as u32;
                let cy = (child.bounds.y.max(0) as f32 * scale) as u32;
                let cw = (child.bounds.w as f32 * scale) as u32;
                let ch = (child.bounds.h as f32 * scale) as u32;
                if cx >= w || cy >= h || cw == 0 || ch == 0 { continue; }
                let cw = cw.min(w - cx);
                let ch = ch.min(h - cy);
                let cr = Rect::at(cx as i32, cy as i32).of_size(cw, ch);
                draw_filled_rect_mut(&mut img, cr, blue_fill);
                draw_hollow_rect_mut(&mut img, cr, blue);
            }
        }
    }

    Ok(img)
}

fn load_font_dir(dir: &str) -> Option<FontVec> {
    // Try to load poppins_regular first, then any .ttf
    let candidates = ["poppins_regular.ttf", "poppins_medium.ttf", "poppins_semi_bold.ttf"];
    for name in &candidates {
        let path = std::path::Path::new(dir).join(name);
        if let Ok(data) = std::fs::read(&path) {
            if let Ok(font) = FontVec::try_from_vec(data) {
                eprintln!("loaded font: {name}");
                return Some(font);
            }
        }
    }
    // Fallback: first .ttf in dir
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("ttf") {
                if let Ok(data) = std::fs::read(&path) {
                    if let Ok(font) = FontVec::try_from_vec(data) {
                        eprintln!("loaded font: {}", path.display());
                        return Some(font);
                    }
                }
            }
        }
    }
    None
}

struct IconCache {
    icons: std::collections::HashMap<String, image::DynamicImage>,
}

impl IconCache {
    fn scan(source_root: &str) -> Self {
        let mut icons = std::collections::HashMap::new();
        let root = std::path::Path::new(source_root);

        // Android: walk drawable dirs, prefer highest density
        for dir_name in &["drawable-xxxhdpi", "drawable-xxhdpi", "drawable-xhdpi", "drawable-hdpi", "drawable-mdpi", "drawable"] {
            let dir = root.join(dir_name);
            if !dir.exists() { continue; }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !matches!(ext, "png" | "jpg" | "webp") { continue; }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.is_empty() || icons.contains_key(stem) { continue; }
                if let Ok(img) = image::open(&path) {
                    icons.insert(stem.to_string(), img);
                }
            }
        }

        // iOS: walk Assets.xcassets/<name>.imageset/Contents.json
        Self::scan_xcassets(root, &mut icons);

        // Also search parent/sibling dirs for xcassets
        if let Some(parent) = root.parent() {
            Self::scan_xcassets(parent, &mut icons);
            // Walk one level deeper for common patterns like Naturkartan/Assets.xcassets
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                        Self::scan_xcassets(&entry.path(), &mut icons);
                    }
                }
            }
        }

        eprintln!("loaded {} raster icons", icons.len());
        Self { icons }
    }

    fn scan_xcassets(root: &std::path::Path, icons: &mut std::collections::HashMap<String, image::DynamicImage>) {
        // Find all .xcassets directories
        for entry in walkdir::WalkDir::new(root)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.ends_with(".imageset") { continue; }

            let asset_name = name.strip_suffix(".imageset").unwrap_or(name);
            if icons.contains_key(asset_name) { continue; }

            let contents = path.join("Contents.json");
            if !contents.exists() { continue; }

            // Parse Contents.json to find the best image file
            if let Ok(json_str) = std::fs::read_to_string(&contents) {
                if let Some(filename) = Self::best_image_from_contents(&json_str) {
                    let img_path = path.join(&filename);
                    if let Ok(img) = image::open(&img_path) {
                        icons.insert(asset_name.to_string(), img);
                    }
                }
            }
        }
    }

    fn best_image_from_contents(json: &str) -> Option<String> {
        // Parse Contents.json — prefer 2x, then 3x, then 1x
        #[derive(serde::Deserialize)]
        struct Contents { images: Vec<ImageEntry> }
        #[derive(serde::Deserialize)]
        struct ImageEntry {
            #[serde(default)]
            filename: Option<String>,
            #[serde(default)]
            scale: Option<String>,
        }

        let contents: Contents = serde_json::from_str(json).ok()?;
        let mut best: Option<&ImageEntry> = None;
        let priority = |s: &str| match s { "2x" => 3, "3x" => 2, "1x" => 1, _ => 0 };

        for img in &contents.images {
            if img.filename.is_none() { continue; }
            if best.is_none() {
                best = Some(img);
            } else if let (Some(bs), Some(is)) = (&best.unwrap().scale, &img.scale) {
                if priority(is) > priority(bs) {
                    best = Some(img);
                }
            }
        }

        best.and_then(|b| b.filename.clone())
    }

    fn get(&self, name: &str) -> Option<&image::DynamicImage> {
        self.icons.get(name)
    }
}

fn render_schema(schema: &SemanticSchema, scale: f32, custom_font: Option<&FontVec>, icon_cache: Option<&IconCache>) -> Result<RgbImage, String> {
    // Compute canvas size — use viewport height, not full scroll content
    let default_w = if schema.platform == "ios" { 390u32 } else { 384u32 };
    let default_h = if schema.platform == "ios" { 844u32 } else { 832u32 };

    // Find viewport height from bottom nav position (fixed elements near screen bottom)
    let viewport_h = schema.elements.iter()
        .filter(|e| {
            let id = e.id.as_str();
            matches!(id, "discover" | "search" | "inspiration" | "my_page" |
                "discovertouch" | "searchtouch" | "inspirationtouch" | "profiletouch" |
                "discoverTextView" | "searchTextView")
                || e.platform_id.as_deref().map_or(false, |p|
                    p.contains("bottomNav") || p.contains("BottomNav") || p == "Discover" || p == "Search")
        })
        .map(|e| (e.bounds.y + e.bounds.h) as u32 + 10)
        .max()
        .unwrap_or(default_h);

    let (content_w, _) = schema
        .elements
        .iter()
        .fold((default_w, default_h), |(mw, mh), e| {
            let ex = (e.bounds.x + e.bounds.w).max(0) as u32;
            let ey = (e.bounds.y + e.bounds.h).max(0) as u32;
            (mw.max(ex.min(default_w + 10)), mh.max(ey + 10))
        });

    let content_h = viewport_h;

    let w = (content_w as f32 * scale) as u32;
    let h = (content_h as f32 * scale) as u32;

    let mut img = RgbImage::from_pixel(w, h, Rgb([250, 250, 250]));

    let fallback =
        FontRef::try_from_slice(FONT_REGULAR).map_err(|e| format!("font error: {e}"))?;

    for elem in &schema.elements {
        if let Some(cf) = custom_font {
            render_element_with_font(&mut img, elem, cf, scale, w, icon_cache);
        } else {
            render_element_with_font(&mut img, elem, &fallback, scale, w, icon_cache);
        }
        if let Some(ref children) = elem.children {
            for child in children {
                if let Some(cf) = custom_font {
                    render_element_with_font(&mut img, child, cf, scale, w, icon_cache);
                } else {
                    render_element_with_font(&mut img, child, &fallback, scale, w, icon_cache);
                }
            }
        }
    }

    // Status bar region for Android
    if schema.platform == "android" {
        let sb_h = (25.0 * scale) as u32;
        let rect = Rect::at(0, 0).of_size(w, sb_h);
        draw_filled_rect_mut(&mut img, rect, Rgb([230, 230, 230]));
    }

    let title = format!(
        "{} — {} ({})",
        schema.screen, schema.device, schema.platform
    );
    draw_text_mut(&mut img, Rgb([80, 80, 80]), 4, 2, 12.0 * scale, &fallback, &title);

    Ok(img)
}

fn render_element_with_font(
    img: &mut RgbImage,
    elem: &SemanticElement,
    font: &impl ab_glyph::Font,
    scale: f32,
    canvas_w: u32,
    icon_cache: Option<&IconCache>,
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
    // Clip to canvas
    let w = w.min(img_w.saturating_sub(x));
    let h = h.min(img_h.saturating_sub(y));
    if w == 0 || h == 0 {
        return;
    }

    // Fill background
    if let Some(ref bg) = elem.background {
        if let Some(color) = parse_hex(bg) {
            let rect = Rect::at(x as i32, y as i32).of_size(w, h);
            draw_filled_rect_mut(img, rect, color);
        }
    }

    // Button tint — skip for compound elements with children (they'd paint over siblings)
    if elem.elem_type == "button" && elem.background.is_none() && elem.children.is_none() {
        // Only tint small buttons (< 200dp in any dimension)
        if w < (200.0 * scale) as u32 && h < (80.0 * scale) as u32 {
            let rect = Rect::at(x as i32, y as i32).of_size(w, h);
            draw_filled_rect_mut(img, rect, Rgb([235, 243, 255]));
        }
    }

    // Border
    let border_color = type_color(&elem.elem_type);
    let rect = Rect::at(x as i32, y as i32).of_size(w, h);
    draw_hollow_rect_mut(img, rect, border_color);
    if elem.clickable && w > 2 && h > 2 {
        let inner = Rect::at(x as i32 + 1, y as i32 + 1).of_size(w - 2, h - 2);
        draw_hollow_rect_mut(img, inner, border_color);
    }

    // Skip parent content text when children exist — render children only
    let content = if elem.children.is_some() {
        None
    } else {
        elem.content.as_ref().map(|c| decode_entities(c))
    };

    // Render icon shapes for vector icons
    if let Some(ref icon) = elem.icon {
        if icon.format == "vector" && !icon.paths.is_empty() {
            let icon_color = elem
                .color
                .as_ref()
                .and_then(|c| parse_hex(c))
                .unwrap_or(Rgb([60, 60, 60]));
            render_svg_paths(img, &icon.paths, x, y, w, h, icon_color);
        } else if matches!(icon.format.as_str(), "raster" | "xcassets" | "unknown") {
            let loaded = icon_cache.and_then(|ic| ic.get(&icon.name));
            if let Some(src_img) = loaded {
                let (nw, nh) = (src_img.width(), src_img.height());
                let icon_w = w.min(nw).min(h * nw / nh.max(1));
                let icon_h = h.min(nh).min(w * nh / nw.max(1));
                let resized = src_img.resize(icon_w.max(1), icon_h.max(1), image::imageops::FilterType::Lanczos3);
                let rgba = resized.to_rgba8();
                let rw = rgba.width();
                let rh = rgba.height();
                let ox = (w.saturating_sub(rw)) / 2;
                let oy = (h.saturating_sub(rh)) / 2;
                for dy in 0..rh.min(img_h.saturating_sub(y + oy)) {
                    for dx in 0..rw.min(img_w.saturating_sub(x + ox)) {
                        let px = rgba.get_pixel(dx, dy);
                        let a = px.0[3] as f32 / 255.0;
                        if a < 0.01 { continue; }
                        let bg = img.get_pixel(x + ox + dx, y + oy + dy);
                        let inv = 1.0 - a;
                        let blended = Rgb([
                            (px.0[0] as f32 * a + bg.0[0] as f32 * inv) as u8,
                            (px.0[1] as f32 * a + bg.0[1] as f32 * inv) as u8,
                            (px.0[2] as f32 * a + bg.0[2] as f32 * inv) as u8,
                        ]);
                        img.put_pixel(x + ox + dx, y + oy + dy, blended);
                    }
                }
            } else {
                let rect = Rect::at(x as i32 + 1, y as i32 + 1).of_size(w.saturating_sub(2), h.saturating_sub(2));
                draw_filled_rect_mut(img, rect, Rgb([220, 220, 220]));
            }
        }
    }

    // Render text with wrapping
    if let Some(ref text) = content {
        let raw_color = elem
            .color
            .as_ref()
            .and_then(|c| parse_hex(c))
            .unwrap_or(Rgb([20, 20, 20]));

        // If text is near-white on white canvas, use dark fallback
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

        // Shrink font to fit bounds if text would overflow badly
        let font_size = if max_w > 0 && max_h > 0 {
            let char_w = (base_size * 0.55) as i32;
            let line_h = (base_size * 1.3) as i32;
            if char_w > 0 && line_h > 0 {
                let chars_per_line = (max_w / char_w).max(1) as usize;
                let text_chars = text.chars().count();
                let lines_needed = (text_chars + chars_per_line - 1) / chars_per_line;
                let lines_available = (max_h / line_h).max(1) as usize;
                if lines_needed > lines_available && lines_available <= 2 {
                    // Shrink to fit single line or available space
                    let needed_char_w = max_w as f32 / text_chars.max(1) as f32;
                    (needed_char_w / 0.55).max(6.0 * scale).min(base_size)
                } else {
                    base_size
                }
            } else {
                base_size
            }
        } else {
            base_size
        };

        let text_x = x as i32 + pad;
        let text_y = y as i32 + pad;

        if text_x >= 0 && text_y >= 0 && max_w > 0 {
            draw_wrapped_text_clipped(img, text_color, text_x, text_y, font_size, max_w, max_h, font, text);
        }

        // Color swatch
        if elem.color.is_some() {
            let ss = (3.0 * scale) as u32;
            if x + ss < img_w && y + ss < img_h {
                let sr = Rect::at(x as i32, y as i32).of_size(ss, ss);
                draw_filled_rect_mut(img, sr, text_color);
            }
        }
    }

    // ID label for non-text, non-icon elements
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

fn draw_wrapped_text(
    img: &mut RgbImage,
    color: Rgb<u8>,
    x: i32,
    y: i32,
    font_size: f32,
    max_width: i32,
    font: &impl ab_glyph::Font,
    text: &str,
) {
    draw_wrapped_text_clipped(img, color, x, y, font_size, max_width, i32::MAX, font, text);
}

fn draw_wrapped_text_clipped(
    img: &mut RgbImage,
    color: Rgb<u8>,
    x: i32,
    y: i32,
    font_size: f32,
    max_width: i32,
    max_height: i32,
    font: &impl ab_glyph::Font,
    text: &str,
) {
    let char_width = (font_size * 0.55) as i32;
    if char_width <= 0 {
        return;
    }
    let chars_per_line = (max_width / char_width).max(1) as usize;
    let line_height = (font_size * 1.3) as i32;
    let max_y = y + max_height;

    let mut cy = y;
    let mut remaining = text;
    let mut first_line = true;

    while !remaining.is_empty() {
        // Always render at least the first line even if it overflows bounds
        if !first_line && cy + line_height > max_y {
            break;
        }
        first_line = false;

        let char_count = remaining.chars().count();
        let line = if char_count <= chars_per_line {
            remaining
        } else {
            // Find byte offset of chars_per_line-th char
            let byte_end = remaining
                .char_indices()
                .nth(chars_per_line)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
            let break_at = remaining[..byte_end]
                .rfind(' ')
                .unwrap_or(byte_end);
            &remaining[..break_at]
        };

        if cy >= 0 && (cy as u32) < img.height() {
            draw_text_mut(img, color, x, cy, font_size, font, line);
        }

        remaining = &remaining[line.len()..];
        remaining = remaining.trim_start();
        cy += line_height;

        if cy as u32 >= img.height() {
            break;
        }
    }
}

fn render_svg_paths(
    img: &mut RgbImage,
    paths: &[String],
    dest_x: u32,
    dest_y: u32,
    dest_w: u32,
    dest_h: u32,
    color: Rgb<u8>,
) {
    for path_data in paths {
        if let Some(lyon_path) = parse_svg_path(path_data) {
            let mut buffers: VertexBuffers<[f32; 2], u16> = VertexBuffers::new();
            let mut tessellator = FillTessellator::new();
            let result = tessellator.tessellate_path(
                &lyon_path,
                &FillOptions::default(),
                &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
                    vertex.position().to_array()
                }),
            );
            if result.is_err() {
                continue;
            }

            // Find path bounds for scaling
            let (mut min_x, mut min_y, mut max_x, mut max_y) =
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for v in &buffers.vertices {
                min_x = min_x.min(v[0]);
                min_y = min_y.min(v[1]);
                max_x = max_x.max(v[0]);
                max_y = max_y.max(v[1]);
            }
            let path_w = (max_x - min_x).max(1.0);
            let path_h = (max_y - min_y).max(1.0);
            let scale_x = dest_w as f32 / path_w;
            let scale_y = dest_h as f32 / path_h;
            let s = scale_x.min(scale_y) * 0.8;
            let offset_x = dest_x as f32 + (dest_w as f32 - path_w * s) / 2.0;
            let offset_y = dest_y as f32 + (dest_h as f32 - path_h * s) / 2.0;

            // Rasterize triangles
            for tri in buffers.indices.chunks(3) {
                if tri.len() < 3 {
                    continue;
                }
                let v0 = buffers.vertices[tri[0] as usize];
                let v1 = buffers.vertices[tri[1] as usize];
                let v2 = buffers.vertices[tri[2] as usize];

                let pts: [(f32, f32); 3] = [
                    (
                        (v0[0] - min_x) * s + offset_x,
                        (v0[1] - min_y) * s + offset_y,
                    ),
                    (
                        (v1[0] - min_x) * s + offset_x,
                        (v1[1] - min_y) * s + offset_y,
                    ),
                    (
                        (v2[0] - min_x) * s + offset_x,
                        (v2[1] - min_y) * s + offset_y,
                    ),
                ];

                fill_triangle(img, &pts, color);
            }
        }
    }
}

fn fill_triangle(img: &mut RgbImage, pts: &[(f32, f32); 3], color: Rgb<u8>) {
    let min_y = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min) as i32;
    let max_y = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max) as i32;
    let min_x = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min) as i32;
    let max_x = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max) as i32;

    let (iw, ih) = (img.width() as i32, img.height() as i32);

    for y in min_y.max(0)..=max_y.min(ih - 1) {
        for x in min_x.max(0)..=max_x.min(iw - 1) {
            if point_in_triangle(x as f32 + 0.5, y as f32 + 0.5, pts) {
                img.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}

fn point_in_triangle(px: f32, py: f32, pts: &[(f32, f32); 3]) -> bool {
    let (x1, y1) = pts[0];
    let (x2, y2) = pts[1];
    let (x3, y3) = pts[2];
    let d1 = (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2);
    let d2 = (px - x3) * (y2 - y3) - (x2 - x3) * (py - y3);
    let d3 = (px - x1) * (y3 - y1) - (x3 - x1) * (py - y1);
    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(has_neg && has_pos)
}

fn parse_svg_path(d: &str) -> Option<LyonPath> {
    use lyon_path::math::point;

    let mut builder = LyonPath::builder();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut started = false;

    let tokens = tokenize_svg(d);
    let mut i = 0;

    while i < tokens.len() {
        match tokens[i].as_str() {
            "M" => {
                if i + 2 < tokens.len() {
                    cx = tokens[i + 1].parse().unwrap_or(0.0);
                    cy = tokens[i + 2].parse().unwrap_or(0.0);
                    if started {
                        builder.end(false);
                    }
                    builder.begin(point(cx, cy));
                    started = true;
                    i += 3;
                    // Implicit lineto after M
                    while i + 1 < tokens.len() {
                        if let (Ok(x), Ok(y)) = (tokens[i].parse::<f32>(), tokens[i + 1].parse::<f32>()) {
                            cx = x;
                            cy = y;
                            builder.line_to(point(cx, cy));
                            i += 2;
                        } else {
                            break;
                        }
                    }
                } else {
                    i += 1;
                }
            }
            "m" => {
                if i + 2 < tokens.len() {
                    cx += tokens[i + 1].parse::<f32>().unwrap_or(0.0);
                    cy += tokens[i + 2].parse::<f32>().unwrap_or(0.0);
                    if started {
                        builder.end(false);
                    }
                    builder.begin(point(cx, cy));
                    started = true;
                    i += 3;
                } else {
                    i += 1;
                }
            }
            "L" => {
                if i + 2 < tokens.len() {
                    cx = tokens[i + 1].parse().unwrap_or(0.0);
                    cy = tokens[i + 2].parse().unwrap_or(0.0);
                    builder.line_to(point(cx, cy));
                    i += 3;
                } else {
                    i += 1;
                }
            }
            "l" => {
                if i + 2 < tokens.len() {
                    cx += tokens[i + 1].parse::<f32>().unwrap_or(0.0);
                    cy += tokens[i + 2].parse::<f32>().unwrap_or(0.0);
                    builder.line_to(point(cx, cy));
                    i += 3;
                } else {
                    i += 1;
                }
            }
            "H" => {
                if i + 1 < tokens.len() {
                    cx = tokens[i + 1].parse().unwrap_or(cx);
                    builder.line_to(point(cx, cy));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "h" => {
                if i + 1 < tokens.len() {
                    cx += tokens[i + 1].parse::<f32>().unwrap_or(0.0);
                    builder.line_to(point(cx, cy));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "V" => {
                if i + 1 < tokens.len() {
                    cy = tokens[i + 1].parse().unwrap_or(cy);
                    builder.line_to(point(cx, cy));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "v" => {
                if i + 1 < tokens.len() {
                    cy += tokens[i + 1].parse::<f32>().unwrap_or(0.0);
                    builder.line_to(point(cx, cy));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "C" => {
                if i + 6 < tokens.len() {
                    let x1: f32 = tokens[i + 1].parse().unwrap_or(0.0);
                    let y1: f32 = tokens[i + 2].parse().unwrap_or(0.0);
                    let x2: f32 = tokens[i + 3].parse().unwrap_or(0.0);
                    let y2: f32 = tokens[i + 4].parse().unwrap_or(0.0);
                    cx = tokens[i + 5].parse().unwrap_or(0.0);
                    cy = tokens[i + 6].parse().unwrap_or(0.0);
                    builder.cubic_bezier_to(point(x1, y1), point(x2, y2), point(cx, cy));
                    i += 7;
                } else {
                    i += 1;
                }
            }
            "c" => {
                if i + 6 < tokens.len() {
                    let dx1: f32 = tokens[i + 1].parse().unwrap_or(0.0);
                    let dy1: f32 = tokens[i + 2].parse().unwrap_or(0.0);
                    let dx2: f32 = tokens[i + 3].parse().unwrap_or(0.0);
                    let dy2: f32 = tokens[i + 4].parse().unwrap_or(0.0);
                    let dx: f32 = tokens[i + 5].parse().unwrap_or(0.0);
                    let dy: f32 = tokens[i + 6].parse().unwrap_or(0.0);
                    builder.cubic_bezier_to(
                        point(cx + dx1, cy + dy1),
                        point(cx + dx2, cy + dy2),
                        point(cx + dx, cy + dy),
                    );
                    cx += dx;
                    cy += dy;
                    i += 7;
                } else {
                    i += 1;
                }
            }
            "S" | "s" | "Q" | "q" | "T" | "t" | "A" | "a" => {
                // Skip unsupported commands for now
                i += 1;
                while i < tokens.len() && !tokens[i].chars().next().map_or(false, |c| c.is_alphabetic()) {
                    i += 1;
                }
            }
            "Z" | "z" => {
                if started {
                    builder.close();
                    started = false;
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    if started {
        builder.end(false);
    }

    Some(builder.build())
}

fn tokenize_svg(d: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut num = String::new();

    for ch in d.chars() {
        if ch.is_alphabetic() {
            if !num.is_empty() {
                tokens.push(num.clone());
                num.clear();
            }
            tokens.push(ch.to_string());
        } else if ch == '-' && !num.is_empty() && !num.ends_with('e') && !num.ends_with('E') {
            tokens.push(num.clone());
            num.clear();
            num.push(ch);
        } else if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == 'e' || ch == 'E' {
            num.push(ch);
        } else if ch == ',' || ch.is_whitespace() {
            if !num.is_empty() {
                tokens.push(num.clone());
                num.clear();
            }
        }
    }
    if !num.is_empty() {
        tokens.push(num);
    }

    tokens
}

fn type_color(elem_type: &str) -> Rgb<u8> {
    match elem_type {
        "button" => Rgb([0, 100, 200]),
        "text" => Rgb([60, 60, 60]),
        "image" => Rgb([0, 140, 60]),
        "input" => Rgb([200, 100, 0]),
        "container" => Rgb([200, 200, 200]),
        "list" | "scroll" | "pager" => Rgb([160, 160, 200]),
        "toggle" => Rgb([180, 0, 180]),
        _ => Rgb([130, 130, 130]),
    }
}

fn parse_hex(hex: &str) -> Option<Rgb<u8>> {
    let hex = hex.trim_start_matches('#');
    let hex = if hex.len() == 8 { &hex[2..] } else { hex };
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb([r, g, b]))
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}
