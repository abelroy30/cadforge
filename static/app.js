/* cadforge — studio UI. Plain DOM + three.js, no build step and no network
   beyond this same origin: three.js and OrbitControls are already global. */
const $ = (sel) => document.querySelector(sel);

const STATE = { assets: [], sources: [], models: [], datasets: [], runs: [] };
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
  renderSources();
  renderModels();
  renderDatasets();
  renderRuns();
  renderPickers();
  renderRewards();
  renderRewardPicker();
  try { await renderStorage(); } catch (_) { /* the panel says so itself */ }
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

/* Each file the studio uploads gets a row here, and the row keeps up with what is
   happening to it: queued, uploading, then done the moment the server has read
   the file and pushed the artifact back over the asset stream. */
const UPLOADS = [];

/* What is happening to folders and files right now, from the work stream: every
   folder the studio walks and every file it reads, as it reads it. Newest first
   and capped, because this is a window on background work rather than a log to
   keep. */
const WORK = [];
const WORK_LIMIT = 200;

function activityRow(name, status, detail) {
  const row = el("div", "row " + status);
  const col = el("div", "col");
  col.append(el("div", "name", name));
  if (detail) col.append(el("div", "err", detail));
  row.append(col);
  row.append(el("span", "status", status));
  return row;
}

/* One feed for both kinds of work, in the order it happened. A folder scan and
   an upload are the same thing to a person watching: something is being read. */
function renderActivity() {
  const list = $("#activity-list");
  list.textContent = "";
  const rows = [
    ...WORK.map((w) => ({ name: w.name, status: w.stage, detail: w.detail, at: w.at || 0 })),
    ...UPLOADS.map((u) => ({ name: u.name, status: u.status, detail: u.detail, at: u.at || 0 })),
  ].sort((a, b) => b.at - a.at).slice(0, 80);
  if (!rows.length) {
    list.append(el("div", "empty", "Nothing happening yet. Add a folder, or drop a file on a lane."));
    return;
  }
  rows.forEach((r) => list.append(activityRow(r.name, r.status, r.detail)));
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
    at: Date.now(),
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

/* -------------------------------------------------------------- rewards */

/* The reward a run is paid for, shown as data rather than buried in a trainer.
   Weight and the on/off switch are editable in place, because tuning a reward is
   a loop worth closing quickly: change a number, score a part, look. */
function renderRewards() {
  const list = $("#reward-list");
  const rewards = STATE.rewards || [];
  const rows = rewards.map((r) => {
    const row = el("div", "row reward" + (r.enabled ? "" : " off"));

    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = r.enabled;
    cb.title = r.enabled ? "Stop this reward voting" : "Let this reward vote again";
    cb.addEventListener("change", () => updateReward(r.id, { enabled: cb.checked }));
    row.append(cb);

    const col = el("div", "col");
    col.append(el("div", "name", r.name));
    const params = r.params && Object.keys(r.params).length
      ? Object.keys(r.params).map((k) => k + " " + r.params[k]).join(", ")
      : "";
    col.append(el("div", "meta", r.kind + (params ? "  ·  " + params : "")));
    row.append(col);

    const w = document.createElement("input");
    w.type = "number";
    w.className = "weight";
    w.step = "0.1";
    w.min = "0.1";
    w.value = r.weight;
    w.title = "How much this reward's opinion counts, against the others";
    w.addEventListener("change", () => {
      const v = parseFloat(w.value);
      if (!isFinite(v) || v <= 0) { w.value = r.weight; return; }
      updateReward(r.id, { weight: v });
    });
    row.append(w);
    row.append(removeButton(r.name, "/api/rewards/" + r.id));
    return row;
  });
  section(list, rows, "No rewards. Add one to say what a run should be paid for.");

  const total = rewards.filter((r) => r.enabled).reduce((a, r) => a + r.weight, 0);
  const off = rewards.filter((r) => !r.enabled).length;
  $("#reward-count").textContent = rewards.length
    ? (off ? off + " switched off, " : "") + "weights add up to " + total.toFixed(1)
    : "";
}

async function addReward() {
  const name = prompt("Name the reward", "matches the part");
  if (!name) return;
  const kind = prompt(
    "Kind - geometry, recorded_history, simplicity or scale",
    "geometry"
  );
  if (!kind) return;
  const weight = parseFloat(prompt("Weight, against the other rewards", "1") || "1");
  try {
    await api("/api/rewards", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name, kind, weight }),
    });
  } catch (err) {
    alert(err.message || String(err));
  }
  await refresh();
}

async function updateReward(id, patch) {
  try {
    await api("/api/rewards/" + id, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    });
  } catch (err) {
    alert(err.message || String(err));
  }
  await refresh();
}

/* Which artifacts have something worth scoring against: a STEP part carries
   geometry, an export carries the real history. A picture carries neither. */
function renderRewardPicker() {
  const sel = $("#sel-reward-asset");
  const keep = sel.value;
  sel.textContent = "";
  STATE.assets
    .filter((a) => a.kind === "step" || a.kind === "history")
    .forEach((a) => sel.append(new Option(a.name, a.id)));
  if (!sel.options.length) {
    sel.append(new Option("import a STEP part or a SolidWorks export first", ""));
  }
  if (keep) sel.value = keep;
}

/* Score one part with the current reward set. The point is to see the working
   before spending hours of training on a number nobody has looked at. */
async function scorePreview() {
  const box = $("#reward-preview");
  const sel = $("#sel-reward-asset");
  box.textContent = "";
  if (!sel.value) {
    box.append(el("p", "hint", "Pick a part first."));
    return;
  }
  box.append(el("p", "hint", "scoring..."));
  try {
    const v = await api("/api/rewards/preview", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ asset_id: sel.value }),
    });
    renderVerdict(v, box);
  } catch (err) {
    box.textContent = "";
    box.append(el("p", "hint", "could not score it: " + err.message));
  }
}

function renderVerdict(v, box) {
  box.textContent = "";
  const head = el("div", "verdict" + (v.valid ? "" : " bad"));
  head.append(el("span", "total", "total " + (v.total * 100).toFixed(1) + "%"));
  head.append(el("span", "gate", v.gate));
  box.append(head);

  if (!v.components.length) {
    box.append(el("p", "hint",
      "Nothing could be measured. That is not a zero: it means no reward had anything " +
      "to compare this against."));
  }
  const table = el("table", "streams");
  const hr = el("tr");
  ["reward", "measure", "score", "weight"].forEach((h) => hr.append(el("th", null, h)));
  table.append(hr);
  (v.components || []).forEach((c) => {
    const tr = el("tr");
    tr.title = c.detail;
    tr.append(el("td", "name", c.name));
    tr.append(el("td", "num", c.measure.toFixed(3)));
    tr.append(el("td", "num", (c.score * 100).toFixed(0) + "%"));
    tr.append(el("td", "num", String(c.weight)));
    table.append(tr);
  });
  box.append(table);
  (v.skipped || []).forEach((s) => box.append(el("p", "hint small", s)));
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
  renderCurve(run);
  const log = $("#run-log");
  log.textContent = "";
  (run.log || []).forEach((line) => {
    log.append(el("div", line.level === "info" ? null : line.level, line.at.slice(11, 19) + "  " + line.message));
  });
  log.scrollTop = log.scrollHeight;
  paintStages(run);
  const note = $("#run-note");
  note.textContent = run.note || "";
  note.className = "verdict-line" + (run.note
    ? (run.note.indexOf("converged:") === 0 ? " ok" : " warn")
    : "");
  const cancel = $("#btn-cancel-run");
  if (cancel) cancel.disabled = !run || !activeRunId;
}

/* The reward curve, drawn as the points arrive.
   A run that stopped at its ceiling without converging looks different from one
   that got there - which is the whole reason the studio records the trainer's
   own verdict instead of assuming every finished run succeeded. */
function renderCurve(run) {
  const box = $("#run-curve");
  const points = (run && run.curve) || [];
  box.textContent = "";
  if (points.length < 2) {
    box.append(el("p", "hint small",
      points.length ? "one point so far" : "no curve reported yet"));
    return;
  }

  const w = 300, h = 70, pad = 5;
  const values = points.map((p) => (typeof p.mean === "number" ? p.mean : p.reward) || 0);
  const high = Math.max.apply(null, values.concat([0.0001]));
  const low = Math.min.apply(null, values.concat([0]));
  const span = Math.max(high - low, 0.0001);
  const x = (i) => pad + (i * (w - 2 * pad)) / (values.length - 1);
  const y = (v) => h - pad - ((v - low) / span) * (h - 2 * pad);

  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 " + w + " " + h);
  svg.setAttribute("class", "spark");
  svg.setAttribute("aria-label", "reward curve");
  const line = document.createElementNS(ns, "polyline");
  line.setAttribute("points", values.map((v, i) => x(i).toFixed(1) + "," + y(v).toFixed(1)).join(" "));
  svg.append(line);
  box.append(svg);

  const last = values[values.length - 1];
  box.append(el("div", "curve-meta",
    "step " + points[points.length - 1].step +
    "  ·  mean " + last.toFixed(3) +
    "  ·  best " + high.toFixed(3) +
    "  ·  " + values.length + " point(s)"));
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

/* --------------------------------------------------------------- storage */

/* Bytes in the units a person reads. The studio measures the data and estimates
   the models, and this is where both are shown - with the difference visible. */
function sizeText(bytes) {
  if (bytes == null) return "-";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return (i === 0 ? v : v.toFixed(1)) + " " + units[i];
}

/* An icon-only button that says what it does on hover and to a screen reader,
   which is the whole point of a compact control. */
function iconButton(icon, title, handler) {
  const b = el("button", "ghost mini");
  b.title = title;
  b.setAttribute("aria-label", title);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ic");
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", icon);
  svg.append(use);
  b.append(svg);
  b.addEventListener("click", (e) => { e.stopPropagation(); handler(); });
  return b;
}

async function renderStorage() {
  const box = $("#storage-box");
  const st = await api("/api/storage");
  box.textContent = "";
  const d = st.data || {};
  const dl = el("dl", "kv");
  const put = (k, v) => { dl.append(el("dt", null, k)); dl.append(el("dd", null, v)); };
  put("data indexed", sizeText(d.indexed_bytes) + "  ·  " + (d.indexed_artifacts || 0) + " file(s)");
  if (d.uploaded_bytes) put("uploaded", sizeText(d.uploaded_bytes));
  put("total data", sizeText(d.total_bytes));
  const store = st.store || {};
  put("studio folder", sizeText(store.data_dir_bytes));
  box.append(dl);

  const lanes = Object.keys(st.by_lane || {});
  if (lanes.length) {
    box.append(el("div", "section-label", "By lane"));
    const table = el("table", "streams");
    const head = el("tr");
    ["lane", "files", "size"].forEach((h) => head.append(el("th", null, h)));
    table.append(head);
    lanes.forEach((lane) => {
      const v = st.by_lane[lane];
      const tr = el("tr");
      tr.append(el("td", "name", lane));
      tr.append(el("td", "num", String(v.artifacts)));
      tr.append(el("td", "num", sizeText(v.bytes)));
      table.append(tr);
    });
    box.append(table);
  }

  if ((st.models || []).length) {
    box.append(el("div", "section-label", "Models"));
    const table = el("table", "streams");
    const head = el("tr");
    ["model", "params", "4-bit", "adapter"].forEach((h) => head.append(el("th", null, h)));
    table.append(head);
    st.models.forEach((m) => {
      const est = m.weights_estimate || {};
      const tr = el("tr");
      tr.append(el("td", "name", m.name));
      tr.append(el("td", "num", m.params ? (m.params / 1e9).toFixed(2) + "B" : "-"));
      tr.append(el("td", "num", m.params ? "~" + sizeText(est.int4) : "-"));
      tr.append(el("td", "num", m.adapters_bytes ? sizeText(m.adapters_bytes) : "none yet"));
      table.append(tr);
    });
    box.append(table);
    box.append(el("p", "hint small",
      "Model weights are estimated from the parameter count - the studio does not download them. " +
      "Adapter sizes are measured from what runs actually wrote."));
  } else {
    box.append(el("p", "hint small", "No base models registered yet."));
  }
}

/* --------------------------------------------------------------- folders */

/* A folder of training data, and everything the last scan found in it. */
function renderSources() {
  const list = $("#source-list");
  const rows = (STATE.sources || []).map((s) => {
    const row = el("div", "row source " + s.state);
    const col = el("div", "col");
    col.append(el("div", "name", s.label));
    col.append(el("div", "path", s.path));
    const bits = [];
    if (s.state === "scanning") bits.push("scanning…");
    else bits.push(s.indexed + " file" + (s.indexed === 1 ? "" : "s"));
    if (s.bytes) bits.push(sizeText(s.bytes));
    if (s.skipped) bits.push(s.skipped + " skipped");
    col.append(el("div", "meta", bits.join("  ·  ")));
    row.append(col);
    row.append(iconButton("#i-refresh", "Read this folder again", () => rescanSource(s.id)));
    row.append(removeButton(s.label, "/api/sources/" + s.id));
    return row;
  });
  section(list, rows, "No folders yet. Add one above to index a whole library at once.");
  const n = (STATE.sources || []).length;
  $("#source-count").textContent = n + " of 10 folders";
}

async function addSource() {
  const input = $("#source-path");
  const path = (input.value || "").trim();
  if (!path) { input.focus(); return; }
  const btn = $("#btn-source");
  btn.disabled = true;
  btn.textContent = "Adding…";
  try {
    await api("/api/sources", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path }),
    });
    input.value = "";
    await refresh();
  } catch (err) {
    alert(err.message || String(err));
  } finally {
    btn.disabled = false;
    btn.textContent = "Add folder";
  }
}

async function rescanSource(id) {
  try {
    await api("/api/sources/" + id + "/scan", { method: "POST" });
  } catch (err) {
    alert(err.message || String(err));
  }
  await refresh();
}

/* Folder and file progress: one event per file the studio reads, so a scan is
   something to watch rather than something to wait for. */
function connectWork() {
  const es = new EventSource("/api/work/events");
  es.addEventListener("work", (ev) => {
    let w;
    try { w = JSON.parse(ev.data); } catch (_) { return; }
    w.at = Date.now();
    WORK.unshift(w);
    while (WORK.length > WORK_LIMIT) WORK.pop();
    renderActivity();
    // A folder scan finishes with 'done', and that is the moment what is on disk
    // has changed - so re-read the state and re-measure rather than guess.
    if (w.done) refresh().catch(() => {});
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
  $("#btn-clear-activity").addEventListener("click", () => {
    UPLOADS.length = 0;
    WORK.length = 0;
    renderActivity();
  });
  $("#btn-refresh-system").addEventListener("click", renderSystem);

  $("#btn-source").addEventListener("click", addSource);
  $("#source-path").addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); addSource(); }
  });
  $("#btn-storage").addEventListener("click", () => renderStorage().catch(() => {}));
  $("#btn-reward").addEventListener("click", addReward);
  $("#btn-reward-preview").addEventListener("click", scorePreview);

  // One file, and its lane read from the extension - the shortest path to
  // watching a single part or drawing being read.
  $("#btn-one-file").addEventListener("click", () => {
    pickerLane = "";
    $("#file-input").click();
  });

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
  connectWork();
  renderActivity();
  renderProps();
  setReadout("no part loaded");
  try { await renderTrainer(); } catch (err) { $("#trainer-pill").textContent = "trainer: offline"; }
  await renderSystem();
  try { await refresh(); } catch (err) { setReadout("could not reach the studio API"); }
})().catch((err) => { console.error(err); });
