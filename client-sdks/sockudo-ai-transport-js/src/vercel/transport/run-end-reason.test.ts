import { describe, expect, it } from "vitest";

import { vercelRunEndReason } from "./run-end-reason.js";

describe("vercelRunEndReason", () => {
  it("returns non-complete pipe results and observes finish rejections", async () => {
    const finish = Promise.reject(new Error("ignored"));
    await expect(vercelRunEndReason({ reason: "cancelled" }, finish)).resolves.toBe("cancelled");
  });

  it("suspends complete streams that finish with tool calls", async () => {
    await expect(
      vercelRunEndReason({ reason: "complete" }, Promise.resolve("tool-calls")),
    ).resolves.toBe("suspended");
  });

  it("completes for other finish reasons", async () => {
    await expect(vercelRunEndReason({ reason: "complete" }, Promise.resolve("stop"))).resolves.toBe(
      "complete",
    );
  });

  it("maps abort-shaped finish rejection to cancelled", async () => {
    await expect(
      vercelRunEndReason(
        { reason: "complete" },
        Promise.reject(new DOMException("aborted", "AbortError")),
      ),
    ).resolves.toBe("cancelled");
  });

  it("maps other finish rejection to error", async () => {
    await expect(
      vercelRunEndReason({ reason: "complete" }, Promise.reject(new Error("failed"))),
    ).resolves.toBe("error");
  });
});
