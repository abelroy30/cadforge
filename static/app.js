/* cadforge — studio UI. Plain DOM + three.js, no build step required. */
const $ = (sel) => document.querySelector(sel);

const STATE = { assets: [], models: [], datasets: [], runs: [] };
let selectedAssetId = null;
let activeRunId = null;

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

/* ------------------------------------------------------------------ lists */

function section(container, rows, emptyText) {
  container.textContent = "";
  if (!rows.length) { container.append(el("div", "empty", emptyText)); return; }
  rows.forEach((row) => container.append(row));
}

function renderAssets() {
  const list = $("#asset-list");
  const rows = STATE.assets.map((a) => {
    const row = el("div", "row" + (a.id === selectedAssetId ? " active" : ""));
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.dataset.id = a.id;
    cb.addEventListener("click", (e) => e.stopPropagation());
    row.append(cb);
    row.append(el("span", "name", a.name));
    row.append(el("span", "kind", a.kind));
    const x = el("span", "x", "\u00d7");
    x.title = "Delete";
    x.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (!confirm("Delete " + a.name + "?")) return;
      await api("/api/assets/" + a.id, { method: "DELETE" });
      if (selectedAssetId === a.id) { selectedAssetId = null; clearPart(); setReadout("no part loaded"); }
      await refresh();
    });
    row.append(x);
    row.addEventListener("click", () => selectAsset(a.id));
    return row;
  });
  section(list, rows, "No artifacts yet. Upload one.");
}

function renderModels() {
  const list = $("#model-list");
  const rows = STATE.models.map((m) => {
    const row = el("div", "row");
    row.append(el("span", "name", m.name));
    row.append(el("span", "kind", m.kind));
    return row;
  });
  section(list, rows, "No base models registered.");
}

function renderDatasets() {
  const list = $("#dataset-list");
  const rows = STATE.datasets.map((d) => {
    const row = el("div", "row");
    row.append(el("span", "name", d.name));
    row.append(el("span", "kind", d.asset_ids.length + " files"));
    return row;
  });
  section(list, rows, "No datasets yet.");
}

function renderRuns() {
  const list = $("#run-list");
  const rows = STATE.runs.slice().reverse().map((r) => {
    const row = el("div", "row" + (r.id === activeRunId ? " active" : ""));
    row.append(el("span", "name", r.name));
    row.append(el("span", "kind", r.status));
    row.addEventListener("click", () => { activeRunId = r.id; renderRuns(); showRun(r); });
    return row;
  });
  section(list, rows, "No runs yet.");
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
  if (asset.kind === "mesh") {
    const mesh = await api("/api/assets/" + id + "/mesh");
    setPart(mesh.positions);
    const b = mesh.bbox;
    setReadout(asset.name + "  ·  " + mesh.triangles.toLocaleString() + " tri  ·  " +
      (b[3] - b[0]).toFixed(2) + " x " + (b[4] - b[1]).toFixed(2) + " x " + (b[5] - b[2]).toFixed(2));
  } else if (asset.kind === "step") {
    const hist = await api("/api/assets/" + id + "/history");
    const p = hist.preview;
    if (p.mode === "triangles") {
      setPart(p.positions);
    } else if (p.mode === "points") {
      setPoints(p.positions);
    } else {
      clearPart();
    }
    const s = hist.summary;
    setReadout(asset.name + "  ·  " + s.faces + " faces  ·  " + s.solids + " solid  ·  " +
      p.mode + " preview");
  } else if (asset.kind === "history") {
    clearPart();
    const s = asset.step;
    const n = s ? Math.max(0, s.steps.length - 1) : 0;
    setReadout(asset.name + "  ·  " + n + " feature" + (n === 1 ? "" : "s") +
      " exported from SolidWorks");
  } else {
    clearPart();
    setReadout(asset.name + "  ·  " + asset.kind + " (no geometry to draw)");
  }
}

function renderProps() {
  const box = $("#props");
  box.textContent = "";
  const asset = STATE.assets.find((a) => a.id === selectedAssetId);
  $("#inspector-title").textContent = asset ? asset.name : "Selection";
  if (!asset) {
    box.append(el("p", "hint", "Select an artifact in the viewport tree."));
    return;
  }
  const dl = el("dl", "kv");
  const put = (k, v, pre) => { dl.append(el("dt", null, k)); dl.append(el("dd", pre ? "pre" : null, v)); };
  put("kind", asset.kind);
  put("format", "." + asset.ext);
  put("size", (asset.size / 1024).toFixed(1) + " KB");
  if (asset.triangles != null) put("triangles", asset.triangles.toLocaleString());
  if (asset.bbox) put("bounds", asset.bbox.map((n) => n.toFixed(2)).join(", "));
  if (asset.header) put("file header", asset.header, true);
  put("uploaded", asset.uploaded_at);
  box.append(dl);

  if (asset.step) {
    const s = asset.step;
    const exported = asset.kind === "history";
    const wrap = el("div", "history");
    wrap.append(el("h3", null,
      exported ? "Feature tree, as recorded by SolidWorks" : "Reconstructed build history"));
    wrap.append(el("p", "hint", exported
      ? "The real modelling history, taken from inside the document - not inferred from geometry."
      : s.vertices + " vertices, " + s.edges + " edges, " + s.faces + " faces - " +
        s.planar_faces + " planar, " + s.cylindrical_faces + " cylindrical, " +
        s.toroidal_faces + " toroidal"));
    const ol = el("ol", "feature-tree");
    s.steps.forEach((step) => {
      const li = el("li");
      li.append(el("span", "op", step.op));
      li.append(el("span", "detail", step.detail));
      ol.append(li);
    });
    wrap.append(ol);

    if (exported) {
      wrap.append(el("p", "hint",
        "This is the ground truth for training: the steps SolidWorks actually recorded. " +
        "Pair it with the geometry of the same part to teach a model to go the other way."));
    } else {
      const result = el("div", "rebuild");
      const btn = el("button", "primary", "Rebuild & compare");
      btn.addEventListener("click", async () => {
        btn.disabled = true;
        btn.textContent = "Rebuilding...";
        try {
          const r = await api("/api/rebuild", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ asset_id: asset.id, engine: "native" }),
          });
          if (r.triangles && r.triangles.length) setPart(r.triangles);
          renderRebuild(r);
        } catch (err) {
          alert("Rebuild failed: " + err.message);
        } finally {
          btn.disabled = false;
          btn.textContent = "Rebuild & compare";
        }
      });
      wrap.append(el("p", "hint", "Rebuild this part from its own inferred operations, then measure the difference."));
      wrap.append(btn, result);
    }
    box.append(wrap);
  }

  if (asset.catalogue) renderCatalogue(asset, box);
  if (asset.actions && asset.actions.length) renderActions(asset, box);
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

/// A SolidWorks document: what is inside it, and its own preview image.
function renderCatalogue(asset, box) {
  const c = asset.catalogue;
  const wrap = el("div", "history");
  wrap.append(el("h3", null, c.kind_label));
  wrap.append(el("p", "hint", c.container + "  ·  " + c.streams.length + " streams  ·  " +
    (c.stream_bytes / 1024).toFixed(0) + " KB of stream data"));

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
  const groups = (c.metadata || []).filter((g) => g.properties.length);
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

  const table = el("table", "streams");
  const head = el("tr");
  ["stream", "size", "role"].forEach((h) => head.append(el("th", null, h)));
  table.append(head);
  c.streams.slice(0, 24).forEach((s) => {
    const tr = el("tr", s.recognised ? "known" : "unknown");
    tr.append(el("td", "name", s.name));
    tr.append(el("td", "num", (s.size / 1024).toFixed(1) + " KB"));
    tr.append(el("td", null, s.role));
    table.append(tr);
  });
  wrap.append(table);
  if (c.streams.length > 24) {
    wrap.append(el("p", "hint", "...and " + (c.streams.length - 24) + " more, largest first."));
  }

  c.notes.forEach((n) => wrap.append(el("p", "hint", n)));
  box.append(wrap);
}

function renderRebuild(r) {
  const box = document.querySelector("#props .rebuild");
  if (!box) return;
  box.textContent = "";
  const s = r.score;
  const dl = el("dl", "kv");
  const put = (k, v) => { dl.append(el("dt", null, k)); dl.append(el("dd", null, v)); };
  put("engine", r.engine + (r.ran ? "" : " (not run)"));
  put("operations", (r.ops || []).join(" + "));
  put("applied", String(r.ops_applied));
  if (s) {
    put("chamfer", s.chamfer.toFixed(4) + " mm");
    put("max gap", s.max_gap.toFixed(4) + " mm");
    put("size delta", s.size_delta.map((n) => n.toFixed(3)).join(", "));
  }
  box.append(dl);
  if (r.note) box.append(el("p", "hint", r.note));
  (r.warnings || []).forEach((w) => box.append(el("p", "hint", w)));
  if (r.script) {
    const pre = el("pre", "console");
    pre.textContent = r.script;
    box.append(pre);
  }
  const readout = document.querySelector("#readout");
  if (readout) {
    readout.textContent = "rebuilt with " + r.engine +
      (s ? "  ·  chamfer " + s.chamfer.toFixed(3) + " mm" : "  ·  not run");
  }
}

/* ------------------------------------------------------------------ upload */

/* Each file in a bulk upload gets a row here, and the row keeps up with what is
   happening to it: queued, uploading, then done the moment the server has read
   the file and pushed the artifact back over the asset stream. */
const UPLOADS = [];

function renderActivity() {
  const list = $("#activity-list");
  if (!UPLOADS.length) {
    list.textContent = "";
    list.append(el("div", "empty", "No uploads yet."));
    return;
  }
  list.textContent = "";
  UPLOADS.slice(0, 80).forEach((u) => {
    const row = el("div", "row " + u.status);
    const col = el("div");
    col.style.flex = "1";
    col.style.minWidth = "0";
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

/* Upload a batch, a few at a time, so a large drop does not open a hundred
   connections at once and the feed stays readable. */
async function uploadFiles(files) {
  const queued = [...files].map((f) => ({ name: f.name, file: f, status: "queued", detail: "" }));
  UPLOADS.unshift(...queued);
  renderActivity();

  let next = 0;
  const worker = async () => {
    while (next < queued.length) {
      const item = queued[next++];
      item.status = "uploading";
      renderActivity();
      try {
        const res = await fetch("/api/assets?name=" + encodeURIComponent(item.file.name), {
          method: "POST",
          headers: { "content-type": "application/octet-stream" },
          body: item.file,
        });
        if (!res.ok) throw new Error((await res.json()).error || res.statusText);
        item.status = "done";
        item.detail = "";
      } catch (err) {
        item.status = "error";
        item.detail = err.message;
      }
      renderActivity();
    }
  };
  const lanes = Math.min(3, queued.length);
  await Promise.all(Array.from({ length: lanes }, worker));
  await refresh();
}

/* ------------------------------------------------------------- run console */

function showRun(run) {
  $("#run-pill").textContent = run.status;
  $("#run-stage").textContent = run.stage + "  ·  " + Math.round(run.progress * 100) + "%";
  $("#run-bar").style.width = Math.round(run.progress * 100) + "%";
  const log = $("#run-log");
  log.textContent = "";
  run.log.forEach((line) => {
    const span = el("div", line.level === "info" ? null : line.level, line.at.slice(11, 19) + "  " + line.message);
    log.append(span);
  });
  log.scrollTop = log.scrollHeight;
  paintStages(run);
}

function paintStages(run) {
  const list = $("#stage-list");
  const done = new Set();
  run.log.forEach((l) => {
    const label = l.message.split(" - ")[0];
    if (label) done.add(label);
  });
  [...list.children].forEach((li) => {
    li.className = "";
    if (done.has(li.dataset.label)) li.className = "done";
    if (li.dataset.label === run.stage) li.className = "now";
  });
}

async function renderTrainer() {
  const info = await api("/api/trainer");
  $("#trainer-pill").textContent = "trainer: " + info.name;
  const list = $("#stage-list");
  list.textContent = "";
  info.stages.forEach((s) => {
    const li = el("li", null, s.label);
    li.dataset.label = s.label;
    li.title = s.detail;
    list.append(li);
  });
}

/* ------------------------------------------------------------------ events */

function connectEvents() {
  const pill = $("#conn-pill");
  const es = new EventSource("/api/events");
  es.addEventListener("open", () => { pill.dataset.state = "open"; pill.lastChild.textContent = "live"; });
  es.addEventListener("error", () => { pill.dataset.state = "closed"; pill.lastChild.textContent = "reconnecting"; });
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

/* -------------------------------------------------------------------- wire */

function wire() {
  $("#file-input").addEventListener("change", (e) => { uploadFiles([...e.target.files]); e.target.value = ""; });
  $("#btn-upload").addEventListener("click", () => $("#file-input").click());

  const dz = $("#dropzone");
  const vp = $("#viewport");
  ["dragenter", "dragover"].forEach((t) => vp.addEventListener(t, (e) => { e.preventDefault(); dz.classList.add("on"); }));
  ["dragleave", "drop"].forEach((t) => vp.addEventListener(t, (e) => { e.preventDefault(); dz.classList.remove("on"); }));
  vp.addEventListener("drop", (e) => { if (e.dataTransfer.files.length) uploadFiles([...e.dataTransfer.files]); });

  document.querySelectorAll(".tool[data-view]").forEach((btn) => btn.addEventListener("click", () => {
    document.querySelectorAll(".tool[data-view]").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    viewMode = btn.dataset.view;
    buildPart();
  }));
  $("#btn-fit").addEventListener("click", fitView);
  $("#btn-grid").addEventListener("click", () => { gridHelper.visible = !gridHelper.visible; });
  $("#btn-clear-activity").addEventListener("click", () => { UPLOADS.length = 0; renderActivity(); });

  $("#btn-model").addEventListener("click", async () => {
    const name = prompt("Name the base model", "qwen-cad-base");
    if (!name) return;
    const kind = prompt("Kind: base or custom", "base") || "base";
    const params = parseInt(prompt("Parameter count, optional", "0") || "0", 10);
    await api("/api/models", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name, kind, params: params || null }),
    });
    await refresh();
  });

  $("#btn-dataset").addEventListener("click", async () => {
    const picked = [...document.querySelectorAll("#asset-list input:checked")].map((cb) => cb.dataset.id).filter(Boolean);
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
}

(async function main() {
  initViewport();
  wire();
  connectEvents();
  renderActivity();
  await renderTrainer();
  await refresh();
  renderProps();
})().catch((err) => { console.error(err); alert(String(err)); });
