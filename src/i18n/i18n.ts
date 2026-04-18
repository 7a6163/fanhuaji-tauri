import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";

export type Locale = "en" | "zh-CN" | "zh-TW";

const messages: Record<Locale, Record<string, string>> = {
  en: en as Record<string, string>,
  "zh-CN": zhCN as Record<string, string>,
  "zh-TW": zhTW as Record<string, string>,
};

const STORAGE_KEY = "fanhuaji-locale";
const SUPPORTED: Locale[] = ["en", "zh-CN", "zh-TW"];
const DEFAULT_LOCALE: Locale = "zh-TW";

let currentLocale: Locale = DEFAULT_LOCALE;

function detectLocale(): Locale {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved && SUPPORTED.includes(saved as Locale)) return saved as Locale;

  const lang = navigator.language;
  if (lang.startsWith("zh")) {
    // zh-TW, zh-Hant → zh-TW; zh-CN, zh-Hans, zh → zh-CN
    if (lang.includes("TW") || lang.includes("Hant")) return "zh-TW";
    if (lang.includes("CN") || lang.includes("Hans") || lang === "zh") return "zh-CN";
    return "zh-TW";
  }
  if (lang.startsWith("en")) return "en";
  return DEFAULT_LOCALE;
}

export function getLocale(): Locale {
  return currentLocale;
}

export function setLocale(locale: Locale): void {
  currentLocale = locale;
  localStorage.setItem(STORAGE_KEY, locale);
  document.documentElement.lang = locale === "zh-TW" ? "zh-Hant" : locale;
  translatePage();
}

export function t(key: string, params?: Record<string, string>): string {
  let text = messages[currentLocale]?.[key] ?? messages[DEFAULT_LOCALE]?.[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      text = text.replace(new RegExp(`\\{${k}\\}`, "g"), () => v);
    }
  }
  return text;
}

export function translatePage(): void {
  // textContent
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n ?? "";
    el.textContent = t(key);
  });
  // title attribute
  document.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle ?? "");
  });
  // placeholder attribute
  document.querySelectorAll<HTMLElement>("[data-i18n-placeholder]").forEach((el) => {
    (el as HTMLInputElement | HTMLTextAreaElement).placeholder = t(
      el.dataset.i18nPlaceholder ?? "",
    );
  });
  // optgroup label
  document.querySelectorAll<HTMLOptGroupElement>("[data-i18n-label]").forEach((el) => {
    el.label = t(el.dataset.i18nLabel ?? "");
  });
}

export function initI18n(): void {
  currentLocale = detectLocale();
  document.documentElement.lang = currentLocale === "zh-TW" ? "zh-Hant" : currentLocale;
  translatePage();
}

const ERROR_CODE_PATTERN = /^([A-Z][A-Z0-9_]*)(?::([\s\S]*))?$/;

/**
 * Translate a Rust-originated error string of the form "CODE" or "CODE:detail"
 * into the current locale. The CODE must match `errors.<CODE>` in the locale
 * JSON. Falls back to the raw string if the prefix is not a recognised code.
 */
export function translateError(raw: string): string {
  if (!raw) return raw;
  const match = raw.match(ERROR_CODE_PATTERN);
  if (!match) return raw;
  const [, code, detail] = match;
  const key = `errors.${code}`;
  const translated = t(key, { detail: detail ?? "" });
  return translated === key ? raw : translated;
}
