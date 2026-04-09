import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { t } from "./i18n/i18n";

function setUpdateStatus(msg: string) {
  const el = document.getElementById("update-status");
  if (el) el.textContent = msg;
}

export async function checkForUpdates(silent = false): Promise<void> {
  if (!silent) setUpdateStatus(t("update.checking"));

  try {
    const update = await check();
    if (!update) {
      setUpdateStatus(silent ? "" : t("update.upToDate"));
      return;
    }

    setUpdateStatus(t("update.found", { version: update.version }));
    const confirmed = window.confirm(t("update.confirm", { version: update.version }));
    if (!confirmed) return;

    setUpdateStatus(t("update.downloading"));

    await update.downloadAndInstall((event) => {
      if (event.event === "Finished") {
        setUpdateStatus(t("update.restarting"));
      }
    });

    await relaunch();
  } catch (err) {
    if (!silent) {
      setUpdateStatus(t("update.failed", { error: String(err) }));
    }
    // Update check failed — status already shown in UI if not silent
  }
}

export function initUpdater() {
  const btn = document.getElementById("btn-check-update");
  btn?.addEventListener("click", () => checkForUpdates(false));

  // Silent check on startup
  void checkForUpdates(true);
}
