import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock Tauri plugins before importing
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));

import { check } from "@tauri-apps/plugin-updater";
import { checkForUpdates, initUpdater } from "../updater";

const mockCheck = vi.mocked(check);

describe("updater", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.innerHTML = `
      <div id="update-status"></div>
      <button id="btn-check-update"></button>
    `;
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  describe("checkForUpdates", () => {
    it("sets status to empty when no update and silent", async () => {
      mockCheck.mockResolvedValue(null);
      await checkForUpdates(true);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toBe("");
    });

    it("shows no update message when not silent", async () => {
      mockCheck.mockResolvedValue(null);
      await checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toBe("目前已是最新版本。");
    });

    it("shows checking message when not silent", async () => {
      mockCheck.mockImplementation(
        () => new Promise((resolve) => setTimeout(() => resolve(null), 100)),
      );
      const promise = checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toBe("正在檢查…");
      await promise;
    });

    it("shows version when update available and user declines", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi.fn(),
      };
      mockCheck.mockResolvedValue(mockUpdate as any);
      vi.spyOn(window, "confirm").mockReturnValue(false);

      await checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toBe("發現新版本 2.0.0");
    });

    it("downloads and installs when user confirms", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi.fn().mockResolvedValue(undefined),
      };
      mockCheck.mockResolvedValue(mockUpdate as any);
      vi.spyOn(window, "confirm").mockReturnValue(true);

      await checkForUpdates(false);
      expect(mockUpdate.downloadAndInstall).toHaveBeenCalled();
    });

    it("handles check error silently", async () => {
      mockCheck.mockRejectedValue(new Error("network error"));
      const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
      await checkForUpdates(true);
      const status = document.getElementById("update-status");
      expect(status?.textContent).not.toContain("network error");
      consoleSpy.mockRestore();
    });

    it("shows error message when not silent", async () => {
      mockCheck.mockRejectedValue(new Error("network error"));
      const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
      await checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toContain("network error");
      consoleSpy.mockRestore();
    });
  });

  describe("initUpdater", () => {
    it("binds click handler to check update button", () => {
      mockCheck.mockResolvedValue(null);
      initUpdater();
      const btn = document.getElementById("btn-check-update") as HTMLButtonElement;
      expect(btn).toBeTruthy();
    });

    it("runs silent check on init", async () => {
      mockCheck.mockResolvedValue(null);
      initUpdater();
      await vi.waitFor(() => {
        expect(mockCheck).toHaveBeenCalled();
      });
    });
  });
});
