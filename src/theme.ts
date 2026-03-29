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

function applyTheme(theme: Theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem(STORAGE_KEY, theme);

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
