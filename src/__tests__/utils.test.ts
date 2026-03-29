import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  activateTab,
  buildModuleOverrides,
  countByStatus,
  escHtml,
  type FileEntry,
  isSafeUrl,
  parseFilePath,
  removeCompleted,
  resetErrors,
  statusLabel,
} from "../utils";

// --- escHtml ---

describe("escHtml", () => {
  it("escapes ampersands", () => {
    expect(escHtml("a&b")).toBe("a&amp;b");
  });

  it("escapes double quotes", () => {
    expect(escHtml('a"b')).toBe("a&quot;b");
  });

  it("escapes less-than and greater-than", () => {
    expect(escHtml("<script>")).toBe("&lt;script&gt;");
  });

  it("handles all special characters together", () => {
    expect(escHtml('<div class="a&b">')).toBe("&lt;div class=&quot;a&amp;b&quot;&gt;");
  });

  it("returns empty string unchanged", () => {
    expect(escHtml("")).toBe("");
  });

  it("returns plain text unchanged", () => {
    expect(escHtml("hello world")).toBe("hello world");
  });

  it("handles Chinese characters", () => {
    expect(escHtml("繁化姬")).toBe("繁化姬");
  });

  it("escapes single quotes", () => {
    expect(escHtml("it's")).toBe("it&#39;s");
  });
});

// --- statusLabel ---

describe("statusLabel", () => {
  it("returns 待轉換 for pending", () => {
    expect(statusLabel("pending")).toBe("待轉換");
  });

  it("returns 轉換中… for converting", () => {
    expect(statusLabel("converting")).toBe("轉換中…");
  });

  it("returns 完成 for success", () => {
    expect(statusLabel("success")).toBe("完成");
  });

  it("returns 錯誤 for error", () => {
    expect(statusLabel("error")).toBe("錯誤");
  });

  it("returns raw value for unknown status", () => {
    expect(statusLabel("unknown")).toBe("unknown");
  });
});

// --- parseFilePath ---

describe("parseFilePath", () => {
  it("parses Unix path", () => {
    expect(parseFilePath("/home/user/file.txt")).toEqual({
      dir: "/home/user",
      name: "file.txt",
    });
  });

  it("parses Windows path", () => {
    expect(parseFilePath("C:\\Users\\user\\file.txt")).toEqual({
      dir: "C:/Users/user",
      name: "file.txt",
    });
  });

  it("handles filename only", () => {
    expect(parseFilePath("file.txt")).toEqual({
      dir: "",
      name: "file.txt",
    });
  });

  it("handles path with Chinese characters", () => {
    expect(parseFilePath("/Users/zac/字幕/test.srt")).toEqual({
      dir: "/Users/zac/字幕",
      name: "test.srt",
    });
  });
});

// --- File list operations ---

function makeFile(overrides: Partial<FileEntry> = {}): FileEntry {
  return {
    id: "test-id",
    inputPath: "/tmp",
    inputName: "test.txt",
    encoding: "UTF-8",
    status: "pending",
    message: "",
    outputName: "",
    outputPath: "",
    ...overrides,
  };
}

describe("removeCompleted", () => {
  it("removes files with success status", () => {
    const files = [
      makeFile({ id: "1", status: "success" }),
      makeFile({ id: "2", status: "pending" }),
      makeFile({ id: "3", status: "error" }),
    ];
    const result = removeCompleted(files);
    expect(result).toHaveLength(2);
    expect(result.map((f) => f.id)).toEqual(["2", "3"]);
  });

  it("returns empty array when all completed", () => {
    const files = [makeFile({ status: "success" })];
    expect(removeCompleted(files)).toHaveLength(0);
  });

  it("does not mutate original array", () => {
    const files = [makeFile({ status: "success" })];
    removeCompleted(files);
    expect(files).toHaveLength(1);
  });
});

describe("resetErrors", () => {
  it("resets error files to pending", () => {
    const files = [
      makeFile({ id: "1", status: "error", message: "fail" }),
      makeFile({ id: "2", status: "success" }),
    ];
    const result = resetErrors(files);
    expect(result[0].status).toBe("pending");
    expect(result[0].message).toBe("");
    expect(result[1].status).toBe("success");
  });

  it("does not mutate original array", () => {
    const files = [makeFile({ status: "error", message: "fail" })];
    const result = resetErrors(files);
    expect(files[0].status).toBe("error");
    expect(result[0].status).toBe("pending");
  });
});

describe("countByStatus", () => {
  it("counts all statuses correctly", () => {
    const files = [
      makeFile({ status: "pending" }),
      makeFile({ status: "pending" }),
      makeFile({ status: "converting" }),
      makeFile({ status: "success" }),
      makeFile({ status: "error" }),
    ];
    expect(countByStatus(files)).toEqual({
      total: 5,
      pending: 2,
      success: 1,
      error: 1,
    });
  });

  it("returns zeros for empty array", () => {
    expect(countByStatus([])).toEqual({
      total: 0,
      pending: 0,
      success: 0,
      error: 0,
    });
  });
});

// --- buildModuleOverrides ---

describe("buildModuleOverrides", () => {
  it("maps enable to 1 and disable to 0", () => {
    const settings = { Naruto: "enable", Typo: "disable", Smooth: "auto" };
    expect(buildModuleOverrides(settings)).toEqual({
      Naruto: 1,
      Typo: 0,
    });
  });

  it("returns empty object for all auto", () => {
    expect(buildModuleOverrides({ A: "auto", B: "auto" })).toEqual({});
  });

  it("returns empty object for empty input", () => {
    expect(buildModuleOverrides({})).toEqual({});
  });
});

// --- activateTab ---

describe("activateTab", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <button class="tab active" data-tab="a">A</button>
      <button class="tab" data-tab="b">B</button>
      <div id="tab-a" class="tab-panel active"></div>
      <div id="tab-b" class="tab-panel"></div>
    `;
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("switches active tab", () => {
    activateTab("b");
    expect(document.querySelector('[data-tab="a"]')?.classList.contains("active")).toBe(false);
    expect(document.querySelector('[data-tab="b"]')?.classList.contains("active")).toBe(true);
    expect(document.getElementById("tab-a")?.classList.contains("active")).toBe(false);
    expect(document.getElementById("tab-b")?.classList.contains("active")).toBe(true);
  });

  it("handles non-existent tab gracefully", () => {
    activateTab("nonexistent");
    expect(document.querySelector('[data-tab="a"]')?.classList.contains("active")).toBe(false);
  });
});

// --- isSafeUrl ---

describe("isSafeUrl", () => {
  it("allows zhconvert.org", () => {
    expect(isSafeUrl("https://zhconvert.org")).toBe(true);
  });

  it("allows docs.zhconvert.org subpath", () => {
    expect(isSafeUrl("https://docs.zhconvert.org/license/")).toBe(true);
  });

  it("allows github repo", () => {
    expect(isSafeUrl("https://github.com/7a6163/fanhuaji-tauri")).toBe(true);
  });

  it("rejects http protocol", () => {
    expect(isSafeUrl("http://zhconvert.org")).toBe(false);
  });

  it("rejects unknown domain", () => {
    expect(isSafeUrl("https://evil.com")).toBe(false);
  });

  it("rejects file protocol", () => {
    expect(isSafeUrl("file:///etc/passwd")).toBe(false);
  });

  it("rejects javascript protocol", () => {
    expect(isSafeUrl("javascript:alert(1)")).toBe(false);
  });

  it("rejects invalid URL", () => {
    expect(isSafeUrl("not-a-url")).toBe(false);
  });

  it("rejects empty string", () => {
    expect(isSafeUrl("")).toBe(false);
  });
});
