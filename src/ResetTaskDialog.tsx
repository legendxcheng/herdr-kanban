import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { tr, useLocale } from "./i18n";

export function ResetTaskDialog({ name, busy, error, onCancel, onConfirm }: {
  name: string; busy: boolean; error: string; onCancel: () => void; onConfirm: () => void;
}) {
  useLocale();
  const id = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const root = document.getElementById("root");
    const inert = root?.inert ?? false;
    const opener = document.activeElement as HTMLElement | null;
    if (root) root.inert = true;
    cancel.current?.focus();
    return () => { if (root) root.inert = inert; if (opener?.isConnected) opener.focus(); };
  }, []);
  return createPortal(<div className="dialog-overlay" onMouseDown={e => {
    if (!busy && e.target === e.currentTarget) onCancel();
  }}>
    <section className="dialog" role="dialog" aria-modal="true" aria-busy={busy}
      aria-labelledby={`${id}-title`} aria-describedby={`${id}-message`} onKeyDown={e => {
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); if (!busy) onCancel(); }
        if (e.key === "Tab") {
          const buttons = [...e.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
          if (!buttons.length) { e.preventDefault(); return; }
          if (e.shiftKey && document.activeElement === buttons[0]) { e.preventDefault(); buttons[buttons.length - 1]?.focus(); }
          else if (!e.shiftKey && document.activeElement === buttons[buttons.length - 1]) { e.preventDefault(); buttons[0].focus(); }
        }
      }}>
      <h3 id={`${id}-title`}>{tr("reset.title")}</h3>
      <p>{name}</p>
      <p id={`${id}-message`}>{tr("reset.message")}</p>
      {error && <p className="error" role="alert">{error}</p>}
      <div className="dialog-actions">
        <button ref={cancel} className="btn-secondary" disabled={busy} onClick={onCancel}>{tr("common.cancel")}</button>
        <button className="btn-primary" disabled={busy} onClick={onConfirm}>{tr("reset.confirm")}</button>
      </div>
    </section>
  </div>, document.body);
}
