import { useCallback, useEffect, useRef } from "react";
import * as api from "./api";

/** A lazy, form-local lease. Cleanup also covers an open response arriving late. */
export function useImageDraft(projectId: string, taskId?: string) {
  const session = useRef<{ closed: boolean; ready?: Promise<string> }>({ closed: false });
  useEffect(() => {
    const current = { closed: false } as { closed: boolean; ready?: Promise<string> };
    session.current = current;
    return () => {
      current.closed = true;
      void current.ready?.then(id => api.closeImageDraft(id)).catch(() => {});
    };
  }, [projectId, taskId]);
  const getId = useCallback(async () => {
    const current = session.current;
    if (current.closed) throw new Error("image.draft_closed");
    current.ready ??= api.openImageDraft(projectId, taskId);
    const ready = current.ready;
    let id: string;
    try { id = await ready; }
    catch (reason) { if (current.ready === ready) current.ready = undefined; throw reason; }
    if (current.closed) throw new Error("image.draft_closed");
    return id;
  }, [projectId, taskId]);
  const existingId = useCallback(() => session.current.ready, []);
  return { getId, existingId };
}
