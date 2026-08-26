import {
  useEffect,
  useRef,
  type ReactNode,
} from "react";

const focusableSelector = [
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[href]",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function initialFocusTarget(dialog: HTMLDialogElement): HTMLElement | null {
  return (
    dialog.querySelector<HTMLElement>("[data-dialog-initial-focus]") ??
    dialog.querySelector<HTMLElement>("[autofocus]") ??
    dialog.querySelector<HTMLElement>(focusableSelector)
  );
}

export function Dialog({
  children,
  className = "modal",
  describedBy,
  labelledBy,
  onClose,
  open,
}: {
  children: ReactNode;
  className?: string;
  describedBy?: string;
  labelledBy: string;
  onClose: () => void;
  open: boolean;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;

    if (open) {
      returnFocusRef.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      const initialFocus = initialFocusTarget(dialog);
      const addedAutofocus = Boolean(
        initialFocus && !initialFocus.hasAttribute("autofocus"),
      );
      if (addedAutofocus) {
        initialFocus?.setAttribute("autofocus", "");
      }
      if (!dialog.open) {
        dialog.showModal();
      }
      const focusTarget = () => (initialFocus ?? dialog).focus();
      focusTarget();
      const animationFrame = window.requestAnimationFrame(focusTarget);
      return () => {
        window.cancelAnimationFrame(animationFrame);
        if (addedAutofocus) {
          initialFocus?.removeAttribute("autofocus");
        }
      };
    }

    if (dialog.open) {
      dialog.close();
    }
    returnFocusRef.current?.focus();
    returnFocusRef.current = null;
  }, [open]);

  return (
    <dialog
      ref={dialogRef}
      className={className}
      aria-labelledby={labelledBy}
      aria-describedby={describedBy}
      aria-modal="true"
      tabIndex={-1}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={() => {
        if (open) onClose();
      }}
    >
      {children}
    </dialog>
  );
}
