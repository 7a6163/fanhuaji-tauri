import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { initTheme } from "./theme";
import { initUpdater } from "./updater";

// --- Types ---

interface FileEntry {
  id: string;
  inputPath: string;
  inputName: string;
  encoding: string;
  status: "pending" | "converting" | "success" | "error";
  message: string;
  outputName: string;
  outputPath: string;
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

// --- Utilities ---

function escHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function generateId(): string {
  return crypto.randomUUID();
}

// --- DOM ---

const $ = <T extends HTMLElement>(sel: string): T => {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`必要元素不存在：${sel}`);
  return el;
};

const converterEl = $<HTMLSelectElement>("#converter");
const dictVersionEl = $<HTMLSpanElement>("#dict-version");
const fileTableBody = $<HTMLTableSectionElement>("#file-table-body");
const convertBtn = $<HTMLButtonElement>("#btn-convert-all");

// Counts
const countTotal = $<HTMLSpanElement>("#count-total");
const countPending = $<HTMLSpanElement>("#count-pending");
const countSuccess = $<HTMLSpanElement>("#count-success");
const countError = $<HTMLSpanElement>("#count-error");

// --- UI Feedback ---

function showError(msg: string) {
  const statusBar = document.querySelector(".status-bar");
  if (!statusBar) return;

  const existing = document.getElementById("error-toast");
  if (existing) existing.remove();

  const toast = document.createElement("span");
  toast.id = "error-toast";
  toast.className = "status-badge red";
  toast.textContent = msg;
  statusBar.appendChild(toast);

  setTimeout(() => toast.remove(), 5000);
}

// --- Tabs ---

document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => {
      t.classList.remove("active");
    });
    document.querySelectorAll(".tab-panel").forEach((p) => {
      p.classList.remove("active");
    });
    tab.classList.add("active");
    const panel = document.getElementById(`tab-${tab.dataset.tab}`);
    panel?.classList.add("active");
  });
});

// --- Preview Nav ---

document.querySelectorAll<HTMLButtonElement>(".preview-nav").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".preview-nav").forEach((b) => {
      b.classList.remove("active");
    });
    btn.classList.add("active");
  });
});

// --- File Operations ---

function updateCounts() {
  const total = files.length;
  const pending = files.filter((f) => f.status === "pending").length;
  const success = files.filter((f) => f.status === "success").length;
  const error = files.filter((f) => f.status === "error").length;
  countTotal.textContent = String(total);
  countPending.textContent = String(pending);
  countSuccess.textContent = String(success);
  countError.textContent = String(error);
}

function renderFileTable() {
  fileTableBody.innerHTML = files
    .map(
      (f) => `
    <tr class="status-${escHtml(f.status)}" data-id="${escHtml(f.id)}">
      <td title="${escHtml(f.inputPath)}">${escHtml(f.inputPath)}</td>
      <td>${escHtml(f.inputName)}</td>
      <td>${escHtml(f.encoding)}</td>
      <td>${escHtml(statusLabel(f.status))}</td>
      <td>${escHtml(f.message)}</td>
      <td>${escHtml(f.outputName)}</td>
      <td title="${escHtml(f.outputPath)}">${escHtml(f.outputPath)}</td>
    </tr>`,
    )
    .join("");
  updateCounts();
}

function statusLabel(s: string): string {
  const map: Record<string, string> = {
    pending: "待轉換",
    converting: "轉換中…",
    success: "完成",
    error: "錯誤",
  };
  return map[s] ?? s;
}

// --- Open Files ---

async function openFiles() {
  try {
    const selected: string[] = await invoke("open_files_dialog");
    const newFiles: FileEntry[] = selected.map((path) => {
      const parts = path.replace(/\\/g, "/").split("/");
      const name = parts.pop() ?? "";
      const dir = parts.join("/");
      return {
        id: generateId(),
        inputPath: dir,
        inputName: name,
        encoding: "UTF-8",
        status: "pending",
        message: "",
        outputName: "",
        outputPath: "",
      };
    });
    files = [...files, ...newFiles];
    renderFileTable();
  } catch (err) {
    showError(`開啟檔案失敗：${String(err)}`);
  }
}

// --- Convert All ---

async function convertAll() {
  if (isConverting) return;

  const converter = converterEl.value;
  if (converter.startsWith("_")) return;

  const saveFolderEl = $<HTMLSelectElement>("#save-folder");
  const namingEl = $<HTMLSelectElement>("#naming");
  const preReplace = $<HTMLTextAreaElement>("#pre-replace").value;
  const postReplace = $<HTMLTextAreaElement>("#post-replace").value;
  const protectReplace = $<HTMLTextAreaElement>("#protect-replace").value;

  const pendingFiles = files.filter((f) => f.status === "pending");
  if (pendingFiles.length === 0) return;

  // Build module overrides
  const moduleOverrides: Record<string, number> = {};
  for (const [name, val] of Object.entries(moduleSettings)) {
    if (val === "enable") moduleOverrides[name] = 1;
    else if (val === "disable") moduleOverrides[name] = 0;
  }

  isConverting = true;
  convertBtn.setAttribute("disabled", "true");

  try {
    for (const file of pendingFiles) {
      files = files.map((f) =>
        f.id === file.id ? { ...f, status: "converting" as const, message: "" } : f,
      );
      renderFileTable();

      try {
        const result: { outputName: string; outputPath: string } = await invoke("convert_file", {
          inputPath: `${file.inputPath}/${file.inputName}`,
          converter,
          saveFolder: saveFolderEl.value,
          naming: namingEl.value,
          preReplace,
          postReplace,
          protectReplace,
          modules: JSON.stringify(moduleOverrides),
        });

        files = files.map((f) =>
          f.id === file.id
            ? {
                ...f,
                status: "success" as const,
                message: "轉換完成",
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
      renderFileTable();
    }
  } finally {
    isConverting = false;
    convertBtn.removeAttribute("disabled");
  }
}

// --- Module loading ---

async function loadServiceInfo() {
  try {
    const info: ServiceInfo = await invoke("get_service_info");
    dictVersionEl.textContent = info.dict_version;
    moduleData = info.modules;
    renderModuleCategories();
  } catch (err) {
    showError(`載入服務資訊失敗：${String(err)}`);
  }
}

function renderModuleCategories() {
  const categories = [...new Set(moduleData.map((m) => m.category))];
  const container = $<HTMLDivElement>("#module-categories");
  container.innerHTML = categories
    .map(
      (c, i) =>
        `<div class="module-category${i === 0 ? " active" : ""}" data-category="${escHtml(c)}">${escHtml(c)}</div>`,
    )
    .join("");

  if (categories.length > 0) {
    activeCategory = categories[0];
    renderModuleList();
  }

  container.querySelectorAll<HTMLDivElement>(".module-category").forEach((el) => {
    el.addEventListener("click", () => {
      container.querySelectorAll(".module-category").forEach((c) => {
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
        <option value="auto"${(moduleSettings[m.name] ?? "auto") === "auto" ? " selected" : ""}>自動偵測</option>
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

$<HTMLButtonElement>("#btn-open").addEventListener("click", openFiles);
convertBtn.addEventListener("click", convertAll);

$<HTMLButtonElement>("#btn-remove-all").addEventListener("click", () => {
  files = [];
  renderFileTable();
});

$<HTMLButtonElement>("#btn-remove-done").addEventListener("click", () => {
  files = files.filter((f) => f.status !== "success");
  renderFileTable();
});

$<HTMLButtonElement>("#btn-reset-errors").addEventListener("click", () => {
  files = files.map((f) =>
    f.status === "error" ? { ...f, status: "pending" as const, message: "" } : f,
  );
  renderFileTable();
});

// --- External links ---

document.querySelectorAll<HTMLAnchorElement>("a[data-href]").forEach((a) => {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    const url = a.dataset.href;
    if (url) openUrl(url);
  });
});

// --- Drag & Drop ---

document.addEventListener("dragover", (e) => {
  e.preventDefault();
});

document.addEventListener("drop", async (e) => {
  e.preventDefault();
});

// --- Init ---

initTheme();
initUpdater();
loadServiceInfo();
renderFileTable();
