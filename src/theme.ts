type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "fanhuaji-theme";

function getStoredTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "light" || stored === "dark" || stored === "system") {
    return stored;
  }
  return "system";
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
      const theme = btn.dataset.theme as Theme;
      if (theme) applyTheme(theme);
    });
  });
}
