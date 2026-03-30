import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { initTheme } from "../theme";

describe("theme", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
    document.body.innerHTML = `
      <div class="theme-options">
        <button type="button" class="theme-option" data-theme="system">系統</button>
        <button type="button" class="theme-option" data-theme="light">淺色</button>
        <button type="button" class="theme-option" data-theme="dark">深色</button>
      </div>
    `;
  });

  afterEach(() => {
    localStorage.clear();
  });

  it("defaults to system theme when no stored preference", () => {
    initTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("system");
  });

  it("restores stored theme from localStorage", () => {
    localStorage.setItem("fanhuaji-theme", "dark");
    initTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("highlights the active theme button", () => {
    localStorage.setItem("fanhuaji-theme", "light");
    initTheme();
    const lightBtn = document.querySelector<HTMLButtonElement>(".theme-option[data-theme='light']");
    const systemBtn = document.querySelector<HTMLButtonElement>(
      ".theme-option[data-theme='system']",
    );
    expect(lightBtn?.classList.contains("active")).toBe(true);
    expect(systemBtn?.classList.contains("active")).toBe(false);
  });

  it("switches theme on button click", () => {
    initTheme();
    const darkBtn = document.querySelector<HTMLButtonElement>('[data-theme="dark"]');
    darkBtn?.click();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(localStorage.getItem("fanhuaji-theme")).toBe("dark");
  });

  it("ignores invalid stored theme and defaults to system", () => {
    localStorage.setItem("fanhuaji-theme", "invalid");
    initTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("system");
  });

  it("ignores click on button without data-theme attribute", () => {
    document.body.innerHTML = `
      <div class="theme-options">
        <button type="button" class="theme-option" data-theme="system">系統</button>
        <button type="button" class="theme-option">無主題</button>
      </div>
    `;
    initTheme();
    const noThemeBtn = document.querySelectorAll<HTMLButtonElement>(".theme-option")[1];
    noThemeBtn?.click();
    expect(document.documentElement.getAttribute("data-theme")).toBe("system");
  });
});
