import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource-variable/noto-sans-tc";
import "@tabler/icons-webfont/dist/tabler-icons.min.css";
import "./styles.css";

import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { getLocale, initI18n, type Locale, setLocale, t, translateError } from "./i18n/i18n";
import { initTheme } from "./theme";
import { initUpdater } from "./updater";
import {
  buildModuleOverrides,
  countByStatus,
  createRequestGuard,
  escHtml,
  type FileEntry,
  fileExt,
  fileMessage,
  isEpubFile,
  isSafeUrl,
  parseFilePath,
  resetErrors,
  shouldResumeConversion,
} from "./utils";

interface EpubProgressPayload {
  fileId: string;
  chapterIndex: number;
  chapterTotal: number;
  chapterName: string;
}

interface ServiceInfo {
  modules: ModuleInfo[];
  dict_version: string;
}

interface ModuleInfo {
  key: string;
  name: string;
  description: string;
  category: string;
}

// --- Storage keys ---

const STORAGE_KEYS = {
  converter: "fanhuaji-converter",
  naming: "fanhuaji-naming",
  customSuffix: "fanhuaji-custom-suffix",
  preReplace: "fanhuaji-pre-replace",
  postReplace: "fanhuaji-post-replace",
  protectReplace: "fanhuaji-protect-replace",
  modules: "fanhuaji-modules",
  autoConvert: "fanhuaji-auto-convert",
} as const;

// --- State ---

let files: FileEntry[] = [];
let isConverting = false;
let moduleData: ModuleInfo[] = [];
let moduleSettings: Record<string, string> = JSON.parse(
  localStorage.getItem(STORAGE_KEYS.modules) ?? "{}",
);
let activeCategory = "";

// --- DOM ---

// Init i18n early so t() is available for error messages
initI18n();

const $ = <T extends HTMLElement>(sel: string): T => {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(t("dom.missingElement", { selector: sel }));
  return el;
};

const dropZone = $<HTMLDivElement>("#drop-zone");
const fileItems = $<HTMLDivElement>("#file-items");
const progressBarContainer = $<HTMLDivElement>("#progress-bar-container");
const progressBar = $<HTMLDivElement>("#progress-bar");
const statusBar = $<HTMLElement>("#status-bar");
const countTotal = $<HTMLSpanElement>("#count-total");
const countSuccess = $<HTMLSpanElement>("#count-success");
const countError = $<HTMLSpanElement>("#count-error");
const countWarning = $<HTMLSpanElement>("#count-warning");
const warningStat = $<HTMLSpanElement>("#stat-warning");
const clearBtn = $<HTMLButtonElement>("#btn-clear");
const retryBtn = $<HTMLButtonElement>("#btn-retry");
const convertBtn = $<HTMLButtonElement>("#btn-convert");
const convertCount = $<HTMLSpanElement>("#convert-count");
const statusMode = $<HTMLSpanElement>("#status-mode");
const autoConvertCheckbox = $<HTMLInputElement>("#auto-convert");
const srStatus = $<HTMLDivElement>("#sr-status");

// --- Auto-convert ---

function isAutoConvert(): boolean {
  return localStorage.getItem(STORAGE_KEYS.autoConvert) !== "false";
}

// Restore auto-convert setting
autoConvertCheckbox.checked = isAutoConvert();
autoConvertCheckbox.addEventListener("change", () => {
  localStorage.setItem(STORAGE_KEYS.autoConvert, String(autoConvertCheckbox.checked));
  render();
});

// --- Helpers ---

// Format-coloured badges; classes (srt/ass/vtt/epub) get accent treatment in CSS.
const BADGE_CLASS: Record<string, string> = {
  srt: "srt",
  ass: "ass",
  ssa: "ass",
  vtt: "vtt",
  epub: "epub",
};

function formatBadge(name: string): string {
  const ext = fileExt(name);
  const cls = BADGE_CLASS[ext] ?? "";
  const label = ext ? ext.toUpperCase().slice(0, 4) : "—";
  return `<span class="fmt-badge ${cls}">${escHtml(label)}</span>`;
}

function statusIcon(f: FileEntry): string {
  // A success with a message finished with warnings (e.g. some EPUB chapters failed).
  if (f.status === "success" && f.message)
    return '<span class="status-icon warning"><i aria-hidden="true" class="ti ti-alert-triangle"></i></span>';
  switch (f.status) {
    case "pending":
      return '<span class="status-icon pending"><i aria-hidden="true" class="ti ti-clock"></i></span>';
    case "converting":
      return '<span class="status-icon converting"><i aria-hidden="true" class="ti ti-loader-2"></i></span>';
    case "success":
      return '<span class="status-icon success"><i aria-hidden="true" class="ti ti-check"></i></span>';
    case "error":
      return '<span class="status-icon error"><i aria-hidden="true" class="ti ti-x"></i></span>';
    case "restored":
      return '<span class="status-icon restored"><i aria-hidden="true" class="ti ti-history"></i></span>';
  }
}

// --- Render ---

// Card subtitles are one truncated line; the tooltip carries the full text.
function metaTitle(f: FileEntry): string {
  if (f.status === "error") return f.message;
  if (f.status === "success") return f.message ? `${f.message}\n${f.outputPath}` : f.outputPath;
  return "";
}

function render() {
  const isEmpty = files.length === 0;

  statusBar.classList.toggle("hidden", isEmpty);
  queueMeta.textContent = isEmpty ? "" : t("queue.count", { count: String(files.length) });

  // Drop the preview back to its empty state if the selected file is gone.
  if (selectedId && !files.some((f) => f.id === selectedId)) showEmptyPreview();

  // Show retry button if there are errors
  const hasErrors = files.some((f) => f.status === "error");
  retryBtn.classList.toggle("hidden", !hasErrors);

  // Show convert button when auto-convert is off and there are pending files
  const hasPending = files.some((f) => f.status === "pending");
  convertBtn.classList.toggle("hidden", isAutoConvert() || !hasPending || isConverting);

  // Counts
  const counts = countByStatus(files);
  countTotal.textContent = String(counts.total);
  countSuccess.textContent = String(counts.success);
  countWarning.textContent = String(counts.warning);
  warningStat.classList.toggle("hidden", counts.warning === 0);
  countError.textContent = String(counts.error);
  // Clearing mid-batch would hide files the running loop is still writing.
  clearBtn.disabled = isConverting;
  convertCount.textContent = counts.pending ? `(${counts.pending})` : "";

  // innerHTML replaces the cards, so put keyboard focus back where it was.
  const focused = (document.activeElement as HTMLElement | null)?.closest<HTMLElement>(
    "#file-items [data-id]",
  );
  const focusedId = focused?.dataset.id;

  // File cards
  fileItems.innerHTML = files
    .map(
      (f) => `
    <div class="file-card${f.id === selectedId ? " is-selected" : ""}" role="listitem" data-id="${escHtml(f.id)}">
      <button type="button" class="fc-main"${f.id === selectedId ? ' aria-current="true"' : ""}>
        ${formatBadge(f.inputName)}
        <span class="fc-body">
          <span class="fc-name" title="${escHtml(`${f.inputPath}/${f.inputName}`)}">${escHtml(f.inputName)}</span>
          <span class="fc-meta" title="${escHtml(metaTitle(f))}">${fileMessage(f)}</span>
        </span>
      </button>
      <div class="fc-status">${
        f.status === "success" && f.backupPath
          ? `<button type="button" class="fc-undo" data-undo="${escHtml(f.id)}" title="${escHtml(t("file.undoTitle"))}"><i aria-hidden="true" class="ti ti-arrow-back-up"></i>${escHtml(t("file.undo"))}</button>`
          : ""
      }${
        f.status === "success" && f.outputPath
          ? `<button type="button" class="fc-reveal" data-reveal="${escHtml(f.id)}" title="${escHtml(t("file.reveal"))}" aria-label="${escHtml(t("file.reveal"))}"><i aria-hidden="true" class="ti ti-folder-open"></i></button>`
          : ""
      }${statusIcon(f)}</div>
    </div>`,
    )
    .join("");

  if (focusedId) {
    const card = [...fileItems.querySelectorAll<HTMLElement>(".file-card")].find(
      (c) => c.dataset.id === focusedId,
    );
    card?.querySelector<HTMLElement>(".fc-main")?.focus();
  }
}

/** Screen-reader announcement, e.g. when a batch finishes. */
function announce(message: string): void {
  srStatus.textContent = message;
}

// --- Diff preview ---

const previewPane = $<HTMLElement>("#preview-pane");
const previewEmpty = $<HTMLDivElement>("#preview-empty");
const queueMeta = $<HTMLSpanElement>("#queue-meta");
const previewBadge = $<HTMLSpanElement>("#preview-badge");
const previewName = $<HTMLDivElement>("#preview-name");
const previewStats = $<HTMLDivElement>("#preview-stats");
const previewBody = $<HTMLDivElement>("#preview-body");

let selectedId: string | null = null;
// A slow preview must not overwrite the one for a file selected after it.
const previewGuard = createRequestGuard();

interface PreviewResult {
  original: string;
  converted: string;
  truncated: boolean;
}

// Return the right pane to its empty state and clear the selection.
function showEmptyPreview(): void {
  previewGuard.invalidate();
  selectedId = null;
  previewPane.classList.add("hidden");
  previewEmpty.classList.remove("hidden");
}

// Highlight the differing middle of a changed line via common prefix/suffix.
function lineDiff(original: string, converted: string): { pre: string; mid: string; suf: string } {
  const al = original.length;
  const bl = converted.length;
  let p = 0;
  while (p < al && p < bl && original[p] === converted[p]) p++;
  let s = 0;
  while (s < al - p && s < bl - p && original[al - 1 - s] === converted[bl - 1 - s]) s++;
  return {
    pre: converted.slice(0, p),
    mid: converted.slice(p, bl - s),
    suf: converted.slice(bl - s),
  };
}

type DiffRow =
  | { t: "equal"; conv: string }
  | { t: "replace"; orig: string; conv: string }
  | { t: "insert"; conv: string }
  | { t: "delete"; orig: string };

// Line-level LCS diff. zhconvert prepends a watermark block (a cue "0" +
// "Processed by 繁化姬 … zhconvert.org") to subtitle output; a naive index
// alignment would then mark every following line as changed. LCS treats the
// watermark as an insertion and keeps the rest aligned.
function diffLines(o: string[], c: string[]): DiffRow[] {
  const n = o.length;
  const m = c.length;

  // Guard against pathological inputs (e.g. a file of single-character lines).
  if (n * m > 4_000_000) {
    const max = Math.max(n, m);
    const rows: DiffRow[] = [];
    for (let i = 0; i < max; i++) {
      const a = o[i] ?? "";
      const b = c[i] ?? "";
      rows.push(a === b ? { t: "equal", conv: b } : { t: "replace", orig: a, conv: b });
    }
    return rows;
  }

  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = o[i] === c[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const rows: DiffRow[] = [];
  const dels: string[] = [];
  const inss: string[] = [];
  // Pair a run of deletes with the following run of inserts as char-highlighted
  // replaces; leftovers become pure insert/delete rows.
  const flush = () => {
    const pairs = Math.max(dels.length, inss.length);
    for (let p = 0; p < pairs; p++) {
      const od = dels[p];
      const ic = inss[p];
      if (od !== undefined && ic !== undefined) rows.push({ t: "replace", orig: od, conv: ic });
      else if (ic !== undefined) rows.push({ t: "insert", conv: ic });
      else rows.push({ t: "delete", orig: od });
    }
    dels.length = 0;
    inss.length = 0;
  };

  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (o[i] === c[j]) {
      flush();
      rows.push({ t: "equal", conv: c[j] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      dels.push(o[i++]);
    } else {
      inss.push(c[j++]);
    }
  }
  while (i < n) dels.push(o[i++]);
  while (j < m) inss.push(c[j++]);
  flush();
  return rows;
}

function renderDiff(original: string, converted: string): { html: string; changedLines: number } {
  const rows = diffLines(original.split("\n"), converted.split("\n"));
  const out: string[] = [];
  let changedLines = 0;
  let ln = 0; // converted-side line number for display
  for (const r of rows) {
    if (r.t === "equal") {
      ln++;
      out.push(
        `<div class="diff-line"><span class="diff-num">${ln}</span><div class="diff-conv">${escHtml(r.conv) || "&nbsp;"}</div></div>`,
      );
    } else if (r.t === "replace") {
      ln++;
      changedLines++;
      const d = lineDiff(r.orig, r.conv);
      const conv = `${escHtml(d.pre)}<span class="chg">${escHtml(d.mid)}</span>${escHtml(d.suf)}`;
      out.push(
        `<div class="diff-line changed"><span class="diff-num">${ln}</span><div><div class="diff-orig">${escHtml(r.orig) || "&nbsp;"}</div><div class="diff-conv">${conv}</div></div></div>`,
      );
    } else if (r.t === "insert") {
      ln++;
      changedLines++;
      out.push(
        `<div class="diff-line added"><span class="diff-num">${ln}</span><div class="diff-conv"><span class="chg">${escHtml(r.conv) || "&nbsp;"}</span></div></div>`,
      );
    } else {
      out.push(
        `<div class="diff-line removed"><span class="diff-num">−</span><div class="diff-orig">${escHtml(r.orig) || "&nbsp;"}</div></div>`,
      );
    }
  }
  return { html: out.join(""), changedLines };
}

/** The convert/preview command arguments for a file, from the current settings. */
function readConvertParams(file: FileEntry) {
  const value = (id: string, fallback: string) =>
    (document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null)?.value ?? fallback;
  return {
    fileId: file.id,
    inputPath: `${file.inputPath}/${file.inputName}`,
    converter: value("converter", "Taiwan"),
    saveFolder: value("save-folder", "same"),
    naming: value("naming", "auto"),
    customSuffix: value("custom-suffix", ""),
    preReplace: value("pre-replace", ""),
    postReplace: value("post-replace", ""),
    protectReplace: value("protect-replace", ""),
    modules: JSON.stringify(buildModuleOverrides(moduleSettings)),
  };
}

async function previewFile(file: FileEntry): Promise<void> {
  const isCurrent = previewGuard.begin();
  const ext = fileExt(file.inputName);
  previewBadge.className = `fmt-badge ${BADGE_CLASS[ext] ?? ""}`;
  previewBadge.textContent = ext ? ext.toUpperCase().slice(0, 4) : "—";
  previewName.textContent = file.inputName;
  selectedId = file.id;
  previewEmpty.classList.add("hidden");
  previewPane.classList.remove("hidden");
  render();

  // The conversion already failed: show why in full instead of re-running the API.
  if (file.status === "error") {
    previewStats.textContent = "";
    previewBody.innerHTML = `<div class="preview-error"><i aria-hidden="true" class="ti ti-alert-triangle"></i> ${escHtml(file.message)}</div>`;
    return;
  }

  if (isEpubFile(file.inputName)) {
    previewStats.textContent = "";
    previewBody.innerHTML = `<div class="preview-loading">${escHtml(t("preview.epubUnsupported"))}</div>`;
    return;
  }

  previewStats.textContent = t("preview.converting");
  previewBody.innerHTML = `<div class="preview-loading"><i aria-hidden="true" class="ti ti-loader-2"></i> ${escHtml(t("preview.loading"))}</div>`;

  try {
    const params = readConvertParams(file);
    // After an overwrite the input holds the converted text; diff the saved original instead.
    if (file.backupPath) params.inputPath = file.backupPath;
    const res = await invoke<PreviewResult>("preview_convert", { params });
    if (!isCurrent()) return;
    const diff = renderDiff(res.original, res.converted);
    const chars = [...res.original].length;
    previewStats.textContent = `${t("preview.stats", { chars: String(chars), changed: String(diff.changedLines) })}${res.truncated ? ` · ${t("preview.truncated")}` : ""}`;
    previewBody.innerHTML =
      diff.html || `<div class="preview-loading">${escHtml(t("preview.empty"))}</div>`;
  } catch (err) {
    if (!isCurrent()) return;
    previewStats.textContent = "";
    previewBody.innerHTML = `<div class="preview-error"><i aria-hidden="true" class="ti ti-alert-triangle"></i> ${escHtml(translateError(String(err)))}</div>`;
  }
}

// --- Progress ---

let progressTimeout: ReturnType<typeof setTimeout> | null = null;

function showProgress(percent: number) {
  progressBarContainer.classList.add("visible");
  progressBar.style.width = `${percent}%`;
}

function hideProgress() {
  if (progressTimeout) clearTimeout(progressTimeout);
  showProgress(100);
  progressTimeout = setTimeout(() => {
    progressBarContainer.classList.remove("visible");
    progressBar.style.width = "0%";
    progressTimeout = null;
  }, 500);
}

// --- Add files & auto-convert ---

function addFiles(paths: string[]) {
  const newFiles: FileEntry[] = paths.map((path) => {
    const { dir, name } = parseFilePath(path);
    return {
      id: crypto.randomUUID(),
      inputPath: dir,
      inputName: name,
      status: "pending" as const,
      message: "",
      outputName: "",
      outputPath: "",
    };
  });
  files = [...files, ...newFiles];
  render();
  // Show the diff preview for the first newly-added file so dropping a file
  // always surfaces a preview, regardless of the auto-convert setting.
  if (newFiles.length > 0) void previewFile(newFiles[0]);
  if (isAutoConvert()) {
    void convertPending();
  }
}

async function openFiles() {
  try {
    const selected: string[] = await invoke("open_files_dialog", {
      title: t("dialog.openFiles"),
      supportedLabel: t("dialog.supportedFiles"),
      allLabel: t("dialog.allFiles"),
    });
    if (selected.length > 0) addFiles(selected);
  } catch {
    // Dialog cancelled or failed — no action needed
  }
}

// --- Convert ---

async function convertPending() {
  if (isConverting) return;

  const pendingFiles = files.filter((f) => f.status === "pending");
  if (pendingFiles.length === 0) return;

  const totalPending = pendingFiles.length;
  let completedCount = 0;

  isConverting = true;
  if (progressTimeout) clearTimeout(progressTimeout);
  showProgress(0);

  try {
    for (const file of pendingFiles) {
      files = files.map((f) =>
        f.id === file.id ? { ...f, status: "converting" as const, message: "" } : f,
      );
      render();

      try {
        const params = readConvertParams(file);
        const result: {
          outputName: string;
          outputPath: string;
          warnings?: string;
          backupPath?: string;
        } = await invoke(isEpubFile(file.inputName) ? "convert_epub" : "convert_file", { params });

        files = files.map((f) =>
          f.id === file.id
            ? {
                ...f,
                status: "success" as const,
                message: result.warnings
                  ? t("file.convertDoneWithWarnings", {
                      warnings: translateError(result.warnings),
                    })
                  : "",
                outputName: result.outputName,
                outputPath: result.outputPath,
                backupPath: result.backupPath,
              }
            : f,
        );
      } catch (err) {
        files = files.map((f) =>
          f.id === file.id
            ? { ...f, status: "error" as const, message: translateError(String(err)) }
            : f,
        );
      }

      completedCount++;
      showProgress((completedCount / totalPending) * 100);
      render();
    }
  } finally {
    isConverting = false;
    hideProgress();
    const c = countByStatus(files);
    announce(
      t("status.announce", {
        success: String(c.success),
        warning: String(c.warning),
        error: String(c.error),
      }),
    );
  }

  // Files added mid-batch were skipped by the isConverting guard: pick them up
  // now, or re-render so the convert button reappears when auto-convert is off.
  render();
  if (shouldResumeConversion(files, isAutoConvert())) void convertPending();
}

// --- Settings drawer ---

const settingsDrawer = $<HTMLElement>("#settings-drawer");
const settingsBtn = $<HTMLButtonElement>("#btn-settings");
// aria-modal alone does not stop Tab: make everything behind the drawer inert while it is open.
const behindDrawer = [...document.querySelectorAll<HTMLElement>(".header, .main, .status-bar")];

function openSettings() {
  $<HTMLDivElement>("#settings-backdrop").classList.add("visible");
  settingsDrawer.classList.add("visible");
  settingsDrawer.inert = false;
  for (const el of behindDrawer) el.inert = true;
  settingsDrawer.querySelector<HTMLElement>('.drawer-tab[aria-selected="true"]')?.focus();
}

function closeSettings() {
  if (settingsDrawer.inert) return;
  $<HTMLDivElement>("#settings-backdrop").classList.remove("visible");
  settingsDrawer.classList.remove("visible");
  // inert keeps the off-screen drawer out of the tab order.
  settingsDrawer.inert = true;
  for (const el of behindDrawer) el.inert = false;
  settingsBtn.focus();
}

settingsBtn.addEventListener("click", openSettings);
$<HTMLButtonElement>("#btn-close-settings").addEventListener("click", closeSettings);
$<HTMLDivElement>("#settings-backdrop").addEventListener("click", closeSettings);

// Click a file card to load its diff preview in the right pane
fileItems.addEventListener("click", async (e) => {
  const reveal = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-reveal]");
  if (reveal) {
    const file = files.find((f) => f.id === reveal.dataset.reveal);
    if (!file) return;
    try {
      await revealItemInDir(file.outputPath);
    } catch (err) {
      files = files.map((f) =>
        f.id === file.id
          ? { ...f, status: "error" as const, message: translateError(`REVEAL_FAILED:${err}`) }
          : f,
      );
      render();
    }
    return;
  }
  const undo = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-undo]");
  if (undo) {
    const id = undo.dataset.undo;
    undo.disabled = true;
    try {
      await invoke("restore_backup", { fileId: id });
      files = files.map((f) =>
        f.id === id ? { ...f, status: "restored" as const, message: "", backupPath: undefined } : f,
      );
    } catch (err) {
      files = files.map((f) =>
        f.id === id ? { ...f, status: "error" as const, message: translateError(String(err)) } : f,
      );
    }
    render();
    return;
  }
  const card = (e.target as HTMLElement).closest<HTMLElement>(".file-card");
  if (!card) return;
  const id = card.getAttribute("data-id");
  const file = files.find((f) => f.id === id);
  if (file) void previewFile(file);
});

// Up/Down move between cards; Tab still reaches each card's own buttons.
fileItems.addEventListener("keydown", (e) => {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  const mains = [...fileItems.querySelectorAll<HTMLElement>(".fc-main")];
  const current = (e.target as HTMLElement).closest(".file-card")?.querySelector(".fc-main");
  const i = mains.indexOf(current as HTMLElement);
  if (i < 0) return;
  e.preventDefault();
  mains[Math.max(0, Math.min(mains.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))]?.focus();
});

// Toolbar mode segmented control — drives the same converter as Settings.
const modeSeg = $<HTMLDivElement>("#mode-seg");
const converterSelect = $<HTMLSelectElement>("#converter");
function syncModeSeg(): void {
  for (const b of modeSeg.querySelectorAll<HTMLButtonElement>("button[data-converter]")) {
    b.setAttribute("aria-pressed", String(b.dataset.converter === converterSelect.value));
  }
}
modeSeg.addEventListener("click", (e) => {
  const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-converter]");
  if (!btn) return;
  converterSelect.value = btn.dataset.converter ?? "Taiwan";
  converterSelect.dispatchEvent(new Event("change"));
});
// Keep the segmented state in sync and re-preview when the converter changes
// (whether from the segmented control or the Settings drawer).
function syncStatusMode(): void {
  statusMode.textContent = converterSelect.selectedOptions[0]?.text ?? "";
}
converterSelect.addEventListener("change", () => {
  syncModeSeg();
  syncStatusMode();
  if (selectedId) {
    const f = files.find((x) => x.id === selectedId);
    if (f) void previewFile(f);
  }
});
syncModeSeg();
syncStatusMode();

// Toolbar output-naming pill — custom popover bound to the Settings naming select.
const namingSelect = $<HTMLSelectElement>("#naming");
const outPillBtn = $<HTMLButtonElement>("#out-pill-btn");
const outPillLabel = $<HTMLSpanElement>("#out-pill-label");
const outPop = $<HTMLDivElement>("#out-pop");

function syncOutPill(): void {
  for (const opt of outPop.querySelectorAll<HTMLButtonElement>(".out-opt")) {
    const on = opt.dataset.naming === namingSelect.value;
    opt.setAttribute("aria-checked", String(on));
    if (on) outPillLabel.textContent = opt.querySelector(".oo-title")?.textContent ?? "";
  }
  // The label is hidden in narrow windows; keep the button's name and tooltip.
  outPillBtn.setAttribute("aria-label", outPillLabel.textContent ?? "");
  outPillBtn.title = outPillLabel.textContent ?? "";
  outPillBtn.classList.toggle("is-overwrite", namingSelect.value === "overwrite");
}
function closeOutPop(): void {
  outPop.classList.add("hidden");
  outPillBtn.setAttribute("aria-expanded", "false");
}
outPillBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  const nowHidden = outPop.classList.toggle("hidden");
  outPillBtn.setAttribute("aria-expanded", String(!nowHidden));
  if (!nowHidden) outPop.querySelector<HTMLElement>('.out-opt[aria-checked="true"]')?.focus();
});
outPop.addEventListener("keydown", (e) => {
  const opts = [...outPop.querySelectorAll<HTMLElement>(".out-opt")];
  const i = opts.indexOf(document.activeElement as HTMLElement);
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const step = e.key === "ArrowDown" ? 1 : -1;
    opts[(i + step + opts.length) % opts.length]?.focus();
  } else if (e.key === "Escape" || e.key === "Tab") {
    closeOutPop();
    if (e.key === "Escape") outPillBtn.focus();
  }
});
outPop.addEventListener("click", (e) => {
  const opt = (e.target as HTMLElement).closest<HTMLButtonElement>(".out-opt");
  if (!opt) return;
  namingSelect.value = opt.dataset.naming ?? "auto";
  namingSelect.dispatchEvent(new Event("change"));
  closeOutPop();
});
document.addEventListener("click", (e) => {
  if (
    !outPop.classList.contains("hidden") &&
    !(e.target as HTMLElement).closest(".out-pill-wrap")
  ) {
    closeOutPop();
  }
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeOutPop();
});
namingSelect.addEventListener("change", syncOutPill);
syncOutPill();
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeSettings();
});

// Shortcuts: Cmd/Ctrl+O add files, Cmd/Ctrl+, settings, Cmd/Ctrl+Enter convert.
document.addEventListener("keydown", (e) => {
  if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
  if (e.key === "o") {
    e.preventDefault();
    void openFiles();
  } else if (e.key === ",") {
    e.preventDefault();
    openSettings();
  } else if (e.key === "Enter" && !convertBtn.classList.contains("hidden")) {
    e.preventDefault();
    void convertPending();
  }
});

// --- Restore persisted settings ---

function restoreSetting(id: string, key: string) {
  const el = document.getElementById(id) as HTMLSelectElement | HTMLTextAreaElement | null;
  const saved = localStorage.getItem(key);
  if (el && saved) el.value = saved;
}

function persistOnChange(id: string, key: string) {
  const el = document.getElementById(id) as HTMLSelectElement | HTMLTextAreaElement | null;
  el?.addEventListener("change", () => {
    localStorage.setItem(key, el.value);
  });
  // For textareas, also persist on input (debounced would be better but change is fine)
  if (el instanceof HTMLTextAreaElement) {
    el.addEventListener("input", () => {
      localStorage.setItem(key, el.value);
    });
  }
}

restoreSetting("converter", STORAGE_KEYS.converter);
restoreSetting("naming", STORAGE_KEYS.naming);
restoreSetting("pre-replace", STORAGE_KEYS.preReplace);
restoreSetting("post-replace", STORAGE_KEYS.postReplace);
restoreSetting("protect-replace", STORAGE_KEYS.protectReplace);

// Refresh the toolbar output pill after restoring the naming select.
syncOutPill();

persistOnChange("converter", STORAGE_KEYS.converter);
persistOnChange("naming", STORAGE_KEYS.naming);
persistOnChange("pre-replace", STORAGE_KEYS.preReplace);

// Custom suffix input — show/hide based on naming selection
const namingEl = document.getElementById("naming") as HTMLSelectElement | null;
const suffixWrapper = document.getElementById("suffix-input-wrapper");
const customSuffixInput = document.getElementById("custom-suffix") as HTMLInputElement | null;

function updateSuffixVisibility() {
  suffixWrapper?.classList.toggle("hidden", namingEl?.value !== "suffix");
}

// Restore custom suffix
const savedSuffix = localStorage.getItem(STORAGE_KEYS.customSuffix);
if (customSuffixInput && savedSuffix) customSuffixInput.value = savedSuffix;

customSuffixInput?.addEventListener("input", () => {
  localStorage.setItem(STORAGE_KEYS.customSuffix, customSuffixInput.value);
});

namingEl?.addEventListener("change", updateSuffixVisibility);
updateSuffixVisibility();
persistOnChange("post-replace", STORAGE_KEYS.postReplace);
persistOnChange("protect-replace", STORAGE_KEYS.protectReplace);

// Custom save folder picker
const SAVE_FOLDER_KEY = "fanhuaji-save-folder";
const saveFolderSelect = document.getElementById("save-folder") as HTMLSelectElement | null;

function setCustomFolder(folder: string) {
  if (!saveFolderSelect) return;
  let customOpt = saveFolderSelect.querySelector<HTMLOptionElement>("option[data-custom-path]");
  if (!customOpt) {
    customOpt = document.createElement("option");
    customOpt.setAttribute("data-custom-path", "true");
    saveFolderSelect.insertBefore(customOpt, saveFolderSelect.lastElementChild);
  }
  customOpt.value = folder;
  customOpt.textContent = folder;
  saveFolderSelect.value = folder;
  localStorage.setItem(SAVE_FOLDER_KEY, folder);
}

// Restore saved custom folder
const savedFolder = localStorage.getItem(SAVE_FOLDER_KEY);
if (savedFolder && saveFolderSelect) {
  setCustomFolder(savedFolder);
}

saveFolderSelect?.addEventListener("change", async () => {
  if (saveFolderSelect.value === "custom") {
    const folder: string | null = await invoke("pick_save_folder", {
      title: t("dialog.pickFolder"),
    });
    if (folder) {
      setCustomFolder(folder);
    } else {
      // Cancelled — revert to previous
      const prev = localStorage.getItem(SAVE_FOLDER_KEY);
      saveFolderSelect.value = prev ?? "same";
    }
  } else if (saveFolderSelect.value === "same") {
    localStorage.removeItem(SAVE_FOLDER_KEY);
  }
});

// Drawer tabs
const drawerTabs = [...document.querySelectorAll<HTMLButtonElement>(".drawer-tab")];
function selectDrawerTab(tab: HTMLButtonElement): void {
  for (const t of drawerTabs) {
    const on = t === tab;
    t.classList.toggle("active", on);
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
  }
  document.querySelectorAll(".drawer-panel").forEach((p) => {
    p.classList.remove("active");
  });
  document.querySelector(`[data-panel="${tab.dataset.drawerTab}"]`)?.classList.add("active");
}
for (const tab of drawerTabs) {
  tab.addEventListener("click", () => selectDrawerTab(tab));
  tab.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    const step = e.key === "ArrowRight" ? 1 : -1;
    const next =
      drawerTabs[(drawerTabs.indexOf(tab) + step + drawerTabs.length) % drawerTabs.length];
    selectDrawerTab(next);
    next.focus();
  });
}

// --- Module loading ---

async function loadServiceInfo() {
  try {
    const info: ServiceInfo = await invoke("get_service_info");
    moduleData = info.modules;
    migrateModuleSettings();
    renderModuleCategories();
  } catch {
    // Service info unavailable — modules panel will be empty
  }
}

// Remap any legacy display-name keys to internal API keys.
function migrateModuleSettings() {
  const nameToKey = new Map(moduleData.map((m) => [m.name, m.key]));
  const migrated: Record<string, string> = {};
  let changed = false;
  for (const [k, v] of Object.entries(moduleSettings)) {
    const newKey = nameToKey.get(k);
    if (newKey && newKey !== k) {
      migrated[newKey] = v;
      changed = true;
    } else {
      migrated[k] = v;
    }
  }
  if (changed) {
    moduleSettings = migrated;
    localStorage.setItem(STORAGE_KEYS.modules, JSON.stringify(moduleSettings));
  }
}

function renderModuleCategories() {
  const categories = [...new Set(moduleData.map((m) => m.category))];
  const container = $<HTMLDivElement>("#module-categories");
  container.innerHTML = categories
    .map(
      (c, i) =>
        `<button type="button" class="module-cat-btn${i === 0 ? " active" : ""}" data-category="${escHtml(c)}"${c ? "" : ' data-i18n="module.categoryUnknown"'}>${escHtml(c || t("module.categoryUnknown"))}</button>`,
    )
    .join("");

  if (categories.length > 0) {
    activeCategory = categories[0];
    renderModuleList();
  }

  container.querySelectorAll<HTMLButtonElement>(".module-cat-btn").forEach((el) => {
    el.addEventListener("click", () => {
      container.querySelectorAll(".module-cat-btn").forEach((c) => {
        c.classList.remove("active");
      });
      el.classList.add("active");
      activeCategory = el.dataset.category ?? "";
      renderModuleList();
    });
  });
}

function renderModuleList() {
  const container = $<HTMLDivElement>("#module-list");
  const filtered = moduleData.filter((m) => m.category === activeCategory);
  container.innerHTML = filtered
    .map(
      (m) => `
    <div class="module-item">
      <select data-module="${escHtml(m.key)}">
        <option value="auto"${(moduleSettings[m.key] ?? "auto") === "auto" ? " selected" : ""}>${escHtml(t("module.auto"))}</option>
        <option value="enable"${moduleSettings[m.key] === "enable" ? " selected" : ""}>${escHtml(t("module.enable"))}</option>
        <option value="disable"${moduleSettings[m.key] === "disable" ? " selected" : ""}>${escHtml(t("module.disable"))}</option>
      </select>
      <span class="module-name">${escHtml(m.name)}</span>
      <span class="module-desc">${escHtml(m.description)}</span>
    </div>`,
    )
    .join("");

  container.querySelectorAll<HTMLSelectElement>("select[data-module]").forEach((sel) => {
    sel.addEventListener("change", () => {
      const name = sel.dataset.module ?? "";
      moduleSettings = { ...moduleSettings, [name]: sel.value };
      localStorage.setItem(STORAGE_KEYS.modules, JSON.stringify(moduleSettings));
    });
  });
}

// --- Button handlers ---

dropZone.addEventListener("click", openFiles);
dropZone.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    openFiles();
  }
});

$<HTMLButtonElement>("#btn-add-more").addEventListener("click", openFiles);

clearBtn.addEventListener("click", () => {
  files = [];
  render();
});

convertBtn.addEventListener("click", () => {
  void convertPending();
});

retryBtn.addEventListener("click", () => {
  files = resetErrors(files);
  render();
  void convertPending();
});

// --- External links ---

document.querySelectorAll<HTMLAnchorElement>("a[data-href]").forEach((a) => {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    const url = a.dataset.href;
    if (url && isSafeUrl(url)) openUrl(url);
  });
});

// --- Drag & Drop ---

void getCurrentWebviewWindow().onDragDropEvent((event) => {
  const { type } = event.payload;
  if (type === "enter" || type === "over") {
    dropZone.classList.add("drag-over");
  } else if (type === "drop") {
    dropZone.classList.remove("drag-over");
    if ("paths" in event.payload) {
      addFiles(event.payload.paths);
    }
  } else if (type === "leave") {
    dropZone.classList.remove("drag-over");
  }
});

// --- Init ---

async function initVersion() {
  try {
    const version = await getVersion();
    const el = document.getElementById("app-version");
    if (el) el.textContent = version;
    const title = `${t("app.title")} ${version}`;
    document.title = title;
    // Keep the native title bar in the current UI language too.
    await getCurrentWebviewWindow().setTitle(title);
  } catch {
    // Version unavailable — title stays as default
  }
}

initTheme();
// Tooltips and the drop zone are where users can discover the shortcuts.
const IS_MAC = /Mac/.test(navigator.userAgent);
const SHORTCUT_HINTS: [selector: string, key: string, label: string][] = [
  ["#btn-add-more", "O", "files.addMore"],
  ["#btn-settings", ",", "header.settings"],
  ["#btn-convert", "Enter", "status.convert"],
];
function shortcutLabel(key: string): string {
  return IS_MAC ? `⌘${key === "Enter" ? "↩" : key}` : `Ctrl+${key}`;
}
function applyShortcutHints(): void {
  for (const [sel, key, label] of SHORTCUT_HINTS) {
    const el = $<HTMLElement>(sel);
    el.title = `${t(label)} (${shortcutLabel(key)})`;
    el.setAttribute("aria-keyshortcuts", `${IS_MAC ? "Meta" : "Control"}+${key}`);
  }
  $<HTMLSpanElement>("#drop-shortcut").textContent = t("drop.shortcut", {
    key: shortcutLabel("O"),
  });
}
applyShortcutHints();

void initVersion();
initUpdater();
void loadServiceInfo();
render();

// --- Language selector ---

const localeSelect = document.getElementById("locale-select") as HTMLSelectElement | null;
if (localeSelect) {
  localeSelect.value = getLocale();
  localeSelect.addEventListener("change", () => {
    setLocale(localeSelect.value as Locale);
    applyShortcutHints();
    render();
    renderModuleList();
    syncOutPill();
    void initVersion();
  });
}

// Listen for EPUB chapter progress
void listen<EpubProgressPayload>("epub-progress", (event) => {
  const { fileId, chapterIndex, chapterTotal, chapterName } = event.payload;
  files = files.map((f) =>
    f.id === fileId ? { ...f, chapterIndex, chapterTotal, chapterName } : f,
  );
  render();
});
