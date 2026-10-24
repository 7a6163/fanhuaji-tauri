import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { confirmDialog } from "./dialog";
import { t } from "./i18n/i18n";

function setUpdateStatus(msg: string) {
  const el = document.getElementById("update-status");
  if (el) el.textContent = msg;
}

async function isPortableBuild(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_portable");
  } catch {
    return false;
  }
}

/** What an update restart would interrupt or throw away, supplied by the app. */
export interface UpdateRisk {
  converting: boolean;
  undoable: number;
}

interface UpdaterOptions {
  getRisk?: () => UpdateRisk;
  /** Called when the startup check finds an update; the app shows its own cue. */
  onAvailable?: (version: string) => void;
}

let options: UpdaterOptions = {};
let pendingUpdate: Update | null = null;

/** Ask, then download, install and relaunch. A running batch blocks the restart. */
async function promptInstall(update: Update): Promise<void> {
  const risk = options.getRisk?.() ?? { converting: false, undoable: 0 };
  const warning = risk.converting
    ? t("update.busy")
    : risk.undoable
      ? t("update.undoWarning", { count: String(risk.undoable) })
      : "";
  const confirmed = await confirmDialog({
    title: t("update.found", { version: update.version }),
    body: t("update.prompt"),
    warning,
    notes: update.body,
    confirmLabel: t("update.install"),
    cancelLabel: t("update.later"),
    blocked: risk.converting,
  });
  if (!confirmed) return;

  setUpdateStatus(t("update.downloading"));
  await update.downloadAndInstall((event) => {
    if (event.event === "Finished") {
      setUpdateStatus(t("update.restarting"));
    }
  });
  await relaunch();
}

/** Open the install prompt for the update the startup check found. */
export async function installPendingUpdate(): Promise<void> {
  if (!pendingUpdate) return;
  try {
    await promptInstall(pendingUpdate);
  } catch (err) {
    setUpdateStatus(t("update.failed", { error: String(err) }));
  }
}

export async function checkForUpdates(silent = false): Promise<void> {
  if (!silent) setUpdateStatus(t("update.checking"));

  try {
    const update = await check();
    if (!update) {
      setUpdateStatus(silent ? "" : t("update.upToDate"));
      return;
    }

    // Portable Windows builds cannot self-update in place (the updater would
    // run an installer that installs a separate copy), so only notify the user.
    if (await isPortableBuild()) {
      setUpdateStatus(t("update.portableAvailable", { version: update.version }));
      return;
    }

    setUpdateStatus(t("update.found", { version: update.version }));
    // The startup check never opens a modal over the user's work; it leaves a cue.
    if (silent) {
      pendingUpdate = update;
      options.onAvailable?.(update.version);
      return;
    }
    await promptInstall(update);
  } catch (err) {
    if (!silent) {
      setUpdateStatus(t("update.failed", { error: String(err) }));
    }
    // Update check failed — status already shown in UI if not silent
  }
}

export function initUpdater(opts: UpdaterOptions = {}) {
  options = opts;
  const btn = document.getElementById("btn-check-update");
  btn?.addEventListener("click", () => checkForUpdates(false));

  // Silent check on startup
  void checkForUpdates(true);
}
