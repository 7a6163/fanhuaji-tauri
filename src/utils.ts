import { t } from "./i18n/i18n";

export function escHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export interface FileEntry {
  id: string;
  inputPath: string;
  inputName: string;
  status: "pending" | "converting" | "success" | "error";
  message: string;
  outputName: string;
  outputPath: string;
  chapterIndex?: number;
  chapterTotal?: number;
  chapterName?: string;
}

export function fileExt(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** The subtitle line on a file card, already HTML-escaped. */
export function fileMessage(f: FileEntry): string {
  // Success carries a message only when there were warnings (e.g. EPUB_PARTIAL_FAILED).
  if (f.status === "success") return escHtml(f.message || t("file.convertDone"));
  if (f.status === "converting") {
    return f.chapterTotal
      ? escHtml(
          t("file.convertingChapter", {
            current: String(f.chapterIndex),
            total: String(f.chapterTotal),
            name: f.chapterName ? ` ${f.chapterName}` : "",
          }),
        )
      : escHtml(t("file.converting"));
  }
  if (f.status === "error") return escHtml(f.message);
  // pending: show the format name as a neutral subtitle
  return escHtml(fileExt(f.inputName).toUpperCase() || "—");
}

/**
 * Whether a finished batch should start another: files dropped mid-batch stay
 * pending because convertPending() bails while a batch is running.
 */
export function shouldResumeConversion(files: readonly FileEntry[], autoConvert: boolean): boolean {
  return autoConvert && files.some((f) => f.status === "pending");
}

/**
 * Tracks overlapping async requests so only the latest one applies its result:
 * call begin() per request and check the returned isCurrent() after awaiting.
 */
export function createRequestGuard() {
  let latest = 0;
  return {
    begin(): () => boolean {
      const id = ++latest;
      return () => id === latest;
    },
    /** Make every in-flight request stale without starting a new one. */
    invalidate(): void {
      latest++;
    },
  };
}

export function parseFilePath(path: string): { dir: string; name: string } {
  const parts = path.replace(/\\/g, "/").split("/");
  const name = parts.pop() ?? "";
  const dir = parts.join("/");
  return { dir, name };
}

export function resetErrors(files: readonly FileEntry[]): FileEntry[] {
  return files.map((f) =>
    f.status === "error" ? { ...f, status: "pending" as const, message: "" } : f,
  );
}

export function countByStatus(files: readonly FileEntry[]) {
  const counts = { total: files.length, pending: 0, success: 0, error: 0 };
  for (const f of files) {
    if (f.status === "pending") counts.pending++;
    else if (f.status === "success") counts.success++;
    else if (f.status === "error") counts.error++;
  }
  return counts;
}

const ALLOWED_URL_PREFIXES = [
  "https://zhconvert.org",
  "https://docs.zhconvert.org",
  "https://github.com/7a6163/fanhuaji-tauri",
];

export function isSafeUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" && ALLOWED_URL_PREFIXES.some((p) => url.startsWith(p));
  } catch {
    return false;
  }
}

export function isEpubFile(filename: string): boolean {
  return filename.toLowerCase().endsWith(".epub");
}

export function buildModuleOverrides(settings: Record<string, string>): Record<string, number> {
  const overrides: Record<string, number> = {};
  for (const [name, val] of Object.entries(settings)) {
    if (val === "enable") overrides[name] = 1;
    else if (val === "disable") overrides[name] = 0;
  }
  return overrides;
}
