import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

export async function checkForUpdates(silent = false): Promise<void> {
  try {
    const update = await check();
    if (!update) {
      if (!silent) {
        showUpdateNotification("目前已是最新版本。");
      }
      return;
    }

    const confirmed = window.confirm(`發現新版本 ${update.version}，是否要下載並安裝？`);
    if (!confirmed) return;

    showUpdateNotification("正在下載更新…");

    await update.downloadAndInstall((event) => {
      if (event.event === "Finished") {
        showUpdateNotification("下載完成，即將重新啟動…");
      }
    });

    await relaunch();
  } catch (err) {
    if (!silent) {
      showUpdateNotification(`檢查更新失敗：${String(err)}`);
    }
    console.error("Update check failed:", err);
  }
}

function showUpdateNotification(msg: string) {
  const existing = document.getElementById("update-toast");
  if (existing) existing.remove();

  const statusBar = document.querySelector(".status-bar");
  if (!statusBar) return;

  const toast = document.createElement("span");
  toast.id = "update-toast";
  toast.className = "status-badge blue";
  toast.textContent = msg;
  statusBar.appendChild(toast);

  setTimeout(() => toast.remove(), 8000);
}
