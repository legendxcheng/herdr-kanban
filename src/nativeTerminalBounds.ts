import type { TerminalBounds } from './api';

export function terminalBounds(host: HTMLElement): TerminalBounds {
  const rect = host.getBoundingClientRect();
  const viewport_width = Math.max(1, window.innerWidth);
  const viewport_height = Math.max(1, window.innerHeight);
  const x = Math.max(0, Math.min(rect.left, viewport_width));
  const y = Math.max(0, Math.min(rect.top, viewport_height));
  return {
    x, y, width: Math.max(0, Math.min(rect.right, viewport_width) - x),
    height: Math.max(0, Math.min(rect.bottom, viewport_height) - y),
    viewport_width, viewport_height,
    // WebView occlusion/activation is not the native window's visibility.
    // Windows owns minimize/restore and stacking; React only reports overlays.
    visible: !document.querySelector('.dialog-overlay, [role="dialog"], .image-preview-overlay') && !host.closest('[inert]'),
  };
}
