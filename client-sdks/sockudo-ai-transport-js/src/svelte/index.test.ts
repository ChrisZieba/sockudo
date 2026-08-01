import { get } from "svelte/store";
import { describe, expect, it } from "vitest";

import { createMockClient } from "../realtime/mocks.js";
import { UIMessageCodec } from "../vercel/codec/index.js";
import { createSessionStore, createViewStore } from "./index.js";

describe("Svelte transport stores", () => {
  it("creates transport and view stores from an explicit client", () => {
    const client = createMockClient({ clientId: "svelte-client" });
    const session = createSessionStore({
      client,
      channelName: "chat",
      codec: UIMessageCodec,
      api: "/api/chat",
      closeOnDestroy: false,
    });
    const state = get(session);
    const view = createViewStore({ session });

    expect(state.sessionError).toBeUndefined();
    expect(state.session).toBeDefined();
    expect(get(view).messages).toEqual([]);
  });
});
