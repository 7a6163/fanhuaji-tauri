type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "fanhuaji-theme";
const VALID_THEMES: readonly string[] = ["light", "dark", "system"];

function isValidTheme(value: string | undefined): value is Theme {
  return typeof value === "string" && VALID_THEMES.includes(value);
}

function getStoredTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY) ?? undefined;
  return isValidTheme(stored) ? stored : "system";
}

// Match the native window chrome (e.g. the macOS title bar) to the app theme.
// "system" hands control back to the OS. Dynamically imported so unit tests
// (jsdom, no Tauri runtime) never load the Tauri API.
function syncWindowTheme(theme: Theme): void {
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(theme === "system" ? null : theme))
    .catch(() => {
      /* not running under Tauri (e.g. unit tests) — ignore */
    });
}

function applyTheme(theme: Theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem(STORAGE_KEY, theme);
  syncWindowTheme(theme);

  document.querySelectorAll<HTMLButtonElement>(".theme-option").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.theme === theme);
  });
}

export function initTheme() {
  applyTheme(getStoredTheme());

  document.querySelectorAll<HTMLButtonElement>(".theme-option").forEach((btn) => {
    btn.addEventListener("click", () => {
      const value = btn.dataset.theme;
      if (isValidTheme(value)) applyTheme(value);
    });
  });
}
