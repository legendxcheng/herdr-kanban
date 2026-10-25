// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { tr, initializeLocale, setLocale, localeStorageKey, errorText } from "./i18n";
import zh from "./locales/zh-CN.json";
import en from "./locales/en-US.json";

afterEach(() => { vi.restoreAllMocks(); localStorage.clear(); document.documentElement.lang = "zh-CN"; });
it("resolves language variants and falls back to English for other languages", () => {
  document.documentElement.lang = "en-GB";
  expect(tr("terminal.connected")).toBe(en["terminal.connected"]);
  document.documentElement.lang = "missing";
  expect(tr("terminal.connected")).toBe(en["terminal.connected"]);
  document.documentElement.lang = "zh-TW";
  expect(tr("terminal.connected")).toBe(zh["terminal.connected"]);
  expect(tr("missing.key")).toBe("missing.key");
});
it("starts in the system language, persists an explicit choice and restores it on next launch", () => {
  vi.spyOn(navigator, "language", "get").mockReturnValue("zh-HK");
  initializeLocale();
  expect(document.documentElement.lang).toBe("zh-CN");
  setLocale("en-US");
  expect(localStorage.getItem(localeStorageKey)).toBe("en-US");
  document.documentElement.lang = "zh-CN";
  initializeLocale();
  expect(document.documentElement.lang).toBe("en-US");
});
it("ignores invalid preferences and handles unavailable storage without blocking the UI", () => {
  vi.spyOn(navigator, "language", "get").mockReturnValue("fr-FR");
  localStorage.setItem(localeStorageKey, "bad-value");
  initializeLocale();
  expect(document.documentElement.lang).toBe("en-US");
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
  initializeLocale();
  setLocale("zh-CN");
  expect(document.documentElement.lang).toBe("zh-CN");
});
it("keeps user text literal during interpolation and preserves external error diagnostics", () => {
  setLocale("en-US");
  expect(tr("task.delete_confirm", { name: "中文 $& {name}" })).toBe('Delete task “中文 $& {name}”?');
  expect(errorText(new Error("herdr.failed: C:\\任务:detail"))).toBe("Herdr request failed: C:\\任务:detail");
  expect(errorText("raw diagnostic")).toBe("raw diagnostic");
});
it("ships matching keys and placeholders in the complete catalogs", () => {
  expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort());
  for (const key of Object.keys(zh) as Array<keyof typeof zh>) {
    expect(en[key].match(/\{[^}]+\}/g) ?? []).toEqual(zh[key].match(/\{[^}]+\}/g) ?? []);
  }
});
