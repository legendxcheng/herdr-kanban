import type { TaskPriority } from "./types";
import { tr } from "./i18n";

export function PrioritySelect({ value, onChange, label, id, disabled }: {
  value: TaskPriority; onChange: (priority: TaskPriority) => void;
  label: string; id?: string; disabled?: boolean;
}) {
  return <select id={id} className="field-input priority-select" aria-label={label} value={value}
    disabled={disabled} onChange={(event) => onChange(event.target.value as TaskPriority)}>
    {(["normal", "high", "urgent"] as const).map((priority) =>
      <option key={priority} value={priority}>{tr(`priority.${priority}`)}</option>)}
  </select>;
}
