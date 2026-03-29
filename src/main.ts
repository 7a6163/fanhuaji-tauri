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

// --- State ---

let files: FileEntry[] = [];
let isConverting = false;
let moduleData: ModuleInfo[] = [];
let moduleSettings: Record<string, string> = {};
let activeCategory = "";

// --- DOM ---

const $ = <T extends HTMLElement>(sel: string): T => {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`必要元素不存在：${sel}`);
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

// --- Helpers ---

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
      <span class="file-name" title="${escHtml(`${f.inputPath}/${f.inputName}`)}">${escHtml(f.inputName)}</span>
      <span class="file-message">${f.status === "success" ? "轉換完成" : f.status === "converting" ? (f.chapterTotal ? `轉換中… (${escHtml(String(f.chapterIndex))}/${escHtml(String(f.chapterTotal))} ${escHtml(f.chapterName ?? "")})` : "轉換中…") : escHtml(f.message)}</span>
    </div>`,
    )
    .join("");
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
  void convertPending();
}

async function openFiles() {
  try {
    const selected: string[] = await invoke("open_files_dialog");
    if (selected.length > 0) addFiles(selected);
  } catch (err) {
    console.error("Failed to open files:", err);
  }
}

// --- Convert ---

async function convertPending() {
  if (isConverting) return;

  const converterEl = document.getElementById("converter") as HTMLSelectElement | null;
  const converter = converterEl?.value ?? "Taiwan";

  const saveFolderEl = document.getElementById("save-folder") as HTMLSelectElement | null;
  const namingEl = document.getElementById("naming") as HTMLSelectElement | null;
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
                message: result.warnings ? `轉換完成（${result.warnings}）` : "轉換完成",
                outputName: result.outputName,
                outputPath: result.outputPath,
              }
            : f,
        );
      } catch (err) {
        files = files.map((f) =>
          f.id === file.id ? { ...f, status: "error" as const, message: String(err) } : f,
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
  $<HTMLDivElement>("#settings-backdrop").classList.remove("hidden");
  $<HTMLElement>("#settings-drawer").classList.remove("hidden");
}

function closeSettings() {
  $<HTMLDivElement>("#settings-backdrop").classList.add("hidden");
  $<HTMLElement>("#settings-drawer").classList.add("hidden");
}

$<HTMLButtonElement>("#btn-settings").addEventListener("click", openSettings);
$<HTMLButtonElement>("#btn-close-settings").addEventListener("click", closeSettings);
$<HTMLDivElement>("#settings-backdrop").addEventListener("click", closeSettings);

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
  } catch (err) {
    console.error("Failed to load service info:", err);
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
        <option value="auto"${(moduleSettings[m.name] ?? "auto") === "auto" ? " selected" : ""}>自動</option>
        <option value="enable"${moduleSettings[m.name] === "enable" ? " selected" : ""}>啟用</option>
        <option value="disable"${moduleSettings[m.name] === "disable" ? " selected" : ""}>停用</option>
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

getCurrentWebviewWindow().onDragDropEvent((event) => {
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
    document.title = `繁化姬 ${version}`;
  } catch (err) {
    console.error("Failed to get version:", err);
  }
}

initTheme();
void initVersion();
initUpdater();
void loadServiceInfo();
render();

// Listen for EPUB chapter progress
void listen<EpubProgressPayload>("epub-progress", (event) => {
  const { fileId, chapterIndex, chapterTotal, chapterName } = event.payload;
  files = files.map((f) =>
    f.id === fileId ? { ...f, chapterIndex, chapterTotal, chapterName } : f,
  );
  render();
});
