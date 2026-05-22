use image::Rgb;
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut};
use imageproc::rect::Rect;

pub fn type_color(elem_type: &str) -> Rgb<u8> {
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

pub fn parse_hex(hex: &str) -> Option<Rgb<u8>> {
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

pub fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

pub fn fill_bg(
    img: &mut image::RgbImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: Rgb<u8>,
) {
    let rect = Rect::at(x as i32, y as i32).of_size(w, h);
    draw_filled_rect_mut(img, rect, color);
}

pub fn draw_border(
    img: &mut image::RgbImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: Rgb<u8>,
    clickable: bool,
) {
    let rect = Rect::at(x as i32, y as i32).of_size(w, h);
    draw_hollow_rect_mut(img, rect, color);
    if clickable && w > 2 && h > 2 {
        let inner = Rect::at(x as i32 + 1, y as i32 + 1).of_size(w - 2, h - 2);
        draw_hollow_rect_mut(img, inner, color);
    }
}
