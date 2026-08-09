import { beforeAll, describe, expect, it, vi } from "vitest";
import { validateOptions } from "../src/core/options";
import {
  authTokenRefreshDelay,
  normalizeAuthToken,
  TokenRevokedError,
} from "../src/core/token_auth";

let Sockudo: typeof import("../src/core/sockudo").default;
let setProtocolVersion: typeof import("../src/core/protocol_prefix").setProtocolVersion;
let ws: typeof import("../src/core/transports/url_schemes").ws;

beforeAll(async () => {
  Object.assign(globalThis, {
    VERSION: "test-version",
    CDN_HTTP: "",
    CDN_HTTPS: "",
    DEPENDENCY_SUFFIX: "",
  });
  ({ default: Sockudo } = await import("../src/core/sockudo"));
  ({ setProtocolVersion } = await import("../src/core/protocol_prefix"));
  ({ ws } = await import("../src/core/transports/url_schemes"));
});

describe("Protocol V2 capability-token auth", () => {
  it("fails closed when token auth is configured for Protocol V1", () => {
    expect(() =>
      validateOptions({ cluster: "local", protocolVersion: 7, token: "secret" }),
    ).toThrow("requires protocolVersion: 2");
  });

  it("adds the current encoded token to V2 websocket URLs", () => {
    setProtocolVersion(2);
    let token = "first token/+";
    const params = {
      useTLS: false,
      hostTLS: "ws.example.com",
      hostNonTLS: "ws.example.com",
      httpPath: "",
      getAuthToken: () => token,
    };

    expect(ws.getInitial("app-key", params)).toContain("token=first%20token%2F%2B");
    token = "second-token";
    expect(ws.getInitial("app-key", params)).toContain("token=second-token");
  });

  it("computes proactive refresh at eighty percent of token lifetime", () => {
    const token = normalizeAuthToken({ token: "opaque", iat: 100, exp: 200 });
    expect(authTokenRefreshDelay(token, 150)).toBe(30_000);

    const payload = Buffer.from(JSON.stringify({ iat: 100, exp: 200 })).toString("base64url");
    const jwt = normalizeAuthToken(`header.${payload}.signature`);
    expect(jwt).toMatchObject({ issuedAt: 100, expiresAt: 200 });
  });

  it("refreshes 40142 in place but surfaces 40160 without reauth", async () => {
    const requests: string[] = [];
    const client = new Sockudo("app-key", {
      cluster: "local",
      protocolVersion: 2,
      token: "initial-token",
      authCallback: async (request) => {
        requests.push(request.reason);
        return "fresh-token";
      },
    });
    const send = vi.spyOn(client, "send_event").mockReturnValue(true);
    const errors: unknown[] = [];
    client.bind("error", (error) => errors.push(error));
    (client.connection as unknown as { state: string }).state = "connected";

    client.connection.emit("message", {
      event: "sockudo:token_expired",
      data: { code: 40142, reason: "expired" },
    });
    await vi.waitFor(() =>
      expect(send).toHaveBeenCalledWith("sockudo:auth", { token: "fresh-token" }),
    );
    expect(requests).toEqual(["expired"]);

    send.mockClear();
    client.connection.emit("message", {
      event: "sockudo:token_expired",
      data: { code: 40160, reason: "revoked" },
    });
    expect(send).not.toHaveBeenCalled();
    expect(errors.at(-1)).toBeInstanceOf(TokenRevokedError);
    client.disconnect();
  });
});
