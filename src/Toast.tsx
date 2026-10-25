import { useState, useCallback, useRef } from "react";

export type ToastKind = "success" | "error" | "info";

interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
}

let _notify: ((msg: string, kind?: ToastKind) => void) | null = null;

/** Call from anywhere to show a toast. */
export function notify(message: string, kind: ToastKind = "success") {
  _notify?.(message, kind);
}

export function ToastContainer() {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const counter = useRef(0);

  const add = useCallback((message: string, kind: ToastKind = "success") => {
    const id = ++counter.current;
    setToasts((prev) => [...prev, { id, message, kind }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 3000);
  }, []);

  // register global handle on first render
  _notify = add;

  if (toasts.length === 0) return null;

  const icons: Record<ToastKind, string> = {
    success: "✓",
    error: "✕",
    info: "ℹ",
  };

  return (
    <div className="toast-container" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          <span>{icons[t.kind]}</span>
          <span>{t.message}</span>
        </div>
      ))}
    </div>
  );
}
