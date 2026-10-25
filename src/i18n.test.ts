// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { tr } from "./i18n";
import zh from "./locales/zh-CN.json";
import en from "./locales/en-US.json";

afterEach(() => { document.documentElement.lang = "zh-CN"; });
it("resolves a language variant and falls back to Chinese for unknown languages", () => {
  document.documentElement.lang = "en-GB";
  expect(tr("terminal.connected")).toBe(en["terminal.connected"]);
  document.documentElement.lang = "missing";
  expect(tr("terminal.connected")).toBe(zh["terminal.connected"]);
  expect(tr("missing.key")).toBe("missing.key");
});
it("ships matching keys and placeholders in the complete catalogs", () => {
  expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort());
  for (const key of Object.keys(zh) as Array<keyof typeof zh>) {
    expect(en[key].match(/\{[^}]+\}/g) ?? []).toEqual(zh[key].match(/\{[^}]+\}/g) ?? []);
  }
});
