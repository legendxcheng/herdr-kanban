import { useEffect, useId, useState } from "react";
import type { CodexCatalog, CodexSettings } from "./types";
import * as api from "./api";
import { errorText, tr } from "./i18n";

export const defaultCodexSettings: CodexSettings = { model: null, effort: null };
export function sameCodexSettings(a: CodexSettings, b: CodexSettings) { return a.model === b.model && a.effort === b.effort; }

export function CodexSettingsSelect({ projectId, remote, value, onChange, disabled, locked, onValid }: {
  projectId: string; remote: boolean; value: CodexSettings; onChange: (value: CodexSettings) => void;
  disabled: boolean; locked: boolean; onValid: (valid: boolean) => void;
}) {
  const id = useId();
  const [result, setResult] = useState<{ projectId: string; catalog?: CodexCatalog; error?: string }>();
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let active = true;
    setLoading(true);
    api.codexModelCatalog(projectId, refresh > 0).then(catalog => {
      if (active) setResult({ projectId, catalog });
    }).catch(error => { if (active) setResult({ projectId, error: String(error) }); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [projectId, refresh]);
  const catalog = result?.projectId === projectId ? result.catalog : undefined;
  const error = result?.projectId === projectId ? result.error ?? catalog?.error : undefined;
  const model = catalog?.models.find(m => m.id === value.model);
  const invalid = !locked && ((!!catalog && !!value.model && !model) || (!!value.effort && (!value.model || (!!model && !model.efforts.includes(value.effort)))));
  useEffect(() => { onValid(!invalid); }, [invalid, onValid]);
  return <div className="codex-settings">
    <div className="codex-settings-fields">
      <div><label className="field-label" htmlFor={`${id}-model`}>{tr("model.label")}</label>
        <select id={`${id}-model`} className="field-input" value={value.model ?? ""} disabled={disabled || locked}
          onChange={e => onChange({ ...value, model: e.target.value || null })}>
          <option value="">{tr("model.default")}</option>
          {value.model && !model && <option value={value.model}>{value.model}</option>}
          {catalog?.models.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}
        </select>
      </div>
      <div><label className="field-label" htmlFor={`${id}-effort`}>{tr("model.effort")}</label>
        <select id={`${id}-effort`} className="field-input" value={value.effort ?? ""} disabled={disabled || locked || (!value.model && !value.effort)}
          onChange={e => onChange({ ...value, effort: e.target.value || null })}>
          <option value="">{tr("model.default")}</option>
          {value.effort && !model?.efforts.includes(value.effort) && <option value={value.effort}>{value.effort}</option>}
          {model?.efforts.map(effort => <option key={effort} value={effort}>{effort}</option>)}
        </select>
      </div>
    </div>
    {locked ? <p className="field-hint">{tr("model.locked_hint")}</p> : <>
      <div className="codex-catalog-status"><span aria-live="polite">{tr(loading ? "model.loading" : catalog?.cached ? remote ? "model.remote_cached" : "model.cached" : "model.catalog_hint")}</span>
        <button type="button" className="btn-ghost" disabled={loading || disabled} onClick={() => setRefresh(n => n + 1)}>{tr("model.refresh")}</button>
      </div>
      {error && <p className="field-hint">{errorText(error)} · {tr(remote ? "model.remote_catalog_fallback" : "model.catalog_fallback")}</p>}
      {invalid && <p role="alert" className="error">{tr("model.invalid_choice")}</p>}
    </>}
  </div>;
}
