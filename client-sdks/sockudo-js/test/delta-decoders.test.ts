import { describe, expect, it } from "vitest";
import { bytesToString, stringToBytes } from "../src/core/delta/decoders";

describe("delta text conversion", () => {
  it("round trips multibyte UTF-8 across repeated calls", () => {
    const values = ["hello", "caf\u00e9", "\u6f22\u5b57", "\ud83d\ude80", ""];

    for (const value of values) {
      expect(bytesToString(stringToBytes(value))).toBe(value);
    }
  });

  it("does not carry malformed input state into the next decode", () => {
    expect(bytesToString(new Uint8Array([0xe2, 0x82]))).toBe("\ufffd");
    expect(bytesToString(stringToBytes("next message"))).toBe("next message");
  });

  it("decodes number arrays without mutating the input", () => {
    const bytes = [83, 111, 99, 107, 117, 100, 111];

    expect(bytesToString(bytes)).toBe("Sockudo");
    expect(bytes).toEqual([83, 111, 99, 107, 117, 100, 111]);
  });
});
