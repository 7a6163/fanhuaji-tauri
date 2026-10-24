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
  status: "pending" | "converting" | "success" | "error" | "restored";
  message: string;
  outputName: string;
  outputPath: string;
  /** Copy of the original kept when overwrite mode replaced it; enables undo. */
  backupPath?: string;
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
  if (f.status === "success") {
    if (f.message) return escHtml(f.message);
    if (f.backupPath) return escHtml(t("file.overwritten"));
    return escHtml(
      f.outputName ? t("file.savedAs", { name: f.outputName }) : t("file.convertDone"),
    );
  }
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
  if (f.status === "restored") return escHtml(t("file.restored"));
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

/** Files whose Undo would be lost if they left the list: they overwrote an original. */
export function countUndoable(files: readonly FileEntry[]): number {
  return files.filter((f) => f.status === "success" && f.backupPath).length;
}

/** Drop finished files (converted or restored), keeping pending, running and failed ones. */
export function removeCompleted(files: readonly FileEntry[]): FileEntry[] {
  return files.filter((f) => f.status !== "success" && f.status !== "restored");
}

export function countByStatus(files: readonly FileEntry[]) {
  const counts = { total: files.length, pending: 0, success: 0, warning: 0, error: 0 };
  for (const f of files) {
    if (f.status === "pending") counts.pending++;
    else if (f.status === "success" && f.message) counts.warning++;
    else if (f.status === "success") counts.success++;
    else if (f.status === "error") counts.error++;
  }
  return counts;
}

const ALLOWED_URL_PREFIXES = [
  "https://zhconvert.org",
  "https://docs.zhconvert.org",
  "https://github.com/fanhuaji-desktop/fanhuaji-tauri",
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

export interface DiffSeg {
  text: string;
  changed: boolean;
}

// Middles longer than this skip the character LCS (O(n·m)) and are marked whole.
const INLINE_LCS_LIMIT = 300;

/** Which characters of x and y belong to their longest common subsequence. */
function lcsKept(x: string[], y: string[]): [boolean[], boolean[]] {
  const n = x.length;
  const m = y.length;
  const dp = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const xKept = new Array<boolean>(n).fill(false);
  const yKept = new Array<boolean>(m).fill(false);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      xKept[i++] = true;
      yKept[j++] = true;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
    else j++;
  }
  return [xKept, yKept];
}

/** Group characters into runs of changed / unchanged text. */
function toSegs(chars: string[], changed: (i: number) => boolean): DiffSeg[] {
  const out: DiffSeg[] = [];
  chars.forEach((ch, i) => {
    const c = changed(i);
    const last = out[out.length - 1];
    if (last && last.changed === c) last.text += ch;
    else out.push({ text: ch, changed: c });
  });
  return out;
}

/**
 * Marks what changed between an original line and its conversion, for both sides.
 * The shared prefix and suffix are trimmed. Equal-length middles (the usual one-for-one
 * character swap) are compared position by position; otherwise a character LCS finds
 * what was kept, as long as the middles are short enough. Works on code points.
 */
export function inlineDiff(
  original: string,
  converted: string,
): { orig: DiffSeg[]; conv: DiffSeg[] } {
  const a = Array.from(original);
  const b = Array.from(converted);
  let p = 0;
  while (p < a.length && p < b.length && a[p] === b[p]) p++;
  let s = 0;
  while (s < a.length - p && s < b.length - p && a[a.length - 1 - s] === b[b.length - 1 - s]) s++;
  const pre = a.slice(0, p).join("");
  const suf = a.slice(a.length - s).join("");
  const am = a.slice(p, a.length - s);
  const bm = b.slice(p, b.length - s);

  let origMid: DiffSeg[];
  let convMid: DiffSeg[];
  if (am.length === bm.length) {
    origMid = toSegs(am, (i) => am[i] !== bm[i]);
    convMid = toSegs(bm, (i) => am[i] !== bm[i]);
  } else if (am.length <= INLINE_LCS_LIMIT && bm.length <= INLINE_LCS_LIMIT) {
    const [aKept, bKept] = lcsKept(am, bm);
    origMid = toSegs(am, (i) => !aKept[i]);
    convMid = toSegs(bm, (i) => !bKept[i]);
  } else {
    origMid = am.length ? [{ text: am.join(""), changed: true }] : [];
    convMid = bm.length ? [{ text: bm.join(""), changed: true }] : [];
  }
  const wrap = (mid: DiffSeg[]): DiffSeg[] => [
    ...(pre ? [{ text: pre, changed: false }] : []),
    ...mid,
    ...(suf ? [{ text: suf, changed: false }] : []),
  ];
  return { orig: wrap(origMid), conv: wrap(convMid) };
}
