#!/usr/bin/env python3
"""Regions visual demo — three-column view with matched region bounding boxes."""

import http.server
import json
import os
import subprocess
import sys
from pathlib import Path

CAPTURE_DIR = Path("/tmp/vdb-captures/final")
VDB = "vdb"
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8787

REGION_COLORS = {
    "action_bar": "#FF6B6B",
    "rating_row": "#4ECDC4",
    "reviews_section": "#45B7D1",
    "qa_section": "#96CEB4",
    "map_cta": "#FFEAA7",
    "social_counts": "#DDA0DD",
    "categories_section": "#98D8C8",
    "contact_section": "#F7DC6F",
}

def discover_regions(yaml_path):
    try:
        result = subprocess.run(
            [VDB, "regions", "discover", str(yaml_path), "--json"],
            capture_output=True, text=True, timeout=10
        )
        if result.returncode == 0:
            return json.loads(result.stdout)
    except Exception:
        pass
    return []

def get_viewport(yaml_path):
    width, height, density = 390, 844, 3.0
    try:
        in_viewport = False
        with open(yaml_path) as f:
            for line in f:
                if line.strip() == "viewport:":
                    in_viewport = True
                elif in_viewport and line.strip().startswith("width:"):
                    width = int(line.split(":")[1].strip())
                elif in_viewport and line.strip().startswith("height:"):
                    height = int(line.split(":")[1].strip())
                elif in_viewport and line.strip().startswith("density:"):
                    density = float(line.split(":")[1].strip())
                elif in_viewport and not line.startswith(" "):
                    in_viewport = False
    except Exception:
        pass
    return width, height, density

HTML = """<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>VDB Regions Demo</title>
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: -apple-system, system-ui, sans-serif; background: #1a1a2e; color: #eee; }
h1 { padding: 16px 24px; font-size: 20px; color: #ccc; font-weight: 400; }
.columns { display: flex; gap: 16px; padding: 0 24px 24px; justify-content: center; align-items: flex-start; }
.column { flex: 0 0 auto; }
.column h2 { text-align: center; font-size: 14px; color: #888; margin-bottom: 8px; font-weight: 400; }
.img-wrapper { position: relative; display: inline-block; }
.img-wrapper img { display: block; height: 700px; width: auto; }
.region-box { position: absolute; border: 2px solid; border-radius: 4px; pointer-events: none; }
.region-label { position: absolute; top: -18px; left: 0; font-size: 10px; padding: 1px 4px;
  border-radius: 2px; white-space: nowrap; font-family: 'SF Mono', Menlo, monospace; }
.legend { padding: 8px 24px 16px; display: flex; gap: 16px; flex-wrap: wrap; }
.legend-item { display: flex; align-items: center; gap: 6px; font-size: 12px; color: #aaa; }
.legend-swatch { width: 14px; height: 14px; border-radius: 3px; }
.match-summary { padding: 0 24px 16px; font-size: 13px; color: #888; }
.match-summary span { color: #4ECDC4; font-weight: 600; }
</style>
</head>
<body>
<h1>VDB Regions — Site Detail</h1>
<div class="match-summary">MATCH_SUMMARY</div>
<div class="legend">LEGEND_HTML</div>
<div class="columns">COLUMNS_HTML</div>
</body>
</html>"""


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/" or self.path == "/index.html":
            self.serve_html()
        elif self.path.startswith("/img/"):
            self.serve_image(self.path[5:])
        else:
            self.send_error(404)

    def serve_html(self):
        sources = [
            ("Android", "android-sitedetail", CAPTURE_DIR / "android-sitedetail.yaml",
             CAPTURE_DIR / "android-sitedetail-screenshot.png"),
            ("iOS", "ios-sitedetail", CAPTURE_DIR / "ios-sitedetail.yaml",
             CAPTURE_DIR / "ios-sitedetail-screenshot.png"),
            ("Figma", "figma-site", Path("/tmp/figma-site.yaml"),
             CAPTURE_DIR / "figma-sitedetail-screenshot.png"),
        ]

        all_regions = {}
        all_region_ids = set()
        columns_html = ""

        for label, key, yaml_path, img_path in sources:
            regions = discover_regions(yaml_path) if yaml_path.exists() else []
            all_regions[key] = regions
            for r in regions:
                all_region_ids.add(r["id"])

            vp_w, vp_h, density = get_viewport(yaml_path) if yaml_path.exists() else (390, 844, 3.0)

            boxes_html = ""
            for r in regions:
                color = REGION_COLORS.get(r["id"], "#888")
                b = r["bounds"]
                boxes_html += f'''<div class="region-box" style="
                    border-color:{color};
                    left:{b['x']}px; top:{b['y']}px;
                    width:{b['w']}px; height:{b['h']}px;
                    transform-origin: top left;
                " data-vp-h="{vp_h}">
                    <span class="region-label" style="background:{color};color:#000">{r['id']}</span>
                </div>'''

            img_src = f"/img/{img_path.name}" if img_path and img_path.exists() else ""
            img_tag = f'<img src="{img_src}" onload="scaleBoxes(this)">' if img_src else f'<div style="width:300px;height:700px;background:#333;display:flex;align-items:center;justify-content:center;color:#666">No screenshot<br>({label})</div>'

            columns_html += f'''
            <div class="column">
                <h2>{label} ({len(regions)} regions)</h2>
                <div class="img-wrapper" style="overflow:hidden">{img_tag}{boxes_html}</div>
            </div>'''

        legend_html = ""
        for rid in sorted(all_region_ids):
            color = REGION_COLORS.get(rid, "#888")
            present = [label for label, key, _, _ in sources if any(r["id"] == rid for r in all_regions.get(key, []))]
            legend_html += f'<div class="legend-item"><div class="legend-swatch" style="background:{color}"></div>{rid} ({", ".join(present)})</div>'

        matched_count = sum(1 for rid in all_region_ids if sum(1 for key in all_regions if any(r["id"] == rid for r in all_regions[key])) >= 2)
        summary = f"<span>{matched_count}</span> regions matched across sources, <span>{len(all_region_ids)}</span> total unique regions"

        html = HTML.replace("COLUMNS_HTML", columns_html)
        html = html.replace("LEGEND_HTML", legend_html)
        html = html.replace("MATCH_SUMMARY", summary)

        # Add JS for scaling boxes to match image display size
        html = html.replace("</body>", """
<script>
function scaleBoxes(img) {
    const wrapper = img.parentElement;
    const displayH = img.clientHeight;
    const boxes = wrapper.querySelectorAll('.region-box');
    const vpH = parseFloat(boxes[0]?.dataset.vpH || '844');
    const scale = displayH / vpH;
    boxes.forEach(box => {
        box.style.transform = 'scale(' + scale + ')';
    });
}
window.addEventListener('load', () => {
    document.querySelectorAll('.img-wrapper img').forEach(scaleBoxes);
});
</script>
</body>""")

        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        self.wfile.write(html.encode())

    def serve_image(self, name):
        path = CAPTURE_DIR / name
        if not path.exists():
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "image/png")
        self.send_header("Cache-Control", "max-age=60")
        self.end_headers()
        self.wfile.write(path.read_bytes())

    def log_message(self, fmt, *args):
        pass


if __name__ == "__main__":
    print(f"Regions demo: http://localhost:{PORT}")
    server = http.server.HTTPServer(("127.0.0.1", PORT), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
