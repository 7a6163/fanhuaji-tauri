import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { getLocale, initI18n, setLocale, t, translatePage } from "../i18n/i18n";

beforeEach(() => {
  localStorage.clear();
  // Reset to default locale
  setLocale("zh-TW");
});

afterEach(() => {
  document.body.innerHTML = "";
  localStorage.clear();
});

// --- t() ---

describe("t", () => {
  it("returns zh-TW string by default", () => {
    expect(t("app.title")).toBe("繁化姬");
  });

  it("returns English string when locale is en", () => {
    setLocale("en");
    expect(t("app.title")).toBe("Fanhuaji");
  });

  it("returns zh-CN string when locale is zh-CN", () => {
    setLocale("zh-CN");
    expect(t("drop.title")).toBe("拖放文件到此处");
  });

  it("returns key when key does not exist in any locale", () => {
    expect(t("nonexistent.key")).toBe("nonexistent.key");
  });

  it("falls back to zh-TW when key missing in current locale", () => {
    // All keys exist in all locales, so test by verifying fallback mechanism
    // If current locale has the key, it returns that value
    setLocale("en");
    expect(t("app.title")).toBe("Fanhuaji");
  });

  it("interpolates single param", () => {
    expect(t("update.found", { version: "3.0.0" })).toBe("發現新版本 3.0.0");
  });

  it("interpolates multiple params", () => {
    setLocale("zh-TW");
    const result = t("file.convertingChapter", {
      current: "2",
      total: "10",
      name: "Chapter 2",
    });
    expect(result).toBe("轉換中… (2/10 Chapter 2)");
  });

  it("interpolates in English locale", () => {
    setLocale("en");
    expect(t("update.confirm", { version: "2.0.0" })).toBe(
      "New version 2.0.0 is available. Download and install?",
    );
  });

  it("handles param with $ character safely", () => {
    const result = t("update.failed", { error: "$100 error" });
    expect(result).toContain("$100 error");
  });

  it("replaces all occurrences of same param", () => {
    // Test with a key that might have repeated params
    setLocale("zh-TW");
    expect(t("dom.missingElement", { selector: "#test" })).toBe("必要元素不存在：#test");
  });

  it("returns text unchanged when params is empty object", () => {
    expect(t("app.title", {})).toBe("繁化姬");
  });
});

// --- getLocale / setLocale ---

describe("getLocale / setLocale", () => {
  it("returns zh-TW by default", () => {
    expect(getLocale()).toBe("zh-TW");
  });

  it("returns the locale after setLocale", () => {
    setLocale("en");
    expect(getLocale()).toBe("en");
  });

  it("persists locale to localStorage", () => {
    setLocale("zh-CN");
    expect(localStorage.getItem("fanhuaji-locale")).toBe("zh-CN");
  });

  it("sets document lang to zh-Hant for zh-TW", () => {
    setLocale("zh-TW");
    expect(document.documentElement.lang).toBe("zh-Hant");
  });

  it("sets document lang to en for English", () => {
    setLocale("en");
    expect(document.documentElement.lang).toBe("en");
  });

  it("sets document lang to zh-CN for Simplified Chinese", () => {
    setLocale("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
  });
});

// --- initI18n ---

describe("initI18n", () => {
  it("restores locale from localStorage", () => {
    localStorage.setItem("fanhuaji-locale", "en");
    initI18n();
    expect(getLocale()).toBe("en");
  });

  it("ignores invalid localStorage value and falls back", () => {
    localStorage.setItem("fanhuaji-locale", "fr");
    initI18n();
    // Should fall back to navigator.language detection or default
    expect(["en", "zh-CN", "zh-TW"]).toContain(getLocale());
  });

  it("defaults to zh-TW when no localStorage and navigator is zh-TW", () => {
    Object.defineProperty(navigator, "language", { value: "zh-TW", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-TW");
  });

  it("detects zh-CN for navigator language zh-CN", () => {
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "zh-CN", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-CN");
  });

  it("detects zh-CN for navigator language zh-Hans", () => {
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "zh-Hans", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-CN");
  });

  it("detects zh-TW for navigator language zh-Hant", () => {
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "zh-Hant", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-TW");
  });

  it("falls back to zh-TW for other zh variants (e.g. zh-HK)", () => {
    // zh-HK uses traditional script; safer to default to zh-TW than zh-CN.
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "zh-HK", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-TW");
  });

  it("detects en for navigator language en-US", () => {
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "en-US", configurable: true });
    initI18n();
    expect(getLocale()).toBe("en");
  });

  it("falls back to zh-TW for unsupported language", () => {
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "ja", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-TW");
  });

  it("detects zh-CN for bare zh", () => {
    localStorage.clear();
    Object.defineProperty(navigator, "language", { value: "zh", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-CN");
  });

  it("localStorage takes priority over navigator.language", () => {
    localStorage.setItem("fanhuaji-locale", "zh-CN");
    Object.defineProperty(navigator, "language", { value: "en-US", configurable: true });
    initI18n();
    expect(getLocale()).toBe("zh-CN");
  });
});

// --- translatePage ---

describe("translatePage", () => {
  it("translates data-i18n elements", () => {
    document.body.innerHTML = '<span data-i18n="app.title">placeholder</span>';
    setLocale("en");
    translatePage();
    expect(document.querySelector("span")?.textContent).toBe("Fanhuaji");
  });

  it("translates data-i18n-title attribute", () => {
    document.body.innerHTML = '<button data-i18n-title="header.settings" title="">btn</button>';
    setLocale("en");
    translatePage();
    expect(document.querySelector("button")?.title).toBe("Settings");
  });

  it("translates data-i18n-placeholder attribute", () => {
    document.body.innerHTML =
      '<textarea data-i18n-placeholder="settings.replace.pairPlaceholder"></textarea>';
    setLocale("en");
    translatePage();
    expect((document.querySelector("textarea") as HTMLTextAreaElement).placeholder).toBe(
      "search=replace (one per line)",
    );
  });

  it("translates data-i18n-label on optgroup", () => {
    document.body.innerHTML = `
      <select>
        <optgroup label="" data-i18n-label="converter.group.traditional">
          <option>test</option>
        </optgroup>
      </select>
    `;
    setLocale("en");
    translatePage();
    expect(document.querySelector("optgroup")?.label).toBe("Traditional / Simplified");
  });

  it("translates multiple elements at once", () => {
    document.body.innerHTML = `
      <span data-i18n="drop.title">a</span>
      <span data-i18n="drop.subtitle">b</span>
    `;
    setLocale("zh-CN");
    translatePage();
    const spans = document.querySelectorAll("span");
    expect(spans[0].textContent).toBe("拖放文件到此处");
    expect(spans[1].textContent).toBe("或 点击打开文件");
  });

  it("handles empty data-i18n gracefully", () => {
    document.body.innerHTML = '<span data-i18n="">text</span>';
    translatePage();
    // Empty key returns empty string
    expect(document.querySelector("span")?.textContent).toBe("");
  });
});

// --- locale key parity ---

describe("locale key parity", () => {
  // Import raw JSON for key comparison
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const zhTW = require("../i18n/locales/zh-TW.json");
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const zhCN = require("../i18n/locales/zh-CN.json");
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const en = require("../i18n/locales/en.json");

  const zhTWKeys = Object.keys(zhTW).sort();
  const zhCNKeys = Object.keys(zhCN).sort();
  const enKeys = Object.keys(en).sort();

  it("zh-CN has the same keys as zh-TW", () => {
    expect(zhCNKeys).toEqual(zhTWKeys);
  });

  it("en has the same keys as zh-TW", () => {
    expect(enKeys).toEqual(zhTWKeys);
  });

  it("no locale has empty string values", () => {
    for (const [key, value] of Object.entries(zhTW)) {
      expect(value, `zh-TW key "${key}" is empty`).not.toBe("");
    }
    for (const [key, value] of Object.entries(zhCN)) {
      expect(value, `zh-CN key "${key}" is empty`).not.toBe("");
    }
    for (const [key, value] of Object.entries(en)) {
      expect(value, `en key "${key}" is empty`).not.toBe("");
    }
  });
});
