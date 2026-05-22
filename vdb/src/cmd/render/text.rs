use ab_glyph::FontVec;
use image::Rgb;
use imageproc::drawing::draw_text_mut;

pub fn load_font_dir(dir: &str) -> Option<FontVec> {
    let candidates = [
        "poppins_regular.ttf",
        "poppins_medium.ttf",
        "poppins_semi_bold.ttf",
    ];
    for name in &candidates {
        let path = std::path::Path::new(dir).join(name);
        if let Ok(data) = std::fs::read(&path) {
            if let Ok(font) = FontVec::try_from_vec(data) {
                eprintln!("loaded font: {name}");
                return Some(font);
            }
        }
    }
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

pub fn shrink_to_fit(base_size: f32, text: &str, max_w: i32, max_h: i32, scale: f32) -> f32 {
    if max_w <= 0 || max_h <= 0 {
        return base_size;
    }
    let char_w = (base_size * 0.55) as i32;
    let line_h = (base_size * 1.3) as i32;
    if char_w <= 0 || line_h <= 0 {
        return base_size;
    }
    let chars_per_line = (max_w / char_w).max(1) as usize;
    let text_chars = text.chars().count();
    let lines_needed = (text_chars + chars_per_line - 1) / chars_per_line;
    let lines_available = (max_h / line_h).max(1) as usize;
    if lines_needed > lines_available && lines_available <= 2 {
        let needed_char_w = max_w as f32 / text_chars.max(1) as f32;
        (needed_char_w / 0.55).max(6.0 * scale).min(base_size)
    } else {
        base_size
    }
}

pub fn draw_wrapped_clipped(
    img: &mut image::RgbImage,
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
        if !first_line && cy + line_height > max_y {
            break;
        }
        first_line = false;

        let char_count = remaining.chars().count();
        let line = if char_count <= chars_per_line {
            remaining
        } else {
            let byte_end = remaining
                .char_indices()
                .nth(chars_per_line)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
            let break_at = remaining[..byte_end].rfind(' ').unwrap_or(byte_end);
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
