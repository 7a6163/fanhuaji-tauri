import { Menu } from "@tauri-apps/api/menu";
import { t } from "./i18n/i18n";

export interface MenuActions {
  openFiles: () => void;
  openSettings: () => void;
  convert: () => void;
}

/**
 * The macOS menu bar: the app's commands with their shortcuts, plus the standard App,
 * Edit and Window items (Edit is what makes copy/paste work in text fields). Call again
 * after a language change. Other platforms get none: a window menu there would add a
 * bar the UI has no use for.
 */
export async function setAppMenu(actions: MenuActions): Promise<void> {
  const menu = await Menu.new({
    items: [
      {
        text: t("app.title"),
        items: [
          { item: { About: null }, text: t("menu.about") },
          { item: "Separator" },
          { text: t("menu.settings"), accelerator: "CmdOrCtrl+,", action: actions.openSettings },
          { item: "Separator" },
          { item: "Services", text: t("menu.services") },
          { item: "Separator" },
          { item: "Hide", text: t("menu.hide") },
          { item: "HideOthers", text: t("menu.hideOthers") },
          { item: "ShowAll", text: t("menu.showAll") },
          { item: "Separator" },
          { item: "Quit", text: t("menu.quit") },
        ],
      },
      {
        text: t("menu.file"),
        items: [
          { text: t("menu.openFiles"), accelerator: "CmdOrCtrl+O", action: actions.openFiles },
          { text: t("menu.convert"), accelerator: "CmdOrCtrl+Enter", action: actions.convert },
          { item: "Separator" },
          { item: "CloseWindow", text: t("menu.closeWindow") },
        ],
      },
      {
        text: t("menu.edit"),
        items: [
          { item: "Undo", text: t("menu.undo") },
          { item: "Redo", text: t("menu.redo") },
          { item: "Separator" },
          { item: "Cut", text: t("menu.cut") },
          { item: "Copy", text: t("menu.copy") },
          { item: "Paste", text: t("menu.paste") },
          { item: "SelectAll", text: t("menu.selectAll") },
        ],
      },
      {
        text: t("menu.window"),
        items: [
          { item: "Minimize", text: t("menu.minimize") },
          { item: "Fullscreen", text: t("menu.fullscreen") },
        ],
      },
    ],
  });
  await menu.setAsAppMenu();
}
