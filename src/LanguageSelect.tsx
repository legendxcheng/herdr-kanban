import { setLocale, tr, useLocale, type Locale } from "./i18n";

export function LanguageSelect() {
  const locale = useLocale();
  return <label className="language-select">
    <span>{tr("language.label")}</span>
    <select value={locale} onChange={event => setLocale(event.target.value as Locale)}>
      <option value="zh-CN">简体中文</option>
      <option value="en-US">English</option>
    </select>
  </label>;
}
