import { describe, expect, it } from "vitest";

import { reorderUnrespondedSteers, unrespondedSteerIds } from "./steer-ordering.js";

const user = (id: string) => ({ id, role: "user" });
const assistant = (id: string) => ({ id, role: "assistant" });
const tool = (id: string) => ({ id, role: "tool" });

describe("unrespondedSteerIds", () => {
  it("treats a steer with assistant output after it as responded", () => {
    const messages = [user("u1"), assistant("a1"), user("s1"), assistant("a2")];
    expect(unrespondedSteerIds(messages, ["s1"])).toEqual([]);
  });

  it("treats a trailing steer as unresponded", () => {
    const messages = [user("u1"), assistant("a1"), user("s1")];
    expect(unrespondedSteerIds(messages, ["s1"])).toEqual(["s1"]);
  });

  it("counts tool output as a response", () => {
    const messages = [user("u1"), user("s1"), tool("t1")];
    expect(unrespondedSteerIds(messages, ["s1"])).toEqual([]);
  });

  it("reports only the steers that are still unanswered", () => {
    const messages = [user("s1"), assistant("a1"), user("s2"), user("s3")];
    expect(unrespondedSteerIds(messages, ["s1", "s2", "s3"])).toEqual(["s2", "s3"]);
  });

  it("ignores ids that are not present", () => {
    expect(unrespondedSteerIds([user("u1")], ["missing"])).toEqual([]);
    expect(unrespondedSteerIds([user("u1")], [])).toEqual([]);
  });
});

describe("reorderUnrespondedSteers", () => {
  it("moves a buried steer to the tail so the prompt ends on it", () => {
    const messages = [user("u1"), assistant("a1"), user("s1"), assistant("a2")];

    const ordered = reorderUnrespondedSteers(messages, { unrespondedSteerIds: ["s1"] });

    expect(ordered.map((message) => message.id)).toEqual(["u1", "a1", "a2", "s1"]);
    expect(ordered.at(-1)?.role).toBe("user");
  });

  it("preserves the relative order of several moved steers", () => {
    const messages = [user("s1"), assistant("a1"), user("s2"), assistant("a2")];

    const ordered = reorderUnrespondedSteers(messages, { unrespondedSteerIds: ["s1", "s2"] });

    expect(ordered.map((message) => message.id)).toEqual(["a1", "a2", "s1", "s2"]);
  });

  it("does not reorder anything else", () => {
    const messages = [user("u1"), assistant("a1"), user("u2"), assistant("a2"), user("s1")];

    const ordered = reorderUnrespondedSteers(messages, { unrespondedSteerIds: ["s1"] });

    expect(ordered.map((message) => message.id)).toEqual(["u1", "a1", "u2", "a2", "s1"]);
  });

  it("returns the same array when the steer is already at the tail", () => {
    const messages = [user("u1"), assistant("a1"), user("s1")];

    // Same reference: callers may run this on every prompt build.
    expect(reorderUnrespondedSteers(messages, { unrespondedSteerIds: ["s1"] })).toBe(messages);
  });

  it("returns the same array when there is nothing to move", () => {
    const messages = [user("u1"), assistant("a1")];

    expect(reorderUnrespondedSteers(messages, { unrespondedSteerIds: [] })).toBe(messages);
    expect(reorderUnrespondedSteers(messages, { unrespondedSteerIds: ["absent"] })).toBe(messages);
    expect(reorderUnrespondedSteers([], { unrespondedSteerIds: ["s1"] })).toEqual([]);
  });

  it("composes with unrespondedSteerIds on a realistic mid-run steer", () => {
    // The wire order: the steer landed while the run was still streaming.
    const messages = [user("u1"), assistant("a1"), user("s1"), assistant("a2"), user("s2")];
    const pending = unrespondedSteerIds(messages, ["s1", "s2"]);
    expect(pending).toEqual(["s2"]);

    // s1 was answered by a2, so only s2 moves — and it was already last.
    expect(reorderUnrespondedSteers(messages, { unrespondedSteerIds: pending })).toBe(messages);
  });
});
