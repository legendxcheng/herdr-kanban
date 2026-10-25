import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { tr, useLocale } from "./i18n";

export function ImagePreview({ source, name, opener, onClose }: {
  source: string; name: string; opener: HTMLElement | null; onClose: () => void;
}) {
  useLocale();
  const closeButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const dialog = opener?.closest<HTMLElement>('[role="dialog"]');
    const wasInert = dialog?.inert ?? false;
    if (dialog) dialog.inert = true;
    closeButton.current?.focus();
    return () => {
      if (dialog) dialog.inert = wasInert;
      if (opener?.isConnected) opener.focus();
    };
  }, [opener]);
  return createPortal(<div className="image-preview-overlay" role="dialog" aria-modal="true"
    aria-label={`${tr("image.preview")}: ${name}`} onMouseDown={event => {
      event.stopPropagation(); if (event.target === event.currentTarget) onClose();
    }} onKeyDown={event => {
      event.stopPropagation();
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
      if (event.key === "Tab") { event.preventDefault(); closeButton.current?.focus(); }
    }}>
    <button ref={closeButton} type="button" className="image-preview-close" aria-label={tr("image.close_preview")} onClick={onClose}>×</button>
    <img className="image-preview-full" src={source} alt={name} />
    <span className="image-preview-name">{name}</span>
  </div>, document.body);
}
