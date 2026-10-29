// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";

beforeEach(() => { vi.restoreAllMocks(); localStorage.clear(); vi.resetModules(); });

it("retains saved identities across reload, reordered discovery and newly added identities", async () => {
  let preferences = await import("./identityColors");
  const machine = preferences.identityColor("machine", "remote-a");
  const workspace = preferences.identityColor("workspace", "project-a");
  const otherWorkspace = preferences.identityColor("workspace", "project-b");
  expect(otherWorkspace).not.toBe(workspace);
  expect(preferences.identityColor("machine", "remote-b")).not.toBe(machine);
  vi.resetModules();
  preferences = await import("./identityColors");
  preferences.identityColor("workspace", "new-project");
  expect(preferences.identityColor("workspace", "project-b")).toBe(otherWorkspace);
  expect(preferences.identityColor("workspace", "project-a")).toBe(workspace);
  expect(preferences.identityColor("machine", "remote-a")).toBe(machine);
});

it("keeps machine and workspace namespaces independent and extends beyond the initial palette", async () => {
  const { identityColor } = await import("./identityColors");
  expect(identityColor("machine", "same-id")).not.toBe(identityColor("workspace", "same-id"));
  const values = Array.from({ length: 30 }, (_, i) => identityColor("workspace", `project-${i}`));
  expect(new Set(values).size).toBe(values.length);
});

it.each(["broken json", '{"machine":{"good":"#93c5fd","bad":"url(invalid)"},"workspace":null}'])("recovers from invalid preferences: %s", async saved => {
  const { identityColor, identityColorsStorageKey } = await import("./identityColors");
  localStorage.setItem(identityColorsStorageKey, saved);
  expect(identityColor("machine", "bad")).toMatch(/^#[\da-f]{6}$/i);
  expect(identityColor("workspace", "project")).toMatch(/^#[\da-f]{6}$/i);
});

it("keeps session colors stable when browser storage is unavailable", async () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("unavailable"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("unavailable"); });
  const { identityColor } = await import("./identityColors");
  const original = identityColor("workspace", "project");
  identityColor("workspace", "another-project");
  expect(identityColor("workspace", "project")).toBe(original);
});
