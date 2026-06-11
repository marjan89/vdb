# vdb — Visual Debug Bridge

Cross-platform semantic UI comparison and validation tool. Compares element trees from Android, iOS, and Figma design files.

## Install

```sh
cd vdb && cargo build --release
# Use `install` (atomic tmp+rename) instead of `cp` — overwriting a
# running binary can break macOS codesign cache and trigger SIGKILL
# (TD-125).
install -m 755 target/release/vdb /opt/homebrew/bin/vdb
```

Requires: Rust 1.94+, macOS (uses system Helvetica for text rendering).

## Commands

### `vdb diff`

Compare two semantic schema YAMLs and output mismatches.

```sh
vdb diff android.yaml ios.yaml
vdb diff android.yaml ios.yaml --json
vdb diff android.yaml ios.yaml --accessible-only
```

**Matching algorithm:** Three-pass element matching:
1. Exact ID match
2. Exact content text match (with entity decoding)
3. Type + position proximity (within 40dp)

**Output categories:**
- `ERRORS` — missing elements, wrong text content
- `WARNINGS` — wrong font, color, type, background, icon, corner radius, font size, line count, truncated
- `INFO` — spacing drift (>4dp), size differences (>8dp), extra elements

**Flags:**
| Flag | Default | Description |
|------|---------|-------------|
| `--json` | false | Output as JSON |
| `--accessible-only` | false | Only compare accessible elements |

**Exit code:** 1 if any errors or warnings.

---

### `vdb validate`

Per-element djb2 color fingerprint validation. Compares agent's on-device overlay against expected element positions from YAML.

```sh
vdb validate screenshot-stroke.png schema.yaml --pass stroke
vdb validate screenshot-fill.png schema.yaml --pass fill
vdb validate screenshot-stroke.png schema.yaml --screenshot original.png
```

**How it works:**
1. Reads YAML and renders a reference overlay using the same djb2 color algorithm as the device agent
2. For each content element, samples pixels at element bounds in the device screenshot
3. Compares sampled colors against expected djb2 color
4. Adjusts for z-order occlusion and viewport clipping

**Flags:**
| Flag | Default | Description |
|------|---------|-------------|
| `--pass` | `stroke` | Validation pass: `stroke` (position) or `fill` (dimensions) |
| `-o, --output` | `/tmp/vdb-validate.png` | Output composite image |
| `--screenshot` | — | Real screenshot for visual overlay |
| `--stroke-width` | `4` | Stroke width in px (must match agent) |
| `--threshold` | `50` | Pass threshold per element (overlap %) |
| `--color-tolerance` | `120` | Color matching tolerance (0-255) |
| `--density` | auto | Device density (dp to px). Auto-detected from YAML viewport |
| `--viewport-width` | auto | Viewport width in dp |
| `--viewport-height` | auto | Viewport height in dp |

**Auto-density:** Uses `viewport.density` from YAML if present, falls back to root element width / screenshot width.

**Occlusion handling:**
- Elements <30% visible (viewport-clipped or sibling-occluded) auto-PASS
- Elements <20dp in either dimension auto-PASS (too small for reliable sampling)
- Proportional threshold for partially occluded elements

**Exit code:** 1 if any element fails.

---

### `vdb validate-content`

Cross-reference YAML content against platform accessibility dump.

```sh
vdb validate-content schema.yaml a11y-dump.xml
vdb validate-content schema.yaml a11y-dump.xml --exclude-external --exclude-offscreen --exclude-system
vdb validate-content schema.yaml a11y-dump.xml --json
```

**Input formats:** Auto-detected:
- uiautomator XML (`<node text="..." ...>`)
- WDA XML (`<XCUIElementTypeStaticText label="..." ...>`)
- WDA JSON (`{"label": "...", "children": [...]}`)

**Matching:** Exact text match + substring matching (for truncated text).

**Flags:**
| Flag | Description |
|------|-------------|
| `--exclude-external` | Exclude elements inside external render surfaces (maps, webviews) |
| `--exclude-offscreen` | Exclude elements beyond viewport bounds + invisible elements |
| `--exclude-system` | Exclude system chrome (scroll bars, app name, icon assets) |
| `--json` | Output as JSON |

**Coordinate handling:** WDA XML uses pt coordinates (1x), uiautomator uses px. Auto-detected.

**Exit code:** 1 if any missing elements.

---

### `vdb render`

Render semantic schema YAML as a PNG reconstruction.

```sh
vdb render schema.yaml -o render.png
vdb render android.yaml ios.yaml -o side-by-side.png
vdb render schema.yaml --font-dir /path/to/fonts --source-root /path/to/res
vdb render schema.yaml --validate
```

**Flags:**
| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output` | `/tmp/vdb-render.png` | Output PNG path |
| `--scale` | `1.0` | Scale factor (2.0 for retina) |
| `--font-dir` | — | Load .ttf fonts for text rendering |
| `--source-root` | — | Android res/ or iOS Assets.xcassets for icon loading |
| `--validate` | false | Validation mode: filled blue rectangles only |
| `--viewport-width` | auto | Viewport width in dp |
| `--viewport-height` | auto | Viewport height in dp |

**Multi-schema:** Pass multiple YAML files to render side-by-side with a 4px gap.

---

### `vdb overlay`

Overlay semantic bounds on a device screenshot.

```sh
vdb overlay screenshot.png schema.yaml -o overlay.png
vdb overlay screenshot.png schema.yaml --safe-area-top 47 --density 3.0
```

**Flags:**
| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output` | `/tmp/vdb-overlay.png` | Output PNG path |
| `--density` | auto | Device density (from YAML viewport) |
| `--safe-area-top` | auto (47 for iOS) | Safe area inset in pt |

**Rendering:** Semi-transparent colored rectangles per element type, with ID labels.

---

### `vdb compare`

Visual screenshot comparison using SSIM + pixel diff.

```sh
vdb compare screenshot-a.png screenshot-b.png -o diff.png
```

Produces a three-panel image: source, target, pixel diff with SSIM score.

## Schema Format

All tools consume the same YAML schema:

```yaml
screen: MainActivity
device: SM-A546B
platform: android
timestamp: "2026-05-22T20:20:43Z"
viewport:
  width: 384
  height: 832
  density: 2.8125
elements:
- id: hiking
  platform_id: button
  type: button
  content: Hiking
  font:
    family: sans-serif
    weight: semibold
    size: 13.15
  color: "#08292F"
  foreground: "#08292F"
  bounds:
    x: 14
    y: 417
    w: 121
    h: 38
  z_index: 25
  clickable: true
  enabled: true
  accessible: true
  a11y_label: Hiking
  background: "#E8F5E9"
  corner_radius: 19.0
  line_count: 1
  truncated: false
```

## Interactive Viewer

`serve.py` serves capture directories as an interactive HTML viewer:

```sh
python3 serve.py /tmp/vdb-captures/final [port]
```

- **Left click/hover:** Element tooltip (ID, type, bounds, content, font, color)
- **Right click:** Add remark pinned to element position
- **Modes:** Toggle stroke/fill/screenshot views
- **Remarks:** Persisted to `remarks.json`, shown as red pins

## djb2 Color Algorithm

Both device agents and vdb use the same color generation:

```
hash = 5381
for byte in id.utf8_bytes:
    hash = hash * 33 + byte  (wrapping u32)
hue = hash % 360
color = HSL(hue, 1.0, 0.5)
```

Stroke overlay: white-fill-then-colored-stroke per element in z-order.
Fill overlay: colored-fill per element in z-order.
