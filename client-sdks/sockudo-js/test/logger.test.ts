import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Logger, { setLoggerConfig } from "../src/core/logger";

beforeEach(() => {
  setLoggerConfig({ log: undefined, logToConsole: false });
});

afterEach(() => {
  setLoggerConfig({ log: undefined, logToConsole: false });
  vi.restoreAllMocks();
});

describe("logger", () => {
  it("does not serialize arguments when logging is disabled", () => {
    const toJSON = vi.fn(() => ({ event: "message" }));

    Logger.debug("event received", { toJSON });

    expect(toJSON).not.toHaveBeenCalled();
  });

  it("preserves formatted output when a logger is configured", () => {
    const log = vi.fn();
    setLoggerConfig({ log });

    Logger.debug("event received", { event: "message", count: 2 });

    expect(log).toHaveBeenCalledOnce();
    expect(log.mock.calls[0][0]).toContain("event received");
    expect(log.mock.calls[0][0]).toContain('"event":"message"');
    expect(log.mock.calls[0][0]).toContain('"count":2');
  });

  it("continues logging to the console when explicitly enabled", () => {
    const consoleLog = vi.spyOn(console, "log").mockImplementation(() => {});
    setLoggerConfig({ logToConsole: true });

    Logger.debug("event received", { event: "message" });

    expect(consoleLog).toHaveBeenCalledOnce();
    expect(consoleLog.mock.calls[0][0]).toContain("event received");
  });
});
