import { afterEach, describe, expect, it, vi } from "vitest";
import { confirmDialog } from "../dialog";

// jsdom has no showModal; stand in for the user picking a button.
function mountDialog(answer: string) {
  document.body.innerHTML = `
    <dialog id="app-dialog">
      <h2 id="app-dialog-title"></h2>
      <p id="app-dialog-body"></p>
      <p id="app-dialog-warning" hidden></p>
      <pre id="app-dialog-notes" hidden></pre>
      <button id="app-dialog-cancel"></button>
      <button id="app-dialog-confirm"></button>
    </dialog>`;
  const dialog = document.getElementById("app-dialog") as HTMLDialogElement;
  dialog.showModal = vi.fn(() => {
    dialog.returnValue = answer;
    dialog.dispatchEvent(new Event("close"));
  });
  return dialog;
}

const base = { title: "Update?", confirmLabel: "Update", cancelLabel: "Later" };

describe("confirmDialog", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("resolves true on confirm and fills only the parts given", async () => {
    const dialog = mountDialog("confirm");
    await expect(confirmDialog({ ...base, notes: "  Faster previews  " })).resolves.toBe(true);
    expect(dialog.querySelector("#app-dialog-title")?.textContent).toBe("Update?");
    expect(dialog.querySelector<HTMLElement>("#app-dialog-notes")?.textContent).toBe(
      "Faster previews",
    );
    expect(dialog.querySelector<HTMLElement>("#app-dialog-warning")?.hidden).toBe(true);
    expect(dialog.querySelector("#app-dialog-confirm")?.textContent).toBe("Update");
  });

  it("resolves false on cancel or Escape", async () => {
    mountDialog("");
    await expect(confirmDialog({ ...base, warning: "3 lose Undo" })).resolves.toBe(false);
  });

  it("disables confirm when blocked", async () => {
    const dialog = mountDialog("");
    await confirmDialog({ ...base, blocked: true });
    expect(dialog.querySelector<HTMLButtonElement>("#app-dialog-confirm")?.disabled).toBe(true);
  });

  it("falls back to window.confirm without the dialog, and never confirms when blocked", async () => {
    const spy = vi.spyOn(window, "confirm").mockReturnValue(true);
    await expect(confirmDialog({ ...base, body: "Now?" })).resolves.toBe(true);
    expect(spy.mock.calls[0]?.[0]).toContain("Now?");
    await expect(confirmDialog({ ...base, blocked: true })).resolves.toBe(false);
  });
});
