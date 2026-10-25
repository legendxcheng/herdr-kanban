import type { Terminal } from "@xterm/xterm";
import { scrollTerminal, type TerminalCell, type TerminalScroll } from "./api";

/** A connection-scoped wheel controller. Herdr, not xterm's local buffer, owns scrolling. */
export function attachTerminalScroll(host: HTMLElement, terminal: Terminal, connectionId: string, fail: (error: unknown) => void) {
  let active = true;
  let enabled = false;
  let remainder = 0;
  let queue: TerminalScroll[] = [];
  let busy = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function flush() {
    if (!active || !enabled || busy) return;
    busy = true;
    try {
      while (active && enabled && queue.length) {
        try { await scrollTerminal(connectionId, queue.shift()!); }
        catch (error) { if (error !== "terminal.scroll_stale_size") throw error; }
      }
    } catch (error) { if (active) fail(error); }
    finally { busy = false; }
  }
  function enqueue(item: TerminalScroll) {
    const last = queue[queue.length - 1];
    if (last && last.direction === item.direction && last.modifiers === item.modifiers && last.column === item.column && last.row === item.row) last.lines = Math.min(24, last.lines + item.lines);
    else if (queue.length < 16) queue.push(item);
    // Do not postpone dispatch indefinitely during a continuous touchpad gesture.
    if (!timer) timer = setTimeout(() => { timer = undefined; void flush(); }, 8);
  }
  function cell(event: MouseEvent): TerminalCell | null {
    const grid = host.querySelector(".xterm-screen")?.getBoundingClientRect();
    if (!grid || grid.width <= 0 || grid.height <= 0 || event.clientX < grid.left || event.clientX >= grid.right || event.clientY < grid.top || event.clientY >= grid.bottom) return null;
    return { column: Math.floor((event.clientX - grid.left) * terminal.cols / grid.width),
      row: Math.floor((event.clientY - grid.top) * terminal.rows / grid.height), cols: terminal.cols, rows: terminal.rows,
      modifiers: (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) };
  }
  function wheel(event: WheelEvent) {
    const position = cell(event);
    if (!position) return;
    event.preventDefault(); event.stopImmediatePropagation();
    if (!enabled || !Number.isFinite(event.deltaY) || event.deltaY === 0) return;
    // One typical wheel notch (3 lines / 120 CSS px) scrolls six lines.
    // Pixel normalization is independent of font size; retain fractional touchpad motion.
    const delta = event.deltaY * (event.deltaMode === 1 ? 2 : event.deltaMode === 2 ? terminal.rows : 1 / 20);
    if (Math.sign(delta) !== Math.sign(remainder)) remainder = 0;
    remainder += delta;
    const lines = Math.min(24, Math.trunc(Math.abs(remainder)));
    if (!lines) return;
    const direction = remainder < 0 ? "up" : "down";
    remainder = (Math.abs(remainder) % 1) * Math.sign(remainder);
    enqueue({ direction, lines, ...position });
  }
  host.addEventListener("wheel", wheel, { capture: true, passive: false });
  return {
    enable() { if (active) enabled = true; },
    pause() { enabled = false; queue = []; remainder = 0; clearTimeout(timer); timer = undefined; },
    dispose() {
      active = false; enabled = false; queue = []; clearTimeout(timer);
      host.removeEventListener("wheel", wheel, true);
    },
  };
}
