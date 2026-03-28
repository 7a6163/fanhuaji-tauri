import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

function setUpdateStatus(msg: string) {
  const el = document.getElementById("update-status");
  if (el) el.textContent = msg;
}

export async function checkForUpdates(silent = false): Promise<void> {
  if (!silent) setUpdateStatus("正在檢查…");

  try {
    const update = await check();
    if (!update) {
      setUpdateStatus(silent ? "" : "目前已是最新版本。");
      return;
    }

    setUpdateStatus(`發現新版本 ${update.version}`);
    const confirmed = window.confirm(`發現新版本 ${update.version}，是否要下載並安裝？`);
    if (!confirmed) return;

    setUpdateStatus("正在下載更新…");

    await update.downloadAndInstall((event) => {
      if (event.event === "Finished") {
        setUpdateStatus("下載完成，即將重新啟動…");
      }
    });

    await relaunch();
  } catch (err) {
    if (!silent) {
      setUpdateStatus(`檢查更新失敗：${String(err)}`);
    }
    console.error("Update check failed:", err);
  }
}

export function initUpdater() {
  const btn = document.getElementById("btn-check-update");
  btn?.addEventListener("click", () => checkForUpdates(false));

  // Silent check on startup
  checkForUpdates(true);
}
