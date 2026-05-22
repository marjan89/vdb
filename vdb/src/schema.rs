use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SemanticSchema {
    #[serde(default)]
    pub screen: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub timestamp: String,
    pub elements: Vec<SemanticElement>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SemanticElement {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub platform_id: Option<String>,
    #[serde(rename = "type")]
    pub elem_type: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub font: Option<Font>,
    #[serde(default)]
    pub color: Option<String>,
    pub bounds: Bounds,
    #[serde(default)]
    pub clickable: bool,
    #[serde(default)]
    pub accessible: Option<bool>,
    #[serde(default)]
    pub a11y_label: Option<String>,
    #[serde(default)]
    pub a11y_id: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub corner_radius: Option<f64>,
    #[serde(default)]
    pub padding: Option<Padding>,
    #[serde(default)]
    pub icon: Option<Icon>,
    #[serde(default)]
    pub z_index: Option<u64>,
    #[serde(default)]
    pub render: Option<String>,
    #[serde(default)]
    pub children: Option<Vec<SemanticElement>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Font {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub weight: String,
    #[serde(default)]
    pub size: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Padding {
    #[serde(default)]
    pub top: i32,
    #[serde(default)]
    pub bottom: i32,
    #[serde(default)]
    pub start: i32,
    #[serde(default)]
    pub end: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Icon {
    pub name: String,
    pub format: String,
    #[serde(default)]
    pub paths: Vec<String>,
}
