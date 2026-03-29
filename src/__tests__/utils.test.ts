import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  activateTab,
  buildModuleOverrides,
  countByStatus,
  escHtml,
  type FileEntry,
  fileTooltip,
  isEpubFile,
  isSafeUrl,
  parseFilePath,
  removeCompleted,
  resetErrors,
  statusDotHtml,
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

// --- isEpubFile ---

describe("isEpubFile", () => {
  it("returns true for .epub extension", () => {
    expect(isEpubFile("book.epub")).toBe(true);
  });

  it("returns true for .EPUB (uppercase)", () => {
    expect(isEpubFile("book.EPUB")).toBe(true);
  });

  it("returns true for .Epub (mixed case)", () => {
    expect(isEpubFile("book.Epub")).toBe(true);
  });

  it("returns false for non-epub file", () => {
    expect(isEpubFile("book.txt")).toBe(false);
  });

  it("returns false for filename with multiple dots", () => {
    expect(isEpubFile("my.book.pdf")).toBe(false);
  });

  it("returns true for filename with multiple dots ending in .epub", () => {
    expect(isEpubFile("my.book.epub")).toBe(true);
  });

  it("returns false for empty string", () => {
    expect(isEpubFile("")).toBe(false);
  });
});

// --- fileTooltip ---

describe("fileTooltip", () => {
  it("returns outputPath for success with output", () => {
    const file = makeFile({ status: "success", outputPath: "/tmp/out.txt" });
    expect(fileTooltip(file)).toBe("/tmp/out.txt");
  });

  it("returns message for error with message", () => {
    const file = makeFile({ status: "error", message: "轉換失敗" });
    expect(fileTooltip(file)).toBe("轉換失敗");
  });

  it("returns empty string for pending with no message", () => {
    const file = makeFile({ status: "pending" });
    expect(fileTooltip(file)).toBe("");
  });

  it("returns empty string for success without outputPath", () => {
    const file = makeFile({ status: "success", outputPath: "" });
    expect(fileTooltip(file)).toBe("");
  });

  it("returns empty string for error without message", () => {
    const file = makeFile({ status: "error", message: "" });
    expect(fileTooltip(file)).toBe("");
  });
});

// --- statusDotHtml ---

describe("statusDotHtml", () => {
  it("produces correct class for pending", () => {
    const html = statusDotHtml("pending");
    expect(html).toContain("status-dot");
    expect(html).toContain("pending");
  });

  it("produces correct class for converting", () => {
    const html = statusDotHtml("converting");
    expect(html).toContain("status-dot");
    expect(html).toContain("converting");
  });

  it("produces correct class for success", () => {
    const html = statusDotHtml("success");
    expect(html).toContain("status-dot");
    expect(html).toContain("success");
  });

  it("produces correct class for error", () => {
    const html = statusDotHtml("error");
    expect(html).toContain("status-dot");
    expect(html).toContain("error");
  });
});
