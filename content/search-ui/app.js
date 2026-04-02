const state = {
  indexStatus: null,
  results: [],
  selectedIndex: -1,
  viewerHit: null,
};

const queryInput = document.getElementById("query");
const statusEl = document.getElementById("status");
const resultsEl = document.getElementById("results");
const resultsTitleEl = document.getElementById("results-title");
const resultsMetaEl = document.getElementById("results-meta");
const viewerTitleEl = document.getElementById("viewer-title");
const viewerMetaEl = document.getElementById("viewer-meta");
const viewerEl = document.getElementById("viewer");
const viewerOpenShell = document.getElementById("viewer-open-shell");
const viewerOpenButton = document.getElementById("viewer-open");
const viewerOpenTitleEl = document.getElementById("viewer-open-title");
const viewerOpenNoteEl = document.getElementById("viewer-open-note");
const formEl = document.getElementById("search-form");
const reindexButton = document.getElementById("reindex");
const progressPanelEl = document.getElementById("progress-panel");
const progressTitleEl = document.getElementById("progress-title");
const progressDetailEl = document.getElementById("progress-detail");
const progressTrackEl = document.getElementById("progress-track");
const progressBarEl = document.getElementById("progress-bar");
const progressMetaEl = document.getElementById("progress-meta");
const progressNoteEl = document.getElementById("progress-note");
const reindexStatsTitleEl = document.getElementById("reindex-stats-title");
const reindexStatStateEl = document.getElementById("reindex-stat-state");
const reindexStatFilesEl = document.getElementById("reindex-stat-files");
const reindexStatLinesEl = document.getElementById("reindex-stat-lines");
const reindexStatSkippedEl = document.getElementById("reindex-stat-skipped");
const reindexStatsNoteEl = document.getElementById("reindex-stats-note");

let searchTimer = null;
let statusTimer = null;

boot().catch((error) => {
  showError(error);
});

async function boot() {
  hydrateFromLocation();
  bindEvents();
  renderZedControls();
  await refreshStatus();

  if (hasActiveSearch()) {
    if (state.indexStatus?.ready) {
      await runSearch();
    } else {
      showWaitingForIndex("Indexing workspace. Results will appear as soon as the first pass finishes.");
    }
    return;
  }

  renderResults(emptySearchPayload());
  renderEmptyViewer();
  statusEl.textContent = idleStatusMessage();
}

function bindEvents() {
  formEl.addEventListener("submit", (event) => {
    event.preventDefault();
    runSearch().catch(showError);
  });

  queryInput.addEventListener("input", () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      runSearch().catch(showError);
    }, 180);
  });

  reindexButton.addEventListener("click", async () => {
    statusEl.textContent = "Starting background rebuild…";
    reindexButton.disabled = true;
    try {
      const response = await fetch("/api/reindex", { method: "POST" });
      if (!response.ok) {
        throw new Error(await response.text());
      }
      const payload = await response.json();
      await applyIndexStatus(payload);
    } catch (error) {
      showError(error);
      reindexButton.disabled = state.indexStatus?.indexing ?? false;
      reindexButton.textContent = reindexButton.disabled ? "Re-indexing…" : "Re-index";
    }
  });

  viewerOpenButton.addEventListener("click", () => {
    const hit = state.results[state.selectedIndex];
    if (hit) {
      openInZed(hit).catch(showError);
    }
  });

  window.addEventListener("keydown", (event) => {
    if (!state.results.length) {
      return;
    }
    if (event.target instanceof HTMLInputElement) {
      return;
    }
    if (event.key === "j" || event.key === "ArrowDown") {
      event.preventDefault();
      selectResult(Math.min(state.selectedIndex + 1, state.results.length - 1));
    } else if (event.key === "k" || event.key === "ArrowUp") {
      event.preventDefault();
      selectResult(Math.max(state.selectedIndex - 1, 0));
    }
  });
}

async function refreshStatus() {
  const response = await fetch("/api/status");
  if (!response.ok) {
    throw new Error(await response.text());
  }
  const payload = await response.json();
  await applyIndexStatus(payload);
}

async function applyIndexStatus(payload) {
  const wasReady = state.indexStatus?.ready ?? false;
  const wasIndexing = state.indexStatus?.indexing ?? false;
  state.indexStatus = payload;

  renderIndexStatus(payload);
  renderZedControls();
  reindexButton.disabled = payload.indexing;
  reindexButton.textContent = payload.indexing ? "Re-indexing…" : "Re-index";
  scheduleStatusPoll(payload.indexing);

  if (payload.ready && (!wasReady || (wasIndexing && !payload.indexing)) && hasActiveSearch()) {
    await runSearch();
  } else if (!hasActiveSearch()) {
    statusEl.textContent = idleStatusMessage();
  }
}

function scheduleStatusPoll(indexing) {
  clearTimeout(statusTimer);
  if (!indexing) {
    return;
  }
  statusTimer = setTimeout(() => {
    refreshStatus().catch(showError);
  }, 250);
}

async function runSearch() {
  const params = buildSearchParams();
  syncLocation(params);

  if (![...params.keys()].length) {
    state.results = [];
    state.selectedIndex = -1;
    renderResults(emptySearchPayload());
    renderEmptyViewer();
    statusEl.textContent = idleStatusMessage();
    return;
  }

  if (!state.indexStatus?.ready) {
    showWaitingForIndex("Indexing workspace. Search will run once the first index finishes.");
    return;
  }

  statusEl.textContent = state.indexStatus?.indexing
    ? "Searching the current index while refresh continues…"
    : "Searching workspace…";

  const response = await fetch(`/api/search?${params.toString()}`);
  if (response.status === 409) {
    await refreshStatus();
    showWaitingForIndex("Indexing workspace. Search will run once the first index finishes.");
    return;
  }
  if (!response.ok) {
    throw new Error(await response.text());
  }

  const payload = await response.json();
  state.results = payload.hits;
  state.selectedIndex = payload.hits.length ? 0 : -1;
  renderResults(payload);

  if (state.results.length) {
    await loadFile(state.results[0]);
  } else {
    renderEmptyViewer("No matches for this query.");
  }

  const baseMessage = payload.total_hits
    ? `Found ${payload.total_hits} matching lines.`
    : "No matching lines.";
  statusEl.textContent = state.indexStatus?.indexing
    ? `${baseMessage} Refreshing in background.`
    : baseMessage;
}

function showWaitingForIndex(message) {
  state.results = [];
  state.selectedIndex = -1;
  renderResults(emptySearchPayload());
  renderEmptyViewer(message);
  statusEl.textContent = state.indexStatus?.message || "Preparing workspace index…";
}

function renderResults(payload) {
  resultsTitleEl.textContent = payload.total_hits ? `${payload.total_hits} matches` : "No results";
  resultsMetaEl.textContent = payload.truncated ? "truncated" : "";

  if (!payload.hits.length) {
    resultsEl.innerHTML = "";
    return;
  }

  resultsEl.innerHTML = payload.hits
    .map((hit, index) => {
      const activeClass = index === state.selectedIndex ? " active" : "";
      return `
        <li class="result-card${activeClass}">
          <button class="result-button" type="button" data-index="${index}">
            <div class="result-topline">
              <strong>${escapeHtml(hit.repo_label)}</strong>
              <span class="result-line">${hit.line_number}</span>
            </div>
            <p class="result-path">${escapeHtml(hit.path)}</p>
            <pre class="result-snippet">${escapeHtml(hit.text)}</pre>
          </button>
        </li>
      `;
    })
    .join("");

  for (const button of resultsEl.querySelectorAll(".result-button")) {
    button.addEventListener("click", () => {
      selectResult(Number(button.dataset.index));
    });
  }
}

function selectResult(index) {
  state.selectedIndex = index;
  for (const [cardIndex, node] of [...resultsEl.children].entries()) {
    node.classList.toggle("active", cardIndex === state.selectedIndex);
  }
  const activeCard = resultsEl.children[index];
  if (activeCard) {
    activeCard.scrollIntoView({ block: "nearest" });
  }
  renderZedControls();
  const hit = state.results[state.selectedIndex];
  if (hit) {
    loadFile(hit).catch(showError);
  }
}

async function loadFile(hit) {
  state.viewerHit = hit;
  renderZedControls();
  viewerTitleEl.textContent = hit.path;
  viewerMetaEl.textContent = `${hit.repo_label} · ${hit.language || "plain text"}`;

  const params = new URLSearchParams({ repo: hit.repo, path: hit.path });
  const response = await fetch(`/api/file?${params.toString()}`);
  if (!response.ok) {
    throw new Error(await response.text());
  }
  const payload = await response.json();
  renderFile(payload, hit.line_number);
}

function renderFile(payload, focusLine) {
  const body = payload.highlighted_html ?? escapeHtml(payload.plain_text ?? "");
  const lines = body.split("\n");
  const zed = isZedAvailable();
  const renderedLines = lines
    .map((line, index) => {
      const lineNumber = index + 1;
      const focusAttr = lineNumber === focusLine ? " data-focus" : "";
      const openBtn = zed
        ? `<button class="line-open btn" type="button" data-line="${lineNumber}">Zed</button>`
        : "";
      return `
        <div class="code-row"${focusAttr}>
          <div class="code-line-number">${lineNumber}</div>
          <pre class="code-line">${line || " "}</pre>
          ${openBtn}
        </div>
      `;
    })
    .join("");

  viewerEl.className = "viewer-code";
  viewerEl.innerHTML = `
    <div class="code-frame">
      <div class="code-grid">${renderedLines}</div>
    </div>
  `;

  if (zed) {
    for (const btn of viewerEl.querySelectorAll(".line-open")) {
      btn.addEventListener("click", () => {
        const hit = state.viewerHit;
        if (!hit) return;
        const line = btn.dataset.line;
        openLineInZed(hit.repo, hit.path, line).catch(showError);
      });
    }
  }

  const focusRow = viewerEl.querySelector(".code-row[data-focus]");
  if (focusRow) {
    scrollElementIntoPanel(viewerEl, focusRow);
  } else {
    viewerEl.scrollTo({ top: 0, behavior: "auto" });
  }
}

function scrollElementIntoPanel(panel, element) {
  const panelBox = panel.getBoundingClientRect();
  const elementBox = element.getBoundingClientRect();
  const top = panel.scrollTop + (elementBox.top - panelBox.top) - (panel.clientHeight / 2) + (elementBox.height / 2);
  panel.scrollTo({ top: Math.max(0, top), behavior: "auto" });
}

async function openLineInZed(repo, path, line) {
  const params = new URLSearchParams({ repo, path, line, column: "1" });
  const response = await fetch(`/api/open?${params.toString()}`, { method: "POST" });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  statusEl.textContent = `Opened ${path}:${line} in Zed.`;
}

function renderEmptyViewer(message = "Select a result to open a file.") {
  state.viewerHit = null;
  viewerTitleEl.textContent = "Select a result";
  viewerMetaEl.textContent = "";
  renderZedControls();
  viewerEl.className = "viewer-empty";
  viewerEl.innerHTML = `<p>${escapeHtml(message)}</p>`;
}

async function openInZed(hit) {
  if (!isZedAvailable()) {
    statusEl.textContent = zedControlNote();
    return;
  }
  const params = new URLSearchParams({
    repo: hit.repo,
    path: hit.path,
    line: String(hit.line_number),
    column: "1",
  });
  const response = await fetch(`/api/open?${params.toString()}`, { method: "POST" });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  statusEl.textContent = `Opened ${hit.path} in Zed at line ${hit.line_number}.`;
}

function renderIndexStatus(payload) {
  const shouldShowProgress = payload.indexing || !payload.ready || Boolean(payload.error);
  progressPanelEl.hidden = !shouldShowProgress;
  progressPanelEl.classList.toggle("is-ready", payload.ready && !payload.indexing && payload.phase !== "failed");
  progressPanelEl.classList.toggle("is-failed", payload.phase === "failed");
  progressTitleEl.textContent = titleForStatus(payload);
  progressDetailEl.textContent = detailForStatus(payload);

  const percent = progressPercent(payload);
  if (percent == null) {
    progressTrackEl.classList.add("progress-track-indeterminate");
    progressBarEl.style.width = "0%";
    progressTrackEl.setAttribute("aria-valuenow", "0");
  } else {
    progressTrackEl.classList.remove("progress-track-indeterminate");
    progressBarEl.style.width = `${percent}%`;
    progressTrackEl.setAttribute("aria-valuenow", String(percent));
  }

  progressMetaEl.textContent = metaForStatus(payload);
  progressNoteEl.textContent = noteForStatus(payload);
  renderIndexStats(payload);
}

function renderZedControls() {
  const hasSelection = state.selectedIndex >= 0 && state.selectedIndex < state.results.length;
  const available = isZedAvailable();

  viewerOpenShell.dataset.state = available ? "ready" : "missing";
  viewerOpenButton.disabled = !hasSelection || !available;
  viewerOpenButton.title = zedControlNote();
  viewerOpenTitleEl.textContent = available ? "Open the selected file in Zed" : "Zed CLI unavailable";
  viewerOpenNoteEl.textContent = hasSelection
    ? zedControlNote()
    : `${zedActionSummary()} Select a result first.`;
}

function renderIndexStats(payload) {
  reindexStatsTitleEl.textContent = titleForStatus(payload);
  reindexStatStateEl.textContent = statsStateLabel(payload);
  reindexStatFilesEl.textContent = statsFilesLabel(payload);
  reindexStatLinesEl.textContent = statsLinesLabel(payload);
  reindexStatSkippedEl.textContent = `${formatNumber(payload.skipped_binary)} binary · ${formatNumber(payload.skipped_large)} large`;
  reindexStatsNoteEl.textContent = payload.error || payload.message;
}

function titleForStatus(payload) {
  if (payload.phase === "failed") {
    return payload.ready ? "Refresh failed, previous index still loaded" : "Index build failed";
  }
  if (payload.phase === "building") {
    return payload.ready ? "Refreshing search index" : "Building the first search index";
  }
  if (payload.phase === "scanning") {
    return payload.ready ? "Refreshing workspace snapshot" : "Scanning workspace repositories";
  }
  if (payload.ready) {
    return "Search index ready";
  }
  return "Preparing workspace index";
}

function detailForStatus(payload) {
  if (payload.error) {
    return payload.error;
  }
  return payload.message;
}

function progressPercent(payload) {
  if (payload.phase === "building" && Number.isFinite(payload.files_total) && payload.files_total > 0) {
    return Math.max(2, Math.min(100, Math.round((payload.files_indexed / payload.files_total) * 100)));
  }
  if (!payload.indexing && payload.ready) {
    return 100;
  }
  return null;
}

function metaForStatus(payload) {
  if (payload.phase === "building" && Number.isFinite(payload.files_total)) {
    return `${formatNumber(payload.files_indexed)} / ${formatNumber(payload.files_total)} files indexed · ${formatNumber(payload.lines_indexed)} lines`;
  }
  if (payload.phase === "scanning") {
    return `${formatNumber(payload.files_scanned)} files scanned so far`;
  }
  if (payload.ready) {
    if (payload.last_report && payload.last_report.rebuilt === false) {
      return `${formatNumber(payload.files_indexed)} files tracked`;
    }
    return `${formatNumber(payload.files_indexed)} files · ${formatNumber(payload.lines_indexed)} lines`;
  }
  return "Waiting for the first snapshot…";
}

function noteForStatus(payload) {
  if (payload.indexing && payload.ready) {
    return "Results remain available from the last loaded index while refresh continues.";
  }
  if (payload.skipped_binary || payload.skipped_large) {
    return `${formatNumber(payload.skipped_binary)} binary and ${formatNumber(payload.skipped_large)} oversized files skipped.`;
  }
  return "";
}

function buildSearchParams() {
  const params = new URLSearchParams();
  if (queryInput.value.trim()) params.set("query", queryInput.value.trim());
  return params;
}

function emptySearchPayload() {
  return {
    hits: [],
    total_hits: 0,
    truncated: false,
  };
}

function hasActiveSearch() {
  return Boolean(queryInput.value.trim());
}

function hydrateFromLocation() {
  const params = new URLSearchParams(window.location.search);
  queryInput.value = params.get("query") ?? "";
}

function syncLocation(params) {
  const suffix = params.toString();
  const next = suffix ? `?${suffix}` : window.location.pathname;
  window.history.replaceState({}, "", next);
}

function showError(error) {
  statusEl.textContent = error.message;
  if (state.indexStatus) {
    progressDetailEl.textContent = error.message;
    reindexStatsNoteEl.textContent = error.message;
  }
}

function idleStatusMessage() {
  if (state.indexStatus?.error) {
    return state.indexStatus.error;
  }
  return state.indexStatus?.ready
    ? "Type a query to search the workspace."
    : state.indexStatus?.message || "Preparing workspace index…";
}

function isZedAvailable() {
  return Boolean(state.indexStatus?.zed_available);
}

function zedActionSummary() {
  return "Opens the selected file in Zed, rooted at its repository, and jumps to the matching line without forcing a new workspace.";
}

function zedControlNote() {
  return state.indexStatus?.zed_help ?? zedActionSummary();
}

function statsStateLabel(payload) {
  if (payload.error) {
    return "failed";
  }
  if (payload.indexing) {
    return payload.phase;
  }
  return payload.ready ? "ready" : "starting";
}

function statsFilesLabel(payload) {
  if (payload.phase === "building" && Number.isFinite(payload.files_total)) {
    return `${formatNumber(payload.files_indexed)} / ${formatNumber(payload.files_total)}`;
  }
  if (payload.ready) {
    return formatNumber(payload.files_indexed);
  }
  return formatNumber(payload.files_scanned);
}

function statsLinesLabel(payload) {
  if (payload.last_report && payload.last_report.rebuilt === false && payload.lines_indexed === 0) {
    return "current";
  }
  return formatNumber(payload.lines_indexed);
}

function formatNumber(value) {
  return new Intl.NumberFormat().format(value ?? 0);
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}
