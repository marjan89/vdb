use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Tolerances {
    pub spatial_pct: f64,
    pub text_size_pct: f64,
    pub color_delta_e: f64,
    pub text_weight: WeightTolerance,
    pub icon_size_pct: f64,
    pub card_size_pct: f64,
    pub corner_radius_px: f64,
    pub border_width_px: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WeightTolerance {
    Exact,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            spatial_pct: 10.0,
            text_size_pct: 5.0,
            color_delta_e: 3.0,
            text_weight: WeightTolerance::Exact,
            icon_size_pct: 5.0,
            card_size_pct: 10.0,
            corner_radius_px: 2.0,
            border_width_px: 1.0,
        }
    }
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    tolerances: Option<RawTolerances>,
    #[serde(flatten)]
    _rest: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Deserialize)]
struct RawTolerances {
    spatial: Option<String>,
    text_size: Option<String>,
    color: Option<f64>,
    text_weight: Option<String>,
    icon_size: Option<String>,
    card_size: Option<String>,
    corner_radius: Option<String>,
    border_width: Option<String>,
}

impl Tolerances {
    pub fn from_manifest(path: &str) -> Result<Self, String> {
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("read manifest {path}: {e}"))?;
        let manifest: Manifest =
            serde_yaml::from_str(&content).map_err(|e| format!("parse manifest: {e}"))?;
        let raw = match manifest.tolerances {
            Some(t) => t,
            None => return Ok(Self::default()),
        };
        let mut t = Self::default();
        if let Some(s) = &raw.spatial {
            t.spatial_pct = parse_pct(s, "spatial")?;
        }
        if let Some(s) = &raw.text_size {
            t.text_size_pct = parse_pct(s, "text_size")?;
        }
        if let Some(v) = raw.color {
            t.color_delta_e = v;
        }
        if let Some(s) = &raw.text_weight {
            if s == "exact" {
                t.text_weight = WeightTolerance::Exact;
            }
        }
        if let Some(s) = &raw.icon_size {
            t.icon_size_pct = parse_pct(s, "icon_size")?;
        }
        if let Some(s) = &raw.card_size {
            t.card_size_pct = parse_pct(s, "card_size")?;
        }
        if let Some(s) = &raw.corner_radius {
            t.corner_radius_px = parse_px(s, "corner_radius")?;
        }
        if let Some(s) = &raw.border_width {
            t.border_width_px = parse_px(s, "border_width")?;
        }
        Ok(t)
    }
}

fn parse_pct(s: &str, field: &str) -> Result<f64, String> {
    let s = s.trim().trim_end_matches('%');
    s.parse::<f64>()
        .map_err(|_| format!("invalid percentage for {field}: {s}"))
}

fn parse_px(s: &str, field: &str) -> Result<f64, String> {
    let s = s.trim().trim_end_matches("px").trim_end_matches("dp");
    s.parse::<f64>()
        .map_err(|_| format!("invalid pixel value for {field}: {s}"))
}

// --- Delta E CIE2000 ---

pub fn delta_e_cie2000(hex1: &str, hex2: &str) -> f64 {
    let (r1, g1, b1) = hex_to_rgb(hex1);
    let (r2, g2, b2) = hex_to_rgb(hex2);
    let lab1 = rgb_to_lab(r1, g1, b1);
    let lab2 = rgb_to_lab(r2, g2, b2);
    ciede2000(lab1, lab2)
}

fn hex_to_rgb(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim_start_matches('#');
    let hex = if hex.len() == 8 { &hex[2..] } else { hex };
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f64 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f64 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f64 / 255.0;
    (r, g, b)
}

fn rgb_to_lab(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let linearize = |c: f64| {
        if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        }
    };
    let r = linearize(r);
    let g = linearize(g);
    let b = linearize(b);

    let x = (r * 0.4124564 + g * 0.3575761 + b * 0.1804375) / 0.95047;
    let y = r * 0.2126729 + g * 0.7151522 + b * 0.0721750;
    let z = (r * 0.0193339 + g * 0.1191920 + b * 0.9503041) / 1.08883;

    let f = |t: f64| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };

    let l = 116.0 * f(y) - 16.0;
    let a = 500.0 * (f(x) - f(y));
    let b_val = 200.0 * (f(y) - f(z));
    (l, a, b_val)
}

fn ciede2000((l1, a1, b1): (f64, f64, f64), (l2, a2, b2): (f64, f64, f64)) -> f64 {
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let c_avg = (c1 + c2) / 2.0;
    let c_avg7 = c_avg.powi(7);
    let g = 0.5 * (1.0 - (c_avg7 / (c_avg7 + 25.0_f64.powi(7))).sqrt());

    let a1p = a1 * (1.0 + g);
    let a2p = a2 * (1.0 + g);
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();

    let h1p = b1.atan2(a1p).to_degrees().rem_euclid(360.0);
    let h2p = b2.atan2(a2p).to_degrees().rem_euclid(360.0);

    let dl = l2 - l1;
    let dc = c2p - c1p;

    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dh_big = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();

    let l_avg = (l1 + l2) / 2.0;
    let c_avgp = (c1p + c2p) / 2.0;

    let h_avgp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };

    let t = 1.0 - 0.17 * ((h_avgp - 30.0).to_radians()).cos()
        + 0.24 * ((2.0 * h_avgp).to_radians()).cos()
        + 0.32 * ((3.0 * h_avgp + 6.0).to_radians()).cos()
        - 0.20 * ((4.0 * h_avgp - 63.0).to_radians()).cos();

    let sl = 1.0 + 0.015 * (l_avg - 50.0).powi(2) / (20.0 + (l_avg - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_avgp;
    let sh = 1.0 + 0.015 * c_avgp * t;

    let c_avgp7 = c_avgp.powi(7);
    let rt = -2.0
        * (c_avgp7 / (c_avgp7 + 25.0_f64.powi(7))).sqrt()
        * (60.0 * (-((h_avgp - 275.0) / 25.0).powi(2)).exp())
            .to_radians()
            .sin();

    let kl = 1.0;
    let kc = 1.0;
    let kh = 1.0;

    ((dl / (kl * sl)).powi(2)
        + (dc / (kc * sc)).powi(2)
        + (dh_big / (kh * sh)).powi(2)
        + rt * (dc / (kc * sc)) * (dh_big / (kh * sh)))
    .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_e_identical() {
        assert!(delta_e_cie2000("#FF0000", "#FF0000") < 0.001);
    }

    #[test]
    fn test_delta_e_similar_colors() {
        let de = delta_e_cie2000("#008080", "#007878");
        assert!(de < 3.0, "expected < 3.0, got {de}");
    }

    #[test]
    fn test_delta_e_different_colors() {
        let de = delta_e_cie2000("#FF0000", "#00FF00");
        assert!(de > 30.0, "expected > 30.0, got {de}");
    }

    #[test]
    fn test_parse_manifest() {
        let dir = std::env::temp_dir().join("vdb_test_manifest");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("manifest.yaml");
        std::fs::write(
            &path,
            "version: 1\ntolerances:\n  spatial: 10%\n  text_size: 5%\n  color: 3.0\n  text_weight: exact\n  corner_radius: 2px\n  border_width: 1px\n",
        )
        .unwrap();
        let t = Tolerances::from_manifest(path.to_str().unwrap()).unwrap();
        assert!((t.spatial_pct - 10.0).abs() < 0.01);
        assert!((t.color_delta_e - 3.0).abs() < 0.01);
        assert!((t.corner_radius_px - 2.0).abs() < 0.01);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
