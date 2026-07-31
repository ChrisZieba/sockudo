import { describe, expect, it } from "vitest";

import {
  HEADER_CODEC_MESSAGE_ID,
  HEADER_RUN_ID,
  HEADER_STEP_CLIENT_ID,
  HEADER_STEP_ID,
  HEADER_STEP_REASON,
  HEADER_STEP_START_SERIAL,
} from "../../constants.js";
import type { DecodedEvent, Reducer, ReducerMeta } from "../codec/index.js";
import { createTree, type Tree } from "./tree.js";

interface Message {
  id: string;
  text: string;
}

/** Minimal reducer that appends each event as a message. */
function reducer(): Reducer<Message, Message[]> {
  return {
    init: (): Message[] => [],
    fold: (projection: Message[], event: Message, _meta: ReducerMeta): Message[] => {
      projection.push(event);
      return projection;
    },
  };
}

const tree = (): Tree<Message, Message[]> => createTree(reducer());

const stepStart = (runId: string, stepId: string, clientId?: string) => ({
  [HEADER_RUN_ID]: runId,
  [HEADER_STEP_ID]: stepId,
  ...(clientId !== undefined ? { [HEADER_STEP_CLIENT_ID]: clientId } : {}),
});

const stepEnd = (runId: string, stepId: string, startSerial: string, reason: string) => ({
  [HEADER_RUN_ID]: runId,
  [HEADER_STEP_ID]: stepId,
  [HEADER_STEP_START_SERIAL]: startSerial,
  [HEADER_STEP_REASON]: reason,
});

const outputHeaders = (runId: string, stepId: string, startSerial: string, msgId: string) => ({
  [HEADER_RUN_ID]: runId,
  [HEADER_STEP_ID]: stepId,
  [HEADER_STEP_START_SERIAL]: startSerial,
  [HEADER_CODEC_MESSAGE_ID]: msgId,
});

const event = (id: string, text: string): DecodedEvent<Message> => ({
  event: { id, text },
  messageId: id,
  meta: { messageId: id, serial: id },
});

describe("step lifecycle", () => {
  it("records a first attempt as active", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 1,
    });

    const step = subject.getRunNode("run-1")?.steps[0];
    expect(step).toMatchObject({ stepId: "s1", status: "active", attemptCount: 1, startSerial: 1 });
  });

  it("counts a retry as a second attempt under the same step id", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 1,
    });
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 5,
    });

    const steps = subject.getRunNode("run-1")?.steps ?? [];
    expect(steps).toHaveLength(1);
    expect(steps[0]).toMatchObject({ attemptCount: 2, startSerial: 5, status: "active" });
  });

  it("keeps the highest serial canonical when a retry arrives out of order", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 5,
    });
    // Redelivery of the older attempt must not un-promote the newer one.
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 1,
    });

    expect(subject.getRunNode("run-1")?.steps[0]).toMatchObject({
      attemptCount: 2,
      startSerial: 5,
    });
  });

  it("carries the step client id", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1", "client-a"),
      serial: 1,
    });
    expect(subject.getRunNode("run-1")?.steps[0]?.stepClientId).toBe("client-a");
  });

  it("closes the canonical attempt with the wire reason", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 3,
    });
    subject.applyStepLifecycle({
      type: "step-end",
      headers: stepEnd("run-1", "s1", "3", "failed"),
      serial: 4,
    });

    expect(subject.getRunNode("run-1")?.steps[0]?.status).toBe("failed");
  });

  it("ignores a step-end belonging to a superseded attempt", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 1,
    });
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 9,
    });

    // The first attempt's failure arrives late; the live attempt must stay open.
    subject.applyStepLifecycle({
      type: "step-end",
      headers: stepEnd("run-1", "s1", "1", "failed"),
      serial: 10,
    });

    expect(subject.getRunNode("run-1")?.steps[0]?.status).toBe("active");
  });

  it("ignores unknown step reasons rather than storing them", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 1,
    });
    subject.applyStepLifecycle({
      type: "step-end",
      headers: stepEnd("run-1", "s1", "1", "exploded"),
      serial: 2,
    });
    expect(subject.getRunNode("run-1")?.steps[0]?.status).toBe("complete");
  });

  it("needs both run id and step id", () => {
    const subject = tree();
    expect(
      subject.applyStepLifecycle({
        type: "step-start",
        headers: { [HEADER_RUN_ID]: "run-1" },
        serial: 1,
      }),
    ).toBeUndefined();
    expect(
      subject.applyStepLifecycle({
        type: "step-start",
        headers: { [HEADER_STEP_ID]: "s1" },
        serial: 1,
      }),
    ).toBeUndefined();
  });
});

describe("step attempt supersession on the message path", () => {
  it("folds output from the canonical attempt", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 2,
    });
    subject.applyMessage([event("m1", "kept")], outputHeaders("run-1", "s1", "2", "m1"), 3);

    expect(subject.getRunNode("run-1")?.projection).toEqual([{ id: "m1", text: "kept" }]);
  });

  it("drops output stamped by a superseded attempt", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 2,
    });
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 8,
    });

    // Straggler from the failed first attempt.
    subject.applyMessage([event("m1", "partial")], outputHeaders("run-1", "s1", "2", "m1"), 9);
    // The retry's own output.
    subject.applyMessage([event("m2", "final")], outputHeaders("run-1", "s1", "8", "m2"), 10);

    expect(subject.getRunNode("run-1")?.projection).toEqual([{ id: "m2", text: "final" }]);
  });

  it("folds output for a step it has not seen a start for", () => {
    const subject = tree();
    // Reordering must not silently discard content: an unseen step cannot be
    // judged superseded.
    subject.applyMessage([event("m1", "early")], outputHeaders("run-1", "s1", "4", "m1"), 5);

    expect(subject.getRunNode("run-1")?.projection).toEqual([{ id: "m1", text: "early" }]);
  });

  it("folds unstamped output regardless of step state", () => {
    const subject = tree();
    subject.applyStepLifecycle({
      type: "step-start",
      headers: stepStart("run-1", "s1"),
      serial: 9,
    });
    subject.applyMessage(
      [event("m1", "plain")],
      { [HEADER_RUN_ID]: "run-1", [HEADER_CODEC_MESSAGE_ID]: "m1" },
      10,
    );

    expect(subject.getRunNode("run-1")?.projection).toEqual([{ id: "m1", text: "plain" }]);
  });
});
