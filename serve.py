#!/usr/bin/env python3
"""Interactive semantic overlay viewer. Serves capture images + YAMLs,
click on stroke/fill overlay to see element info via tooltip."""

import http.server
import json
import os
import re
import sys
from pathlib import Path

CAPTURE_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/vdb-captures/v3-test")
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8080

HTML = """<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>VDB Semantic Overlay Viewer</title>
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: -apple-system, system-ui, sans-serif; background: #1a1a2e; color: #eee; }
h1 { padding: 16px 24px; font-size: 18px; color: #999; font-weight: 400; }
.controls { padding: 0 24px 16px; display: flex; gap: 12px; flex-wrap: wrap; }
.controls button { padding: 6px 16px; border: 1px solid #444; background: #2a2a3e;
  color: #ccc; border-radius: 6px; cursor: pointer; font-size: 13px; }
.controls button.active { background: #4a4a6e; border-color: #7a7aae; color: #fff; }
.grid { display: flex; flex-wrap: wrap; gap: 16px; padding: 0 24px 24px; justify-content: center; }
.card { position: relative; background: #222238; border-radius: 8px; overflow: hidden; }
.card img { display: block; height: 600px; width: auto; cursor: crosshair; }
.card .label { padding: 8px 12px; font-size: 12px; color: #888; text-align: center; }
#tooltip { position: fixed; display: none; background: rgba(0,0,0,0.92); color: #eee;
  padding: 10px 14px; border-radius: 8px; font-size: 12px; line-height: 1.6;
  max-width: 360px; pointer-events: none; z-index: 1000; border: 1px solid #555;
  font-family: 'SF Mono', Menlo, monospace; }
#tooltip .field { color: #888; }
#tooltip .value { color: #7af; }
#tooltip .id { color: #fa7; font-weight: 600; font-size: 14px; }
#remark-dialog { position: fixed; display: none; background: #2a2a3e; border: 1px solid #666;
  border-radius: 8px; padding: 12px; z-index: 1001; width: 300px; }
#remark-dialog textarea { width: 100%; height: 60px; background: #1a1a2e; color: #eee;
  border: 1px solid #555; border-radius: 4px; padding: 6px; font-size: 12px; resize: vertical; }
#remark-dialog .btn-row { display: flex; gap: 8px; margin-top: 8px; justify-content: flex-end; }
#remark-dialog button { padding: 4px 12px; border-radius: 4px; border: 1px solid #555;
  background: #3a3a5e; color: #ccc; cursor: pointer; font-size: 12px; }
#remark-dialog button.save { background: #4a6a4a; border-color: #6a9a6a; }
#remark-dialog .elem-id { color: #fa7; font-size: 11px; margin-bottom: 6px; }
.remark-pin { position: absolute; width: 12px; height: 12px; background: #f55; border: 2px solid #fff;
  border-radius: 50%; transform: translate(-50%, -50%); cursor: pointer; z-index: 10;
  box-shadow: 0 1px 3px rgba(0,0,0,0.5); }
.remark-pin:hover { background: #ff8; }
#remarks-panel { position: fixed; right: 0; top: 0; width: 280px; height: 100vh; background: #1e1e32;
  border-left: 1px solid #444; overflow-y: auto; padding: 12px; display: none; z-index: 900; }
#remarks-panel h3 { color: #999; font-size: 13px; margin-bottom: 8px; }
.remark-item { background: #2a2a3e; border-radius: 6px; padding: 8px; margin-bottom: 8px; font-size: 11px; }
.remark-item .ri-id { color: #fa7; font-weight: 600; }
.remark-item .ri-loc { color: #666; }
.remark-item .ri-text { color: #ccc; margin-top: 4px; }
.remark-item .ri-delete { color: #a55; cursor: pointer; float: right; font-size: 10px; }
</style>
</head>
<body>
<h1>VDB Semantic Overlay Viewer</h1>
<div class="controls">
  <button class="active" data-mode="stroke">Stroke</button>
  <button data-mode="fill">Fill</button>
  <button data-mode="screenshot">Screenshot</button>
  <button id="toggle-remarks" style="margin-left:auto">Remarks (0)</button>
</div>
<div class="grid" id="grid"></div>
<div id="tooltip"></div>
<div id="remark-dialog">
  <div class="elem-id" id="remark-elem"></div>
  <textarea id="remark-text" placeholder="Add remark..."></textarea>
  <div class="btn-row">
    <button onclick="closeRemark()">Cancel</button>
    <button class="save" onclick="saveRemark()">Save</button>
  </div>
</div>
<div id="remarks-panel"><h3>Remarks</h3><div id="remarks-list"></div></div>

<script>
const captures = CAPTURES_JSON;
let mode = 'stroke';
let yamlData = {};

document.querySelectorAll('.controls button').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.controls button').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    mode = btn.dataset.mode;
    renderGrid();
  });
});

function renderGrid() {
  const grid = document.getElementById('grid');
  grid.innerHTML = '';
  for (const cap of captures) {
    const card = document.createElement('div');
    card.className = 'card';
    const img = document.createElement('img');
    img.src = '/img/' + cap[mode];
    img.dataset.platform = cap.platform;
    img.dataset.screen = cap.screen;
    img.addEventListener('mousemove', onImageMove);
    img.addEventListener('mouseleave', () => {
      document.getElementById('tooltip').style.display = 'none';
    });
    img.addEventListener('contextmenu', onRightClick);
    card.appendChild(img);
    const label = document.createElement('div');
    label.className = 'label';
    label.textContent = cap.platform + ' / ' + cap.screen;
    card.appendChild(label);
    grid.appendChild(card);
  }
}

async function loadYamls() {
  for (const cap of captures) {
    const key = cap.platform + '-' + cap.screen;
    try {
      const resp = await fetch('/yaml/' + key);
      if (resp.ok) {
        const data = await resp.json();
        // Compute density from first loaded image
        const img = document.querySelector('img[data-platform="'+cap.platform+'"][data-screen="'+cap.screen+'"]');
        if (img && img.naturalWidth > 0 && data.elements && data.elements.length > 0) {
          const maxXW = Math.max(...data.elements.map(e => (e.bounds.x||0) + (e.bounds.w||0)));
          if (maxXW > 100) data._density = img.naturalWidth / maxXW;
          else data._density = 3.0;
        }
        yamlData[key] = data;
      }
    } catch(e) {}
  }
}

function findElement(platform, screen, dpX, dpY) {
  const key = platform + '-' + screen;
  const data = yamlData[key];
  if (!data || !data.elements) return null;
  let best = null;
  for (const el of data.elements) {
    const b = el.bounds;
    if (dpX >= b.x && dpX < b.x + b.w && dpY >= b.y && dpY < b.y + b.h) {
      const z = el.z_index || 0;
      if (!best || z >= (best.z_index || 0)) best = el;
    }
  }
  return best;
}

function onImageMove(e) {
  const img = e.target;
  const rect = img.getBoundingClientRect();
  const key = img.dataset.platform + '-' + img.dataset.screen;
  const data = yamlData[key];
  if (!data) return;

  const scaleX = img.naturalWidth / rect.width;
  const scaleY = img.naturalHeight / rect.height;
  const pxX = (e.clientX - rect.left) * scaleX;
  const pxY = (e.clientY - rect.top) * scaleY;

  const density = data._density || 1;
  const dpX = pxX / density;
  const dpY = pxY / density;

  const el = findElement(img.dataset.platform, img.dataset.screen, dpX, dpY);
  const tooltip = document.getElementById('tooltip');
  if (el) {
    let html = '<div class="id">' + esc(el.id) + '</div>';
    html += '<div><span class="field">type:</span> <span class="value">' + esc(el.type) + '</span></div>';
    html += '<div><span class="field">bounds:</span> <span class="value">' +
      el.bounds.x + ',' + el.bounds.y + ' ' + el.bounds.w + 'x' + el.bounds.h + '</span></div>';
    if (el.content) html += '<div><span class="field">content:</span> <span class="value">' + esc(el.content) + '</span></div>';
    if (el.z_index != null) html += '<div><span class="field">z:</span> <span class="value">' + el.z_index + '</span></div>';
    if (el.clickable) html += '<div><span class="field">clickable:</span> <span class="value">true</span></div>';
    if (el.accessible) html += '<div><span class="field">accessible:</span> <span class="value">true</span></div>';
    if (el.a11y_label) html += '<div><span class="field">a11y:</span> <span class="value">' + esc(el.a11y_label) + '</span></div>';
    if (el.font) html += '<div><span class="field">font:</span> <span class="value">' +
      el.font.family + ' ' + el.font.weight + ' ' + el.font.size + 'sp</span></div>';
    if (el.color) html += '<div><span class="field">color:</span> <span class="value">' + el.color + '</span></div>';
    if (el.background) html += '<div><span class="field">bg:</span> <span class="value">' + el.background + '</span></div>';
    tooltip.innerHTML = html;
    tooltip.style.display = 'block';
    tooltip.style.left = Math.min(e.clientX + 16, window.innerWidth - 380) + 'px';
    tooltip.style.top = Math.min(e.clientY + 16, window.innerHeight - 200) + 'px';
  } else {
    tooltip.style.display = 'none';
  }
}

let remarks = [];
let pendingRemark = null;

function onRightClick(e) {
  e.preventDefault();
  const img = e.target;
  const rect = img.getBoundingClientRect();
  const key = img.dataset.platform + '-' + img.dataset.screen;
  const data = yamlData[key];
  if (!data) return;

  const scaleX = img.naturalWidth / rect.width;
  const scaleY = img.naturalHeight / rect.height;
  const pxX = (e.clientX - rect.left) * scaleX;
  const pxY = (e.clientY - rect.top) * scaleY;
  const density = data._density || 1;
  const dpX = pxX / density;
  const dpY = pxY / density;

  const el = findElement(img.dataset.platform, img.dataset.screen, dpX, dpY);
  const dialog = document.getElementById('remark-dialog');
  document.getElementById('remark-elem').textContent = el ? el.id + ' (' + el.type + ')' : 'dp(' + Math.round(dpX) + ',' + Math.round(dpY) + ')';
  document.getElementById('remark-text').value = '';
  dialog.style.display = 'block';
  dialog.style.left = Math.min(e.clientX, window.innerWidth - 320) + 'px';
  dialog.style.top = Math.min(e.clientY, window.innerHeight - 160) + 'px';
  document.getElementById('remark-text').focus();

  const relX = (e.clientX - rect.left) / rect.width;
  const relY = (e.clientY - rect.top) / rect.height;
  pendingRemark = { platform: img.dataset.platform, screen: img.dataset.screen,
    elemId: el ? el.id : null, elemType: el ? el.type : null,
    dpX: Math.round(dpX), dpY: Math.round(dpY), relX, relY, mode };
}

function closeRemark() {
  document.getElementById('remark-dialog').style.display = 'none';
  pendingRemark = null;
}

function saveRemark() {
  const text = document.getElementById('remark-text').value.trim();
  if (!text || !pendingRemark) return;
  const r = { ...pendingRemark, text, ts: new Date().toISOString() };
  remarks.push(r);
  fetch('/remarks', { method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify(r) });
  closeRemark();
  renderPins();
  updateRemarksPanel();
}

function deleteRemark(idx) {
  remarks.splice(idx, 1);
  fetch('/remarks', { method: 'PUT', headers: {'Content-Type':'application/json'}, body: JSON.stringify(remarks) });
  renderPins();
  updateRemarksPanel();
}

function renderPins() {
  document.querySelectorAll('.remark-pin').forEach(p => p.remove());
  for (const [i, r] of remarks.entries()) {
    const img = document.querySelector('img[data-platform="'+r.platform+'"][data-screen="'+r.screen+'"]');
    if (!img) continue;
    const card = img.parentElement;
    const rect = img.getBoundingClientRect();
    const cardRect = card.getBoundingClientRect();
    const pin = document.createElement('div');
    pin.className = 'remark-pin';
    pin.style.left = (r.relX * rect.width) + 'px';
    pin.style.top = (r.relY * rect.height) + 'px';
    pin.title = r.text;
    card.appendChild(pin);
  }
}

function updateRemarksPanel() {
  const btn = document.getElementById('toggle-remarks');
  btn.textContent = 'Remarks (' + remarks.length + ')';
  const list = document.getElementById('remarks-list');
  list.innerHTML = '';
  for (const [i, r] of remarks.entries()) {
    const div = document.createElement('div');
    div.className = 'remark-item';
    div.innerHTML = '<span class="ri-delete" onclick="deleteRemark('+i+')">x</span>' +
      '<div class="ri-id">' + esc(r.elemId || 'no element') + '</div>' +
      '<div class="ri-loc">' + r.platform + '/' + r.screen + ' dp(' + r.dpX + ',' + r.dpY + ')</div>' +
      '<div class="ri-text">' + esc(r.text) + '</div>';
    list.appendChild(div);
  }
}

document.getElementById('toggle-remarks').addEventListener('click', () => {
  const panel = document.getElementById('remarks-panel');
  panel.style.display = panel.style.display === 'none' ? 'block' : 'none';
});

function esc(s) {
  if (!s) return '';
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

renderGrid();
loadYamls();
fetch('/remarks').then(r => r.ok ? r.json() : []).then(r => { remarks = r || []; renderPins(); updateRemarksPanel(); });
</script>
</body>
</html>"""


REMARKS_FILE = CAPTURE_DIR / "remarks.json"


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/" or self.path == "/index.html":
            self.serve_html()
        elif self.path.startswith("/img/"):
            self.serve_image(self.path[5:])
        elif self.path.startswith("/yaml/"):
            self.serve_yaml(self.path[6:])
        elif self.path == "/remarks":
            self.serve_remarks()
        else:
            self.send_error(404)

    def do_POST(self):
        if self.path == "/remarks":
            self.add_remark()
        else:
            self.send_error(404)

    def do_PUT(self):
        if self.path == "/remarks":
            self.replace_remarks()
        else:
            self.send_error(404)

    def serve_html(self):
        captures = discover_captures()
        html = HTML.replace("CAPTURES_JSON", json.dumps(captures))
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        self.wfile.write(html.encode())

    def serve_image(self, name):
        path = CAPTURE_DIR / name
        if not path.exists() or not path.is_file():
            self.send_error(404)
            return
        ext = path.suffix.lower()
        ct = {"png": "image/png", "jpg": "image/jpeg", "webp": "image/webp"}.get(ext.lstrip("."), "application/octet-stream")
        self.send_response(200)
        self.send_header("Content-Type", ct)
        self.send_header("Cache-Control", "max-age=60")
        self.end_headers()
        self.wfile.write(path.read_bytes())

    def serve_yaml(self, key):
        path = CAPTURE_DIR / f"{key}.yaml"
        if not path.exists():
            self.send_error(404)
            return
        import re as _re
        text = path.read_text()
        data = parse_yaml_to_json(text)
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(data).encode())

    def serve_remarks(self):
        remarks = []
        if REMARKS_FILE.exists():
            try:
                remarks = json.loads(REMARKS_FILE.read_text())
            except Exception:
                pass
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(remarks).encode())

    def add_remark(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        remark = json.loads(body)
        remarks = []
        if REMARKS_FILE.exists():
            try:
                remarks = json.loads(REMARKS_FILE.read_text())
            except Exception:
                pass
        remarks.append(remark)
        REMARKS_FILE.write_text(json.dumps(remarks, indent=2))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def replace_remarks(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        remarks = json.loads(body)
        REMARKS_FILE.write_text(json.dumps(remarks, indent=2))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, fmt, *args):
        pass


def discover_captures():
    caps = {}
    for f in sorted(CAPTURE_DIR.iterdir()):
        m = re.match(r"^(android|ios)-([\w]+)-(screenshot|stroke|fill)\.(png|jpg)$", f.name)
        if m:
            platform, screen, mode = m.group(1), m.group(2), m.group(3)
            key = f"{platform}-{screen}"
            if key not in caps:
                caps[key] = {"platform": platform, "screen": screen}
            caps[key][mode] = f.name
    return list(caps.values())


def parse_yaml_to_json(text):
    """Minimal YAML-to-dict parser for our semantic schema format."""
    result = {"elements": []}
    current = None
    sub = None

    for line in text.split("\n"):
        stripped = line.rstrip()
        if not stripped:
            continue

        if stripped.startswith("screen:"):
            result["screen"] = unquote(stripped.split(":", 1)[1].strip())
        elif stripped.startswith("device:"):
            result["device"] = unquote(stripped.split(":", 1)[1].strip())
        elif stripped.startswith("platform:"):
            result["platform"] = stripped.split(":", 1)[1].strip()
        elif stripped.startswith("timestamp:"):
            result["timestamp"] = unquote(stripped.split(":", 1)[1].strip())
        elif stripped.startswith("- id:"):
            current = {"id": unquote(stripped.split(":", 1)[1].strip()), "bounds": {}}
            sub = None
            result["elements"].append(current)
        elif current is not None:
            indent = len(line) - len(line.lstrip())
            if indent >= 4 and sub == "bounds":
                k, v = stripped.split(":", 1)
                current["bounds"][k.strip()] = safe_num(v.strip())
            elif indent >= 4 and sub == "font":
                k, v = stripped.split(":", 1)
                if "font" not in current:
                    current["font"] = {}
                current["font"][k.strip()] = safe_num(v.strip())
            elif indent >= 4 and sub == "padding":
                k, v = stripped.split(":", 1)
                if "padding" not in current:
                    current["padding"] = {}
                current["padding"][k.strip()] = safe_num(v.strip())
            elif stripped.strip() == "bounds:":
                sub = "bounds"
            elif stripped.strip() == "font:":
                sub = "font"
            elif stripped.strip() == "padding:":
                sub = "padding"
            else:
                sub = None
                if ":" in stripped:
                    k, v = stripped.strip().split(":", 1)
                    v = v.strip()
                    if k == "type":
                        current["type"] = v
                    elif k == "content":
                        current["content"] = unquote(v)
                    elif k in ("clickable", "enabled", "accessible"):
                        current[k] = v == "true"
                    elif k in ("z_index", "z-index"):
                        current["z_index"] = safe_num(v)
                    elif k in ("color", "background"):
                        current[k] = v.strip("'\"")
                    elif k in ("a11y_label", "a11y_id", "render", "platform_id"):
                        current[k] = unquote(v)
                    elif k == "corner_radius":
                        current[k] = safe_num(v)

    # Compute density hint from element bounds
    if result.get("platform") and result["elements"]:
        max_xw = max((e["bounds"].get("x", 0) + e["bounds"].get("w", 0)) for e in result["elements"])
        if max_xw > 0:
            result["_density"] = 1.0  # placeholder — JS needs image naturalWidth / max_xw

    return result


def unquote(s):
    s = s.strip()
    if (s.startswith('"') and s.endswith('"')) or (s.startswith("'") and s.endswith("'")):
        s = s[1:-1]
    return s.replace("\\n", "\n").replace('\\"', '"')


def safe_num(s):
    s = s.strip().strip("'\"")
    try:
        if "." in s:
            return float(s)
        return int(s)
    except ValueError:
        return s


if __name__ == "__main__":
    print(f"serving {CAPTURE_DIR} on http://localhost:{PORT}")
    server = http.server.HTTPServer(("127.0.0.1", PORT), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
