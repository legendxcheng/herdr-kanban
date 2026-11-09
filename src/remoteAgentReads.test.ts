// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { readRemoteAgents } from "./remoteAgentReads";
import * as api from "./api";
vi.mock("./api", () => ({ herdrAgentSnapshots: vi.fn() }));
afterEach(() => vi.restoreAllMocks());

describe("remote agent batches", () => {
  it("combines a machine's projects in the same poll without a timer", async () => {
    const snapshot = { agents: [], failures: [], age_ms: 0 };
    vi.mocked(api.herdrAgentSnapshots).mockResolvedValue({ a: { snapshot, error: null }, b: { snapshot, error: null } });
    const results = await Promise.all([readRemoteAgents("m", "a"), readRemoteAgents("m", "b"), readRemoteAgents("m", "a")]);
    expect(api.herdrAgentSnapshots).toHaveBeenCalledTimes(1);
    expect(api.herdrAgentSnapshots).toHaveBeenCalledWith(["a", "b"]);
    expect(results).toEqual([snapshot, snapshot, snapshot]);
  });
  it("isolates machine batches and individual project failures", async () => {
    vi.mocked(api.herdrAgentSnapshots).mockReset().mockImplementation(async ids => Object.fromEntries(ids.map(id => [id,
      id === "bad" ? { snapshot: null, error: { code: "workspace.missing" } as never } : { snapshot: { agents: [], failures: [] }, error: null }])));
    const results = await Promise.allSettled([readRemoteAgents("m1", "good"), readRemoteAgents("m1", "bad"), readRemoteAgents("m2", "other")]);
    expect(api.herdrAgentSnapshots).toHaveBeenCalledTimes(2);
    expect(results.map(r => r.status)).toEqual(["fulfilled", "rejected", "fulfilled"]);
  });
  it("does not start SSH work when the window becomes hidden", async () => {
    vi.mocked(api.herdrAgentSnapshots).mockClear();
    const pending = readRemoteAgents("m", "a");
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    expect(await pending).toBeNull();
    expect(api.herdrAgentSnapshots).not.toHaveBeenCalled();
  });
});
