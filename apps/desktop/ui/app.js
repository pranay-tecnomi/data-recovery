// UI logic. Owns no recovery decisions: it renders what the engine reports.

const invoke = window.__TAURI__?.core?.invoke;
const el = (id) => document.getElementById(id);

const views = {
  source: el("view-source"),
  results: el("view-results"),
  recover: el("view-recover"),
};

let candidates = [];
let scan = null;
const selected = new Set();
let chanceFilter = null;
let typeFilter = null;
let detailId = null;

/** Confidence bands, strongest first. */
const BANDS = ["high", "medium", "low", "unknown"];
const BAND_WORD = { high: "High", medium: "Medium", low: "Low", unknown: "Unknown" };

/** Groups extensions the way people actually look for files. */
const TYPE_GROUPS = [
  { id: "pictures", label: "Pictures", ext: ["jpg", "jpeg", "png", "gif", "bmp", "heic", "tif", "tiff", "webp"] },
  { id: "documents", label: "Documents", ext: ["pdf", "doc", "docx", "txt", "rtf", "pages", "md"] },
  { id: "video", label: "Video", ext: ["mp4", "mov", "avi", "mkv", "m4v"] },
  { id: "audio", label: "Audio", ext: ["mp3", "wav", "aac", "flac", "m4a"] },
  { id: "archives", label: "Archives", ext: ["zip", "gz", "tar", "7z", "rar"] },
];

function extensionOf(name) {
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot + 1).toLowerCase();
}

function groupOf(name) {
  const ext = extensionOf(name);
  return TYPE_GROUPS.find((g) => g.ext.includes(ext))?.id ?? "other";
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

function show(name) {
  for (const [key, node] of Object.entries(views)) node.hidden = key !== name;
}

function setStatus(node, message, state = "") {
  node.textContent = message;
  node.className = `status${state ? ` ${state}` : ""}`;
}

/* ---------- source rail ---------- */

function renderSources() {
  const list = el("source-list");
  list.textContent = "";
  if (!scan) return;

  const li = document.createElement("li");
  li.className = "source-item";
  li.setAttribute("aria-current", "true");

  const icon = document.createElement("span");
  icon.className = "source-icon";
  icon.textContent = "IMG";

  const text = document.createElement("span");
  text.className = "source-text";
  const name = document.createElement("span");
  name.className = "source-name";
  name.textContent = scan.source.split("/").pop() || scan.source;
  const size = document.createElement("span");
  size.className = "source-size";
  size.textContent = formatSize(scan.capacity);
  text.append(name, size);

  li.append(icon, text);
  list.append(li);
}

/* ---------- filtering ---------- */

function afterChance(list) {
  return chanceFilter ? list.filter((c) => c.confidence === chanceFilter) : list;
}

function visible() {
  let rows = afterChance(candidates);
  if (typeFilter) rows = rows.filter((c) => groupOf(c.name) === typeFilter);
  const term = el("search").value.trim().toLowerCase();
  if (term) rows = rows.filter((c) => c.name.toLowerCase().includes(term));
  return rows;
}

function renderChances() {
  const box = el("chances");
  box.textContent = "";

  const make = (band, count, label) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "chance";
    button.setAttribute("aria-pressed", String(chanceFilter === band));

    const n = document.createElement("span");
    n.className = "chance-n";
    n.textContent = String(count);

    const l = document.createElement("span");
    l.className = "chance-l";
    if (band) {
      const dot = document.createElement("span");
      dot.className = `dot d-${band}`;
      l.append(dot);
    }
    l.append(label);

    const stack = document.createElement("span");
    stack.append(n, l);
    stack.style.display = "grid";
    button.append(stack);

    button.addEventListener("click", () => {
      chanceFilter = chanceFilter === band ? null : band;
      renderChances();
      renderTypes();
      renderTable();
    });
    return button;
  };

  box.append(make(null, candidates.length, "All files"));
  for (const band of BANDS) {
    const n = candidates.filter((c) => c.confidence === band).length;
    if (n > 0) box.append(make(band, n, `${BAND_WORD[band]} chance`));
  }
}

function renderTypes() {
  const list = el("type-filters");
  list.textContent = "";
  const pool = afterChance(candidates);

  const entries = [{ id: null, label: "All types", n: pool.length }];
  for (const group of TYPE_GROUPS) {
    const n = pool.filter((c) => groupOf(c.name) === group.id).length;
    if (n > 0) entries.push({ id: group.id, label: group.label, n });
  }
  const other = pool.filter((c) => groupOf(c.name) === "other").length;
  if (other > 0) entries.push({ id: "other", label: "Other", n: other });

  for (const entry of entries) {
    const li = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "type-filter";
    button.setAttribute("aria-pressed", String(typeFilter === entry.id));
    const label = document.createElement("span");
    label.textContent = entry.label;
    const n = document.createElement("span");
    n.className = "n";
    n.textContent = String(entry.n);
    button.append(label, n);
    button.addEventListener("click", () => {
      typeFilter = typeFilter === entry.id ? null : entry.id;
      renderTypes();
      renderTable();
    });
    li.append(button);
    list.append(li);
  }
}

/* ---------- detail panel ---------- */

function renderDetail() {
  const panel = el("detail");
  panel.textContent = "";
  const candidate = candidates.find((c) => c.id === detailId);
  if (!candidate) {
    const p = document.createElement("p");
    p.className = "detail-empty";
    p.textContent = "Select a file to see why it is rated this way.";
    panel.append(p);
    return;
  }

  const wrap = document.createElement("div");
  wrap.className = "detail";

  const title = document.createElement("h2");
  title.textContent = candidate.name;
  const sub = document.createElement("p");
  sub.className = "sub";
  sub.textContent = candidate.path;
  wrap.append(title, sub);

  const dl = document.createElement("dl");
  const rows = [
    ["Chance", BAND_WORD[candidate.confidence]],
    ["Size", formatSize(candidate.size)],
    ["Content check", candidate.validation],
    ["Completeness", candidate.completeness],
    ["Found by", candidate.origin],
  ];
  if (candidate.completeness !== "complete") {
    rows.splice(2, 0, ["Recoverable", formatSize(candidate.recovered_size)]);
  }
  for (const [key, value] of rows) {
    const dt = document.createElement("dt");
    dt.textContent = key;
    const dd = document.createElement("dd");
    dd.textContent = value;
    dl.append(dt, dd);
  }
  wrap.append(dl);

  // The evidence behind the rating, rather than a bare score.
  const h3 = document.createElement("h3");
  h3.textContent = "Why this rating";
  wrap.append(h3);

  const ul = document.createElement("ul");
  ul.className = "ev";
  if (candidate.evidence.length === 0) {
    const li = document.createElement("li");
    li.textContent = "No additional evidence was recorded.";
    ul.append(li);
  }
  for (const item of candidate.evidence) {
    const li = document.createElement("li");
    li.className = item.supporting ? "for" : "against";
    const sign = document.createElement("span");
    sign.className = "sign";
    // Marked in text as well as colour.
    sign.textContent = item.supporting ? "+" : "−";
    const text = document.createElement("span");
    text.textContent = item.detail;
    li.append(sign, text);
    ul.append(li);
  }
  wrap.append(ul);
  panel.append(wrap);
}

/* ---------- results table ---------- */

function renderRow(candidate) {
  const row = document.createElement("tr");
  row.setAttribute("aria-selected", String(detailId === candidate.id));
  row.addEventListener("click", (event) => {
    if (event.target.type === "checkbox") return;
    detailId = candidate.id;
    renderTable();
    renderDetail();
  });

  const tick = document.createElement("td");
  tick.className = "tick";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = selected.has(candidate.id);
  box.setAttribute("aria-label", `Select ${candidate.name}`);
  box.addEventListener("change", () => {
    box.checked ? selected.add(candidate.id) : selected.delete(candidate.id);
    updateRecover();
  });
  tick.append(box);

  const name = document.createElement("td");
  const fname = document.createElement("div");
  fname.className = "fname";
  const ico = document.createElement("span");
  ico.className = "fico";
  ico.textContent = (extensionOf(candidate.name) || "?").slice(0, 3).toUpperCase();
  const ftext = document.createElement("span");
  ftext.className = "ftext";
  const ftitle = document.createElement("span");
  ftitle.className = "ftitle";
  ftitle.textContent = candidate.name;
  ftext.append(ftitle);
  if (candidate.path && candidate.path !== candidate.name) {
    const fpath = document.createElement("span");
    fpath.className = "fpath";
    fpath.textContent = candidate.path;
    ftext.append(fpath);
  }
  fname.append(ico, ftext);
  name.append(fname);

  const size = document.createElement("td");
  size.className = "size";
  size.append(formatSize(candidate.size));
  if (candidate.completeness !== "complete") {
    const note = document.createElement("span");
    note.className = "partial-note";
    // A partial file must never read as complete.
    note.textContent = `${formatSize(candidate.recovered_size)} recoverable`;
    size.append(note);
  }

  const chance = document.createElement("td");
  const chip = document.createElement("span");
  chip.className = "chip";
  const dot = document.createElement("span");
  dot.className = `dot d-${candidate.confidence}`;
  chip.append(dot, BAND_WORD[candidate.confidence]);
  chance.append(chip);

  const origin = document.createElement("td");
  origin.className = "origin";
  origin.textContent = candidate.origin;

  row.append(tick, name, size, chance, origin);
  return row;
}

function renderTable() {
  const rows = visible();
  const body = document.querySelector("#results tbody");
  body.textContent = "";

  el("count").textContent = `${rows.length} shown`;

  if (rows.length === 0) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 5;
    td.className = "empty";
    td.textContent = candidates.length === 0
      ? "No recoverable files were found on this source."
      : "No files match these filters.";
    tr.append(td);
    body.append(tr);
    return;
  }
  for (const candidate of rows) body.append(renderRow(candidate));
}

function updateRecover() {
  const n = selected.size;
  el("recover").disabled = n === 0;
  el("recover").textContent = n === 0 ? "Recover" : `Recover ${n}`;
}

/* ---------- actions ---------- */

el("add-source").addEventListener("click", () => {
  show("source");
  el("source").focus();
});

el("search").addEventListener("input", renderTable);

el("select-all").addEventListener("change", (event) => {
  const rows = visible();
  for (const candidate of rows) {
    event.target.checked ? selected.add(candidate.id) : selected.delete(candidate.id);
  }
  renderTable();
  updateRecover();
});

el("back").addEventListener("click", () => show("source"));
el("recover").addEventListener("click", () => {
  el("recover-count").textContent =
    `${selected.size} file${selected.size === 1 ? "" : "s"} selected.`;
  show("recover");
  el("destination").focus();
});
el("cancel-recover").addEventListener("click", () => show("results"));

el("scan").addEventListener("click", async () => {
  const path = el("source").value.trim();
  if (!path) {
    setStatus(el("status"), "Enter the path to a disk image.", "error");
    return;
  }
  if (!invoke) {
    setStatus(el("status"), "Engine unavailable: run this inside the desktop app.", "error");
    return;
  }

  const deep = document.querySelector('input[name="mode"]:checked').value === "deep";
  el("scan").disabled = true;
  setStatus(el("status"), "Scanning. The source is only read, never modified.", "busy");

  try {
    scan = await invoke("scan_image", { path, includeCarving: deep });
    candidates = scan.candidates;
    selected.clear();
    chanceFilter = null;
    typeFilter = null;
    detailId = null;
    el("search").value = "";
    el("select-all").checked = false;

    renderSources();
    renderChances();
    renderTypes();
    renderTable();
    renderDetail();
    updateRecover();

    el("results-title").textContent =
      `${candidates.length} file${candidates.length === 1 ? "" : "s"} found`;
    const fs = scan.partitions.map((p) => p.filesystem).join(", ") || "no filesystem";
    el("results-sub").textContent =
      `${scan.source.split("/").pop()} — ${formatSize(scan.capacity)}, ${fs}`;
    show("results");
  } catch (error) {
    setStatus(el("status"), `Scan failed: ${error}`, "error");
  } finally {
    el("scan").disabled = false;
  }
});

el("do-recover").addEventListener("click", async () => {
  const destination = el("destination").value.trim();
  if (!destination) {
    setStatus(el("recover-status"), "Choose a destination folder.", "error");
    return;
  }
  const deep = document.querySelector('input[name="mode"]:checked').value === "deep";

  el("do-recover").disabled = true;
  setStatus(el("recover-status"), "Recovering files.", "busy");
  try {
    const result = await invoke("recover_files", {
      source: el("source").value.trim(),
      destination,
      selected: [...selected],
      includeCarving: deep,
    });
    const parts = [`${result.written} recovered`];
    if (result.partial) parts.push(`${result.partial} partial`);
    if (result.skipped) parts.push(`${result.skipped} skipped`);
    if (result.failed) parts.push(`${result.failed} failed`);
    setStatus(el("recover-status"), `${parts.join(", ")}. Manifest: ${result.manifest_path}`);
  } catch (error) {
    setStatus(el("recover-status"), `Recovery failed: ${error}`, "error");
  } finally {
    el("do-recover").disabled = false;
  }
});

updateRecover();
