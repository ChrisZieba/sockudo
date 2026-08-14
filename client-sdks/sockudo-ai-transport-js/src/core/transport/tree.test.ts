import fc from "fast-check";
import { describe, expect, it } from "vitest";

import {
  HEADER_CODEC_MESSAGE_ID,
  HEADER_FORK_OF,
  HEADER_INPUT_CLIENT_ID,
  HEADER_INVOCATION_ID,
  HEADER_MSG_REGENERATE,
  HEADER_PARENT,
  HEADER_ROLE,
  HEADER_RUN_CLIENT_ID,
  HEADER_RUN_CONTINUE,
  HEADER_RUN_ID,
  HEADER_RUN_REASON,
  HEADER_SUPERSEDES,
} from "../../constants.js";
import type { HeaderMap } from "../../utils.js";
import { createTree } from "./tree.js";
import type { Tree, TreeSerial, RunEndReason, RunNode } from "./tree.js";
import type { DecodedEvent, Reducer } from "../codec/index.js";

const REPLAY_BUDGET_MS = process.env.CI === "true" ? 750 : 250;

describe("conversation tree", () => {
  it("guards against phantom turns from metadata-only messages", () => {
    const tree = createTestTree();

    expect(tree.applyMessage([], headers({ runId: "turn-1" }), 1)).toBeUndefined();
    expect(tree.getRunNode("turn-1")).toBeUndefined();
    expect(tree.structuralVersion).toBe(0);
  });

  it("routes fresh input and assistant output, promotes serials, and tracks active suspended turns", () => {
    const tree = createTestTree();

    const user = tree.applyMessage(
      [decoded("user", "msg-user", 2)],
      headers({
        runId: "turn-1",
        codecMessageId: "msg-user",
        role: "user",
        inputClientId: "client-1",
      }),
      2,
    );
    tree.applyRunLifecycle({
      type: "start",
      headers: headers({
        runId: "turn-1",
        runClientId: "client-1",
        invocationId: "inv-1",
      }),
      serial: 1,
    });
    tree.applyMessage(
      [decoded("assistant", "msg-assistant", 3)],
      headers({
        runId: "turn-1",
        codecMessageId: "msg-assistant",
        role: "assistant",
      }),
      3,
    );
    tree.applyRunLifecycle({
      type: "end",
      headers: headers({
        runId: "turn-1",
        runReason: "suspended",
      }),
      serial: 4,
    });

    expect(user?.runId).toBe("turn-1");
    expect(tree.getNodeByCodecMessageId("msg-assistant")?.projection.events).toEqual([
      "2:msg-user:user",
      "3:msg-assistant:assistant",
    ]);
    expect(tree.getRunNode("turn-1")).toMatchObject({
      startSerial: 1,
      endSerial: 4,
      status: "suspended",
      invocationId: "inv-1",
      clientId: "client-1",
    });
    expect(Array.from(tree.getActiveRunIds().get("client-1") ?? [])).toEqual(["turn-1"]);
  });

  it("tolerates assistant output before turn-start and backfills parent metadata", () => {
    const tree = createTestTree();

    tree.applyMessage(
      [decoded("parent", "msg-parent", 1)],
      headers({ runId: "parent", codecMessageId: "msg-parent" }),
      1,
    );
    tree.applyMessage(
      [decoded("child", "msg-child", 3)],
      headers({
        runId: "child",
        codecMessageId: "msg-child",
        parent: "msg-parent",
      }),
      3,
    );
    tree.applyRunLifecycle({
      type: "start",
      headers: headers({
        runId: "child",
        parent: "msg-parent",
        invocationId: "inv-child",
      }),
      serial: 2,
    });

    expect(tree.getRunNode("child")).toMatchObject({
      parentRunId: "parent",
      startSerial: 2,
      invocationId: "inv-child",
    });
  });

  it("routes continuation folds by codec message id and avoids self-cycle graph backfill", () => {
    const tree = createTestTree();

    tree.applyMessage(
      [decoded("start", "msg-1", 1)],
      headers({ runId: "turn-1", codecMessageId: "msg-1" }),
      1,
    );
    tree.applyMessage(
      [decoded("continue", "msg-1", 2)],
      headers({
        runId: "turn-other",
        codecMessageId: "msg-1",
        runContinue: true,
        forkOf: "msg-1",
      }),
      2,
    );
    tree.applyRunLifecycle({
      type: "start",
      headers: headers({
        runId: "turn-1",
        codecMessageId: "msg-1",
        runContinue: true,
        invocationId: "inv-continue",
        forkOf: "msg-1",
      }),
      serial: 3,
    });

    expect(tree.getRunNode("turn-other")).toBeUndefined();
    expect(tree.getRunNode("turn-1")?.projection.events).toEqual([
      "1:msg-1:start",
      "2:msg-1:continue",
    ]);
    expect(tree.getRunNode("turn-1")?.forkOf).toBeUndefined();
    expect(tree.getLatestContinuationInvocation("turn-1")).toBe("inv-continue");
  });

  it("computes fork siblings transitively and excludes descendants", () => {
    const tree = createTestTree();

    createRunNode(tree, "parent", "msg-parent", 1);
    createRunNode(tree, "a", "msg-a", 2, { parent: "msg-parent" });
    createRunNode(tree, "b", "msg-b", 3, { forkOf: "msg-a" });
    createRunNode(tree, "c", "msg-c", 4, { forkOf: "msg-b" });
    createRunNode(tree, "descendant", "msg-descendant", 5, { parent: "msg-a" });

    expect(tree.getSiblingNodes("msg-a").map((node) => node.runId)).toEqual(["a", "b", "c"]);
    expect(tree.hasSiblingNodes("msg-c")).toBe(true);
    expect(tree.getSiblingNodes("msg-descendant").map((node) => node.runId)).toEqual([
      "descendant",
    ]);
  });

  it("orders regenerate groups owner first and backfills unresolved owners", () => {
    const tree = createTestTree();

    createRunNode(tree, "regen", "msg-regen", 2, {
      regenerates: "msg-owner",
    });
    createRunNode(tree, "owner", "msg-owner", 1);

    expect(tree.getRegenerateGroup("msg-owner").map((node) => node.runId)).toEqual([
      "owner",
      "regen",
    ]);
    expect(tree.getRunNode("regen")).toMatchObject({
      regeneratesCodecMessageId: "msg-owner",
    });
  });

  it("orders newly inserted turns by server serial even when delivery is out of order", () => {
    const tree = createTestTree();

    createRunNode(tree, "late", "msg-late", 20);
    createRunNode(tree, "early", "msg-early", 10);

    expect(tree.getRunNodes().map((node) => node.runId)).toEqual(["early", "late"]);
  });

  it("hides superseded trunks while retaining lookup and visible concurrent forks", () => {
    const tree = createTestTree();

    createRunNode(tree, "parent", "msg-parent", 1);
    createRunNode(tree, "trunk", "msg-trunk", 2, { parent: "msg-parent" });
    createRunNode(tree, "fork-a", "msg-fork-a", 3, {
      role: "assistant",
      forkOf: "msg-trunk",
      supersedes: "trunk",
    });
    createRunNode(tree, "fork-b", "msg-fork-b", 4, {
      role: "assistant",
      forkOf: "msg-trunk",
      supersedes: "trunk",
    });

    expect(tree.getRunNode("trunk")?.runId).toBe("trunk");
    expect(tree.getRunNodes().map((node) => node.runId)).toEqual(["parent", "fork-a", "fork-b"]);
    expect(tree.getSiblingNodes("fork-a").map((node) => node.runId)).toEqual(["fork-a", "fork-b"]);
  });

  it("applies supersession before the trunk arrives and restores it when the fork is deleted", () => {
    const tree = createTestTree();

    createRunNode(tree, "fork", "msg-shared", 2, {
      role: "assistant",
      supersedes: "trunk",
    });
    createRunNode(tree, "trunk", "msg-shared", 1);

    expect(tree.getRunNodes().map((node) => node.runId)).toEqual(["fork"]);
    expect(tree.getNodeByCodecMessageId("msg-shared")?.runId).toBe("fork");
    tree.delete("msg-shared");
    expect(tree.getRunNodes().map((node) => node.runId)).toEqual(["trunk"]);
    expect(tree.getNodeByCodecMessageId("msg-shared")?.runId).toBe("trunk");
  });

  it("ignores supersedes on user inputs and suspended-run continuations", () => {
    const tree = createTestTree();

    createRunNode(tree, "trunk", "msg-trunk", 1);
    createRunNode(tree, "user", "msg-user", 2, {
      role: "user",
      supersedes: "trunk",
    });
    createRunNode(tree, "resume", "msg-resume", 3, {
      role: "assistant",
      runContinue: true,
      supersedes: "trunk",
    });

    expect(tree.getRunNodes().map((node) => node.runId)).toEqual(["trunk", "user", "resume"]);
  });

  it("deletes unreachable turns and descendants by codec message id", () => {
    const tree = createTestTree();

    createRunNode(tree, "parent", "msg-parent", 1);
    createRunNode(tree, "child", "msg-child", 2, { parent: "msg-parent" });

    tree.delete("msg-parent");

    expect(tree.getRunNode("parent")).toBeUndefined();
    expect(tree.getRunNode("child")).toBeUndefined();
    expect(tree.getHeaders("msg-parent")).toBeUndefined();
  });

  it("emits structural and fold events without bumping on content-only folds", () => {
    const tree = createTestTree();
    const emitted: string[] = [];
    tree.on("message", (event) => emitted.push(`message:${event.runId}`));
    tree.on("ably-message", (event) => emitted.push(`ably:${event.runId}`));
    tree.on("turn-projection-updated", (event) => emitted.push(`projection:${event.runId}`));
    tree.on("update", (event) => emitted.push(`update:${String(event.structuralVersion)}`));

    tree.applyMessage(
      [decoded("first", "msg-1", 1)],
      headers({ runId: "turn-1", codecMessageId: "msg-1" }),
      1,
    );
    const afterCreate = tree.structuralVersion;
    tree.applyMessage(
      [decoded("append", "msg-1", 2)],
      headers({ runId: "turn-1", codecMessageId: "msg-1" }),
      2,
    );

    expect(tree.structuralVersion).toBe(afterCreate);
    expect(emitted).toContain("message:turn-1");
    expect(emitted).toContain("ably:turn-1");
    expect(emitted).toContain("projection:turn-1");
  });

  it("folds shuffled valid op logs to an identical final tree", () => {
    const canonical = summarize(applyOps(validOps));

    fc.assert(
      fc.property(shuffledOps(), (ops) => {
        expect(summarize(applyOps(ops))).toEqual(canonical);
      }),
      { numRuns: 100 },
    );
  });

  it("replays 100k operations within the tree budget", () => {
    const tree = createTestTree();
    const started = performance.now();
    for (let index = 0; index < 100_000; index += 1) {
      tree.applyMessage(
        [decoded("x", "msg-1", index + 1)],
        headers({ runId: "turn-1", codecMessageId: "msg-1" }),
        index + 1,
      );
    }
    const elapsed = performance.now() - started;

    expect(tree.getRunNode("turn-1")?.projection.events).toHaveLength(100_000);
    expect(elapsed).toBeLessThan(REPLAY_BUDGET_MS);
  });
});

interface Projection {
  events: string[];
}

interface HeaderOptions {
  runId?: string;
  codecMessageId?: string;
  parent?: string;
  forkOf?: string;
  regenerates?: string | boolean;
  role?: string;
  runClientId?: string;
  inputClientId?: string;
  invocationId?: string;
  runContinue?: boolean;
  supersedes?: string;
  runReason?: RunEndReason;
}

type TestTree = Tree<string, Projection>;

const reducer: Reducer<string, Projection> = {
  init() {
    return { events: [] };
  },
  fold(state, event, meta) {
    state.events.push(`${String(meta.serial)}:${meta.messageId ?? "none"}:${event}`);
    return state;
  },
};

function createTestTree(): TestTree {
  return createTree(reducer);
}

function headers(options: HeaderOptions): HeaderMap {
  const map = Object.create(null) as Record<string, string>;
  set(map, HEADER_RUN_ID, options.runId);
  set(map, HEADER_CODEC_MESSAGE_ID, options.codecMessageId);
  set(map, HEADER_PARENT, options.parent);
  set(map, HEADER_FORK_OF, options.forkOf);
  set(map, HEADER_MSG_REGENERATE, headerValue(options.regenerates));
  set(map, HEADER_ROLE, options.role);
  set(map, HEADER_RUN_CLIENT_ID, options.runClientId);
  set(map, HEADER_INPUT_CLIENT_ID, options.inputClientId);
  set(map, HEADER_INVOCATION_ID, options.invocationId);
  set(map, HEADER_RUN_CONTINUE, bool(options.runContinue));
  set(map, HEADER_SUPERSEDES, options.supersedes);
  set(map, HEADER_RUN_REASON, options.runReason);
  return map;
}

function decoded(event: string, messageId: string, serial: TreeSerial): DecodedEvent<string> {
  return {
    event,
    messageId,
    meta: {
      serial,
      messageId,
    },
  };
}

function createRunNode(
  tree: TestTree,
  runId: string,
  codecMessageId: string,
  serial: number,
  metadata: HeaderOptions = {},
): RunNode<Projection> | undefined {
  return tree.applyMessage(
    [decoded(runId, codecMessageId, serial)],
    headers({
      ...metadata,
      runId,
      codecMessageId,
    }),
    serial,
  );
}

interface Op {
  kind: "message" | "start" | "end";
  runId: string;
  codecMessageId?: string;
  serial: number;
  parent?: string;
  forkOf?: string;
  regenerates?: string | boolean;
  reason?: RunEndReason;
}

const validOps: Op[] = [
  { kind: "start", runId: "root", serial: 1 },
  { kind: "message", runId: "root", codecMessageId: "msg-root", serial: 2 },
  { kind: "end", runId: "root", serial: 3, reason: "complete" },
  { kind: "start", runId: "child", serial: 4, parent: "msg-root" },
  {
    kind: "message",
    runId: "child",
    codecMessageId: "msg-child",
    serial: 5,
    parent: "msg-root",
  },
  { kind: "end", runId: "child", serial: 6, reason: "complete" },
  {
    kind: "start",
    runId: "regen",
    serial: 7,
    parent: "msg-root",
    regenerates: "msg-child",
  },
  {
    kind: "message",
    runId: "regen",
    codecMessageId: "msg-regen",
    serial: 8,
    parent: "msg-root",
    regenerates: "msg-child",
  },
  { kind: "end", runId: "regen", serial: 9, reason: "suspended" },
];

function shuffledOps(): fc.Arbitrary<Op[]> {
  return fc.shuffledSubarray(validOps, {
    minLength: validOps.length,
    maxLength: validOps.length,
  });
}

function applyOps(ops: readonly Op[]): TestTree {
  const tree = createTestTree();
  for (const op of ops) {
    if (op.kind === "message" && op.codecMessageId !== undefined) {
      const headerOptions: HeaderOptions = {
        runId: op.runId,
        codecMessageId: op.codecMessageId,
      };
      setOptional(headerOptions, "parent", op.parent);
      setOptional(headerOptions, "forkOf", op.forkOf);
      setOptional(headerOptions, "regenerates", op.regenerates);
      tree.applyMessage(
        [decoded(op.runId, op.codecMessageId, op.serial)],
        headers(headerOptions),
        op.serial,
      );
    } else if (op.kind === "start") {
      const headerOptions: HeaderOptions = {
        runId: op.runId,
      };
      setOptional(headerOptions, "parent", op.parent);
      setOptional(headerOptions, "forkOf", op.forkOf);
      setOptional(headerOptions, "regenerates", op.regenerates);
      tree.applyRunLifecycle({
        type: "start",
        headers: headers(headerOptions),
        serial: op.serial,
      });
    } else {
      tree.applyRunLifecycle({
        type: "end",
        headers: headers({
          runId: op.runId,
          runReason: op.reason ?? "complete",
        }),
        serial: op.serial,
      });
    }
  }
  return tree;
}

function summarize(tree: TestTree): unknown {
  return ["root", "child", "regen"].map((runId) => {
    const node = tree.getRunNode(runId);
    return {
      runId,
      parentRunId: node?.parentRunId,
      forkOf: node?.forkOf,
      regeneratesCodecMessageId: node?.regeneratesCodecMessageId,
      status: node?.status,
      startSerial: node?.startSerial,
      endSerial: node?.endSerial,
      events: node?.projection.events.slice().sort(),
    };
  });
}

function set(target: Record<string, string>, key: string, value: string | undefined): void {
  if (value !== undefined) {
    target[key] = value;
  }
}

function bool(value: boolean | undefined): string | undefined {
  return value === undefined ? undefined : value ? "true" : "false";
}

function headerValue(value: string | boolean | undefined): string | undefined {
  return typeof value === "boolean" ? bool(value) : value;
}

function setOptional<T extends object, K extends keyof T>(
  target: T,
  key: K,
  value: T[K] | undefined,
): void {
  if (value !== undefined) {
    target[key] = value;
  }
}
