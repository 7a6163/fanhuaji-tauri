import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock Tauri plugins before importing
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { checkForUpdates, initUpdater, installPendingUpdate } from "../updater";

const mockCheck = vi.mocked(check);
const mockInvoke = vi.mocked(invoke);

describe("updater", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: not portable (installed build)
    mockInvoke.mockResolvedValue(false);
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
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
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
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
      vi.spyOn(window, "confirm").mockReturnValue(true);

      await checkForUpdates(false);
      expect(mockUpdate.downloadAndInstall).toHaveBeenCalled();
    });

    it("sets download complete message when Finished event received", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi
          .fn()
          .mockImplementation(async (cb: (event: { event: string }) => void) => {
            cb({ event: "Finished" });
          }),
      };
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
      vi.spyOn(window, "confirm").mockReturnValue(true);

      await checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toBe("下載完成，即將重新啟動…");
    });

    it("does not set complete message for non-Finished events", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi
          .fn()
          .mockImplementation(async (cb: (event: { event: string }) => void) => {
            cb({ event: "Progress" });
          }),
      };
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
      vi.spyOn(window, "confirm").mockReturnValue(true);

      await checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toBe("正在下載更新…");
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

    it("shows portableAvailable message when portable and update found", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi.fn(),
      };
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
      mockInvoke.mockResolvedValue(true); // is_portable = true
      const confirmSpy = vi.spyOn(window, "confirm");

      await checkForUpdates(false);
      const status = document.getElementById("update-status");
      expect(status?.textContent).toContain("攜帶版無法自動更新");
      expect(mockUpdate.downloadAndInstall).not.toHaveBeenCalled();
      expect(confirmSpy).not.toHaveBeenCalled();
    });

    it("does not download or install when portable", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi.fn(),
      };
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
      mockInvoke.mockResolvedValue(true);

      await checkForUpdates(false);
      expect(mockUpdate.downloadAndInstall).not.toHaveBeenCalled();
    });

    it("falls back to normal flow when invoke throws", async () => {
      const mockUpdate = {
        version: "2.0.0",
        downloadAndInstall: vi.fn().mockResolvedValue(undefined),
      };
      mockCheck.mockResolvedValue(mockUpdate as unknown as Update);
      mockInvoke.mockRejectedValue(new Error("command not found"));
      vi.spyOn(window, "confirm").mockReturnValue(true);

      await checkForUpdates(false);
      expect(mockUpdate.downloadAndInstall).toHaveBeenCalled();
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

    it("handles missing button gracefully", () => {
      document.body.innerHTML = '<div id="update-status"></div>';
      mockCheck.mockResolvedValue(null);
      expect(() => initUpdater()).not.toThrow();
    });

    it("triggers checkForUpdates when button clicked", async () => {
      mockCheck.mockResolvedValue(null);
      initUpdater();
      mockCheck.mockClear();

      const btn = document.getElementById("btn-check-update") as HTMLButtonElement;
      btn.click();

      await vi.waitFor(() => {
        expect(mockCheck).toHaveBeenCalled();
      });
    });
  });

  describe("setUpdateStatus edge cases", () => {
    it("handles missing status element gracefully", async () => {
      document.body.innerHTML = '<button id="btn-check-update"></button>';
      mockCheck.mockResolvedValue(null);
      await expect(checkForUpdates(false)).resolves.not.toThrow();
    });
  });

  describe("startup check and install guards", () => {
    const update = () =>
      ({
        version: "2.5.0",
        downloadAndInstall: vi.fn().mockResolvedValue(undefined),
      }) as unknown as Update & {
        downloadAndInstall: ReturnType<typeof vi.fn>;
      };

    it("leaves a cue instead of a dialog when the startup check finds an update", async () => {
      const u = update();
      mockCheck.mockResolvedValue(u);
      const confirmSpy = vi.spyOn(window, "confirm");
      const onAvailable = vi.fn();
      initUpdater({ onAvailable });
      await vi.waitFor(() => expect(onAvailable).toHaveBeenCalledWith("2.5.0"));
      expect(confirmSpy).not.toHaveBeenCalled();

      confirmSpy.mockReturnValue(true);
      await installPendingUpdate();
      expect(u.downloadAndInstall).toHaveBeenCalled();
    });

    it("blocks the restart while a batch is converting", async () => {
      const u = update();
      mockCheck.mockResolvedValue(u);
      vi.spyOn(window, "confirm").mockReturnValue(true);
      initUpdater({ getRisk: () => ({ converting: true, undoable: 0 }), onAvailable: vi.fn() });
      await vi.waitFor(() => expect(mockCheck).toHaveBeenCalled());
      await installPendingUpdate();
      expect(u.downloadAndInstall).not.toHaveBeenCalled();
    });

    it("does nothing when there is no pending update", async () => {
      vi.resetModules();
      const fresh = await import("../updater");
      const confirmSpy = vi.spyOn(window, "confirm");
      await fresh.installPendingUpdate();
      expect(confirmSpy).not.toHaveBeenCalled();
    });

    it("shows the failure when installing the pending update fails", async () => {
      const u = update();
      u.downloadAndInstall.mockRejectedValue(new Error("disk full"));
      mockCheck.mockResolvedValue(u);
      vi.spyOn(window, "confirm").mockReturnValue(true);
      const onAvailable = vi.fn();
      initUpdater({ onAvailable });
      await vi.waitFor(() => expect(onAvailable).toHaveBeenCalled());
      await installPendingUpdate();
      expect(document.getElementById("update-status")?.textContent).toContain("disk full");
    });

    it("warns that overwritten files lose Undo", async () => {
      const u = update();
      mockCheck.mockResolvedValue(u);
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
      initUpdater({ getRisk: () => ({ converting: false, undoable: 3 }) });
      await checkForUpdates(false);
      const calls = confirmSpy.mock.calls;
      expect(calls[calls.length - 1]?.[0]).toContain("3");
      expect(u.downloadAndInstall).not.toHaveBeenCalled();
    });
  });
});
