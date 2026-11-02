// The app shell: one obvious path (drop a mesh, Process, Export), with
// options tucked away and results shown as soon as they exist.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";

import { decode } from "./transfer";
import { Viewer } from "./viewer";

// Shapes returned by the commands in app/src-tauri/src/commands.rs.

interface Opened {
  generation: number;
  name: string;
  meshes: number;
  triangles: number;
  vertices: number;
  has_uvs: boolean;
  import_warnings: string[];
}

interface Warning {
  mesh: string;
  kind: string;
  message: string;
}

interface InputStats {
  stats: {
    meshes: { uv_channels: number; boundary_loops: number; non_manifold_edges: number }[];
    materials: number;
    images: number;
  };
  warnings: Warning[];
}

interface LodReport {
  level: number;
  triangles: number;
  error: number;
  acmr_after: number;
}

interface UvMetrics {
  islands: number;
  flipped_triangles: number;
  zero_area_triangles: number;
  overlap_texels: number;
  packing_efficiency: number;
}

interface Processed {
  report: { meshes: { name: string; warnings: Warning[]; uv: { metrics: UvMetrics } | null; lods: LodReport[] }[] };
  lod_levels: number;
  triangles: number;
  elapsed_ms: number;
}

interface Options {
  auto_uvs: boolean;
  uv_resolution: number;
  lods: boolean;
}

type View = "input" | "output";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const fmt = (n: number) => n.toLocaleString("en-US");
const STALE = "stale";

const viewer = new Viewer($("canvas-host"));

const state = {
  generation: 0,
  file: null as Opened | null,
  path: "",
  processed: null as Processed | null,
  busy: false,
  view: "input" as View,
  lod: 0,
  /** Bumped per view request, so a slow earlier one can't overwrite a later one. */
  request: 0,
  wireframe: false,
  checker: false,
};

// ---- Remembered options -------------------------------------------------

const OPTIONS_KEY = "littlemesh.options";

function defaults(file: Opened | null): Options {
  return { auto_uvs: !(file?.has_uvs ?? true), uv_resolution: 1024, lods: true };
}

/** Saved choices that differ from the defaults; `auto_uvs` is never saved,
 *  since the right default depends on whether the file has UVs. */
function savedOptions(): Partial<Options> {
  try {
    return JSON.parse(localStorage.getItem(OPTIONS_KEY) ?? "{}");
  } catch {
    return {};
  }
}

function saveOptions() {
  const o = readOptions();
  try {
    localStorage.setItem(OPTIONS_KEY, JSON.stringify({ uv_resolution: o.uv_resolution, lods: o.lods }));
  } catch {
    // Storage can be unavailable; the defaults still work.
  }
}

function applyOptions(o: Options) {
  $<HTMLInputElement>("opt-uvs").checked = o.auto_uvs;
  $<HTMLSelectElement>("opt-res").value = String(o.uv_resolution);
  $<HTMLInputElement>("opt-lods").checked = o.lods;
  updateProcessHint();
}

function readOptions(): Options {
  return {
    auto_uvs: $<HTMLInputElement>("opt-uvs").checked,
    uv_resolution: Number($<HTMLSelectElement>("opt-res").value),
    lods: $<HTMLInputElement>("opt-lods").checked,
  };
}

// ---- Helpers ------------------------------------------------------------

function showError(message: string | null) {
  const el = $("error");
  el.hidden = message === null;
  el.textContent = message ?? "";
}

function isStale(e: unknown) {
  return String(e) === STALE;
}

async function loadView(view: View, lod: number, frame: boolean) {
  const generation = state.generation;
  const request = ++state.request;
  state.view = view;
  state.lod = lod;
  renderViewButtons();
  const buffer = await invoke<ArrayBuffer>("scene_buffer", { generation, view, lod });
  if (generation !== state.generation || request !== state.request) {
    return;
  }
  const meshes = decode(buffer);
  viewer.setMeshes(meshes, frame);
  const triangles = meshes.reduce((sum, m) => sum + m.indices.length / 3, 0);
  const label = view === "input" ? "Original" : `Result · LOD${lod}`;
  $("view-label").textContent = `${label} · ${fmt(triangles)} triangles`;
  $("view-label").hidden = false;
  if (state.checker && !meshes.every((m) => m.hasUv)) {
    $("view-label").textContent += " · no UVs on some meshes";
  }
}

function seconds(ms: number) {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

// ---- Open ---------------------------------------------------------------

async function openPath(path: string) {
  if (state.busy) {
    return;
  }
  showError(null);
  const started = performance.now();
  try {
    const file = await invoke<Opened>("open_file", { path });
    state.generation = file.generation;
    state.file = file;
    state.path = path;
    state.processed = null;
    renderFile(file);
    await loadView("input", 0, true);
    $("file-summary").textContent += ` · shown in ${seconds(performance.now() - started)}`;
    const stats = await invoke<InputStats>("input_stats", { generation: file.generation });
    if (file.generation === state.generation) {
      renderStats(stats, file.import_warnings);
    }
  } catch (e) {
    if (!isStale(e)) {
      showError(`Couldn't open ${path}: ${e}`);
    }
  }
}

async function chooseFile() {
  const path = await open({
    multiple: false,
    filters: [{ name: "3D models", extensions: ["gltf", "glb", "obj"] }],
  });
  if (typeof path === "string") {
    await openPath(path);
  }
}

function renderFile(file: Opened) {
  $("empty").hidden = true;
  $("toolbar").hidden = false;
  $("file").hidden = false;
  $("process").hidden = false;
  $("results").hidden = true;
  $("process-status").hidden = true;
  setProcessed(false);
  $("file-name").textContent = file.name;
  const meshes = file.meshes === 1 ? "1 mesh" : `${fmt(file.meshes)} meshes`;
  $("file-summary").textContent = `${meshes} · ${fmt(file.triangles)} triangles · ${fmt(file.vertices)} vertices`;
  $("file-stats").innerHTML = `<dt>Topology</dt><dd><span class="spinner"></span>checking…</dd>`;
  $("warnings").hidden = true;
  applyOptions({ ...defaults(file), ...savedOptions(), auto_uvs: defaults(file).auto_uvs });
}

function renderStats(input: InputStats, importWarnings: string[]) {
  const s = input.stats;
  const withUvs = s.meshes.filter((m) => m.uv_channels > 0).length;
  const open = s.meshes.reduce((n, m) => n + m.boundary_loops, 0);
  const nonManifold = s.meshes.reduce((n, m) => n + m.non_manifold_edges, 0);
  const rows: [string, string][] = [
    ["Materials", fmt(s.materials)],
    ["Textures", fmt(s.images)],
    ["Has UVs", withUvs === s.meshes.length ? "yes" : withUvs === 0 ? "no" : `${withUvs} of ${s.meshes.length} meshes`],
    ["Open borders", fmt(open)],
    ["Non-manifold edges", fmt(nonManifold)],
  ];
  $("file-stats").innerHTML = rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join("");
  renderWarnings([...importWarnings.map((message) => ({ mesh: "", kind: "import", message })), ...input.warnings]);
}

function renderWarnings(warnings: Warning[]) {
  const list = $("warnings");
  list.replaceChildren(
    ...warnings.map((w) => {
      const li = document.createElement("li");
      if (w.mesh) {
        const b = document.createElement("b");
        b.textContent = w.mesh;
        li.append(b);
      }
      li.append(w.message);
      return li;
    }),
  );
  list.hidden = warnings.length === 0;
}

// ---- Process ------------------------------------------------------------

function updateProcessHint() {
  const file = state.file;
  const status = $("process-status");
  if (state.busy || !file) {
    return;
  }
  if (readOptions().auto_uvs && file.triangles > 50_000) {
    status.textContent = `Generating UVs for ${fmt(file.triangles)} triangles can take a minute or more.`;
    status.hidden = false;
  } else {
    status.hidden = true;
  }
}

async function runProcess() {
  if (!state.file || state.busy) {
    return;
  }
  showError(null);
  const generation = state.generation;
  const options = readOptions();
  saveOptions();
  setBusy(true);
  const started = performance.now();
  const status = $("process-status");
  status.hidden = false;
  const tick = () => {
    const seconds = Math.round((performance.now() - started) / 1000);
    status.innerHTML = `<span class="spinner"></span>Processing… ${seconds} s`;
  };
  tick();
  const timer = setInterval(tick, 1000);
  try {
    const processed = await invoke<Processed>("process", { generation, options });
    clearInterval(timer);
    if (generation !== state.generation) {
      return;
    }
    state.processed = processed;
    status.textContent = `Done in ${seconds(processed.elapsed_ms)}. Check the result, then export it.`;
    setProcessed(true);
    renderResults(processed);
    await loadView("output", 0, false);
  } catch (e) {
    if (!isStale(e)) {
      status.hidden = true;
      showError(`Processing failed: ${e}`);
    }
  } finally {
    clearInterval(timer);
    setBusy(false);
  }
}

/** After processing, Export is the next step, so Process steps back. */
function setProcessed(done: boolean) {
  const button = $("btn-process");
  button.textContent = done ? "Process again" : "Process";
  button.classList.toggle("primary", !done);
}

function setBusy(busy: boolean) {
  state.busy = busy;
  for (const id of ["btn-process", "btn-open", "btn-open-empty", "btn-export"]) {
    $<HTMLButtonElement>(id).disabled = busy;
  }
  $("options").toggleAttribute("inert", busy);
}

function renderResults(p: Processed) {
  $("results").hidden = false;
  const levels: { triangles: number; acmr: number[] }[] = [];
  for (const mesh of p.report.meshes) {
    mesh.lods.forEach((lod, i) => {
      levels[i] ??= { triangles: 0, acmr: [] };
      levels[i].triangles += lod.triangles;
      levels[i].acmr.push(lod.acmr_after);
    });
  }
  const mean = (xs: number[]) => xs.reduce((a, b) => a + b, 0) / xs.length;
  $("lod-table").innerHTML =
    `<tr><th>Level</th><th>Triangles</th><th title="Average cache miss ratio: lower draws faster">ACMR</th></tr>` +
    levels.map((l, i) => `<tr><td>LOD${i}</td><td>${fmt(l.triangles)}</td><td>${mean(l.acmr).toFixed(2)}</td></tr>`).join("");

  const uvs = p.report.meshes.flatMap((m) => (m.uv ? [m.uv.metrics] : []));
  if (uvs.length > 0) {
    const sum = (f: (m: UvMetrics) => number) => uvs.reduce((n, m) => n + f(m), 0);
    const flips = sum((m) => m.flipped_triangles + m.zero_area_triangles);
    const overlaps = sum((m) => m.overlap_texels);
    const packing = (sum((m) => m.packing_efficiency) / uvs.length) * 100;
    $("uv-summary").innerHTML = `<dl>
      <dt>UV islands</dt><dd>${fmt(sum((m) => m.islands))}</dd>
      <dt title="Triangles folded over in UV space">Flipped</dt><dd class="${flips ? "bad" : "good"}">${fmt(flips)}</dd>
      <dt title="Texture pixels used by two triangles at once">Overlapping pixels</dt><dd class="${overlaps ? "bad" : "good"}">${fmt(overlaps)}</dd>
      <dt title="How much of the texture the islands cover">Texture used</dt><dd>${packing.toFixed(0)}%</dd>
    </dl>`;
  } else {
    $("uv-summary").innerHTML = "";
  }
  renderWarnings(p.report.meshes.flatMap((m) => m.warnings));

  const picker = $("lod-picker");
  picker.replaceChildren(
    ...Array.from({ length: p.lod_levels }, (_, i) => {
      const b = document.createElement("button");
      b.textContent = `LOD${i}`;
      b.addEventListener("click", () => loadView("output", i, false));
      return b;
    }),
  );
  picker.hidden = p.lod_levels < 2;
}

function renderViewButtons() {
  const output = state.view === "output";
  $("btn-view-input").setAttribute("aria-pressed", String(!output));
  $("btn-view-output").setAttribute("aria-pressed", String(output));
  $("lod-picker")
    .querySelectorAll("button")
    .forEach((b, i) => b.setAttribute("aria-pressed", String(output && i === state.lod)));
}

function stepLod(delta: number) {
  const p = state.processed;
  if (!p || state.busy) {
    return;
  }
  const lod = Math.min(Math.max((state.view === "output" ? state.lod : -1) + delta, 0), p.lod_levels - 1);
  loadView("output", lod, false).catch(reportError);
}

// ---- Export -------------------------------------------------------------

async function runExport() {
  if (!state.processed || state.busy) {
    return;
  }
  const stem = state.file!.name.replace(/\.[^.]+$/, "");
  const path = await save({
    defaultPath: state.path.replace(/[^\\/]+$/, `${stem}_littlemesh.glb`),
    filters: [
      { name: "glTF binary", extensions: ["glb"] },
      { name: "glTF", extensions: ["gltf"] },
    ],
  });
  if (!path) {
    return;
  }
  const status = $("export-status");
  try {
    setBusy(true);
    await invoke("export", { path });
    status.textContent = `Saved ${path}`;
    status.hidden = false;
  } catch (e) {
    showError(`Export failed: ${e}`);
  } finally {
    setBusy(false);
  }
}

function reportError(e: unknown) {
  if (!isStale(e)) {
    showError(String(e));
  }
}

// ---- Wiring -------------------------------------------------------------

function toggle(id: string, on: boolean) {
  $(id).setAttribute("aria-pressed", String(on));
}

function setWireframe(on: boolean) {
  state.wireframe = on;
  viewer.setWireframe(on);
  toggle("btn-wire", on);
}

function setChecker(on: boolean) {
  state.checker = on;
  viewer.setChecker(on);
  toggle("btn-checker", on);
}

$("btn-open").addEventListener("click", () => chooseFile().catch(reportError));
$("btn-open-empty").addEventListener("click", () => chooseFile().catch(reportError));
$("btn-process").addEventListener("click", () => runProcess());
$("btn-export").addEventListener("click", () => runExport().catch(reportError));
$("btn-frame").addEventListener("click", () => viewer.frame());
$("btn-wire").addEventListener("click", () => setWireframe(!state.wireframe));
$("btn-checker").addEventListener("click", () => setChecker(!state.checker));
$("btn-view-input").addEventListener("click", () => loadView("input", 0, false).catch(reportError));
$("btn-view-output").addEventListener("click", () => loadView("output", state.lod, false).catch(reportError));
$("btn-reset").addEventListener("click", () => {
  applyOptions(defaults(state.file));
  saveOptions();
});
for (const id of ["opt-uvs", "opt-res", "opt-lods"]) {
  $(id).addEventListener("change", () => {
    saveOptions();
    updateProcessHint();
  });
}

window.addEventListener("keydown", (e) => {
  if (e.target instanceof HTMLSelectElement || e.target instanceof HTMLInputElement) {
    return;
  }
  const ctrl = e.ctrlKey || e.metaKey;
  if (ctrl && e.key.toLowerCase() === "o") {
    e.preventDefault();
    chooseFile().catch(reportError);
  } else if (ctrl && e.key === "Enter") {
    e.preventDefault();
    runProcess();
  } else if (ctrl && e.key.toLowerCase() === "e") {
    e.preventDefault();
    runExport().catch(reportError);
  } else if (!ctrl && state.file) {
    switch (e.key.toLowerCase()) {
      case "f":
        viewer.frame();
        break;
      case "w":
        setWireframe(!state.wireframe);
        break;
      case "c":
        setChecker(!state.checker);
        break;
      case "[":
        stepLod(-1);
        break;
      case "]":
        stepLod(1);
        break;
    }
  }
});

getCurrentWebview().onDragDropEvent((event) => {
  const hint = $("drop-hint");
  const p = event.payload;
  if (p.type === "enter" || p.type === "over") {
    hint.hidden = state.busy;
  } else if (p.type === "drop") {
    hint.hidden = true;
    if (p.paths.length > 0) {
      openPath(p.paths[0]);
    }
  } else {
    hint.hidden = true;
  }
});

invoke<string | null>("startup_file").then((path) => {
  if (path) {
    openPath(path);
  }
});
