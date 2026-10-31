// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from "vitest";

beforeEach(() => { localStorage.clear(); vi.resetModules(); });
afterEach(() => { vi.restoreAllMocks(); });

it("persists independent Project choices across module reloads", async () => {
  let preferences = await import("./workspaceTaskSelection");
  preferences.rememberWorkspaceTaskSelection("local-project", "task-a");
  preferences.rememberWorkspaceTaskSelection("remote-project", "task-b");
  vi.resetModules();
  preferences = await import("./workspaceTaskSelection");
  expect(preferences.readWorkspaceTaskSelection("remote-project")).toBe("task-b");
  expect(preferences.readWorkspaceTaskSelection("local-project")).toBe("task-a");
  preferences.rememberWorkspaceTaskSelection("local-project", null);
  vi.resetModules();
  preferences = await import("./workspaceTaskSelection");
  expect(preferences.readWorkspaceTaskSelection("local-project")).toBeNull();
  expect(preferences.readWorkspaceTaskSelection("remote-project")).toBe("task-b");
});

it.each(["broken json", "{}", "[]", "42", '"  "'])("ignores malformed saved choices: %s", async value => {
  localStorage.setItem("kanban.workspaceTaskSelection.v1:project", value);
  const preferences = await import("./workspaceTaskSelection");
  expect(preferences.readWorkspaceTaskSelection("project")).toBeNull();
});

it("keeps the latest session choice when writes fail but old storage remains readable", async () => {
  const preferences = await import("./workspaceTaskSelection");
  preferences.rememberWorkspaceTaskSelection("project", "old-task");
  const writes = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
  preferences.rememberWorkspaceTaskSelection("project", "latest-task");
  expect(preferences.readWorkspaceTaskSelection("project")).toBe("latest-task");
  preferences.rememberWorkspaceTaskSelection("project", null);
  expect(preferences.readWorkspaceTaskSelection("project")).toBeNull();
  writes.mockRestore();
  preferences.rememberWorkspaceTaskSelection("project", "persisted-again");
  vi.resetModules();
  expect((await import("./workspaceTaskSelection")).readWorkspaceTaskSelection("project")).toBe("persisted-again");
});

it("allows session choices when storage reads and writes are unavailable", async () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
  const preferences = await import("./workspaceTaskSelection");
  expect(preferences.readWorkspaceTaskSelection("project")).toBeNull();
  preferences.rememberWorkspaceTaskSelection("project", "task");
  expect(preferences.readWorkspaceTaskSelection("project")).toBe("task");
});
