/* cadforge — studio UI. Plain DOM + three.js, no build step and no network
   beyond this same origin: three.js and OrbitControls are already global. */
const $ = (sel) => document.querySelector(sel);

const STATE = { assets: [], models: [], datasets: [], runs: [] };
const TICKED = new Set();          // artifact checkboxes ticked for a new dataset
let selectedAssetId = null;
let activeRunId = null;
let assetFilter = "all";
let pickerLane = "other";          // the lane whose tile opened the file picker

/* ---------------------------------------------------------------- viewport */

let renderer, scene, camera, controls, partGroup, gridHelper;
let viewMode = "shaded";
let partGeometry = null;
let fitRadius = 1;

function initViewport() {
  const host = $("#viewport");
  const canvas = $("#scene");
  renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setClearColor(0x0a0e13, 1);

  scene = new THREE.Scene();
  scene.background = new THREE.Color(0x0a0e13);

  camera = new THREE.PerspectiveCamera(45, 1, 0.05, 10000);
  camera.position.set(3.2, 2.4, 3.6);

  controls = new THREE.OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.screenSpacePanning = true;

  scene.add(new THREE.HemisphereLight(0xbdd6ff, 0x161c24, 1.05));
  const key = new THREE.DirectionalLight(0xffffff, 1.45);
  key.position.set(5, 7, 6);
  scene.add(key);
  const rim = new THREE.DirectionalLight(0x8fb4ff, 0.65);
  rim.position.set(-6, -3, -5);
  scene.add(rim);

  gridHelper = new THREE.GridHelper(10, 20, 0x2b3644, 0x161d26);
  scene.add(gridHelper);

  partGroup = new THREE.Group();
  scene.add(partGroup);

  new ResizeObserver(resizeViewport).observe(host);
  resizeViewport();
  renderLoop();
}

function resizeViewport() {
  const host = $("#viewport");
  const w = host.clientWidth || 1;
  const h = host.clientHeight || 1;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}

function renderLoop() {
  requestAnimationFrame(renderLoop);
  controls.update();
  renderer.render(scene, camera);
}

function clearPart() {
  while (partGroup.children.length) {
    const child = partGroup.children.pop();
    if (child.geometry) child.geometry.dispose();
    if (child.material) child.material.dispose();
  }
}

function hasGeometry() {
  return partGroup.children.length > 0;
}

function setPart(positions) {
  clearPart();
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geom.computeVertexNormals();
  geom.computeBoundingSphere();
  const c = geom.boundingSphere.center;
  geom.translate(-c.x, -c.y, -c.z);
  geom.computeBoundingSphere();
  partGeometry = geom;
  fitRadius = geom.boundingSphere.radius;
  buildPart();
  fitView();
}

function setPoints(positions) {
  clearPart();
  partGeometry = null;
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geom.computeBoundingSphere();
  const c = geom.boundingSphere.center;
  geom.translate(-c.x, -c.y, -c.z);
  geom.computeBoundingSphere();
  fitRadius = geom.boundingSphere.radius;
  const mat = new THREE.PointsMaterial({ color: 0x7dd3fc, size: fitRadius / 90, sizeAttenuation: true });
  partGroup.add(new THREE.Points(geom, mat));
  fitView();
}

function buildPart() {
  clearPart();
  if (!partGeometry) return;
  if (viewMode === "wire" || viewMode === "both") {
    const edges = new THREE.EdgesGeometry(partGeometry, 28);
    partGroup.add(new THREE.LineSegments(edges, new THREE.LineBasicMaterial({ color: 0x7dd3fc })));
  }
  if (viewMode === "shaded" || viewMode === "both") {
    const solid = new THREE.MeshStandardMaterial({ color: 0x9fb7d0, metalness: 0.55, roughness: 0.42 });
    partGroup.add(new THREE.Mesh(partGeometry, solid));
  }
}

function fitView() {
  const r = Math.max(fitRadius, 0.4);
  const dist = (r / Math.sin((camera.fov * Math.PI) / 360)) * 1.35;
  camera.position.set(0.85, 0.62, 1).normalize().multiplyScalar(dist);
  controls.target.set(0, 0, 0);
  controls.update();
}

/* --------------------------------------------------------------------- api */

async function api(path, opts) {
  const res = await fetch(path, opts);
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).error || detail; } catch (_) {}
    throw new Error(detail);
  }
  if (res.status === 204) return null;
  return res.json();
}

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}

async function refresh() {
  Object.assign(STATE, await api("/api/state"));
  renderAssets();
  renderModels();
  renderDatasets();
  renderRuns();
  renderPickers();
}

/* ------------------------------------------------------------------- lists */

function section(container, rows, emptyText) {
  container.textContent = "";
  if (!rows.length) { container.append(el("div", "empty", emptyText)); return; }
  rows.forEach((row) => container.append(row));
}

function renderAssets() {
  const list = $("#asset-list");
  const rows = STATE.assets
    .filter((a) => assetFilter === "all" || a.lane === assetFilter)
    .map((a) => {
      const row = el("div", "row" + (a.id === selectedAssetId ? " active" : ""));

      // The checkbox chooses an artifact for a dataset; it must not also select
      // the row, so its click is stopped from bubbling to the row handler.
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.dataset.id = a.id;
      cb.checked = TICKED.has(a.id);
      cb.addEventListener("click", (e) => e.stopPropagation());
      cb.addEventListener("change", () => {
        if (cb.checked) TICKED.add(a.id); else TICKED.delete(a.id);
      });
      row.append(cb);

      row.append(el("span", "chip-kind", a.kind));
      row.append(el("span", "name", a.name));

      const x = el("span", "x", "\u00d7");
      x.title = "Delete";
      x.addEventListener("click", async (e) => {
        e.stopPropagation();
        if (!confirm("Delete " + a.name + "?")) return;
        await api("/api/assets/" + a.id, { method: "DELETE" });
        TICKED.delete(a.id);
        if (selectedAssetId === a.id) {
          selectedAssetId = null;
          clearPart();
          setReadout("no part loaded");
          renderProps();
        }
        await refresh();
      });
      row.append(x);

      row.addEventListener("click", () => selectAsset(a.id));
      return row;
    });
  const empty = STATE.assets.length ? "No artifacts in this lane." : "No artifacts yet. Add files above.";
  section(list, rows, empty);
}

// A delete affordance for a row that can be removed. A model or a dataset a run
// is still using is refused by the server, and the reason is shown as-is rather
// than swallowing it behind a generic failure.
function removeButton(label, path) {
  const x = el("span", "x", "\u00d7");
  x.title = "Delete";
  x.addEventListener("click", async (e) => {
    e.stopPropagation();
    if (!confirm("Delete " + label + "?")) return;
    try {
      await api(path, { method: "DELETE" });
    } catch (err) {
      alert(err.message || String(err));
      return;
    }
    await refresh();
  });
  return x;
}

function renderModels() {
  const list = $("#model-list");
  const rows = STATE.models.map((m) => {
    const row = el("div", "row");
    row.append(el("span", "name", m.name));
    row.append(el("span", "chip-kind", m.kind));
    row.append(removeButton(m.name, "/api/models/" + m.id));
    return row;
  });
  section(list, rows, "No base models registered.");
}

function renderDatasets() {
  const list = $("#dataset-list");
  const rows = STATE.datasets.map((d) => {
    const row = el("div", "row");
    row.append(el("span", "name", d.name));
    const n = (d.asset_ids || []).length;
    row.append(el("span", "chip-kind", n + " file" + (n === 1 ? "" : "s")));
    row.append(removeButton(d.name, "/api/datasets/" + d.id));
    return row;
  });
  section(list, rows, "No datasets yet.");
}

function renderRuns() {
  const list = $("#run-list");
  const rows = STATE.runs.slice().reverse().map((r) => {
    const row = el("div", "row" + (r.id === activeRunId ? " active" : ""));
    row.append(el("span", "name", r.name));
    row.append(el("span", "chip-kind", r.status));
    row.addEventListener("click", () => { activeRunId = r.id; renderRuns(); showRun(r); });
    return row;
  });
  section(list, rows, "No runs yet.");
  const cancel = $("#btn-cancel-run");
  if (cancel) cancel.disabled = !activeRunId;
}

function renderPickers() {
  const ms = $("#sel-model");
  const ds = $("#sel-dataset");
  ms.textContent = "";
  ds.textContent = "";
  STATE.models.forEach((m) => ms.append(new Option(m.name, m.id)));
  STATE.datasets.forEach((d) => ds.append(new Option(d.name, d.id)));
  if (!STATE.models.length) ms.append(new Option("register a model first", ""));
  if (!STATE.datasets.length) ds.append(new Option("create a dataset first", ""));
  $("#btn-run").disabled = !(STATE.models.length && STATE.datasets.length);
}

/* --------------------------------------------------------------- selection */

function setReadout(text) { $("#readout").textContent = text; }

async function selectAsset(id) {
  selectedAssetId = id;
  renderAssets();
  renderProps();
  const asset = STATE.assets.find((a) => a.id === id);
  if (!asset) return;
  try {
    if (asset.kind === "mesh") {
      const mesh = await api("/api/assets/" + id + "/mesh");
      setPart(mesh.positions);
      const b = mesh.bbox;
      setReadout(asset.name + "  \u00b7  " + mesh.triangles.toLocaleString() + " tri  \u00b7  " +
        (b[3] - b[0]).toFixed(2) + " x " + (b[4] - b[1]).toFixed(2) + " x " + (b[5] - b[2]).toFixed(2));
    } else if (asset.kind === "step") {
      const hist = await api("/api/assets/" + id + "/history");
      const p = hist.preview || {};
      if (p.mode === "triangles") {
        setPart(p.positions);
      } else if (p.mode === "points") {
        setPoints(p.positions);
      } else {
        clearPart();
      }
      const s = hist.summary || {};
      setReadout(asset.name + "  \u00b7  " + s.faces + " faces  \u00b7  " + s.solids + " solid  \u00b7  " + p.mode + " preview");
    } else if (asset.kind === "history") {
      clearPart();
      const s = asset.step;
      const n = s ? Math.max(0, s.steps.length - 1) : 0;
      setReadout(asset.name + "  \u00b7  " + n + " feature" + (n === 1 ? "" : "s") + " exported from SolidWorks");
    } else if (asset.kind === "image" || asset.kind === "video") {
      clearPart();
      setReadout(asset.name + "  \u00b7  " + asset.kind + " — shown in the inspector");
    } else {
      clearPart();
      setReadout(asset.name + "  \u00b7  " + asset.kind + " (no geometry to draw)");
    }
  } catch (err) {
    clearPart();
    setReadout(asset.name + "  \u00b7  could not load: " + err.message);
  }
}

function renderProps() {
  const box = $("#props");
  box.textContent = "";
  const asset = STATE.assets.find((a) => a.id === selectedAssetId);
  $("#inspector-title").textContent = asset ? asset.name : "Inspector";
  if (!asset) {
    box.append(el("p", "hint", "Select an artifact to see what is inside it."));
    return;
  }

  const dl = el("dl", "kv");
  const put = (k, v, pre) => { dl.append(el("dt", null, k)); dl.append(el("dd", pre ? "pre" : null, v)); };
  put("kind", asset.kind);
  if (asset.lane) put("lane", asset.lane);
  if (asset.ext) put("format", "." + asset.ext);
  if (asset.size != null) put("size", (asset.size / 1024).toFixed(1) + " KB");
  if (asset.triangles != null) put("triangles", asset.triangles.toLocaleString());
  if (asset.bbox) put("bounds", asset.bbox.map((n) => n.toFixed(2)).join(", "));
  if (asset.header) put("file header", asset.header, true);
  if (asset.uploaded_at) put("uploaded", asset.uploaded_at);
  box.append(dl);

  // Anything the studio can show as a plain picture: an uploaded image, a
  // captured view or a drawing sheet are all served from the same file route.
  if (asset.kind === "image" || asset.kind === "view" || asset.kind === "drawing") {
    const wrap = el("div", "history");
    wrap.append(el("h3", null, "Picture"));
    const img = el("img", "sw-preview");
    img.src = "/api/assets/" + asset.id + "/file";
    img.alt = asset.name;
    wrap.append(img);
    box.append(wrap);
  }

  if (asset.step) renderStep(asset, box);
  if (asset.catalogue) renderCatalogue(asset, box);
  if (asset.actions && asset.actions.length) renderActions(asset, box);
}

/* The inferred or recorded feature tree of a STEP or SolidWorks document. */
function renderStep(asset, box) {
  const s = asset.step;
  const exported = asset.kind === "history";
  const wrap = el("div", "history");
  wrap.append(el("h3", null,
    exported ? "Feature tree, as recorded by SolidWorks" : "Reconstructed build history"));
  if (exported) {
    wrap.append(el("p", "hint", "The real modelling history, taken from inside the document - not inferred from geometry."));
  } else if (s.vertices != null) {
    wrap.append(el("p", "hint", s.vertices + " vertices, " + s.edges + " edges, " + s.faces + " faces - " +
      s.planar_faces + " planar, " + s.cylindrical_faces + " cylindrical, " + s.toroidal_faces + " toroidal"));
  }

  const ol = el("ol", "feature-tree");
  (s.steps || []).forEach((step) => {
    const li = el("li");
    li.append(el("span", "op", step.op));
    li.append(el("span", "detail", step.detail));
    ol.append(li);
  });
  wrap.append(ol);

  if (exported) {
    wrap.append(el("p", "hint", "This is the ground truth for training: the steps SolidWorks actually recorded. " +
      "Pair it with the geometry of the same part to teach a model to go the other way."));
  } else {
    wrap.append(el("p", "hint", "Rebuild this part from its own inferred operations, then measure the difference."));
    const btn = el("button", "primary", "Rebuild & compare");
    const result = el("div", "rebuild");
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      btn.textContent = "Rebuilding...";
      result.textContent = "";
      try {
        const r = await api("/api/rebuild", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ asset_id: asset.id, engine: "native" }),
        });
        if (r.triangles && r.triangles.length) setPart(r.triangles);
        renderRebuild(r, result);
      } catch (err) {
        result.append(el("p", "hint", "Rebuild failed: " + err.message));
      } finally {
        btn.disabled = false;
        btn.textContent = "Rebuild & compare";
      }
    });
    wrap.append(btn, result);
  }
  box.append(wrap);
}

/* The whole path from upload to result, drawn as a tree. It works the same for
   every kind of artifact, so nothing about how a file was read is hidden. */
function renderActions(asset, box) {
  const wrap = el("div", "history");
  wrap.append(el("h3", null, "How this was processed"));
  wrap.append(el("p", "hint", "Every stage, from the upload to what was read out of the file."));
  wrap.append(actionTree(asset.actions));
  box.append(wrap);
}

function actionTree(actions) {
  const ul = el("ul", "action-tree");
  actions.forEach((a) => {
    const li = el("li");
    const head = el("div", "act");
    head.append(el("span", "op", a.step));
    const detail = el("span", "detail", a.detail);
    if (a.children && a.children.length) detail.classList.add("muted");
    head.append(detail);
    li.append(head);
    if (a.children && a.children.length) li.append(actionTree(a.children));
    ul.append(li);
  });
  return ul;
}

/* A SolidWorks document: what is inside it, and its own preview image. */
function renderCatalogue(asset, box) {
  const c = asset.catalogue || {};
  const streams = c.streams || [];
  const wrap = el("div", "history");
  wrap.append(el("h3", null, c.kind_label || "SolidWorks document"));
  if (c.container) {
    wrap.append(el("p", "hint", c.container + "  \u00b7  " + streams.length + " streams  \u00b7  " +
      ((c.stream_bytes || 0) / 1024).toFixed(0) + " KB of stream data"));
  }

  if (c.preview) {
    const img = el("img", "sw-preview");
    img.src = "/api/assets/" + asset.id + "/preview";
    img.alt = "embedded preview of " + asset.name;
    wrap.append(img);
    wrap.append(el("p", "hint", "Preview embedded in the document (" + c.preview.stream + ", " +
      Math.round(c.preview.bytes / 1024) + " KB)."));
  }

  // The parameters the file itself carries: author, part number, and the custom
  // properties a shop fills in (Material, Mass, Revision...).
  const groups = (c.metadata || []).filter((g) => g.properties && g.properties.length);
  if (groups.length) {
    wrap.append(el("div", "section-label", "Parameters stored in the file"));
    const dl = el("dl", "params");
    groups.forEach((g) => {
      g.properties.forEach((p) => {
        dl.append(el("dt", null, p.name));
        dl.append(el("dd", null, p.value));
      });
    });
    wrap.append(dl);
  }

  // Documents this one points at: an assembly's components, a drawing's model.
  if (c.references && c.references.length) {
    wrap.append(el("div", "section-label", "Referenced documents (" + c.references.length + ")"));
    const ul = el("ul", "refs");
    c.references.forEach((r) => {
      const li = el("li");
      li.append(el("span", null, r.name));
      li.append(el("span", "from", r.stream));
      ul.append(li);
    });
    wrap.append(ul);
  }

  if (streams.length) {
    const table = el("table", "streams");
    const head = el("tr");
    ["stream", "size", "role"].forEach((h) => head.append(el("th", null, h)));
    table.append(head);
    streams.slice(0, 24).forEach((s) => {
      const tr = el("tr", s.recognised ? "known" : "unknown");
      tr.append(el("td", "name", s.name));
      tr.append(el("td", "num", (s.size / 1024).toFixed(1) + " KB"));
      tr.append(el("td", null, s.role));
      table.append(tr);
    });
    wrap.append(table);
    if (streams.length > 24) {
      wrap.append(el("p", "hint", "...and " + (streams.length - 24) + " more, largest first."));
    }
  }

  (c.notes || []).forEach((n) => wrap.append(el("p", "hint", n)));
  box.append(wrap);
}

/* The result of a rebuild: which engine ran, how close it came, and the script
   it produced. It is drawn into the box that the button created. */
function renderRebuild(r, box) {
  if (!box) box = document.querySelector("#props .rebuild");
  if (!box) return;
  box.textContent = "";
  const s = r.score;
  const dl = el("dl", "kv");
  const put = (k, v) => { dl.append(el("dt", null, k)); dl.append(el("dd", null, v)); };
  put("engine", r.engine + (r.ran ? "" : " (not run)"));
  put("operations", (r.ops || []).join(" + "));
  put("applied", String(r.ops_applied));
  if (s) {
    if (s.chamfer != null) put("chamfer", s.chamfer.toFixed(4) + " mm");
    if (s.max_gap != null) put("max gap", s.max_gap.toFixed(4) + " mm");
    if (s.size_delta) put("size delta", s.size_delta.map((n) => n.toFixed(3)).join(", "));
  }
  box.append(dl);
  if (r.note) box.append(el("p", "hint", r.note));
  (r.warnings || []).forEach((w) => box.append(el("p", "hint", w)));
  if (r.script) {
    const pre = el("pre", "console");
    pre.textContent = r.script;
    box.append(pre);
  }
  setReadout("rebuilt with " + r.engine + (s ? "  \u00b7  chamfer " + s.chamfer.toFixed(3) + " mm" : "  \u00b7  not run"));
}

/* ------------------------------------------------------------------ upload */

/* Each file in a bulk upload gets a row here, and the row keeps up with what is
   happening to it: queued, uploading, then done the moment the server has read
   the file and pushed the artifact back over the asset stream. */
const UPLOADS = [];

function renderActivity() {
  const list = $("#activity-list");
  list.textContent = "";
  if (!UPLOADS.length) {
    list.append(el("div", "empty", "No uploads yet."));
    return;
  }
  UPLOADS.slice(0, 80).forEach((u) => {
    const row = el("div", "row " + u.status);
    const col = el("div", "col");
    col.append(el("div", "name", u.name));
    if (u.detail) col.append(el("div", "err", u.detail));
    row.append(col);
    row.append(el("span", "status", u.status));
    list.append(row);
  });
}

/* A file the server has just finished reading: flip its row to done. */
function markUploaded(name) {
  const u = UPLOADS.find((x) => x.name === name && x.status === "uploading");
  if (u) {
    u.status = "done";
    renderActivity();
  }
}

/* Read the lane from a file name, so the viewport dropzone can sort a mixed
   batch without asking. The lane tiles pass their own lane explicitly instead. */
function laneFor(name) {
  const ext = ((name || "").split(".").pop() || "").toLowerCase();
  if (ext === "sldprt" || ext === "sldasm" || ext === "slddrw") return "sldworks";
  if (ext === "step" || ext === "stp") return "step";
  if (ext === "png" || ext === "jpg" || ext === "jpeg" || ext === "bmp" || ext === "webp" || ext === "gif") return "image";
  return "other";
}

async function postAsset(name, lane, body) {
  const q = "/api/assets?name=" + encodeURIComponent(name) + "&lane=" + encodeURIComponent(lane);
  const res = await fetch(q, {
    method: "POST",
    headers: { "content-type": "application/octet-stream" },
    body,
  });
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).error || detail; } catch (_) {}
    throw new Error(detail);
  }
  return res.json();
}

/* Upload a batch, a few at a time, so a large drop does not open a hundred
   connections at once and the feed stays readable. */
async function uploadFiles(files, lane) {
  const queued = [...files].map((f) => ({
    name: f.name,
    file: f,
    lane: lane || laneFor(f.name),
    status: "queued",
    detail: "",
  }));
  if (!queued.length) return;
  UPLOADS.unshift(...queued);
  renderActivity();

  let next = 0;
  const worker = async () => {
    while (next < queued.length) {
      const item = queued[next++];
      item.status = "uploading";
      renderActivity();
      try {
        await postAsset(item.file.name, item.lane, item.file);
        item.status = "done";
        item.detail = "";
      } catch (err) {
        item.status = "error";
        item.detail = err.message;
      }
      renderActivity();
    }
  };
  await Promise.all(Array.from({ length: Math.min(3, queued.length) }, worker));
  try { await refresh(); } catch (_) {}
}

/* ------------------------------------------------------- capture and sheet */

function dataURLToBlob(url) {
  const bin = atob(url.slice(url.indexOf(",") + 1));
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new Blob([bytes], { type: "image/png" });
}

function loadImage(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("could not load a view image"));
    img.src = url;
  });
}

function stripExt(name) {
  const i = (name || "").lastIndexOf(".");
  return i > 0 ? name.slice(0, i) : name;
}

/* Render the loaded part into four flat views. A second renderer with an alpha
   buffer is used so the background comes out transparent and the pictures sit
   cleanly on a drawing sheet; the main renderer is left untouched. */
async function renderViews() {
  const size = 512;
  const r = Math.max(fitRadius, 0.4);
  const dist = (r / Math.sin((45 * Math.PI) / 360)) * 1.2;
  const dirs = [
    { name: "front", pos: [0, 0, dist], up: [0, 1, 0] },
    { name: "top", pos: [0, dist, 0], up: [0, 0, -1] },
    { name: "right", pos: [dist, 0, 0], up: [0, 1, 0] },
    { name: "iso", pos: [0.9 * dist, 0.75 * dist, 0.9 * dist], up: [0, 1, 0] },
  ];

  const off = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
  off.setPixelRatio(1);
  off.setSize(size, size, false);
  off.setClearAlpha(0);
  const cam = new THREE.PerspectiveCamera(45, 1, 0.05, 10000);

  const wasGrid = gridHelper.visible;
  const wasBg = scene.background;
  gridHelper.visible = false;   // a grid would drown the part in a clean view
  scene.background = null;      // with an alpha buffer the background must stay empty

  const out = [];
  for (const d of dirs) {
    cam.position.set(d.pos[0], d.pos[1], d.pos[2]);
    cam.up.set(d.up[0], d.up[1], d.up[2]);
    cam.lookAt(0, 0, 0);
    off.render(scene, cam);
    out.push({ name: d.name, url: off.domElement.toDataURL("image/png") });
  }

  gridHelper.visible = wasGrid;
  scene.background = wasBg;
  off.dispose();
  if (off.forceContextLoss) off.forceContextLoss();
  return out;
}

/* Find a named property in a SolidWorks catalogue, so the title block can show
   title/author/material without the caller knowing the metadata layout. */
function metaValue(asset, key) {
  const groups = (asset && asset.catalogue && asset.catalogue.metadata) || [];
  for (const grp of groups) {
    for (const p of (grp.properties || [])) {
      if (String(p.name).toLowerCase() === key) return p.value;
    }
  }
  return null;
}

/* Compose the four views into one 1024 sheet with a title block, the way a
   drawing is normally laid out: 2 x 2 with the block in the bottom right. */
async function composeSheet(views, asset, name) {
  const S = 1024, cell = 512;
  const c = document.createElement("canvas");
  c.width = S; c.height = S;
  const g = c.getContext("2d");
  g.fillStyle = "#f5f7fa";
  g.fillRect(0, 0, S, S);

  const imgs = await Promise.all(views.map((v) => loadImage(v.url)));
  for (let i = 0; i < imgs.length; i++) {
    const x = (i % 2) * cell;
    const y = Math.floor(i / 2) * cell;
    g.strokeStyle = "#c9d2dc";
    g.lineWidth = 1.5;
    g.strokeRect(x + 8.5, y + 8.5, cell - 17, cell - 17);
    g.drawImage(imgs[i], x + 16, y + 16, cell - 32, cell - 32);
    g.fillStyle = "#6b7683";
    g.font = "14px ui-monospace, monospace";
    g.fillText(views[i].name, x + 22, y + cell - 20);
  }

  const bw = 400, bh = 160;
  const bx = S - bw - 14, by = S - bh - 14;
  g.fillStyle = "rgba(255,255,255,0.95)";
  g.fillRect(bx, by, bw, bh);
  g.strokeStyle = "#8b98a5";
  g.lineWidth = 1;
  g.strokeRect(bx + 0.5, by + 0.5, bw - 1, bh - 1);
  g.fillStyle = "#0f1216";
  g.font = "600 20px system-ui, sans-serif";
  g.fillText(String(name).slice(0, 30), bx + 14, by + 32);
  g.font = "13px system-ui, sans-serif";
  g.fillStyle = "#3a424c";
  const lines = [];
  const title = metaValue(asset, "title");
  const author = metaValue(asset, "author");
  const material = metaValue(asset, "material");
  if (title) lines.push("title     " + title);
  if (author) lines.push("author    " + author);
  if (material) lines.push("material  " + material);
  lines.push("date      " + new Date().toISOString().slice(0, 10));
  lines.forEach((t, i) => g.fillText(t, bx + 14, by + 60 + i * 21));

  return c.toDataURL("image/png");
}

function appendThumb(url, title) {
  const strip = $("#view-strip");
  const img = el("img", "view-thumb");
  img.src = url;
  img.alt = title;
  img.title = title;
  strip.append(img);
  while (strip.children.length > 12) strip.removeChild(strip.firstChild);
}

/* Capture the current geometry. "views" does the full job (four views plus a
   sheet); "drawing" only lays out the sheet, for a quick drawing export. */
async function captureViews(mode) {
  if (!hasGeometry()) {
    setReadout("nothing to capture: load a part first");
    return;
  }
  const asset = STATE.assets.find((a) => a.id === selectedAssetId);
  const name = asset ? stripExt(asset.name) : "part";
  try {
    setReadout("capturing views of " + name + "...");
    const views = await renderViews();
    views.forEach((v) => appendThumb(v.url, name + " " + v.name));
    if (mode === "views") {
      for (const v of views) {
        await postAsset(name + "-" + v.name + ".png", "view", dataURLToBlob(v.url));
      }
    }
    const sheetUrl = await composeSheet(views, asset, name);
    appendThumb(sheetUrl, name + " sheet");
    await postAsset(name + "-sheet.png", "drawing", dataURLToBlob(sheetUrl));
    try { await refresh(); } catch (_) {}
    setReadout(mode === "views"
      ? "captured " + views.length + " views and a sheet for " + name
      : "exported a drawing sheet for " + name);
  } catch (err) {
    setReadout("capture failed: " + err.message);
  }
}

/* ------------------------------------------------------- picture to steps */

let visionReady = false;

function setVisionStatus(text) { $("#vision-status").textContent = text; }

function setupVision() {
  const drop = $("#vision-drop");
  const input = $("#vision-input");
  drop.addEventListener("click", () => input.click());
  drop.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); input.click(); }
  });
  ["dragenter", "dragover"].forEach((t) => drop.addEventListener(t, (e) => {
    e.preventDefault(); drop.classList.add("on");
  }));
  ["dragleave", "drop"].forEach((t) => drop.addEventListener(t, (e) => {
    e.preventDefault(); drop.classList.remove("on");
  }));
  drop.addEventListener("drop", (e) => {
    const f = e.dataTransfer.files[0];
    if (f) loadVisionImage(f);
  });
  input.addEventListener("change", (e) => {
    const f = e.target.files[0];
    if (f) loadVisionImage(f);
    e.target.value = "";
  });
  $("#vision-run").addEventListener("click", runVision);
}

function loadVisionImage(file) {
  const url = URL.createObjectURL(file);
  const img = new Image();
  img.onload = () => {
    drawVisionImage(img);
    URL.revokeObjectURL(url);
    visionReady = true;
    setVisionStatus("picture ready — press Show the steps");
  };
  img.onerror = () => {
    URL.revokeObjectURL(url);
    setVisionStatus("could not read that picture");
  };
  img.src = url;
}

/* Letterbox the picture into the 256 x 256 canvas, keeping its aspect ratio and
   leaving the unused area transparent, which is exactly what the model wants. */
function drawVisionImage(img) {
  const c = $("#vision-canvas");
  const g = c.getContext("2d");
  const W = c.width, H = c.height;
  g.clearRect(0, 0, W, H);
  const scale = Math.min(W / img.width, H / img.height) || 1;
  const w = img.width * scale, h = img.height * scale;
  g.drawImage(img, (W - w) / 2, (H - h) / 2, w, h);
}

async function runVision() {
  if (!visionReady) { setVisionStatus("choose a picture first"); return; }
  const btn = $("#vision-run");
  const box = $("#vision-result");
  btn.disabled = true;
  box.textContent = "";
  setVisionStatus("thinking...");
  try {
    const c = $("#vision-canvas");
    const data = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let q = "/api/predict?w=" + c.width + "&h=" + c.height;
    const asset = STATE.assets.find((a) => a.id === selectedAssetId);
    if (asset && asset.kind === "image") q += "&asset_id=" + encodeURIComponent(asset.id);
    const res = await fetch(q, {
      method: "POST",
      headers: { "content-type": "application/octet-stream" },
      body: data,
    });
    if (!res.ok) {
      let detail = res.statusText;
      try { detail = (await res.json()).error || detail; } catch (_) {}
      throw new Error(detail);
    }
    const out = await res.json();
    renderVision(out);
    setVisionStatus((out.steps || []).length + " steps, confidence " +
      Math.round((out.confidence || 0) * 100) + "%");
  } catch (err) {
    setVisionStatus("error: " + err.message);
  } finally {
    btn.disabled = false;
  }
}

function confidenceMeter(value) {
  const pct = Math.round(Math.max(0, Math.min(1, value || 0)) * 100);
  const wrap = el("div", "confidence");
  wrap.append(el("span", null, "confidence"));
  const meter = el("div", "meter");
  const fill = el("i");
  fill.style.width = pct + "%";
  meter.append(fill);
  wrap.append(meter);
  wrap.append(el("span", null, pct + "%"));
  return wrap;
}

function renderVision(out) {
  const box = $("#vision-result");
  box.textContent = "";
  if (out.source) box.append(el("p", "hint", "source: " + out.source));

  const ol = el("ol", "steps");
  (out.steps || []).forEach((s) => {
    const li = el("li");
    li.append(el("span", "op", s.op));
    li.append(el("span", "detail", s.detail));
    ol.append(li);
  });
  box.append(ol);
  box.append(confidenceMeter(out.confidence));
  (out.notes || []).forEach((n) => box.append(el("p", "hint", n)));

  const btn = el("button", "primary", "Rebuild this");
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "Rebuilding...";
    try {
      const r = await api("/api/rebuild", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ program: out.program, engine: "native" }),
      });
      if (r.triangles && r.triangles.length) setPart(r.triangles);
      setReadout("rebuilt the predicted steps: " + (r.ops || []).join(" + "));
    } catch (err) {
      box.append(el("p", "hint", "rebuild failed: " + err.message));
    } finally {
      btn.disabled = false;
      btn.textContent = "Rebuild this";
    }
  });
  box.append(btn);
}

/* ---------------------------------------------------------------- system */

function sysLine(tone, label, value) {
  const line = el("div", "sys-line " + tone);
  line.append(el("i", "dot"));
  line.append(el("span", "sys-label", label));
  line.append(el("span", "sys-value", value));
  return line;
}

async function renderSystem() {
  const box = $("#system-status");
  box.textContent = "";
  let sys;
  try {
    sys = await api("/api/system");
  } catch (err) {
    box.append(sysLine("err", "System", "unavailable: " + err.message));
    return;
  }
  const gpu = sys.gpu || {};
  box.append(sysLine(gpu.present ? "ok" : "warn", "GPU",
    gpu.present
      ? (gpu.name || "present") + (gpu.vram_mb ? "  \u00b7  " + gpu.vram_mb + " MB" : "")
      : "none found — training falls back to CPU"));
  const py = sys.python || {};
  box.append(sysLine(py.present ? "ok" : "err", "Python",
    py.present
      ? (py.version || "present") + (py.torch ? "  \u00b7  torch " + py.torch : "  \u00b7  torch missing") + (py.cuda ? "  \u00b7  cuda" : "")
      : "not found"));
  box.append(sysLine(sys.trainer ? "ok" : "warn", "Training", sys.trainer || "no trainer available"));
  (sys.advice || []).forEach((a) => box.append(el("p", "hint", a)));
}

/* ------------------------------------------------------------- run console */

function showRun(run) {
  $("#run-pill").textContent = run.status;
  $("#run-stage").textContent = run.stage + "  \u00b7  " + Math.round(run.progress * 100) + "%";
  $("#run-bar").style.width = Math.round(run.progress * 100) + "%";
  const log = $("#run-log");
  log.textContent = "";
  (run.log || []).forEach((line) => {
    log.append(el("div", line.level === "info" ? null : line.level, line.at.slice(11, 19) + "  " + line.message));
  });
  log.scrollTop = log.scrollHeight;
  paintStages(run);
  const cancel = $("#btn-cancel-run");
  if (cancel) cancel.disabled = !run || !activeRunId;
}

function paintStages(run) {
  const list = $("#stage-list");
  const done = new Set();
  (run.log || []).forEach((l) => {
    const label = l.message.split(" - ")[0];
    if (label) done.add(label);
  });
  [...list.children].forEach((li) => {
    li.className = "";
    if (done.has(li.dataset.label)) li.className = "done";
    if (li.dataset.label === run.stage) li.className = "now";
  });
}

/* The trainer names its own stages, so the ladder is built from the server, not
   hard-coded here; a run then just marks which rung it is on. */
async function renderTrainer() {
  const info = await api("/api/trainer");
  $("#trainer-pill").textContent = "trainer: " + (info.name || "unknown");
  const list = $("#stage-list");
  list.textContent = "";
  (info.stages || []).forEach((s) => {
    const li = el("li", null, s.label);
    li.dataset.label = s.label;
    li.title = s.detail;
    list.append(li);
  });
}

/* ------------------------------------------------------------------ events */

function connectEvents() {
  const pill = $("#conn-pill");
  const text = pill.lastChild;
  const es = new EventSource("/api/events");
  es.addEventListener("open", () => { pill.dataset.state = "open"; text.textContent = "live"; });
  es.addEventListener("error", () => { pill.dataset.state = "closed"; text.textContent = "reconnecting"; });
  es.addEventListener("run", (ev) => {
    const run = JSON.parse(ev.data);
    const i = STATE.runs.findIndex((r) => r.id === run.id);
    if (i >= 0) STATE.runs[i] = run; else STATE.runs.push(run);
    renderRuns();
    if (!activeRunId || activeRunId === run.id) { activeRunId = run.id; showRun(run); }
  });

  // Artifacts are announced the moment they have been processed, so a bulk
  // upload fills the list and the feed as it goes.
  const assets = new EventSource("/api/assets/events");
  assets.addEventListener("asset", (ev) => {
    const a = JSON.parse(ev.data);
    const i = STATE.assets.findIndex((x) => x.id === a.id);
    if (i >= 0) STATE.assets[i] = a; else STATE.assets.push(a);
    renderAssets();
    markUploaded(a.name);
    if (selectedAssetId === a.id) renderProps();
  });
}

/* ----------------------------------------------------------------- filter */

function wireFilters() {
  document.querySelectorAll("#asset-filters .chip").forEach((chip) => {
    chip.addEventListener("click", () => {
      assetFilter = chip.dataset.filter || "all";
      document.querySelectorAll("#asset-filters .chip").forEach((c) => c.classList.remove("on"));
      chip.classList.add("on");
      renderAssets();
    });
  });
}

/* -------------------------------------------------------------------- wire */

function wire() {
  // Each lane tile opens the same hidden picker, but remembers which lane it was
  // so the files land in the right place even if their extension says otherwise.
  $("#file-input").addEventListener("change", (e) => {
    const files = [...e.target.files];
    uploadFiles(files, pickerLane);
    e.target.value = "";
  });
  const laneSelectors = ["#lane-sldworks", "#lane-step", "#lane-image"];
  laneSelectors.forEach((sel) => {
    const tile = $(sel);
    tile.addEventListener("click", () => { pickerLane = tile.dataset.lane; $("#file-input").click(); });
    ["dragenter", "dragover"].forEach((t) => tile.addEventListener(t, (e) => {
      e.preventDefault(); e.stopPropagation(); tile.classList.add("drag");
    }));
    ["dragleave", "drop"].forEach((t) => tile.addEventListener(t, (e) => {
      e.preventDefault(); e.stopPropagation(); tile.classList.remove("drag");
    }));
    tile.addEventListener("drop", (e) => {
      if (e.dataTransfer.files.length) uploadFiles([...e.dataTransfer.files], tile.dataset.lane);
    });
  });

  const dz = $("#dropzone");
  const vp = $("#viewport");
  ["dragenter", "dragover"].forEach((t) => vp.addEventListener(t, (e) => {
    e.preventDefault(); dz.classList.add("on");
  }));
  ["dragleave", "drop"].forEach((t) => vp.addEventListener(t, (e) => {
    e.preventDefault(); dz.classList.remove("on");
  }));
  vp.addEventListener("drop", (e) => {
    if (e.dataTransfer.files.length) uploadFiles([...e.dataTransfer.files], null);
  });

  document.querySelectorAll(".tool[data-view]").forEach((btn) => btn.addEventListener("click", () => {
    document.querySelectorAll(".tool[data-view]").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    viewMode = btn.dataset.view;
    buildPart();
  }));
  $("#btn-fit").addEventListener("click", fitView);
  $("#btn-grid").addEventListener("click", () => { gridHelper.visible = !gridHelper.visible; });
  $("#btn-capture").addEventListener("click", () => captureViews("views"));
  $("#btn-drawing").addEventListener("click", () => captureViews("drawing"));
  $("#btn-clear-activity").addEventListener("click", () => { UPLOADS.length = 0; renderActivity(); });
  $("#btn-refresh-system").addEventListener("click", renderSystem);

  $("#btn-model").addEventListener("click", async () => {
    const name = prompt("Name the base model", "qwen-cad-base");
    if (!name) return;
    const kind = prompt("Kind: vision, text, base or custom", "base") || "base";
    const params = parseInt(prompt("Parameter count, optional", "0") || "0", 10);
    const notes = prompt("Notes, optional", "") || "";
    await api("/api/models", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name, kind, params: params || null, notes }),
    });
    await refresh();
  });

  $("#btn-dataset").addEventListener("click", async () => {
    const known = new Set(STATE.assets.map((a) => a.id));
    const picked = [...TICKED].filter((id) => known.has(id));
    if (!picked.length) { alert("Tick one or more artifacts first."); return; }
    const name = prompt("Dataset name", "cad-history-v1");
    if (!name) return;
    await api("/api/datasets", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name, asset_ids: picked }),
    });
    await refresh();
  });

  $("#btn-run").addEventListener("click", async () => {
    const base_model_id = $("#sel-model").value;
    const dataset_id = $("#sel-dataset").value;
    if (!base_model_id || !dataset_id) return;
    const run = await api("/api/runs", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ base_model_id, dataset_id }),
    });
    activeRunId = run.id;
    const i = STATE.runs.findIndex((r) => r.id === run.id);
    if (i >= 0) STATE.runs[i] = run; else STATE.runs.push(run);
    renderRuns();
    showRun(run);
  });

  $("#btn-cancel-run").addEventListener("click", async () => {
    if (!activeRunId) return;
    const run = await api("/api/runs/" + activeRunId + "/cancel", { method: "POST" });
    const i = STATE.runs.findIndex((r) => r.id === run.id);
    if (i >= 0) STATE.runs[i] = run; else STATE.runs.push(run);
    renderRuns();
    showRun(run);
  });
}

(async function main() {
  initViewport();
  wire();
  wireFilters();
  setupVision();
  connectEvents();
  renderActivity();
  renderProps();
  setReadout("no part loaded");
  try { await renderTrainer(); } catch (err) { $("#trainer-pill").textContent = "trainer: offline"; }
  await renderSystem();
  try { await refresh(); } catch (err) { setReadout("could not reach the studio API"); }
})().catch((err) => { console.error(err); });
