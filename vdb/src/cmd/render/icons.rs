use image::{Rgb, RgbImage};
use lyon_path::Path as LyonPath;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};

pub struct IconCache {
    icons: std::collections::HashMap<String, image::DynamicImage>,
}

impl IconCache {
    pub fn scan(source_root: &str) -> Self {
        let mut icons = std::collections::HashMap::new();
        let root = std::path::Path::new(source_root);

        for dir_name in &[
            "drawable-xxxhdpi",
            "drawable-xxhdpi",
            "drawable-xhdpi",
            "drawable-hdpi",
            "drawable-mdpi",
            "drawable",
        ] {
            let dir = root.join(dir_name);
            if !dir.exists() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !matches!(ext, "png" | "jpg" | "webp") {
                    continue;
                }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.is_empty() || icons.contains_key(stem) {
                    continue;
                }
                if let Ok(img) = image::open(&path) {
                    icons.insert(stem.to_string(), img);
                }
            }
        }

        Self::scan_xcassets(root, &mut icons);

        if let Some(parent) = root.parent() {
            Self::scan_xcassets(parent, &mut icons);
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

    fn scan_xcassets(
        root: &std::path::Path,
        icons: &mut std::collections::HashMap<String, image::DynamicImage>,
    ) {
        for entry in walkdir::WalkDir::new(root)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.ends_with(".imageset") {
                continue;
            }
            let asset_name = name.strip_suffix(".imageset").unwrap_or(name);
            if icons.contains_key(asset_name) {
                continue;
            }
            let contents = path.join("Contents.json");
            if !contents.exists() {
                continue;
            }
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
        #[derive(serde::Deserialize)]
        struct Contents {
            images: Vec<ImageEntry>,
        }
        #[derive(serde::Deserialize)]
        struct ImageEntry {
            #[serde(default)]
            filename: Option<String>,
            #[serde(default)]
            scale: Option<String>,
        }

        let contents: Contents = serde_json::from_str(json).ok()?;
        let priority = |s: &str| match s {
            "2x" => 3,
            "3x" => 2,
            "1x" => 1,
            _ => 0,
        };

        let mut best: Option<&ImageEntry> = None;
        for img in &contents.images {
            if img.filename.is_none() {
                continue;
            }
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

    pub fn get(&self, name: &str) -> Option<&image::DynamicImage> {
        self.icons.get(name)
    }
}

pub fn render_raster_icon(
    img: &mut RgbImage,
    src_img: &image::DynamicImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) {
    let (nw, nh) = (src_img.width(), src_img.height());
    let icon_w = w.min(nw).min(h * nw / nh.max(1));
    let icon_h = h.min(nh).min(w * nh / nw.max(1));
    let resized = src_img.resize(
        icon_w.max(1),
        icon_h.max(1),
        image::imageops::FilterType::Lanczos3,
    );
    let rgba = resized.to_rgba8();
    let rw = rgba.width();
    let rh = rgba.height();
    let (img_w, img_h) = img.dimensions();
    let ox = (w.saturating_sub(rw)) / 2;
    let oy = (h.saturating_sub(rh)) / 2;
    for dy in 0..rh.min(img_h.saturating_sub(y + oy)) {
        for dx in 0..rw.min(img_w.saturating_sub(x + ox)) {
            let px = rgba.get_pixel(dx, dy);
            let a = px.0[3] as f32 / 255.0;
            if a < 0.01 {
                continue;
            }
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
}

pub fn render_svg_paths(
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
                    while i + 1 < tokens.len() {
                        if let (Ok(x), Ok(y)) =
                            (tokens[i].parse::<f32>(), tokens[i + 1].parse::<f32>())
                        {
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
                i += 1;
                while i < tokens.len()
                    && !tokens[i]
                        .chars()
                        .next()
                        .map_or(false, |c| c.is_alphabetic())
                {
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
