import { describe, expect, it } from "vitest";

import { HEADER_RUN_ID, HEADER_STEER_CODEC_MESSAGE_IDS } from "../../constants.js";
import { ErrorCode } from "../../errors.js";
import { MAX_STEER_IDS_PER_STAMP, RunSteerTracker } from "./run-steer-tracker.js";
import { readSteerStamp, SteerCoordinator } from "./steer.js";

const stamp = (runId: string, ids: readonly string[]) => ({
  [HEADER_RUN_ID]: runId,
  [HEADER_STEER_CODEC_MESSAGE_IDS]: JSON.stringify(ids),
});

describe("readSteerStamp", () => {
  it("reads a JSON array of ids", () => {
    expect(readSteerStamp(stamp("run-1", ["a", "b"]))).toEqual(["a", "b"]);
  });

  it("treats absent, empty, malformed, and non-array values as no stamp", () => {
    expect(readSteerStamp({})).toEqual([]);
    expect(readSteerStamp({ [HEADER_STEER_CODEC_MESSAGE_IDS]: "" })).toEqual([]);
    expect(readSteerStamp({ [HEADER_STEER_CODEC_MESSAGE_IDS]: "{not json" })).toEqual([]);
    expect(readSteerStamp({ [HEADER_STEER_CODEC_MESSAGE_IDS]: '"a"' })).toEqual([]);
  });

  it("drops non-string members rather than failing the whole stamp", () => {
    expect(readSteerStamp({ [HEADER_STEER_CODEC_MESSAGE_IDS]: '["a",2,null,"b"]' })).toEqual([
      "a",
      "b",
    ]);
  });
});

describe("SteerCoordinator", () => {
  it("resolves consumed when a stamp names the steer", async () => {
    const coordinator = new SteerCoordinator();
    const outcome = coordinator.track("run-1", "steer-1");

    coordinator.observe(stamp("run-1", ["steer-1"]));
    coordinator.settle("run-1", "complete", true);

    await expect(outcome).resolves.toEqual({ consumed: true, runTerminalReason: "complete" });
  });

  it("resolves not-consumed when the run ends without ever stamping it", async () => {
    const coordinator = new SteerCoordinator();
    const outcome = coordinator.track("run-1", "steer-1");

    coordinator.settle("run-1", "cancelled", true);

    await expect(outcome).resolves.toEqual({ consumed: false, runTerminalReason: "cancelled" });
  });

  it("accumulates the union across responses rather than replacing", async () => {
    const coordinator = new SteerCoordinator();
    const first = coordinator.track("run-1", "steer-1");
    const second = coordinator.track("run-1", "steer-2");

    // Each response reports only what its own step attempt drained.
    coordinator.observe(stamp("run-1", ["steer-1"]));
    coordinator.observe(stamp("run-1", ["steer-2"]));
    coordinator.settle("run-1", "complete", true);

    await expect(first).resolves.toMatchObject({ consumed: true });
    await expect(second).resolves.toMatchObject({ consumed: true });
  });

  it("keeps an unconsumed steer pending across suspend so a resume can claim it", async () => {
    const coordinator = new SteerCoordinator();
    const outcome = coordinator.track("run-1", "steer-1");
    let settled = false;
    void outcome.then(() => {
      settled = true;
    });

    coordinator.settle("run-1", undefined, false);
    await Promise.resolve();
    expect(settled).toBe(false);

    // The resumed run finally drains it.
    coordinator.observe(stamp("run-1", ["steer-1"]));
    coordinator.settle("run-1", "complete", true);

    await expect(outcome).resolves.toEqual({ consumed: true, runTerminalReason: "complete" });
  });

  it("settles a consumed steer at suspend, without a terminal reason", async () => {
    const coordinator = new SteerCoordinator();
    const outcome = coordinator.track("run-1", "steer-1");

    coordinator.observe(stamp("run-1", ["steer-1"]));
    coordinator.settle("run-1", undefined, false);

    await expect(outcome).resolves.toEqual({ consumed: true });
  });

  it("ignores stamps from other runs", async () => {
    const coordinator = new SteerCoordinator();
    const outcome = coordinator.track("run-1", "steer-1");

    coordinator.observe(stamp("run-2", ["steer-1"]));
    coordinator.settle("run-1", "complete", true);

    await expect(outcome).resolves.toMatchObject({ consumed: false });
  });

  it("ignores a stamp with no run id", () => {
    const coordinator = new SteerCoordinator();
    expect(() => {
      coordinator.observe({ [HEADER_STEER_CODEC_MESSAGE_IDS]: '["steer-1"]' });
    }).not.toThrow();
  });

  it("rejects everything still pending when drained", async () => {
    const coordinator = new SteerCoordinator();
    const outcome = coordinator.track("run-1", "steer-1");

    coordinator.drain("unable to steer; session closed");

    await expect(outcome).rejects.toMatchObject({ code: ErrorCode.SessionClosed });
  });
});

describe("RunSteerTracker", () => {
  it("reports pending input and claims it once", () => {
    const tracker = new RunSteerTracker();
    expect(tracker.hasPending).toBe(false);

    tracker.add("steer-1");
    expect(tracker.hasPending).toBe(true);
    expect(tracker.drain()).toEqual(["steer-1"]);
    expect(tracker.hasPending).toBe(false);
    expect(tracker.drain()).toEqual([]);
  });

  it("ignores a duplicate id whether pending or already drained", () => {
    const tracker = new RunSteerTracker();
    tracker.add("steer-1");
    tracker.add("steer-1");
    expect(tracker.drain()).toEqual(["steer-1"]);
    tracker.add("steer-1");
    expect(tracker.drain()).toEqual([]);
  });

  it("stamps every id drained so far, not just the latest batch", () => {
    const tracker = new RunSteerTracker();
    tracker.add("steer-1");
    tracker.drain();
    tracker.add("steer-2");
    tracker.drain();

    expect(JSON.parse(tracker.stampHeaders()[HEADER_STEER_CODEC_MESSAGE_IDS] ?? "[]")).toEqual([
      "steer-1",
      "steer-2",
    ]);
  });

  it("omits the header entirely when nothing was drained", () => {
    const tracker = new RunSteerTracker();
    tracker.add("steer-1");
    // Added but not drained: not yet visible to any inference.
    expect(tracker.stampHeaders()).toEqual({});
  });

  it("caps the stamp, keeping the newest ids", () => {
    const tracker = new RunSteerTracker();
    const total = MAX_STEER_IDS_PER_STAMP + 5;
    for (let index = 0; index < total; index += 1) {
      tracker.add(`steer-${index}`);
    }
    tracker.drain();

    const ids = tracker.stampIds;
    expect(ids).toHaveLength(MAX_STEER_IDS_PER_STAMP);
    expect(ids.at(-1)).toBe(`steer-${total - 1}`);
    expect(ids).not.toContain("steer-0");
  });

  it("keeps the capped stamp inside the server's 2 KiB ceiling for the key", () => {
    const tracker = new RunSteerTracker();
    for (let index = 0; index < MAX_STEER_IDS_PER_STAMP; index += 1) {
      tracker.add(`msg_00000000-0000-4000-8000-${String(index).padStart(12, "0")}`);
    }
    tracker.drain();

    const value = tracker.stampHeaders()[HEADER_STEER_CODEC_MESSAGE_IDS] ?? "";
    // Overflow would reject the whole assistant output publish, not just the
    // stamp, so this bound is a correctness property rather than a nicety.
    expect(value.length).toBeLessThanOrEqual(2 * 1024);
  });

  it("forgets state on clear", () => {
    const tracker = new RunSteerTracker();
    tracker.add("steer-1");
    tracker.drain();
    tracker.clear();
    expect(tracker.stampHeaders()).toEqual({});
    expect(tracker.hasPending).toBe(false);
  });
});
