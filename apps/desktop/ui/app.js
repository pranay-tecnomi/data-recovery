// UI logic. Owns no recovery decisions: it renders what the engine reports.

const invoke = window.__TAURI__?.core?.invoke;

const el = (id) => document.getElementById(id);
const sourceInput = el("source");
const carvingInput = el("carving");
const scanButton = el("scan");
const statusLine = el("status");
const resultsPanel = el("results-panel");
const recoverPanel = el("recover-panel");
const tableBody = document.querySelector("#results tbody");
const filterSelect = el("filter");
const summary = el("summary");
const destinationInput = el("destination");
const recoverButton = el("recover");
const recoverStatus = el("recover-status");
const selectAllButton = el("select-all");

let candidates = [];
const selected = new Set();

/** Confidence bands in descending strength, for filtering. */
const BANDS = ["high", "medium", "low", "unknown"];

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

function setStatus(node, message, isError = false) {
  node.textContent = message;
  node.classList.toggle("error", isError);
}

/** Candidates passing the current confidence filter. */
function visibleCandidates() {
  const choice = filterSelect.value;
  if (choice === "all") return candidates;
  const limit = BANDS.indexOf(choice);
  return candidates.filter((c) => BANDS.indexOf(c.confidence) <= limit);
}

function renderSummary() {
  const counts = BANDS.map((band) => ({
    band,
    n: candidates.filter((c) => c.confidence === band).length,
  })).filter((entry) => entry.n > 0);

  summary.textContent = "";
  const total = document.createElement("span");
  total.textContent = `${candidates.length} candidate${candidates.length === 1 ? "" : "s"}`;
  summary.append(total);
  for (const { band, n } of counts) {
    const span = document.createElement("span");
    span.className = `band band-${band}`;
    const dot = document.createElement("span");
    dot.className = "dot";
    span.append(dot, `${n} ${band}`);
    summary.append(span);
  }
}

/** Builds the expandable evidence list for one candidate. */
function evidenceList(candidate) {
  const list = document.createElement("ul");
  list.className = "evidence";
  list.hidden = true;
  for (const item of candidate.evidence) {
    const li = document.createElement("li");
    // Evidence against a candidate is marked in text, not colour alone.
    li.className = item.supporting ? "for" : "against";
    li.textContent = item.supporting ? item.detail : `Against: ${item.detail}`;
    list.append(li);
  }
  if (candidate.evidence.length === 0) {
    const li = document.createElement("li");
    li.textContent = "No additional evidence recorded.";
    list.append(li);
  }
  return list;
}

function renderRow(candidate) {
  const row = document.createElement("tr");

  const tick = document.createElement("td");
  tick.className = "tick";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = selected.has(candidate.id);
  box.setAttribute("aria-label", `Select ${candidate.name}`);
  box.addEventListener("change", () => {
    box.checked ? selected.add(candidate.id) : selected.delete(candidate.id);
    updateRecoverButton();
  });
  tick.append(box);

  const name = document.createElement("td");
  const label = document.createElement("span");
  label.className = "name";
  label.textContent = candidate.name;
  name.append(label);
  if (candidate.path && candidate.path !== candidate.name) {
    const path = document.createElement("span");
    path.className = "path";
    path.textContent = candidate.path;
    name.append(path);
  }
  // "Why" exposes the evidence behind the score rather than asserting it.
  const why = document.createElement("button");
  why.className = "why";
  why.type = "button";
  why.textContent = "Why this rating?";
  why.setAttribute("aria-expanded", "false");
  const evidence = evidenceList(candidate);
  why.addEventListener("click", () => {
    evidence.hidden = !evidence.hidden;
    why.setAttribute("aria-expanded", String(!evidence.hidden));
  });
  name.append(why, evidence);

  const origin = document.createElement("td");
  origin.className = "tag";
  origin.textContent = candidate.origin;

  const size = document.createElement("td");
  size.className = "size";
  size.textContent = formatSize(candidate.size);
  if (candidate.completeness !== "complete") {
    const note = document.createElement("span");
    note.className = "path";
    // A partial file must never look complete.
    note.textContent = `${candidate.completeness} (${formatSize(candidate.recovered_size)} recoverable)`;
    size.append(document.createElement("br"), note);
  }

  const band = document.createElement("td");
  const bandSpan = document.createElement("span");
  bandSpan.className = `band band-${candidate.confidence}`;
  const dot = document.createElement("span");
  dot.className = "dot";
  bandSpan.append(dot, candidate.confidence);
  band.append(bandSpan);

  const check = document.createElement("td");
  check.className = candidate.validation === "invalid" ? "tag invalid" : "tag";
  check.textContent = candidate.validation;

  row.append(tick, name, origin, size, band, check);
  return row;
}

function renderTable() {
  const rows = visibleCandidates();
  tableBody.textContent = "";
  if (rows.length === 0) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 6;
    td.className = "empty";
    td.textContent = candidates.length === 0
      ? "No recoverable files were found."
      : "No candidates match this filter.";
    tr.append(td);
    tableBody.append(tr);
    return;
  }
  for (const candidate of rows) tableBody.append(renderRow(candidate));
}

function updateRecoverButton() {
  recoverButton.disabled = selected.size === 0;
  recoverButton.textContent = selected.size === 0
    ? "Recover selected"
    : `Recover ${selected.size} file${selected.size === 1 ? "" : "s"}`;
}

filterSelect.addEventListener("change", renderTable);

selectAllButton.addEventListener("click", () => {
  for (const candidate of visibleCandidates()) selected.add(candidate.id);
  renderTable();
  updateRecoverButton();
});

scanButton.addEventListener("click", async () => {
  const path = sourceInput.value.trim();
  if (!path) {
    setStatus(statusLine, "Enter the path to a disk image.", true);
    return;
  }
  if (!invoke) {
    setStatus(statusLine, "Engine unavailable: run this inside the desktop app.", true);
    return;
  }

  scanButton.disabled = true;
  setStatus(statusLine, "Scanning. The source is only read, never modified.");
  selected.clear();

  try {
    const result = await invoke("scan_image", {
      path,
      includeCarving: carvingInput.checked,
    });
    candidates = result.candidates;
    renderSummary();
    renderTable();
    updateRecoverButton();
    resultsPanel.hidden = false;
    recoverPanel.hidden = false;

    const parts = [
      `${result.partitions.length} partition${result.partitions.length === 1 ? "" : "s"}`,
      ...result.partitions.map((p) => p.filesystem),
    ];
    const notes = result.diagnostics.length ? ` ${result.diagnostics.join(" ")}` : "";
    setStatus(statusLine, `Scanned ${formatSize(result.capacity)}: ${parts.join(", ")}.${notes}`);
  } catch (error) {
    setStatus(statusLine, `Scan failed: ${error}`, true);
    resultsPanel.hidden = true;
    recoverPanel.hidden = true;
  } finally {
    scanButton.disabled = false;
  }
});

recoverButton.addEventListener("click", async () => {
  const destination = destinationInput.value.trim();
  if (!destination) {
    setStatus(recoverStatus, "Choose a destination directory.", true);
    return;
  }

  recoverButton.disabled = true;
  setStatus(recoverStatus, "Recovering.");
  try {
    const result = await invoke("recover_files", {
      source: sourceInput.value.trim(),
      destination,
      selected: [...selected],
      includeCarving: carvingInput.checked,
    });
    const parts = [`${result.written} written`];
    if (result.partial) parts.push(`${result.partial} partial`);
    if (result.skipped) parts.push(`${result.skipped} skipped`);
    if (result.failed) parts.push(`${result.failed} failed`);
    setStatus(recoverStatus, `${parts.join(", ")}. Manifest: ${result.manifest_path}`);
  } catch (error) {
    setStatus(recoverStatus, `Recovery failed: ${error}`, true);
  } finally {
    updateRecoverButton();
  }
});

updateRecoverButton();
