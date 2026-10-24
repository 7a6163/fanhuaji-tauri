import { describe, expect, it, vi } from "vitest";

const setAsAppMenu = vi.fn();
vi.mock("@tauri-apps/api/menu", () => ({
  Menu: { new: vi.fn(async () => ({ setAsAppMenu })) },
}));

import { Menu } from "@tauri-apps/api/menu";
import { setAppMenu } from "../menu";

type Item = {
  text?: string;
  item?: unknown;
  accelerator?: string;
  action?: () => void;
  items?: Item[];
};

describe("setAppMenu", () => {
  it("installs a menu whose shortcuts run the app's actions", async () => {
    const actions = { openFiles: vi.fn(), openSettings: vi.fn(), convert: vi.fn(), quit: vi.fn() };
    await setAppMenu(actions);

    expect(setAsAppMenu).toHaveBeenCalled();
    const opts = vi.mocked(Menu.new).mock.calls[0]?.[0] as { items: Item[] };
    const all = opts.items.flatMap((sub) => sub.items ?? []);
    const byKey = (key: string) => all.find((i) => i.accelerator === key);

    byKey("CmdOrCtrl+O")?.action?.();
    byKey("CmdOrCtrl+,")?.action?.();
    byKey("CmdOrCtrl+Enter")?.action?.();
    byKey("CmdOrCtrl+Q")?.action?.();
    expect(actions.openFiles).toHaveBeenCalled();
    expect(actions.openSettings).toHaveBeenCalled();
    expect(actions.convert).toHaveBeenCalled();
    // Quit goes through the app so it can confirm before losing Undo.
    expect(actions.quit).toHaveBeenCalled();

    // The Edit items are what give text fields copy/paste on macOS.
    const predefined = all.map((i) => i.item);
    for (const name of ["Copy", "Paste", "SelectAll", "Undo"]) {
      expect(predefined).toContain(name);
    }
  });
});
