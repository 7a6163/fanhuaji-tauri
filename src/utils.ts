export function escHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function statusLabel(s: string): string {
  const map: Record<string, string> = {
    pending: "待轉換",
    converting: "轉換中…",
    success: "完成",
    error: "錯誤",
  };
  return map[s] ?? s;
}

export interface FileEntry {
  id: string;
  inputPath: string;
  inputName: string;
  encoding: string;
  status: "pending" | "converting" | "success" | "error";
  message: string;
  outputName: string;
  outputPath: string;
}

export function parseFilePath(path: string): { dir: string; name: string } {
  const parts = path.replace(/\\/g, "/").split("/");
  const name = parts.pop() ?? "";
  const dir = parts.join("/");
  return { dir, name };
}

export function removeCompleted(files: readonly FileEntry[]): FileEntry[] {
  return files.filter((f) => f.status !== "success");
}

export function resetErrors(files: readonly FileEntry[]): FileEntry[] {
  return files.map((f) =>
    f.status === "error" ? { ...f, status: "pending" as const, message: "" } : f,
  );
}

export function countByStatus(files: readonly FileEntry[]) {
  return {
    total: files.length,
    pending: files.filter((f) => f.status === "pending").length,
    success: files.filter((f) => f.status === "success").length,
    error: files.filter((f) => f.status === "error").length,
  };
}

export function buildModuleOverrides(settings: Record<string, string>): Record<string, number> {
  const overrides: Record<string, number> = {};
  for (const [name, val] of Object.entries(settings)) {
    if (val === "enable") overrides[name] = 1;
    else if (val === "disable") overrides[name] = 0;
  }
  return overrides;
}
