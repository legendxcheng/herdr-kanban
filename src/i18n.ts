// Vite bundles every catalog; language resolution does not name supported locales.
const resources = import.meta.glob<Record<string, string>>("./locales/*.json", { eager: true, import: "default" });
const catalogs = Object.fromEntries(Object.entries(resources).map(([path, catalog]) => [path.split("/").pop()!.replace(/\.json$/, "").toLowerCase(), catalog]));
const fallback = catalogs["zh-cn"];

export function tr(key: string): string {
  const locale = document.documentElement.lang.toLowerCase();
  const language = locale.split("-")[0];
  const matching = Object.keys(catalogs).sort().find((name) => name.split("-")[0] === language);
  const catalog = catalogs[locale] ?? catalogs[matching ?? ""] ?? fallback;
  return catalog[key] ?? fallback[key] ?? key;
}

export function errorText(error: unknown): string {
  const message = String(error).replace(/^Error: /, "");
  const [key, ...details] = message.split(":");
  const translated = tr(key);
  return translated === key ? String(error) : `${translated}${details.length ? `: ${details.join(":")}` : ""}`;
}
