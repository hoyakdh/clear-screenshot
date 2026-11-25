const { invoke, convertFileSrc } = window.__TAURI__.core;

const state = {
  dir: "",
  home: "",
  items: [],            // [{path, name, size, created}] newest first
  byPath: new Map(),
  thumbs: new Map(),    // path -> asset url
  selected: new Set(),
  view: "all",          // "all" | "similar" | "month:YYYY-MM"
  groups: null,         // similar groups (arrays of paths) or null when stale
  order: [],            // visible paths in display order (for shift-click + preview nav)
  anchor: null,         // last clicked path, for shift-click ranges
  preview: null,        // path being previewed
  analysis: new Map(),  // path -> {text, tags:[{kind,label}]} from on-device OCR
  analyzing: null,      // {done, total} while OCR runs in the background
  query: "",            // text search
  kept: new Set(),      // screenshots the user chose to keep (persisted by the backend)
  overrides: {},        // manual category corrections, path -> {add, remove}
  tagFilter: new Set(), // active tag-label filters (OR between labels)
};
const catLabel = (k) => t(`cat.${k}`);
const DEVICE_KINDS = ["iphone", "ipad", "mac", "watch", "other"];
const devLabel = (k) => t(`dev.${k}`);
const devOf = (p) => state.byPath.get(p)?.device?.kind || "other";
/** "iPhone 15 Pro · 16" when known, else the generic device name. */
const devDetail = (p) => state.byPath.get(p)?.device?.model || devLabel(devOf(p));
const dimsOf = (p) => { const it = state.byPath.get(p); return it?.width ? `${it.width}×${it.height}` : ""; };

// ---------- settings ----------
const DEFAULT_SETTINGS = {
  theme: "system",     // system | light | dark
  root: "",            // organize folder; "" = the macOS screenshot folder
  layout: "month",     // month (2026-05) | year_month (2026/05) | year (2026)
  wholeMac: true,      // Spotlight search across the Mac
  photos: true,        // include the Photos library
  ocr: true,           // text recognition + auto categories
  similarity: "normal", // strict | normal | loose
  lang: "auto",        // auto | ko | en | ja | es
  captureDirs: null,   // folders other capture apps save into; null = not detected yet
  ai: true,            // Apple on-device model describes pictures (macOS 27+, Apple Intelligence)
};
state.settings = { ...DEFAULT_SETTINGS };
/** Where "organize" puts files. */
const orgRoot = () => state.settings.root || state.dir;
function layoutPath(ts) {
  const d = new Date(ts * 1000), y = d.getFullYear(), m = String(d.getMonth() + 1).padStart(2, "0");
  return state.settings.layout === "year_month" ? `${y}/${m}` : state.settings.layout === "year" ? `${y}` : `${y}-${m}`;
}
let cardEls = new Map();
let thumbJob = 0;

const $ = (id) => document.getElementById(id);
/** Bottom-bar progress: `label` + percentage, with the capsule filling up. `label = null` hides it. */
// ---------- menu-bar menu ----------
let trayStatus = "";
let trayTimer = null;
/** Push menu texts (current language) and the status line to the menu-bar menu, at most once a second. */
function updateTray(status) {
  if (status != null) trayStatus = status;
  if (trayTimer) return;
  trayTimer = setTimeout(() => {
    trayTimer = null;
    invoke("update_tray", { labels: {
      status: trayStatus || "Clear Screenshot",
      open: t("tray.open"), tidy: t("tidy.button"), search: t("tray.search"), settings: t("tray.settings"), quit: t("tray.quit"),
    } }).catch(() => {});
  }, 1000);
}

function setProgress(label, done = 0, total = 0) {
  const on = label != null && total > 0;
  if (on) updateTray(t("tray.analyzing", { pct: Math.min(100, Math.floor((done / total) * 100)), done, total }));
  $("sb-progress").hidden = !on;
  $("sb-pct").hidden = !on;
  if (!on) return;
  const pct = Math.min(100, Math.floor((done / total) * 100));
  $("sb-fill").style.width = `${pct}%`;
  $("sb-pct").textContent = `${pct}%`;
  $("status").textContent = `${label} ${done}/${total}`;
}
/** Toolbar buttons have an icon + label; in narrow windows only the icon shows, so mirror the label into the tooltip. */
function setLabel(id, text, tip) {
  const b = $(id);
  b.querySelector(".lbl").textContent = text;
  b.title = tip || text;
  b.setAttribute("aria-label", text);
}

// ---------- formatting ----------
function fmtSize(b) {
  if (b < 1024) return `${b} B`;
  const units = ["KB", "MB", "GB"];
  let i = -1;
  do { b /= 1024; i++; } while (b >= 1024 && i < units.length - 1);
  return `${b.toFixed(b < 10 ? 1 : 0)} ${units[i]}`;
}
const pad = (n) => String(n).padStart(2, "0");
function monthKey(ts) { const d = new Date(ts * 1000); return `${d.getFullYear()}-${pad(d.getMonth() + 1)}`; }
function monthLabel(k) { return monthLabelI18n(k); }
function fmtDate(ts) {
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}.${pad(d.getMonth() + 1)}.${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
function shortTime(ts) { const d = new Date(ts * 1000); return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`; }
function tilde(p) { return state.home && p.startsWith(state.home) ? "~" + p.slice(state.home.length) : p; }
function folderOf(p) { return p.slice(0, p.lastIndexOf("/")); }
const sumSize = (paths) => paths.reduce((s, p) => s + (state.byPath.get(p)?.size || 0), 0);
const nfc = (s) => s.normalize("NFC");
const PHOTOS = "photos://";
const isPhoto = (p) => p.startsWith(PHOTOS);
/** "photos" | "folder" (inside the screenshot folder) | "other" (anywhere else on the Mac) */
function srcOf(p) {
  if (isPhoto(p)) return "photos";
  const inside = (dir) => dir && nfc(p).startsWith(nfc(dir) + "/");
  return inside(orgRoot()) || inside(state.dir) ? "folder" : "other";
}
const srcLabel = (k) => t(`src.${k}`);
function locationOf(p) { return isPhoto(p) ? t("src.photos") : tilde(folderOf(p)); }
function targetFolder(it) { return `${orgRoot()}/${layoutPath(it.created)}`; }
/** Photos assets always count as unorganized: organizing exports them into the folder. */
function isOrganized(path) {
  if (isPhoto(path)) return false;
  const it = state.byPath.get(path);
  return !!it && nfc(folderOf(path)) === nfc(targetFolder(it));
}
const unorganized = (paths) => paths.filter((p) => !isOrganized(p));

// ---------- OCR analysis ----------
const tagsOf = (p) => state.analysis.get(p)?.tags || [];
const hasKind = (p, kind) => tagsOf(p).some((tag) => tag.kind === kind);
const isKept = (p) => state.kept.has(p);
/** Auto-category membership; kept screenshots are no longer suggested. */
const inCat = (p, kind) => !isKept(p) && hasKind(p, kind);
/** Similar groups still worth looking at (not every member already kept). */
const visibleGroups = () => (state.groups || []).filter((g) => !g.every(isKept));

let saveKeptTimer;
function persistKept() {
  clearTimeout(saveKeptTimer);
  saveKeptTimer = setTimeout(() => {
    invoke("save_kept", { kept: [...state.kept] }).catch((e) => toast(t("keep.saveFailed", { e: tErr(e) })));
  }, 200);
}
function refreshChips(paths) {
  for (const p of paths) {
    const chips = cardEls.get(p)?.querySelector(".chips");
    if (chips) chips.innerHTML = chipsHtml(p);
  }
}
function setKept(paths, on) {
  for (const p of paths) on ? state.kept.add(p) : state.kept.delete(p);
  persistKept();
  refreshChips(paths);
  renderSidebar();
}
const squash = (s) => nfc(s).toLowerCase().replace(/\s+/g, "");
// Picture-content search: the categories a query refers to (memoized per query string).
let catQueryMemo = { q: null, keys: [] };
function queryCategoryKeys(query) {
  if (catQueryMemo.q !== query) catQueryMemo = { q: query, keys: categoriesForQuery(query).map((c) => c.key) };
  return catQueryMemo.keys;
}
/** Categories after the user's corrections: (automatic ∪ added) − removed. */
function objCatsOf(p) {
  const auto = state.analysis.get(p)?.cats || [];
  const o = state.overrides[p];
  if (!o) return auto;
  return [...new Set([...auto, ...(o.add || [])])].filter((k) => !(o.remove || []).includes(k));
}
let saveOverridesTimer;
function persistOverrides() {
  clearTimeout(saveOverridesTimer);
  saveOverridesTimer = setTimeout(() => invoke("save_overrides", { overrides: state.overrides }).catch(() => {}), 200);
}
/** The user says category `key` is (on=true) / is not (on=false) in screenshot `p`. */
function correctCategory(p, key, on) {
  const auto = (state.analysis.get(p)?.cats || []).includes(key);
  const o = state.overrides[p] || { add: [], remove: [] };
  o.add = (o.add || []).filter((k) => k !== key);
  o.remove = (o.remove || []).filter((k) => k !== key);
  if (on && !auto) o.add.push(key);
  if (!on && auto) o.remove.push(key);
  if (o.add.length || o.remove.length) state.overrides[p] = o; else delete state.overrides[p];
  persistOverrides();
}
/** True when the screenshot's picture (or tell-tale text) matches what the query describes. */
function matchesObjects(p, query) {
  const keys = queryCategoryKeys(query);
  return keys.length > 0 && objCatsOf(p).some((k) => keys.includes(k));
}
const objCatLabels = (p) => objCatsOf(p).map((k) => objCatLabel(k, LANG));
const appOf = (p) => state.analysis.get(p)?.app || null;

function matchesQuery(p, query = state.query) {
  if (!query) return true;
  if (matchesObjects(p, query)) return true;
  const app = appOf(p);
  if (app && appsForQuery(query).includes(app)) return true;
  const q = squash(query);
  const it = state.byPath.get(p);
  // Captures of this app only contain our own UI words; searching them would echo our labels.
  const a = state.analysis.get(p);
  const text = a && !a.ownApp ? a.text || "" : "";
  const ai = a?.ai;
  if (ai && (squash(ai.summary || "").includes(q) || (ai.subjects || []).some((s) => squash(s) === q || (q.length >= 3 && squash(s).startsWith(q))))) return true;
  return squash(text).includes(q) || squash(it?.name || "").includes(q) || squash(devDetail(p)).includes(q) || squash(devLabel(devOf(p))).includes(q)
    || tagsOf(p).some((tag) => squash(tagLabel(tag.label)).includes(q));
}
const labelsOf = (p) => [...new Set(tagsOf(p).map((tag) => tag.label))];
function matchesTags(p) {
  if (!state.tagFilter.size) return true;
  return labelsOf(p).some((l) => state.tagFilter.has(l));
}

/** Filter bar: one toggle per tag label found in the current view + search (ignoring tag filters). */
function renderFilterBar(basePaths) {
  const bar = $("filterbar");
  if (!basePaths) { bar.hidden = true; return; }
  const counts = new Map(); // label -> {kind, n}; each screenshot counts once per label
  for (const p of basePaths) {
    for (const label of labelsOf(p)) {
      const e = counts.get(label) || { kind: tagsOf(p).find((tag) => tag.label === label).kind, n: 0 };
      e.n++;
      counts.set(label, e);
    }
  }
  for (const l of state.tagFilter) if (!counts.has(l)) counts.set(l, { kind: "temp", n: 0 });
  if (!counts.size) { bar.hidden = true; return; }
  const entries = [...counts.entries()].sort((a, b) =>
    a[1].kind !== b[1].kind ? (a[1].kind === "temp" ? -1 : 1) : b[1].n - a[1].n);
  bar.hidden = false;
  bar.innerHTML = `<span class="filter-label">${t("filter.label")}</span>` + entries.map(([label, e]) =>
    `<button class="fchip ${e.kind}${state.tagFilter.has(label) ? " on" : ""}" data-label="${label}">${tagLabel(label)}<span>${e.n}</span></button>`
  ).join("") + (state.tagFilter.size ? `<button class="link fclear" id="fclear">${t("filter.clear")}</button>` : "");
}

function chipsHtml(p) {
  // de-duplicate labels, temp first
  const seen = new Set();
  const kept = isKept(p) ? `<span class="chip kept">${t("chip.kept")}</span>` : "";
  return kept + tagsOf(p)
    .slice()
    .sort((a, b) => (a.kind === b.kind ? 0 : a.kind === "temp" ? -1 : 1))
    .filter((tag) => !seen.has(tag.label) && seen.add(tag.label))
    .map((tag) => `<span class="chip ${tag.kind}">${tagLabel(tag.label)}</span>`)
    .join("");
}

let analyzeJob = 0;
/** Show the blocking first-run window when this many screenshots still need analysis. */
const MODAL_MIN = 20;
let modalDismissed = false;

function storeAnalysis(res) {
  for (const a of res) {
    state.analysis.set(a.path, { text: a.text, tags: a.tags, objects: a.objects || [], ai: a.ai || null, cats: objectCategoriesOf(a.objects, a.own_app ? "" : a.text, a.ai), app: a.own_app ? null : detectApp(a.text, a.ai?.app, a.ai?.kind), ownApp: !!a.own_app, ok: a.ok });
    const chips = cardEls.get(a.path)?.querySelector(".chips");
    if (chips) chips.innerHTML = chipsHtml(a.path);
  }
}

function showAnalyzeModal(done, total, startedAt) {
  if (modalDismissed) return;
  $("analyzing").hidden = false;
  const pct = Math.floor((done / total) * 100);
  $("an-fill").style.width = `${pct}%`;
  $("an-pct").textContent = `${pct}%`;
  $("an-count").textContent = t("an.count", { done, total });
  const per = done ? (Date.now() - startedAt) / done : 0;
  const left = per ? Math.round((per * (total - done)) / 1000) : 0;
  $("an-eta").textContent = left > 0 ? t("an.eta", { time: left >= 60 ? t("an.min", { n: Math.ceil(left / 60) }) : t("an.sec", { n: left }) }) : "";
}
const hideAnalyzeModal = () => { $("analyzing").hidden = true; };
$("an-bg").onclick = () => { modalDismissed = true; hideAnalyzeModal(); };

async function analyzeAll() {
  const job = ++analyzeJob;
  if (!state.settings.ocr) return;
  const all = state.items.map((i) => i.path).filter((p) => !state.analysis.has(p));
  if (!all.length) return;
  // Already-analyzed screenshots load instantly from storage; only the rest is real work.
  // (That is also what makes an interrupted run continue where it stopped.)
  let pendingList = [];
  try { pendingList = await invoke("pending_analysis", { paths: all, ai: !!state.settings.ai }); } catch { pendingList = all; }
  if (job !== analyzeJob) return;
  const pendingSet = new Set(pendingList);
  const cached = all.filter((p) => !pendingSet.has(p));
  for (let i = 0; i < cached.length; i += 48) {
    if (job !== analyzeJob) return;
    try { storeAnalysis(await invoke("analyze", { paths: cached.slice(i, i + 48), ai: !!state.settings.ai })); } catch { /* keep going */ }
  }
  renderSidebar();
  const todo = pendingList;
  if (!todo.length) {
    if (state.view.startsWith("cat:") || state.view.startsWith("app:") || state.query || state.tagFilter.size) render();
    return;
  }
  const useModal = todo.length >= MODAL_MIN;
  const startedAt = Date.now();
  state.analyzing = { done: 0, total: todo.length };
  setProgress(t("progress.analyze"), 0, todo.length);
  if (useModal) showAnalyzeModal(0, todo.length, startedAt);
  renderSidebar();
  // The on-device model handles one picture at a time, so smaller batches keep the progress bar moving.
  const BATCH = state.settings.ai ? 3 : 6;
  for (let i = 0; i < todo.length; i += BATCH) {
    if (job !== analyzeJob) return;
    let res = [];
    try { res = await invoke("analyze", { paths: todo.slice(i, i + BATCH), ai: !!state.settings.ai }); } catch { /* keep going */ }
    storeAnalysis(res);
    state.analyzing.done = Math.min(todo.length, i + BATCH);
    setProgress(t("progress.analyze"), state.analyzing.done, todo.length);
    if (useModal) showAnalyzeModal(state.analyzing.done, todo.length, startedAt);
    renderSidebar();
    if (state.lastBase) renderFilterBar(state.lastBase);
  }
  state.analyzing = null;
  setProgress(null);
  hideAnalyzeModal();
  setStatus(t("status.summary", { n: state.items.length, size: fmtSize(sumSize(state.items.map((i) => i.path))) }));
  renderSidebar();
  // views that depend on OCR results get their final contents now
  if (state.view.startsWith("cat:") || state.view.startsWith("app:") || state.query || state.tagFilter.size) render();
}
/** What "organize" acts on: the selection, otherwise exactly what the current view shows. */
const organizeScope = () => (state.selected.size ? [...state.selected] : state.order);
function organizeLabel(left) {
  if (left === 0) return t("org.allDone");
  const by = t(state.settings.layout === "year" ? "org.byYear" : "org.byMonth");
  if (state.selected.size) return t("org.sel", { n: left, by });
  if (state.view.startsWith("month:")) return t("org.month", { n: left, month: monthNameI18n(+state.view.slice(11)) });
  if (state.view === "similar" || state.view.startsWith("src:") || state.view.startsWith("cat:") || state.view.startsWith("dev:") || state.view.startsWith("app:") || state.view === "kept") return t("org.list", { n: left });
  return t("org.all", { n: left, by });
}

function setStatus(text) { $("status").textContent = text; updateTray(text); }
let toastTimer;
function toast(text) {
  const el = $("toast");
  el.textContent = text;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 3200);
}

function confirmDialog(title, body, yesLabel = t("common.ok"), danger = false) {
  return new Promise((resolve) => {
    $("confirm-title").textContent = title;
    $("confirm-body").textContent = body;
    const yes = $("confirm-yes");
    yes.textContent = yesLabel;
    yes.classList.toggle("danger", danger);
    $("confirm").hidden = false;
    yes.focus();
    const done = (v) => {
      $("confirm").hidden = true;
      yes.onclick = $("confirm-no").onclick = null;
      resolve(v);
    };
    yes.onclick = () => done(true);
    $("confirm-no").onclick = () => done(false);
  });
}

// ---------- data ----------
function setItems(items) {
  state.items = items.sort((a, b) => b.created - a.created);
  state.byPath = new Map(state.items.map((i) => [i.path, i]));
  state.selected = new Set([...state.selected].filter((p) => state.byPath.has(p)));
}

let refreshing = null, refreshAgain = false;
async function refresh() {
  // Coalesce bursts (live updates while a scan is running) into one extra scan.
  if (refreshing) { refreshAgain = true; return refreshing; }
  refreshing = doRefresh();
  try { await refreshing; } finally {
    refreshing = null;
    if (refreshAgain) { refreshAgain = false; refresh(); }
  }
}
async function doRefresh() {
  setStatus(t("status.scanning"));
  try {
    const [r, ph] = await Promise.all([
      invoke("scan", {
        wholeMac: state.settings.wholeMac,
        extraDirs: state.settings.root ? [state.settings.root] : [],
        captureDirs: state.settings.captureDirs || [],
      }),
      state.settings.photos
        ? invoke("photos_scan").catch((e) => ({ status: "error", items: [], error: String(e) }))
        : Promise.resolve({ status: "off", items: [] }),
    ]);
    state.dir = r.dir;
    state.home = r.home;
    state.photosStatus = ph.status;
    state.groups = null;
    if (!state.keptLoaded) {
      try { state.kept = new Set(await invoke("load_kept")); } catch { /* start empty */ }
      try { state.overrides = (await invoke("load_overrides")) || {}; } catch { /* start empty */ }
      state.keptLoaded = true;
    }
    setItems([...r.items, ...ph.items]);
    showRoot();
    updateWatch();
    renderSidebar();
    await render();
  } catch (e) {
    setStatus(t("status.scanFailed", { e: tErr(e) }));
  }
}

function months() {
  const m = new Map();
  for (const it of state.items) {
    const k = monthKey(it.created);
    const e = m.get(k) || { count: 0, size: 0 };
    e.count++; e.size += it.size;
    m.set(k, e);
  }
  return [...m.entries()].sort((a, b) => (a[0] < b[0] ? 1 : -1));
}

async function ensureGroups() {
  if (state.groups) return;
  setStatus(t("status.similar", { n: state.items.length }));
  state.groups = await invoke("similar", {
    paths: state.items.map((i) => i.path),
    times: state.items.map((i) => i.created),
    level: state.settings.similarity || "normal",
  });
  renderSidebar();
}

function dropPaths(paths) {
  const gone = new Set(paths);
  setItems(state.items.filter((i) => !gone.has(i.path)));
  if (paths.some(isKept)) { paths.forEach((p) => state.kept.delete(p)); persistKept(); }
  if (paths.some((p) => state.overrides[p])) { paths.forEach((p) => delete state.overrides[p]); persistOverrides(); }
  if (state.groups) {
    state.groups = state.groups.map((g) => g.filter((p) => !gone.has(p))).filter((g) => g.length > 1);
  }
}

// ---------- rendering ----------
function renderSidebar() {
  $("count-all").textContent = state.items.length;
  $("count-similar").textContent = state.groups ? visibleGroups().reduce((s, g) => s + g.length, 0) || "0" : "";
  const catBox = $("cats");
  catBox.innerHTML = "";
  for (const k of ["temp", "private"]) {
    const n = state.items.filter((i) => inCat(i.path, k)).length;
    const b = document.createElement("button");
    b.className = "nav-item";
    b.dataset.view = `cat:${k}`;
    b.innerHTML = `<span>${catLabel(k)}</span><span class="count">${n}</span>`;
    catBox.appendChild(b);
  }
  const keptBtn = document.createElement("button");
  keptBtn.className = "nav-item";
  keptBtn.dataset.view = "kept";
  keptBtn.innerHTML = `<span>${t("nav.kept")}</span><span class="count">${state.items.filter((i) => isKept(i.path)).length}</span>`;
  catBox.appendChild(keptBtn);

  const srcBox = $("sources");
  srcBox.innerHTML = "";
  const counts = { photos: 0, folder: 0, other: 0 };
  for (const it of state.items) counts[srcOf(it.path)]++;
  for (const k of ["photos", "folder", "other"]) {
    if (k === "other" && !counts.other) continue;
    const b = document.createElement("button");
    b.className = "nav-item";
    b.dataset.view = `src:${k}`;
    b.innerHTML = `<span>${srcLabel(k)}</span><span class="count">${counts[k]}</span>`;
    srcBox.appendChild(b);
  }
  if (state.photosStatus === "denied" || state.photosStatus === "error") {
    const n = document.createElement("div");
    n.className = "nav-note";
    n.textContent = t("side.photosDenied");
    srcBox.appendChild(n);
  } else if (state.photosStatus === "limited") {
    const n = document.createElement("div");
    n.className = "nav-note";
    n.textContent = t("side.photosLimited");
    srcBox.appendChild(n);
  }

  const appBox = $("apps");
  appBox.innerHTML = "";
  const appCounts = new Map();
  for (const it of state.items) { const a = appOf(it.path); if (a) appCounts.set(a, (appCounts.get(a) || 0) + 1); }
  for (const [k, n] of [...appCounts].sort((a, b) => b[1] - a[1])) {
    const b = document.createElement("button");
    b.className = "nav-item";
    b.dataset.view = `app:${k}`;
    b.innerHTML = `<span>${appName(k)}</span><span class="count">${n}</span>`;
    appBox.appendChild(b);
  }
  appBox.previousElementSibling.hidden = !appCounts.size;

  const devBox = $("devices");
  devBox.innerHTML = "";
  const devCounts = Object.fromEntries(DEVICE_KINDS.map((k) => [k, 0]));
  for (const it of state.items) devCounts[it.device?.kind || "other"]++;
  for (const k of DEVICE_KINDS) {
    if (!devCounts[k]) continue;
    const b = document.createElement("button");
    b.className = "nav-item";
    b.dataset.view = `dev:${k}`;
    b.innerHTML = `<span>${devLabel(k)}</span><span class="count">${devCounts[k]}</span>`;
    devBox.appendChild(b);
  }

  const box = $("months");
  box.innerHTML = "";
  for (const [k, e] of months()) {
    const b = document.createElement("button");
    b.className = "nav-item";
    b.dataset.view = `month:${k}`;
    b.innerHTML = `<span>${monthLabel(k)}</span><span class="count">${e.count}</span>`;
    box.appendChild(b);
  }
  document.querySelectorAll(".nav-item").forEach((b) => b.classList.toggle("active", b.dataset.view === state.view));
}

function makeCard(path) {
  const it = state.byPath.get(path);
  const card = document.createElement("div");
  card.className = "card" + (state.selected.has(path) ? " selected" : "") + (state.cursor === path ? " cursor" : "");
  card.dataset.path = path;
  card.title = [it.name, [devDetail(path), dimsOf(path)].filter(Boolean).join(" · "), locationOf(path)].join("\n");
  // Badge = which device took it; fall back to where it lives when the device is unknown.
  const dev = devOf(path);
  const badgeText = dev !== "other" ? devLabel(dev) : isPhoto(path) ? t("badge.photo") : srcOf(path) === "other" ? t("badge.other") : "";
  const badge = badgeText ? `<span class="badge">${badgeText}</span>` : "";
  card.innerHTML = `<div class="thumb"><img alt="" loading="lazy" draggable="false"></div>
    <div class="check"></div><button class="zoom">${t("card.zoom")}</button>${badge}<div class="chips">${chipsHtml(path)}</div>${state.tidyKeep?.has(path) ? `<span class="keeper">${t("tidy.keep")}</span>` : ""}
    <div class="meta"><span>${shortTime(it.created)}</span><span>${fmtSize(it.size)}</span></div>`;
  const src = state.thumbs.get(path);
  if (src) card.querySelector("img").src = src;
  cardEls.set(path, card);
  return card;
}

function section(title, sub, paths, extra) {
  const wrap = document.createElement("div");
  const head = document.createElement("div");
  head.className = "section-head";
  head.innerHTML = `<h2>${title}</h2><div class="right"><span class="sub">${sub}</span></div>`;
  if (extra) head.querySelector(".right").appendChild(extra);
  const grid = document.createElement("div");
  grid.className = "grid";
  for (const p of paths) grid.appendChild(makeCard(p));
  wrap.append(head, grid);
  return wrap;
}

async function render() {
  const content = $("content");
  cardEls = new Map();
  content.innerHTML = "";
  state.order = [];
  let title = t("nav.all");

  if (state.view === "similar") {
    title = t("nav.similar");
    content.innerHTML = `<div class="empty">${t("similar.loading")}</div>`;
    try { await ensureGroups(); } catch (e) { content.innerHTML = `<div class="empty">${t("similar.failed", { e: tErr(e) })}</div>`; return; }
    if (state.view !== "similar") return; // user navigated away meanwhile
    content.innerHTML = "";
    const groups = visibleGroups();
    if (!groups.length) content.innerHTML = `<div class="empty">${t("similar.none")}</div>`;
    groups.forEach((g, i) => {
      const keep = document.createElement("button");
      keep.className = "btn ghost";
      keep.textContent = t("similar.keepNewest");
      keep.onclick = () => { g.slice(1).forEach((p) => state.selected.add(p)); g.slice(0, 1).forEach((p) => state.selected.delete(p)); syncSelection(); };
      const compareBtn = document.createElement("button");
      compareBtn.className = "btn";
      compareBtn.textContent = g.length > MAX_COMPARE ? t("tb.compareFirst", { n: MAX_COMPARE }) : t("tb.compare");
      compareBtn.onclick = () => openCompare(g);
      const btns = document.createElement("span");
      btns.className = "actions";
      btns.append(keep, compareBtn);
      content.appendChild(section(t("similar.group", { i: i + 1 }), `${t("count", { n: g.length })} · ${fmtSize(sumSize(g))}`, g, btns));
      state.order.push(...g);
    });
  } else {
    const filter = state.view.startsWith("month:") ? state.view.slice(6) : null;
    const src = state.view.startsWith("src:") ? state.view.slice(4) : null;
    const devView = state.view.startsWith("dev:") ? state.view.slice(4) : null;
    const appView = state.view.startsWith("app:") ? state.view.slice(4) : null;
    const cat = state.view.startsWith("cat:") ? state.view.slice(4) : null;
    if (filter) title = monthLabel(filter);
    if (src) title = srcLabel(src);
    if (devView) title = devLabel(devView);
    if (appView) title = appName(appView);
    if (cat) title = catLabel(cat);
    if (state.view === "kept") title = t("nav.kept");
    if (state.query) title += ` · ${t("view.search", { q: state.query })}`;
    if (state.tagFilter.size) title += ` · ${[...state.tagFilter].map(tagLabel).join(", ")}`;
    const byMonth = new Map();
    const base = [];
    for (const it of state.items) {
      const k = monthKey(it.created);
      if (filter && k !== filter) continue;
      if (src && srcOf(it.path) !== src) continue;
      if (devView && devOf(it.path) !== devView) continue;
      if (appView && appOf(it.path) !== appView) continue;
      if (cat && !inCat(it.path, cat)) continue;
      if (state.view === "kept" && !isKept(it.path)) continue;
      if (!matchesQuery(it.path)) continue;
      base.push(it.path);
      if (!matchesTags(it.path)) continue;
      if (!byMonth.has(k)) byMonth.set(k, []);
      byMonth.get(k).push(it.path);
    }
    if (!byMonth.size) {
      const msg = state.tagFilter.size ? t("empty.filter")
        : state.query ? t("empty.query", { q: state.query })
        : cat && state.analyzing ? t("empty.analyzing")
        : cat === "temp" ? t("empty.temp")
        : cat === "private" ? t("empty.private")
        : t("empty.none");
      content.innerHTML = `<div class="empty">${msg.replace(/</g, "&lt;")}</div>`;
    }
    for (const [k, paths] of byMonth) {
      const left = unorganized(paths).length;
      const status = left ? ` · ${t("sec.unorg", { n: left })}` : ` · ${t("sec.inFolder", { k: layoutPath(state.byPath.get(paths[0]).created) })}`;
      content.appendChild(section(monthLabel(k), `${t("count", { n: paths.length })} · ${fmtSize(sumSize(paths))}${status}`, paths));
      state.order.push(...paths);
    }
    state.lastBase = base;
    renderFilterBar(base);
  }
  if (state.view === "similar") { state.lastBase = null; renderFilterBar(null); }

  $("view-title").textContent = title;
  $("view-sub").textContent = `${t("count", { n: state.order.length })} · ${fmtSize(sumSize(state.order))}`;
  syncSelection();
  loadThumbs(state.order);
}

function syncSelection() {
  for (const [p, el] of cardEls) el.classList.toggle("selected", state.selected.has(p));
  const n = state.selected.size;
  document.body.classList.toggle("selecting", n > 0);
  $("trash").disabled = n === 0;
  $("clear-sel").hidden = n === 0;
  $("compare-sel").hidden = n < 2;
  const allKept = n > 0 && [...state.selected].every(isKept);
  $("keep-sel").hidden = n === 0;
  setLabel("keep-sel", t(allKept ? "tb.unkeep" : "tb.keep"), t(allKept ? "tb.unkeep.tip" : "tb.keep.tip"));
  setLabel("compare-sel", n > MAX_COMPARE ? t("tb.compareFirst", { n: MAX_COMPARE }) : t("tb.compareN", { n }), t("tb.compare.tip"));
  const left = unorganized(organizeScope()).length;
  const org = $("organize");
  org.disabled = left === 0;
  setLabel("organize", organizeLabel(left));
  $("selinfo").textContent = n
    ? t("sel.info", { n, size: fmtSize(sumSize([...state.selected])) })
    : t("sel.hint");
  $("selinfo").classList.toggle("hint", n === 0);
  fitToolbar(); // buttons appear/disappear with the selection
}

async function loadThumbs(paths) {
  const job = ++thumbJob;
  const todo = paths.filter((p) => !state.thumbs.has(p));
  const total = todo.length;
  for (let i = 0; i < total; i += 24) {
    if (job !== thumbJob) return;
    setProgress(t("progress.thumbs"), i, total);
    let res;
    try { res = await invoke("thumbnails", { paths: todo.slice(i, i + 24) }); } catch { continue; }
    for (const th of res) {
      if (!th.thumb) continue;
      const url = convertFileSrc(th.thumb);
      state.thumbs.set(th.path, url);
      const img = cardEls.get(th.path)?.querySelector("img");
      if (img) img.src = url;
    }
  }
  if (job === thumbJob) {
    setProgress(null);
    setStatus(t("status.summary", { n: state.items.length, size: fmtSize(sumSize(state.items.map((i) => i.path))) }));
    // OCR runs after thumbnails so the grid fills in first; it resumes where it left off.
    if (!state.analyzing) analyzeAll();
  }
}

// ---------- actions ----------
async function trashPaths(paths, { confirmed = false } = {}) {
  if (!paths.length) return false;
  const nPhotos = paths.filter(isPhoto).length;
  const nFiles = paths.length - nPhotos;
  const lines = [t("trash.free", { size: fmtSize(sumSize(paths)) })];
  if (nFiles) lines.push(t("trash.files", { n: nFiles }));
  if (nPhotos) {
    lines.push(t("trash.photos", { n: nPhotos }));
    lines.push(t("trash.photosNote"));
  }
  const ok = confirmed || (await confirmDialog(t("trash.title", { n: paths.length }), lines.join("\n"), t("trash.yes"), true));
  if (!ok) return false;
  let removed;
  try {
    removed = await invoke("trash", { paths });
  } catch (e) {
    toast(t("trash.failed", { e: tErr(e) }));
    return false;
  }
  dropPaths(removed);
  renderSidebar();
  await render();
  const skipped = paths.length - removed.length;
  toast(skipped ? t("trash.partial", { done: removed.length, skip: skipped }) : t("trash.done", { n: removed.length }));
  return removed.length > 0;
}

async function organize() {
  // Same scope as the button label: selection, otherwise what the current view shows.
  const paths = unorganized(organizeScope());
  if (!paths.length) return toast(t("org.already"));
  const plan = new Map();
  for (const p of paths) {
    const k = layoutPath(state.byPath.get(p).created);
    plan.set(k, (plan.get(k) || 0) + 1);
  }
  const lines = [...plan.entries()].sort((a, b) => (a[0] < b[0] ? 1 : -1)).map(([k, c]) => `• ${k}/  ←  ${t("count", { n: c })}`);
  const nPhotos = paths.filter(isPhoto).length;
  if (nPhotos) {
    lines.push(
      "",
      t("org.photos1", { n: nPhotos }), t("org.photos2"), t("org.photos3"), t("org.photos4"));
  }
  const ok = await confirmDialog(
    t("org.title", { n: paths.length }),
    `${t("org.into", { root: tilde(orgRoot()) })}\n${lines.join("\n")}`,
    t("org.yes"));
  if (!ok) return;
  setStatus(nPhotos ? t("status.exporting", { n: nPhotos }) : t("status.organizing"));
  let r;
  try {
    r = await invoke("organize", { paths, root: orgRoot(), layout: state.settings.layout });
  } catch (e) {
    return toast(t("org.failed", { e: tErr(e) }));
  }
  for (const { from, to } of r.moved) {
    const it = state.byPath.get(from);
    if (it) { it.path = to; it.name = to.slice(to.lastIndexOf("/") + 1); }
    if (state.kept.delete(from)) { state.kept.add(to); persistKept(); }
    if (state.overrides[from]) { state.overrides[to] = state.overrides[from]; delete state.overrides[from]; persistOverrides(); }
    const a = state.analysis.get(from);
    if (a) { state.analysis.delete(from); state.analysis.set(to, a); }
    const thumb = state.thumbs.get(from);
    if (thumb) { state.thumbs.delete(from); state.thumbs.set(to, thumb); }
    if (state.selected.delete(from)) state.selected.add(to);
  }
  if (state.groups) {
    const map = new Map(r.moved.map((m) => [m.from, m.to]));
    state.groups = state.groups.map((g) => g.map((p) => map.get(p) || p));
  }
  setItems(state.items);
  await render();
  const failText = (f) => (f.path ? `${state.byPath.get(f.path)?.name || f.path.split("/").pop()} — ` : "") + tErr(f.error);
  toast(r.failed.length
    ? t("org.partial", { n: r.moved.length, f: r.failed.length, first: failText(r.failed[0]) })
    : t("org.result", { n: r.moved.length }));
  setStatus(t("status.summary", { n: state.items.length, size: fmtSize(sumSize(state.items.map((i) => i.path))) }));
  if (r.failed.length) console.warn(r.failed);
}

// ---------- preview ----------
async function openPreview(path) {
  state.preview = path;
  const it = state.byPath.get(path);
  if (!it) return closePreview();
  const img = $("preview-img");
  img.src = state.thumbs.get(path) || ""; // show the thumbnail instantly, then upgrade
  $("preview-name").textContent = it.name;
  $("preview-sub").textContent = [fmtDate(it.created), appOf(path) ? appName(appOf(path)) : "", devDetail(path), dimsOf(path), fmtSize(it.size), locationOf(path)].filter(Boolean).join(" · ");
  renderPreviewCats(path);
  const aiSummary = state.analysis.get(path)?.ai?.summary;
  $("preview-sub").title = aiSummary ? `${t("ai.desc")}: ${aiSummary}` : "";
  if (aiSummary) $("preview-sub").insertAdjacentHTML("beforeend", `<div class="ai-desc">✦ ${esc(aiSummary)}</div>`);
  const chips = chipsHtml(path);
  if (chips) $("preview-sub").insertAdjacentHTML("beforeend", `<span class="preview-tags">${chips}</span>`);
  $("pv-copy").disabled = !state.analysis.get(path)?.text;
  const photo = isPhoto(path);
  $("pv-reveal").textContent = t(photo ? "pv.openPhotos" : "pv.reveal");
  $("pv-open").hidden = photo;
  $("preview").hidden = false;
  try {
    const full = await invoke("preview_path", { path });
    if (state.preview === path) img.src = convertFileSrc(full);
  } catch (e) {
    toast(String(e));
  }
}
function closePreview() { state.preview = null; $("preview").hidden = true; $("preview-img").removeAttribute("src"); }
function stepPreview(d) {
  const i = state.order.indexOf(state.preview);
  const next = state.order[i + d];
  if (next) openPreview(next);
}

// ---------- category corrections (preview) ----------
function renderPreviewCats(path) {
  const box = $("pv-cats");
  const cats = objCatsOf(path);
  const options = OBJECT_CATEGORIES.filter((c) => !cats.includes(c.key))
    .map((c) => `<option value="${c.key}">${esc(objCatLabel(c.key, LANG))}</option>`).join("");
  box.innerHTML = (cats.length ? `<span class="pv-cats-label">${t("obj.inPhotoLabel")}</span>` : "")
    + cats.map((k) => `<span class="cat-chip">${esc(objCatLabel(k, LANG))}<button data-rm="${k}" title="${esc(t("obj.notThis"))}">✕</button></span>`).join("")
    + `<select class="cat-add" id="pv-cat-add"><option value="">${esc(t("obj.add"))}</option>${options}</select>`;
}
function afterCategoryEdit(path) {
  renderPreviewCats(path);
  renderSidebar();
  if (state.query || state.view.startsWith("cat:")) render();
}
$("pv-cats").addEventListener("click", (e) => {
  const b = e.target.closest("[data-rm]");
  if (!b || !state.preview) return;
  correctCategory(state.preview, b.dataset.rm, false);
  afterCategoryEdit(state.preview);
});
$("pv-cats").addEventListener("change", (e) => {
  if (e.target.id !== "pv-cat-add" || !e.target.value || !state.preview) return;
  correctCategory(state.preview, e.target.value, true);
  afterCategoryEdit(state.preview);
});

// ---------- compare ----------
const MAX_COMPARE = 8;
const cmp = { paths: [], marked: new Set(), dims: new Map(), zoomed: false, syncing: false };

function openCompare(paths) {
  paths = paths.filter((p) => state.byPath.has(p)).slice(0, MAX_COMPARE);
  if (paths.length < 2) return toast(t("cmp.need2"));
  Object.assign(cmp, { paths, marked: new Set(), dims: new Map(), zoomed: false });
  $("compare").hidden = false;
  renderCompare();
}

function closeCompare() {
  $("compare").hidden = true;
  $("cmp-grid").innerHTML = "";
  cmp.paths = [];
}

function compareTags(p) {
  const items = cmp.paths.map((q) => state.byPath.get(q));
  const it = state.byPath.get(p);
  const tags = [];
  const uniqueMax = (val) => {
    const vals = cmp.paths.map(val);
    const max = Math.max(...vals);
    return val(p) === max && vals.filter((v) => v === max).length === 1;
  };
  if (uniqueMax((q) => state.byPath.get(q).created)) tags.push(t("cmp.newest"));
  if (uniqueMax((q) => state.byPath.get(q).size)) tags.push(t("cmp.largest"));
  if (cmp.dims.size === cmp.paths.length && uniqueMax((q) => { const d = cmp.dims.get(q); return d ? d.w * d.h : 0; })) tags.push(t("cmp.sharpest"));
  if (items.length && it.created === Math.min(...items.map((i) => i.created)) && !tags.includes(t("cmp.newest"))) tags.push(t("cmp.oldest"));
  return tags;
}

function paneInfo(p) {
  const it = state.byPath.get(p);
  const d = cmp.dims.get(p);
  return `${fmtDate(it.created)} · ${fmtSize(it.size)}${d ? ` · ${d.w}×${d.h}` : ""}<br>${devDetail(p)} · ${locationOf(p)}`;
}

function refreshCompareMeta() {
  for (const pane of $("cmp-grid").children) {
    const p = pane.dataset.path;
    pane.classList.toggle("marked", cmp.marked.has(p));
    pane.querySelector(".sub").innerHTML = paneInfo(p);
    pane.querySelector(".cmp-tags").innerHTML = compareTags(p).map((x) => `<span class="tag">${x}</span>`).join("");
    pane.querySelector("[data-act=mark]").textContent = t(cmp.marked.has(p) ? "cmp.unmark" : "cmp.mark");
  }
  const n = cmp.marked.size;
  const del = $("cmp-delete");
  del.disabled = n === 0;
  del.textContent = n ? t("cmp.deleteN", { n, size: fmtSize(sumSize([...cmp.marked])) }) : t("cmp.pick");
  $("cmp-title").textContent = t("cmp.title", { n: cmp.paths.length });
  $("cmp-zoom").textContent = t(cmp.zoomed ? "cmp.fit" : "cmp.actual");
}

function renderCompare() {
  const grid = $("cmp-grid");
  grid.innerHTML = "";
  grid.classList.toggle("zoomed", cmp.zoomed);
  const n = cmp.paths.length;
  const cols = n <= 4 ? n : Math.ceil(n / 2);
  grid.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
  cmp.paths.forEach((p, i) => {
    const it = state.byPath.get(p);
    const pane = document.createElement("div");
    pane.className = "cmp-pane";
    pane.dataset.path = p;
    pane.innerHTML = `
      <div class="cmp-view"><span class="cmp-num">${i + 1}</span><img alt="" draggable="false"></div>
      <div class="cmp-info">
        <div class="cmp-name"></div>
        <div class="sub"></div>
        <div class="cmp-tags"></div>
        <div class="actions">
          <button class="btn ghost" data-act="keep">${t("cmp.keepOnly")}</button>
          <button class="btn ghost" data-act="mark">${t("cmp.mark")}</button>
        </div>
      </div>`;
    pane.querySelector(".cmp-name").textContent = it.name;
    pane.querySelector(".cmp-name").title = it.name;
    const img = pane.querySelector("img");
    img.src = state.thumbs.get(p) || "";
    img.addEventListener("load", () => {
      // record real dimensions only from the full-size image, not the thumbnail
      if (img.dataset.full) { cmp.dims.set(p, { w: img.naturalWidth, h: img.naturalHeight }); refreshCompareMeta(); }
    });
    invoke("preview_path", { path: p })
      .then((full) => { if (cmp.paths.includes(p)) { img.dataset.full = "1"; img.src = convertFileSrc(full); } })
      .catch(() => {});
    pane.querySelector(".cmp-view").addEventListener("scroll", onCompareScroll);
    grid.appendChild(pane);
  });
  refreshCompareMeta();
}

function toggleMark(p) {
  cmp.marked.has(p) ? cmp.marked.delete(p) : cmp.marked.add(p);
  refreshCompareMeta();
}

/** In actual-size mode, scrolling one image scrolls the others to the same relative spot. */
function onCompareScroll(e) {
  if (!cmp.zoomed || cmp.syncing) return;
  const src = e.currentTarget;
  const fx = src.scrollLeft / Math.max(1, src.scrollWidth - src.clientWidth);
  const fy = src.scrollTop / Math.max(1, src.scrollHeight - src.clientHeight);
  cmp.syncing = true;
  for (const v of document.querySelectorAll("#cmp-grid .cmp-view")) {
    if (v === src) continue;
    v.scrollLeft = fx * (v.scrollWidth - v.clientWidth);
    v.scrollTop = fy * (v.scrollHeight - v.clientHeight);
  }
  requestAnimationFrame(() => (cmp.syncing = false));
}

function toggleCompareZoom() {
  cmp.zoomed = !cmp.zoomed;
  $("cmp-grid").classList.toggle("zoomed", cmp.zoomed);
  refreshCompareMeta();
}

async function deleteMarked() {
  const marked = cmp.paths.filter((p) => cmp.marked.has(p));
  if (!marked.length) return;
  if (!(await trashPaths(marked))) return;
  cmp.paths = cmp.paths.filter((p) => state.byPath.has(p));
  cmp.marked = new Set([...cmp.marked].filter((p) => state.byPath.has(p)));
  if (cmp.paths.length < 2) return closeCompare();
  renderCompare();
}

$("cmp-grid").addEventListener("click", (e) => {
  const pane = e.target.closest(".cmp-pane");
  if (!pane) return;
  const p = pane.dataset.path;
  const act = e.target.closest("[data-act]")?.dataset.act;
  if (act === "keep") {
    cmp.marked = new Set(cmp.paths.filter((q) => q !== p));
    refreshCompareMeta();
  } else if (act === "mark" || (e.target.closest(".cmp-view") && !cmp.zoomed)) {
    toggleMark(p);
  }
});
$("cmp-grid").addEventListener("dblclick", (e) => {
  if (cmp.zoomed && e.target.closest(".cmp-view")) toggleMark(e.target.closest(".cmp-pane").dataset.path);
});
$("cmp-zoom").onclick = toggleCompareZoom;
$("cmp-delete").onclick = deleteMarked;
$("cmp-close").onclick = closeCompare;
$("compare-sel").onclick = () => openCompare(state.order.filter((p) => state.selected.has(p)));

// ---------- quick tidy: keep one per group of similar screenshots ----------
/**
 * Which screenshot of a similar group to keep: anything the user marked "kept" always stays;
 * otherwise the highest resolution, then the largest file, then the newest.
 */
function keepersOf(group) {
  const kept = group.filter(isKept);
  if (kept.length) return kept;
  const score = (p) => {
    const it = state.byPath.get(p) || {};
    return [(it.width || 0) * (it.height || 0), it.size || 0, it.created || 0];
  };
  const best = group.slice().sort((a, b) => {
    const [x, y] = [score(a), score(b)];
    return y[0] - x[0] || y[1] - x[1] || y[2] - x[2];
  })[0];
  return [best];
}

async function quickTidy() {
  try { await ensureGroups(); } catch (e) { return toast(t("similar.failed", { e: tErr(e) })); }
  const groups = visibleGroups();
  if (!groups.length) return toast(t("tidy.none"));
  const keep = new Set(), drop = [];
  for (const g of groups) {
    const k = keepersOf(g);
    k.forEach((p) => keep.add(p));
    g.filter((p) => !k.includes(p)).forEach((p) => drop.push(p));
  }
  if (!drop.length) return toast(t("tidy.none"));
  // Show the plan first: similar view, the ones to delete selected, the keepers labelled.
  state.view = "similar";
  state.tidyKeep = keep;
  state.selected = new Set(drop);
  renderSidebar();
  await render();
  $("content").scrollTop = 0;
  const body = [
    t("tidy.body", { groups: groups.length, keep: keep.size, size: fmtSize(sumSize(drop)) }),
    t("tidy.rule"),
    drop.some(isPhoto) ? t("trash.photosNote").trim() : "",
  ].filter(Boolean).join("\n");
  const ok = await confirmDialog(t("tidy.title", { n: drop.length }), body, t("trash.yes"), true);
  if (!ok) return toast(t("tidy.cancelled"));
  await trashPaths(drop, { confirmed: true });
  state.tidyKeep = null;
  render();
}

$("review-start").onclick = quickTidy;

// ---------- keyboard cursor ----------
function setCursor(path, scroll = true) {
  cardEls.get(state.cursor)?.classList.remove("cursor");
  state.cursor = path;
  const el = cardEls.get(path);
  if (!el) return;
  el.classList.add("cursor");
  if (scroll) el.scrollIntoView({ block: "nearest" });
}

function selectRange(a, b) {
  const i = state.order.indexOf(a), j = state.order.indexOf(b);
  if (i < 0 || j < 0) return;
  const [lo, hi] = i < j ? [i, j] : [j, i];
  state.order.slice(lo, hi + 1).forEach((p) => state.selected.add(p));
}

/** Card in the nearest row above/below `from`, horizontally closest. Works across sections. */
function verticalNeighbour(from, dir) {
  const r = cardEls.get(from).getBoundingClientRect();
  const cx = r.left + r.width / 2;
  let best = null, bestRow = null, bestDx = Infinity;
  for (const p of state.order) {
    const q = cardEls.get(p).getBoundingClientRect();
    if (dir === "up" ? q.bottom > r.top + 1 : q.top < r.bottom - 1) continue;
    const dx = Math.abs(q.left + q.width / 2 - cx);
    const closerRow = bestRow === null || (dir === "up" ? q.top > bestRow + 1 : q.top < bestRow - 1);
    if (closerRow) { bestRow = q.top; best = p; bestDx = dx; }
    else if (Math.abs(q.top - bestRow) <= 1 && dx < bestDx) { best = p; bestDx = dx; }
  }
  return best;
}

function moveCursor(dir, extend) {
  if (!state.order.length) return;
  const cur = cardEls.has(state.cursor) ? state.cursor : null;
  if (!cur) { setCursor(state.order[0]); state.anchor = state.order[0]; return; }
  const i = state.order.indexOf(cur);
  const next =
    dir === "left" ? state.order[i - 1] :
    dir === "right" ? state.order[i + 1] :
    verticalNeighbour(cur, dir);
  if (!next) return;
  if (extend) {
    if (!state.anchor || !cardEls.has(state.anchor)) state.anchor = cur;
    selectRange(state.anchor, next);
    syncSelection();
  } else {
    state.anchor = next;
  }
  setCursor(next);
}

// ---------- drag (rubber-band) selection ----------
const drag = { active: false, started: false, x0: 0, y0: 0, base: null, rects: null, box: null, last: null, raf: 0, suppressClick: false };

function contentPoint(e) {
  const c = $("content"), r = c.getBoundingClientRect();
  return { x: e.clientX - r.left + c.scrollLeft, y: e.clientY - r.top + c.scrollTop };
}

function updateDrag() {
  const e = drag.last;
  const { x, y } = contentPoint(e);
  const left = Math.min(x, drag.x0), top = Math.min(y, drag.y0);
  const right = Math.max(x, drag.x0), bottom = Math.max(y, drag.y0);
  Object.assign(drag.box.style, { left: `${left}px`, top: `${top}px`, width: `${right - left}px`, height: `${bottom - top}px` });
  state.selected = new Set(drag.base);
  let lastHit = null;
  for (const r of drag.rects) {
    if (r.left < right && r.right > left && r.top < bottom && r.bottom > top) { state.selected.add(r.p); lastHit = r.p; }
  }
  if (lastHit) setCursor(lastHit, false);
  syncSelection();
}

function autoScroll() {
  if (!drag.active || !drag.started) return;
  const c = $("content"), r = c.getBoundingClientRect(), y = drag.last.clientY;
  const edge = 40;
  const dy = y > r.bottom - edge ? Math.min(24, (y - (r.bottom - edge)) / 2 + 4)
           : y < r.top + edge ? -Math.min(24, ((r.top + edge) - y) / 2 + 4) : 0;
  if (dy) { c.scrollTop += dy; updateDrag(); }
  drag.raf = requestAnimationFrame(autoScroll);
}

$("content").addEventListener("mousedown", (e) => {
  if (e.button !== 0 || e.target.closest("button")) return;
  e.preventDefault(); // no text selection / native image drag
  const p = contentPoint(e);
  Object.assign(drag, { active: true, started: false, x0: p.x, y0: p.y, last: e, additive: e.shiftKey || e.metaKey });
});

window.addEventListener("mousemove", (e) => {
  if (!drag.active) return;
  drag.last = e;
  if (!drag.started) {
    const p = contentPoint(e);
    if (Math.hypot(p.x - drag.x0, p.y - drag.y0) < 6) return;
    drag.started = true;
    drag.base = drag.additive ? new Set(state.selected) : new Set();
    const c = $("content"), cr = c.getBoundingClientRect();
    drag.rects = state.order.map((path) => {
      const q = cardEls.get(path).getBoundingClientRect();
      const top = q.top - cr.top + c.scrollTop, left = q.left - cr.left + c.scrollLeft;
      return { p: path, left, top, right: left + q.width, bottom: top + q.height };
    });
    drag.box = document.createElement("div");
    drag.box.className = "marquee";
    c.appendChild(drag.box);
    drag.raf = requestAnimationFrame(autoScroll);
  }
  updateDrag();
});

window.addEventListener("mouseup", () => {
  if (!drag.active) return;
  if (drag.started) {
    drag.box.remove();
    cancelAnimationFrame(drag.raf);
    drag.suppressClick = true; // the click that follows this mouseup is part of the drag
    setTimeout(() => (drag.suppressClick = false), 0);
  }
  drag.active = drag.started = false;
});

// ---------- events ----------
document.querySelector("nav").addEventListener("click", (e) => {
  const b = e.target.closest(".nav-item");
  if (!b) return;
  state.view = b.dataset.view;
  state.anchor = null;
  state.tidyKeep = null;
  renderSidebar();
  render();
  $("content").scrollTop = 0;
});

$("content").addEventListener("click", (e) => {
  if (drag.suppressClick) return;
  const card = e.target.closest(".card");
  if (!card) return;
  const path = card.dataset.path;
  if (e.target.closest(".zoom")) return openPreview(path);
  if (e.shiftKey && state.anchor) {
    selectRange(state.anchor, path);
  } else if (state.selected.has(path)) {
    state.selected.delete(path);
  } else {
    state.selected.add(path);
  }
  state.anchor = path;
  setCursor(path, false);
  syncSelection();
});
$("content").addEventListener("dblclick", (e) => {
  const card = e.target.closest(".card");
  if (card) openPreview(card.dataset.path);
});

$("select-all").onclick = () => { state.order.forEach((p) => state.selected.add(p)); syncSelection(); };
$("clear-sel").onclick = () => { state.selected.clear(); syncSelection(); };
$("trash").onclick = () => trashPaths([...state.selected]);
$("organize").onclick = organize;
$("refresh").onclick = refresh;
$("dir").onclick = () => invoke("open_file", { path: orgRoot() });

$("pv-copy").onclick = async () => {
  const text = state.analysis.get(state.preview)?.text;
  if (!text) return;
  try { await navigator.clipboard.writeText(text); toast(t("copy.done")); }
  catch { toast(t("copy.failed")); }
};

function setQuery(q) {
  state.query = q.trim();
  state.anchor = null;
  $("qpill").hidden = !state.query;
  $("qpill-text").textContent = state.query ? `“${state.query}”` : "";
  fitToolbar();
  return render();
}
$("qpill").onclick = () => setQuery("");

// ---------- Spotlight-style search (⌘F / ⌘K) ----------
const SPOT_MAX = 60;
const spot = { results: [], sel: 0 };
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

/** The first OCR line containing the query (space/case-insensitive), with the hit highlighted. */
function snippet(p, q) {
  const text = state.analysis.get(p)?.text || "";
  const sq = squash(q);
  const line = text.split("\n").find((l) => squash(l).includes(sq));
  if (!line) {
    // Found through a tell-tale word in the text (e.g. "아메리카노" for 커피): show that line.
    const words = queryCategoryKeys(q).flatMap((k) => OBJ_CAT_BY_KEY[k].ocr);
    for (const l of text.split("\n")) {
      const low = nfc(l).toLowerCase();
      const w = words.find((x) => low.includes(nfc(x).toLowerCase()));
      if (!w) continue;
      const j = low.indexOf(nfc(w).toLowerCase());
      const start = Math.max(0, j - 24);
      return (start ? "…" : "") + esc(l.slice(start, j)) + `<mark>${esc(l.slice(j, j + w.length))}</mark>` + esc(l.slice(j + w.length));
    }
    // Found by what is in the picture: say so instead of showing an unrelated text line.
    const summary = state.analysis.get(p)?.ai?.summary;
    const cats = objCatLabels(p);
    if (summary) return esc(cats.length ? `${t("obj.inPhoto", { list: cats.slice(0, 3).join(" · ") })} — ${summary}` : summary);
    return esc(cats.length ? t("obj.inPhoto", { list: cats.slice(0, 4).join(" · ") }) : devDetail(p));
  }
  const i = nfc(line).toLowerCase().indexOf(nfc(q).toLowerCase());
  if (i < 0) return esc(line);
  const start = Math.max(0, i - 24);
  return (start ? "…" : "") + esc(line.slice(start, i)) + `<mark>${esc(line.slice(i, i + q.length))}</mark>` + esc(line.slice(i + q.length));
}

function renderSpot() {
  const q = $("spot-input").value.trim();
  const box = $("spot-results");
  if (!q) {
    // Suggestions: tags and devices present in the library, one click to search.
    const counts = new Map();
    for (const it of state.items) {
      for (const l of labelsOf(it.path)) counts.set(tagLabel(l), (counts.get(tagLabel(l)) || 0) + 1);
    }
    const devs = new Map();
    for (const it of state.items) { const d = devOf(it.path); if (d !== "other") devs.set(devLabel(d), (devs.get(devLabel(d)) || 0) + 1); }
    const chip = ([label, n]) => `<button class="fchip" data-q="${esc(label)}">${esc(label)}<span>${n}</span></button>`;
    const objCounts = new Map();
    for (const it of state.items) for (const k of objCatsOf(it.path)) objCounts.set(k, (objCounts.get(k) || 0) + 1);
    const objChips = [...objCounts].sort((a, b) => b[1] - a[1]).slice(0, 12).map(([k, n]) => [objCatLabel(k, LANG), n]);
    box.innerHTML = (objChips.length ? `<div class="spot-sec">${t("spot.objects")}</div><div class="spot-sugs">${objChips.map(chip).join("")}</div>` : "")
      + (counts.size ? `<div class="spot-sec">${t("spot.tags")}</div><div class="spot-sugs">${[...counts].map(chip).join("")}</div>` : "")
      + (devs.size ? `<div class="spot-sec">${t("nav.devices")}</div><div class="spot-sugs">${[...devs].map(chip).join("")}</div>` : "");
    spot.results = [];
    $("spot-count").textContent = "";
    return;
  }
  const all = state.items.filter((it) => matchesQuery(it.path, q)).map((it) => it.path);
  spot.results = all.slice(0, SPOT_MAX);
  spot.sel = Math.min(spot.sel, Math.max(0, spot.results.length - 1));
  $("spot-count").textContent = t("spot.count", { n: all.length });
  if (!all.length) { box.innerHTML = `<div class="spot-empty">${esc(t("empty.query", { q }))}</div>`; return; }
  box.innerHTML = spot.results.map((p, i) => {
    const it = state.byPath.get(p);
    const th = state.thumbs.get(p);
    return `<div class="spot-row${i === spot.sel ? " sel" : ""}" data-i="${i}">
      <div class="spot-thumb">${th ? `<img src="${th}" alt="">` : ""}</div>
      <div class="spot-main"><div class="spot-name">${esc(it.name)}</div><div class="sub spot-snip">${snippet(p, q)}</div></div>
      <div class="sub">${fmtDate(it.created).slice(0, 10)}</div>
      ${chipsHtml(p)}
    </div>`;
  }).join("") + (all.length > SPOT_MAX ? `<div class="spot-empty">${t("spot.more", { n: all.length - SPOT_MAX })}</div>` : "");
  // Fill in thumbnails that the main grid has not loaded yet.
  const missing = spot.results.filter((p) => !state.thumbs.has(p)).slice(0, 24);
  if (missing.length) {
    invoke("thumbnails", { paths: missing }).then((res) => {
      for (const th of res) {
        if (!th.thumb) continue;
        state.thumbs.set(th.path, convertFileSrc(th.thumb));
        const i = spot.results.indexOf(th.path);
        const cell = box.querySelector(`.spot-row[data-i="${i}"] .spot-thumb`);
        if (cell) cell.innerHTML = `<img src="${state.thumbs.get(th.path)}" alt="">`;
      }
    }).catch(() => {});
  }
}

function moveSpot(d) {
  if (!spot.results.length) return;
  spot.sel = (spot.sel + d + spot.results.length) % spot.results.length;
  const rows = $("spot-results").querySelectorAll(".spot-row");
  rows.forEach((r, i) => r.classList.toggle("sel", i === spot.sel));
  rows[spot.sel]?.scrollIntoView({ block: "nearest" });
}

function openSpot() {
  if (!$("spot").hidden) return $("spot-input").select();
  $("spot").hidden = false;
  $("spot-input").value = state.query;
  spot.sel = 0;
  renderSpot();
  $("spot-input").focus();
  $("spot-input").select();
}
function closeSpot() { $("spot").hidden = true; $("spot-input").blur(); }

/** Apply the query to the main list (⌘Enter), optionally opening one result in the large view. */
async function applySpot(openPath) {
  const q = $("spot-input").value.trim();
  closeSpot();
  if (state.view === "similar") { state.view = "all"; renderSidebar(); }
  await setQuery(q);
  if (openPath) { state.anchor = openPath; openPreview(openPath); }
}

let spotTimer;
$("spot-input").addEventListener("input", () => {
  clearTimeout(spotTimer);
  spot.sel = 0;
  spotTimer = setTimeout(renderSpot, 60);
});
$("spot").addEventListener("click", (e) => {
  if (e.target.id === "spot") return closeSpot();
  const sug = e.target.closest("[data-q]");
  if (sug) { $("spot-input").value = sug.dataset.q; spot.sel = 0; renderSpot(); $("spot-input").focus(); return; }
  const row = e.target.closest(".spot-row");
  if (row) applySpot(spot.results[+row.dataset.i]);
});

$("keep-sel").onclick = () => {
  const sel = [...state.selected];
  const on = !sel.every(isKept);
  setKept(sel, on);
  toast(t(on ? "keep.added" : "keep.removed", { n: sel.length }));
  state.selected.clear();
  render();
};

$("filterbar").addEventListener("click", (e) => {
  if (e.target.closest("#fclear")) { state.tagFilter.clear(); }
  else {
    const chip = e.target.closest(".fchip");
    if (!chip) return;
    const l = chip.dataset.label;
    state.tagFilter.has(l) ? state.tagFilter.delete(l) : state.tagFilter.add(l);
  }
  state.anchor = null;
  render();
  $("content").scrollTop = 0;
});

$("pv-close").onclick = closePreview;
$("pv-prev").onclick = () => stepPreview(-1);
$("pv-next").onclick = () => stepPreview(1);
$("pv-reveal").onclick = () => invoke("reveal", { path: state.preview });
$("pv-open").onclick = () => invoke("open_file", { path: state.preview });
$("pv-trash").onclick = async () => {
  const p = state.preview;
  const i = state.order.indexOf(p);
  if (await trashPaths([p])) {
    const next = state.order[i] || state.order[i - 1];
    next ? openPreview(next) : closePreview();
  }
};
$("preview").addEventListener("click", (e) => { if (e.target.id === "preview") closePreview(); });

document.addEventListener("keydown", (e) => {
  // The first-run analysis window blocks the app; Esc sends it to the background.
  if (!$("analyzing").hidden) { if (e.key === "Escape") $("an-bg").click(); e.preventDefault(); return; }
  if (!$("confirm").hidden) {
    if (e.key === "Escape") $("confirm-no").click();
    if (e.key === "Enter") $("confirm-yes").click();
    return;
  }
  if (!$("spot").hidden) {
    if (e.key === "Escape") { e.preventDefault(); closeSpot(); }
    else if (e.key === "ArrowDown") { e.preventDefault(); moveSpot(1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); moveSpot(-1); }
    else if (e.key === "Enter" && e.metaKey) { e.preventDefault(); applySpot(); }
    else if (e.key === "Enter") { e.preventDefault(); spot.results.length ? applySpot(spot.results[spot.sel]) : applySpot(); }
    else if (e.metaKey && (e.key === "f" || e.key === "k")) { e.preventDefault(); $("spot-input").select(); }
    return; // everything else is typing
  }
  if (e.metaKey && e.key === ",") { e.preventDefault(); openSettings(); return; }
  if (!$("settings").hidden) {
    if (e.key === "Escape" || e.key === "Enter") closeSettings();
    return;
  }
  if (e.metaKey && (e.key === "f" || e.key === "k") && $("compare").hidden) { e.preventDefault(); openSpot(); return; }
  if (!$("compare").hidden) {
    const n = parseInt(e.key, 10);
    if (e.key === "Escape") closeCompare();
    else if (n >= 1 && n <= cmp.paths.length) toggleMark(cmp.paths[n - 1]);
    else if (e.key === "z" || e.key === "Z") toggleCompareZoom();
    else if ((e.key === "Backspace" || e.key === "Delete") && cmp.marked.size) { e.preventDefault(); deleteMarked(); }
    return;
  }
  if (!$("preview").hidden) {
    if (e.key === "Escape" || e.key === " ") { e.preventDefault(); closePreview(); }
    else if (e.key === "ArrowLeft") stepPreview(-1);
    else if (e.key === "ArrowRight") stepPreview(1);
    else if (e.key === "Backspace" || e.key === "Delete") $("pv-trash").click();
    return;
  }
  if ((e.key === "c" || e.key === "C") && !e.metaKey && state.selected.size >= 2) { $("compare-sel").click(); }
  else if ((e.key === "k" || e.key === "K") && !e.metaKey && state.selected.size) { $("keep-sel").click(); }
  else if ((e.key === "r" || e.key === "R") && !e.metaKey) { quickTidy(); }
  else if (e.metaKey && e.key === "a") { e.preventDefault(); $("select-all").click(); }
  else if (e.key === "Escape" && !state.selected.size && state.tagFilter.size) { state.tagFilter.clear(); render(); }
  else if (e.key === "Escape" && !state.selected.size && state.query) setQuery("");
  else if (e.key === "Escape") $("clear-sel").click();
  else if ((e.key === "Backspace" || e.key === "Delete") && state.selected.size) $("trash").click();
  else if (e.key.startsWith("Arrow")) {
    e.preventDefault();
    moveCursor(e.key.slice(5).toLowerCase(), e.shiftKey);
  } else if (e.key === " ") {
    e.preventDefault();
    if (!cardEls.has(state.cursor)) return moveCursor("right", false);
    const p = state.cursor;
    state.selected.has(p) ? state.selected.delete(p) : state.selected.add(p);
    state.anchor = p;
    syncSelection();
  } else if (e.key === "Enter" && cardEls.has(state.cursor)) {
    e.preventDefault();
    openPreview(state.cursor);
  }
  else if (e.metaKey && e.key === "r") { e.preventDefault(); refresh(); }
});

// ---------- settings panel ----------
function showRoot() {
  const root = orgRoot();
  $("dir").textContent = root ? tilde(root) : "–";
  $("dir").title = `${root}\n${t("foot.open")}`;
}

function applyTheme() {
  const theme = state.settings.theme;
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
  invoke("set_theme", { theme }).catch(() => {});
}

function applyLanguage() {
  LANG = resolveLang(state.settings.lang);
  applyStaticI18n();
}

async function loadSettings() {
  try { Object.assign(state.settings, await invoke("load_settings")); } catch { /* defaults */ }
  applyLanguage();
  applyTheme();
  // First run: pick up folders of capture apps that are installed (e.g. ~/Desktop/oh-my-opensnap).
  if (!Array.isArray(state.settings.captureDirs)) {
    const found = await invoke("detect_capture_dirs").catch(() => []);
    state.settings.captureDirs = found;
    saveSettings();
    if (found.length) setTimeout(() => toast(t("capture.detected", { dirs: found.map(tilde).join(", ") })), 1200);
  }
}

// ---------- live updates ----------
let watchedKey = "";
/** (Re)start the folder watcher when the set of folders changes. */
function updateWatch() {
  const dirs = [...new Set([state.dir, orgRoot(), ...(state.settings.captureDirs || [])].filter(Boolean))];
  const key = dirs.join("\n");
  if (key === watchedKey) return;
  watchedKey = key;
  invoke("watch_dirs", { dirs }).catch(() => {});
}
let liveTimer;
function scheduleLiveRefresh() {
  clearTimeout(liveTimer);
  liveTimer = setTimeout(() => {
    // Don't reshuffle the list under an open review/compare/dialog; try again shortly.
    const busy = ["compare", "confirm"].some((id) => !$(id).hidden);
    busy ? scheduleLiveRefresh() : refresh();
  }, busyRetryMs());
}
const busyRetryMs = () => (["compare", "confirm"].some((id) => !$(id).hidden) ? 2000 : 800);
window.__TAURI__.event?.listen("shots-changed", scheduleLiveRefresh);
// Menu-bar menu actions (the window has already been brought to front by the backend).
window.__TAURI__.event?.listen("tray-action", (e) => {
  for (const id of ["confirm", "spot", "settings", "preview", "compare"]) if (!$(id).hidden && id !== "settings") $(id).hidden = true;
  if (e.payload === "quick_tidy") quickTidy();
  else if (e.payload === "search") openSpot();
  else if (e.payload === "settings") openSettings();
});

let saveSettingsTimer;
function saveSettings() {
  clearTimeout(saveSettingsTimer);
  saveSettingsTimer = setTimeout(() => {
    invoke("save_settings", { settings: state.settings }).catch((e) => toast(t("set.saveFailed", { e: tErr(e) })));
  }, 150);
}

const LAYOUT_EXAMPLE = { month: "2026-05/…png", year_month: "2026/05/…png", year: "2026/…png" };

function renderSettings() {
  const st = state.settings;
  for (const seg of document.querySelectorAll("#settings .seg")) {
    for (const b of seg.children) b.classList.toggle("on", st[seg.dataset.key] === b.dataset.v);
  }
  for (const sw of document.querySelectorAll("#settings .switch")) sw.checked = !!st[sw.dataset.key];
  $("set-root").textContent = tilde(orgRoot()) + (st.root ? "" : t("set.root.default"));
  $("set-root").title = orgRoot();
  $("set-sc").title = state.dir || "";
  $("set-root-reset").disabled = !st.root;
  $("set-layout-ex").textContent = t("set.layout.ex", { path: `${tilde(orgRoot())}/${LAYOUT_EXAMPLE[st.layout]}` });
  $("set-sc").textContent = tilde(state.dir || "–");
  const caps = st.captureDirs || [];
  $("set-capture-list").innerHTML = caps.length
    ? caps.map((d, i) => `<div class="cap-item" title="${d}"><span class="path">${tilde(d)}</span><button class="link cap-del" data-i="${i}">${t("set.capture.remove")}</button></div>`).join("")
    : `<div class="cap-empty">${t("set.capture.none")}</div>`;
  const same = nfc(state.dir || "") === nfc(orgRoot());
  $("set-sc-apply").disabled = same;
  $("set-sc-apply").title = t(same ? "set.sc.same" : "set.sc.tip");
}

async function refreshCacheInfo() {
  try {
    const c = await invoke("cache_info");
    $("set-cache-img").textContent = t("set.cache.img.sub", { size: fmtSize(c.images) });
    $("set-cache-ocr").textContent = t("set.cache.ocr.sub", { size: fmtSize(c.ocr) });
  } catch { /* leave as is */ }
}

async function refreshAiAvailability() {
  const ok = await invoke("ai_available").catch(() => false);
  $("set-ai-sub").textContent = t(ok ? "set.ai.sub" : "set.ai.unavailable");
  $("set-ai-switch").disabled = !ok;
}

function openSettings() {
  renderSettings();
  refreshAiAvailability();
  refreshCacheInfo();
  $("settings").hidden = false;
}
function closeSettings() { $("settings").hidden = true; }

/** Apply one changed setting: persist it, then redo only what depends on it. */
async function changeSetting(key, value) {
  state.settings[key] = value;
  saveSettings();
  renderSettings();
  switch (key) {
    case "theme": applyTheme(); break;
    case "lang":
      applyLanguage();
      updateTray();
      showRoot(); renderSidebar(); render(); renderSettings(); refreshCacheInfo();
      if (!$("compare").hidden) refreshCompareMeta();
      break;
    case "layout": renderSidebar(); render(); break;
    case "root": showRoot(); await refresh(); break;
    case "wholeMac": case "photos": case "captureDirs": await refresh(); break;
    case "ai":
      // Re-run analysis so screenshots get (or drop) the on-device description; base results are cached.
      analyzeJob++; state.analyzing = null; setProgress(null); state.analysis.clear();
      renderSidebar(); render(); analyzeAll();
      break;
    case "similarity": state.groups = null; renderSidebar(); if (state.view === "similar") render(); break;
    case "ocr":
      if (value) analyzeAll();
      else { analyzeJob++; state.analyzing = null; setProgress(null); state.analysis.clear(); state.tagFilter.clear(); renderSidebar(); render(); }
      break;
  }
}

$("settings").addEventListener("click", (e) => {
  if (e.target.id === "settings") return closeSettings();
  const b = e.target.closest(".seg button");
  if (b) changeSetting(b.parentElement.dataset.key, b.dataset.v);
});
$("settings").addEventListener("change", (e) => {
  if (e.target.classList.contains("switch")) changeSetting(e.target.dataset.key, e.target.checked);
});
$("open-settings").onclick = openSettings;
$("set-close").onclick = closeSettings;
$("set-root-pick").onclick = async () => {
  const picked = await invoke("pick_folder", { start: orgRoot(), title: t("set.pickTitle") }).catch(() => null);
  if (picked) changeSetting("root", picked);
};
$("set-root-reset").onclick = () => changeSetting("root", "");
$("set-capture-add").onclick = async () => {
  const picked = await invoke("pick_folder", { title: t("set.capture.pickTitle") }).catch(() => null);
  const caps = state.settings.captureDirs || [];
  if (picked && !caps.includes(picked)) changeSetting("captureDirs", [...caps, picked]);
};
$("set-capture-list").addEventListener("click", (e) => {
  const b = e.target.closest(".cap-del");
  if (!b) return;
  changeSetting("captureDirs", (state.settings.captureDirs || []).filter((_, i) => i !== +b.dataset.i));
});
$("set-sc-apply").onclick = async () => {
  const target = orgRoot();
  const ok = await confirmDialog(
    t("set.sc.confirm.title"),
    t("set.sc.confirm.body", { path: tilde(target) }),
    t("set.sc.confirm.yes"));
  if (!ok) return;
  try {
    await invoke("set_screencapture_location", { path: target });
    if (state.settings.root && nfc(state.settings.root) === nfc(target)) state.settings.root = ""; // now the default
    saveSettings();
    toast(t("set.sc.done"));
    await refresh();
    renderSettings();
  } catch (e) { toast(tErr(e)); }
};
$("set-clear-img").onclick = async () => {
  await invoke("clear_cache", { what: "images" }).catch((e) => toast(String(e)));
  state.thumbs.clear();
  render();
  refreshCacheInfo();
  toast(t("set.cache.cleared"));
};
$("set-clear-ocr").onclick = async () => {
  const ok = await confirmDialog(t("set.ocrClear.title"), t("set.ocrClear.body"), t("set.ocrClear.yes"), true);
  if (!ok) return;
  await invoke("clear_cache", { what: "ocr" }).catch((e) => toast(String(e)));
  analyzeJob++; state.analyzing = null; state.analysis.clear();
  renderSidebar(); render();
  refreshCacheInfo();
  analyzeAll();
};

// ---------- glass chrome ----------
// Content scrolls underneath the floating glass bars; keep its padding in sync with their size.
/** Show button labels only if every toolbar button fits on the title's row; otherwise icons only. */
function fitToolbar() {
  const tb = $("topbar");
  const acts = document.querySelector(".toolbar > .actions");
  const title = document.querySelector(".title-block");
  tb.classList.remove("compact");
  const a = acts.getBoundingClientRect(), t = title.getBoundingClientRect(), bar = tb.getBoundingClientRect();
  const wrapped = a.top > t.top + t.height / 2;   // buttons fell below the title
  const overflow = a.right > bar.right - 8;
  tb.classList.toggle("compact", wrapped || overflow || t.width < 110);
}
const barObserver = new ResizeObserver(() => {
  fitToolbar();
  const root = document.documentElement.style;
  root.setProperty("--topbar-h", `${$("topbar").offsetHeight}px`);
  root.setProperty("--bottombar-h", `${$("statusbar").offsetHeight}px`);
});
barObserver.observe($("topbar"));
barObserver.observe($("statusbar"));
// The backend draws a native Liquid Glass sidebar behind the (transparent) webview when it can.
invoke("native_glass").then((on) => document.documentElement.classList.toggle("native-glass", !!on)).catch(() => {});

loadSettings().then(refresh);
