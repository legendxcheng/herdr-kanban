import { useState, useSyncExternalStore } from "react";
import zh from "./locales/zh-CN.json";
import en from "./locales/en-US.json";

export type Locale = "zh-CN" | "en-US";
export const localeStorageKey = "kanban.locale";
const catalogs: Record<Locale, Record<string, string>> = { "zh-CN": zh, "en-US": en };
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

export function resolveLocale(language: string): Locale {
  return /^zh(?:-|$)/i.test(language) ? "zh-CN" : "en-US";
}

export function getLocale(): Locale {
  return resolveLocale(document.documentElement.lang || "zh-CN");
}

function applyLocale(locale: Locale) {
  document.documentElement.lang = locale;
  listeners.forEach(listener => listener());
}

/** Run before mounting React, so the first frame already uses the saved language. */
export function initializeLocale() {
  let saved: string | null = null;
  try { saved = localStorage.getItem(localeStorageKey); } catch { /* Preferences are optional. */ }
  applyLocale(saved === "zh-CN" || saved === "en-US" ? saved : resolveLocale(navigator.language));
}

export function setLocale(locale: Locale) {
  const supported = resolveLocale(locale);
  try { localStorage.setItem(localeStorageKey, supported); } catch { /* Switching still works for this session. */ }
  applyLocale(supported);
}

export function useLocale() {
  return useSyncExternalStore(subscribe, getLocale);
}

export function tr(key: string, params: Record<string, string | number> = {}): string {
  const text = catalogs[getLocale()][key] ?? catalogs["en-US"][key] ?? key;
  return text.replace(/\{(\w+)\}/g, (placeholder, name: string) => String(params[name] ?? placeholder));
}

export function formatDateTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString(getLocale());
}

export function errorText(error: unknown): string {
  const message = String(error).replace(/^Error: /, "");
  const [key, ...details] = message.split(":");
  const translated = tr(key);
  return translated === key ? String(error) : `${translated}${details.length ? `: ${details.join(":").trimStart()}` : ""}`;
}

/** Keep keys and diagnostics in state; translate only when rendering. */
export type Message = string | { key: string; params?: Record<string, string | number> }
  | { error: string; prefix?: string };

export function messageText(message: Message): string {
  if (typeof message === "string") return errorText(message);
  if ("key" in message) return tr(message.key, message.params);
  return `${message.prefix ? `${tr(message.prefix)} ` : ""}${errorText(message.error)}`;
}

export function useLocalizedMessage(): [string, React.Dispatch<React.SetStateAction<Message>>] {
  useLocale();
  const [message, setMessage] = useState<Message>("");
  return [messageText(message), setMessage];
}
