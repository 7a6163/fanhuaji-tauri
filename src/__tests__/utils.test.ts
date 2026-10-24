import { describe, expect, it } from "vitest";
import { t } from "../i18n/i18n";
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
    status: "pending",
    message: "",
    outputName: "",
    outputPath: "",
    ...overrides,
  };
}

describe("fileExt", () => {
  it("returns the lower-cased extension", () => {
    expect(fileExt("Book.EPUB")).toBe("epub");
  });

  it("uses the last dot", () => {
    expect(fileExt("movie.zh.srt")).toBe("srt");
  });

  it("returns empty string without an extension", () => {
    expect(fileExt("README")).toBe("");
  });
});

describe("fileMessage", () => {
  it("shows done for a success without warnings", () => {
    expect(fileMessage(makeFile({ status: "success" }))).toBe(t("file.convertDone"));
  });

  it("shows the warning for a success with warnings", () => {
    const f = makeFile({ status: "success", message: "轉換完成（部分章節失敗（3/40））" });
    expect(fileMessage(f)).toBe("轉換完成（部分章節失敗（3/40））");
  });

  it("names the output file for a success", () => {
    const f = makeFile({ status: "success", outputName: "test.Taiwan.txt" });
    expect(fileMessage(f)).toBe(t("file.savedAs", { name: "test.Taiwan.txt" }));
  });

  it("says the original was overwritten when a backup exists", () => {
    const f = makeFile({ status: "success", outputName: "test.txt", backupPath: "/b/x" });
    expect(fileMessage(f)).toBe(t("file.overwritten"));
  });

  it("shows restored after an undo", () => {
    expect(fileMessage(makeFile({ status: "restored" }))).toBe(t("file.restored"));
  });

  it("escapes the message", () => {
    const f = makeFile({ status: "error", message: "<b>bad</b>" });
    expect(fileMessage(f)).toBe("&lt;b&gt;bad&lt;/b&gt;");
  });

  it("shows chapter progress while converting an EPUB", () => {
    const f = makeFile({
      status: "converting",
      chapterIndex: 2,
      chapterTotal: 10,
      chapterName: "ch02",
    });
    expect(fileMessage(f)).toBe(
      escHtml(t("file.convertingChapter", { current: "2", total: "10", name: " ch02" })),
    );
  });

  it("omits the chapter name when there is none", () => {
    const f = makeFile({ status: "converting", chapterIndex: 3, chapterTotal: 5 });
    expect(fileMessage(f)).toBe(
      escHtml(t("file.convertingChapter", { current: "3", total: "5", name: "" })),
    );
  });

  it("shows plain converting without chapter info", () => {
    expect(fileMessage(makeFile({ status: "converting" }))).toBe(escHtml(t("file.converting")));
  });

  it("shows the format for a pending file", () => {
    expect(fileMessage(makeFile({ inputName: "a.srt" }))).toBe("SRT");
    expect(fileMessage(makeFile({ inputName: "noext" }))).toBe("—");
  });
});

describe("shouldResumeConversion", () => {
  const added = makeFile({ id: "added", status: "pending" });
  const done = makeFile({ id: "done", status: "success" });

  it("resumes when auto-convert is on and files are pending", () => {
    expect(shouldResumeConversion([done, added], true)).toBe(true);
  });

  it("does not resume when auto-convert is off", () => {
    expect(shouldResumeConversion([done, added], false)).toBe(false);
  });

  it("does not resume when nothing is pending", () => {
    const failed = makeFile({ id: "failed", status: "error" });
    expect(shouldResumeConversion([done, failed], true)).toBe(false);
  });
});

describe("createRequestGuard", () => {
  it("keeps a lone request current", () => {
    const guard = createRequestGuard();
    const isCurrent = guard.begin();
    expect(isCurrent()).toBe(true);
  });

  it("makes an earlier request stale once a later one begins", () => {
    // A slow preview of file A must not overwrite the preview of file B.
    const guard = createRequestGuard();
    const previewA = guard.begin();
    const previewB = guard.begin();
    expect(previewA()).toBe(false);
    expect(previewB()).toBe(true);
  });

  it("invalidate makes every in-flight request stale", () => {
    const guard = createRequestGuard();
    const isCurrent = guard.begin();
    guard.invalidate();
    expect(isCurrent()).toBe(false);
  });

  it("keeps separate guards independent", () => {
    const a = createRequestGuard();
    const b = createRequestGuard();
    const isCurrent = a.begin();
    b.begin();
    expect(isCurrent()).toBe(true);
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
      makeFile({ status: "success", message: "部分章節失敗" }),
      makeFile({ status: "error" }),
    ];
    expect(countByStatus(files)).toEqual({
      total: 6,
      pending: 2,
      success: 1,
      warning: 1,
      error: 1,
    });
  });

  it("returns zeros for empty array", () => {
    expect(countByStatus([])).toEqual({
      total: 0,
      pending: 0,
      success: 0,
      warning: 0,
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

// --- statusDotHtml ---
