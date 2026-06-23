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
import { openUrl } from "@tauri-apps/plugin-opener";
import { getLocale, initI18n, type Locale, setLocale, t, translateError } from "./i18n/i18n";
import { initTheme } from "./theme";
import { initUpdater } from "./updater";
import {
  buildModuleOverrides,
  countByStatus,
  escHtml,
  type FileEntry,
  isEpubFile,
  isSafeUrl,
  parseFilePath,
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
const fileList = $<HTMLDivElement>("#file-list");
const fileItems = $<HTMLDivElement>("#file-items");
const progressBarContainer = $<HTMLDivElement>("#progress-bar-container");
const progressBar = $<HTMLDivElement>("#progress-bar");
const statusBar = $<HTMLElement>("#status-bar");
const countTotal = $<HTMLSpanElement>("#count-total");
const countSuccess = $<HTMLSpanElement>("#count-success");
const countError = $<HTMLSpanElement>("#count-error");
const retryBtn = $<HTMLButtonElement>("#btn-retry");
const convertBtn = $<HTMLButtonElement>("#btn-convert");
const autoConvertCheckbox = $<HTMLInputElement>("#auto-convert");

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

function fileExt(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function formatBadge(name: string): string {
  const ext = fileExt(name);
  const cls = BADGE_CLASS[ext] ?? "";
  const label = ext ? ext.toUpperCase().slice(0, 4) : "—";
  return `<span class="fmt-badge ${cls}">${escHtml(label)}</span>`;
}

function statusIcon(status: FileEntry["status"]): string {
  switch (status) {
    case "pending":
      return '<span class="status-icon pending"><i class="ti ti-clock"></i></span>';
    case "converting":
      return '<span class="status-icon converting"><i class="ti ti-loader-2"></i></span>';
    case "success":
      return '<span class="status-icon success"><i class="ti ti-check"></i></span>';
    case "error":
      return '<span class="status-icon error"><i class="ti ti-x"></i></span>';
  }
}

// --- Render ---

function render() {
  const isEmpty = files.length === 0;

  dropZone.classList.toggle("hidden", !isEmpty);
  fileList.classList.toggle("hidden", isEmpty);
  statusBar.classList.toggle("hidden", isEmpty);

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
  countError.textContent = String(counts.error);

  // File items
  fileItems.innerHTML = files
    .map(
      (f) => `
    <div class="file-item" data-id="${escHtml(f.id)}">
      ${statusIcon(f.status)}
      ${formatBadge(f.inputName)}
      <span class="file-name" title="${escHtml(`${f.inputPath}/${f.inputName}`)}">${escHtml(f.inputName)}</span>
      <span class="file-message">${f.status === "success" ? escHtml(t("file.convertDone")) : f.status === "converting" ? (f.chapterTotal ? escHtml(t("file.convertingChapter", { current: String(f.chapterIndex), total: String(f.chapterTotal), name: f.chapterName ?? "" })) : escHtml(t("file.converting"))) : escHtml(f.message)}</span>
    </div>`,
    )
    .join("");
}

// --- Diff preview ---

const previewBackdrop = $<HTMLDivElement>("#preview-backdrop");
const previewPanel = $<HTMLElement>("#preview-panel");
const previewBadge = $<HTMLSpanElement>("#preview-badge");
const previewName = $<HTMLDivElement>("#preview-name");
const previewStats = $<HTMLDivElement>("#preview-stats");
const previewBody = $<HTMLDivElement>("#preview-body");

interface PreviewResult {
  original: string;
  converted: string;
  truncated: boolean;
}

function closePreview(): void {
  previewPanel.classList.remove("visible");
  previewBackdrop.classList.remove("visible");
  previewPanel.setAttribute("aria-hidden", "true");
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

function renderDiff(original: string, converted: string): { html: string; changedLines: number } {
  const oLines = original.split("\n");
  const cLines = converted.split("\n");
  const max = Math.max(oLines.length, cLines.length);
  const rows: string[] = [];
  let changedLines = 0;
  for (let i = 0; i < max; i++) {
    const o = oLines[i] ?? "";
    const c = cLines[i] ?? "";
    const num = `<span class="diff-num">${i + 1}</span>`;
    if (o === c) {
      rows.push(
        `<div class="diff-line">${num}<div class="diff-conv">${escHtml(c) || "&nbsp;"}</div></div>`,
      );
      continue;
    }
    changedLines++;
    const d = lineDiff(o, c);
    const conv = `${escHtml(d.pre)}<span class="chg">${escHtml(d.mid)}</span>${escHtml(d.suf)}`;
    rows.push(
      `<div class="diff-line changed">${num}<div><div class="diff-orig">${escHtml(o) || "&nbsp;"}</div><div class="diff-conv">${conv}</div></div></div>`,
    );
  }
  return { html: rows.join(""), changedLines };
}

async function previewFile(file: FileEntry): Promise<void> {
  const ext = fileExt(file.inputName);
  previewBadge.className = `fmt-badge ${BADGE_CLASS[ext] ?? ""}`;
  previewBadge.textContent = ext ? ext.toUpperCase().slice(0, 4) : "—";
  previewName.textContent = file.inputName;
  previewBackdrop.classList.add("visible");
  previewPanel.classList.add("visible");
  previewPanel.setAttribute("aria-hidden", "false");

  if (isEpubFile(file.inputName)) {
    previewStats.textContent = "";
    previewBody.innerHTML = `<div class="preview-loading">${escHtml(t("preview.epubUnsupported"))}</div>`;
    return;
  }

  previewStats.textContent = t("preview.converting");
  previewBody.innerHTML = `<div class="preview-loading"><i class="ti ti-loader-2"></i> ${escHtml(t("preview.loading"))}</div>`;

  try {
    const converter =
      (document.getElementById("converter") as HTMLSelectElement | null)?.value ?? "Taiwan";
    const params = {
      inputPath: `${file.inputPath}/${file.inputName}`,
      converter,
      saveFolder:
        (document.getElementById("save-folder") as HTMLSelectElement | null)?.value ?? "same",
      naming: (document.getElementById("naming") as HTMLSelectElement | null)?.value ?? "auto",
      customSuffix:
        (document.getElementById("custom-suffix") as HTMLInputElement | null)?.value ?? "",
      preReplace:
        (document.getElementById("pre-replace") as HTMLTextAreaElement | null)?.value ?? "",
      postReplace:
        (document.getElementById("post-replace") as HTMLTextAreaElement | null)?.value ?? "",
      protectReplace:
        (document.getElementById("protect-replace") as HTMLTextAreaElement | null)?.value ?? "",
      modules: JSON.stringify(buildModuleOverrides(moduleSettings)),
    };
    const res = await invoke<PreviewResult>("preview_convert", { params });
    const diff = renderDiff(res.original, res.converted);
    const chars = [...res.original].length;
    previewStats.textContent = `${t("preview.stats", { chars: String(chars), changed: String(diff.changedLines) })}${res.truncated ? ` · ${t("preview.truncated")}` : ""}`;
    previewBody.innerHTML =
      diff.html || `<div class="preview-loading">${escHtml(t("preview.empty"))}</div>`;
  } catch (err) {
    previewStats.textContent = "";
    previewBody.innerHTML = `<div class="preview-error"><i class="ti ti-alert-triangle"></i> ${escHtml(translateError(String(err)))}</div>`;
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
      encoding: "UTF-8",
      status: "pending" as const,
      message: "",
      outputName: "",
      outputPath: "",
    };
  });
  files = [...files, ...newFiles];
  render();
  if (isAutoConvert()) {
    void convertPending();
  }
}

async function openFiles() {
  try {
    const selected: string[] = await invoke("open_files_dialog");
    if (selected.length > 0) addFiles(selected);
  } catch {
    // Dialog cancelled or failed — no action needed
  }
}

// --- Convert ---

async function convertPending() {
  if (isConverting) return;

  const converterEl = document.getElementById("converter") as HTMLSelectElement | null;
  const converter = converterEl?.value ?? "Taiwan";

  const saveFolderEl = document.getElementById("save-folder") as HTMLSelectElement | null;
  const preReplace =
    (document.getElementById("pre-replace") as HTMLTextAreaElement | null)?.value ?? "";
  const postReplace =
    (document.getElementById("post-replace") as HTMLTextAreaElement | null)?.value ?? "";
  const protectReplace =
    (document.getElementById("protect-replace") as HTMLTextAreaElement | null)?.value ?? "";

  const pendingFiles = files.filter((f) => f.status === "pending");
  if (pendingFiles.length === 0) return;

  const moduleOverrides = buildModuleOverrides(moduleSettings);
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
        const fullPath = `${file.inputPath}/${file.inputName}`;
        const commonParams = {
          inputPath: fullPath,
          converter,
          saveFolder: saveFolderEl?.value ?? "same",
          naming: namingEl?.value ?? "auto",
          customSuffix:
            (document.getElementById("custom-suffix") as HTMLInputElement | null)?.value ?? "",
          preReplace,
          postReplace,
          protectReplace,
          modules: JSON.stringify(moduleOverrides),
        };

        const result: { outputName: string; outputPath: string; warnings?: string } = isEpubFile(
          file.inputName,
        )
          ? await invoke("convert_epub", { params: { ...commonParams, fileId: file.id } })
          : await invoke("convert_file", { params: commonParams });

        files = files.map((f) =>
          f.id === file.id
            ? {
                ...f,
                status: "success" as const,
                message: result.warnings
                  ? t("file.convertDoneWithWarnings", {
                      warnings: translateError(result.warnings),
                    })
                  : t("file.convertDone"),
                outputName: result.outputName,
                outputPath: result.outputPath,
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
  }
}

// --- Settings drawer ---

function openSettings() {
  $<HTMLDivElement>("#settings-backdrop").classList.add("visible");
  $<HTMLElement>("#settings-drawer").classList.add("visible");
}

function closeSettings() {
  $<HTMLDivElement>("#settings-backdrop").classList.remove("visible");
  $<HTMLElement>("#settings-drawer").classList.remove("visible");
}

$<HTMLButtonElement>("#btn-settings").addEventListener("click", openSettings);
$<HTMLButtonElement>("#btn-close-settings").addEventListener("click", closeSettings);
$<HTMLDivElement>("#settings-backdrop").addEventListener("click", closeSettings);

// Click a file row to open its diff preview
fileItems.addEventListener("click", (e) => {
  const row = (e.target as HTMLElement).closest<HTMLElement>(".file-item");
  if (!row) return;
  const id = row.getAttribute("data-id");
  const file = files.find((f) => f.id === id);
  if (file) void previewFile(file);
});
$<HTMLButtonElement>("#btn-close-preview").addEventListener("click", closePreview);
$<HTMLDivElement>("#preview-backdrop").addEventListener("click", closePreview);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && previewPanel.classList.contains("visible")) closePreview();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeSettings();
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
    const folder: string | null = await invoke("pick_save_folder");
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
document.querySelectorAll<HTMLButtonElement>(".drawer-tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".drawer-tab").forEach((t) => {
      t.classList.remove("active");
    });
    document.querySelectorAll(".drawer-panel").forEach((p) => {
      p.classList.remove("active");
    });
    tab.classList.add("active");
    const panel = document.querySelector(`[data-panel="${tab.dataset.drawerTab}"]`);
    panel?.classList.add("active");
  });
});

// --- Module loading ---

async function loadServiceInfo() {
  try {
    const info: ServiceInfo = await invoke("get_service_info");
    moduleData = info.modules;
    renderModuleCategories();
  } catch {
    // Service info unavailable — modules panel will be empty
  }
}

function renderModuleCategories() {
  const categories = [...new Set(moduleData.map((m) => m.category))];
  const container = $<HTMLDivElement>("#module-categories");
  container.innerHTML = categories
    .map(
      (c, i) =>
        `<button type="button" class="module-cat-btn${i === 0 ? " active" : ""}" data-category="${escHtml(c)}">${escHtml(c)}</button>`,
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
      <select data-module="${escHtml(m.name)}">
        <option value="auto"${(moduleSettings[m.name] ?? "auto") === "auto" ? " selected" : ""}>${escHtml(t("module.auto"))}</option>
        <option value="enable"${moduleSettings[m.name] === "enable" ? " selected" : ""}>${escHtml(t("module.enable"))}</option>
        <option value="disable"${moduleSettings[m.name] === "disable" ? " selected" : ""}>${escHtml(t("module.disable"))}</option>
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

$<HTMLButtonElement>("#btn-clear").addEventListener("click", () => {
  files = [];
  render();
});

convertBtn.addEventListener("click", () => {
  void convertPending();
});

retryBtn.addEventListener("click", () => {
  files = files.map((f) =>
    f.status === "error" ? { ...f, status: "pending" as const, message: "" } : f,
  );
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
    document.title = `${t("app.title")} ${version}`;
  } catch {
    // Version unavailable — title stays as default
  }
}

initTheme();
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
    render();
    renderModuleList();
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
