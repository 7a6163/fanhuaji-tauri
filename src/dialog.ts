export interface ConfirmOptions {
  title: string;
  body?: string;
  /** Shown in the warning style, e.g. what would be lost. */
  warning?: string;
  /** Long text in a scrollable box, e.g. release notes. */
  notes?: string;
  confirmLabel: string;
  cancelLabel: string;
  /** Inform only: the confirm button is disabled. */
  blocked?: boolean;
}

/**
 * Ask in the app's own modal dialog (#app-dialog in index.html). Cancel takes focus
 * first, so Enter never confirms by accident; Escape cancels. Falls back to the native
 * confirm when there is no dialog (tests) or the webview lacks showModal.
 */
export function confirmDialog(o: ConfirmOptions): Promise<boolean> {
  const dialog = document.getElementById("app-dialog") as HTMLDialogElement | null;
  if (!dialog || typeof dialog.showModal !== "function") {
    if (o.blocked) return Promise.resolve(false);
    const text = [o.title, o.body, o.warning].filter(Boolean).join("\n\n");
    return Promise.resolve(window.confirm(text));
  }
  const field = (id: string, text = "") => {
    const el = dialog.querySelector(`#${id}`) as HTMLElement;
    el.textContent = text.trim();
    el.hidden = !el.textContent;
  };
  field("app-dialog-title", o.title);
  field("app-dialog-body", o.body);
  field("app-dialog-warning", o.warning);
  field("app-dialog-notes", o.notes);
  const confirmBtn = dialog.querySelector("#app-dialog-confirm") as HTMLButtonElement;
  const cancelBtn = dialog.querySelector("#app-dialog-cancel") as HTMLButtonElement;
  confirmBtn.textContent = o.confirmLabel;
  confirmBtn.disabled = Boolean(o.blocked);
  cancelBtn.textContent = o.cancelLabel;
  return new Promise((resolve) => {
    // The form's method="dialog" buttons set returnValue; Escape leaves it empty.
    dialog.returnValue = "";
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "confirm"), {
      once: true,
    });
    dialog.showModal();
    cancelBtn.focus();
  });
}
