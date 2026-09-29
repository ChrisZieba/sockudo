import assert from "node:assert/strict";
import test from "node:test";
import { AitProtocolClient, normalizeTranscript } from "./protocol-client.mjs";

function fixture() {
  const client = new AitProtocolClient({ appId: "app" });
  client.recordAcknowledgement("room", "create", {
    message_serial: "v1", version_serial: "v1", history_serial: 3, delivery_serial: 3,
  });
  client.recordAcknowledgement("room", "append", {
    message_serial: "v1", version_serial: "v2", history_serial: 3, delivery_serial: 4,
  });
  const frame = {
    event: "sockudo:message.append", channel: "room", data: "fragment", message_id: "id",
    serial: 4, stream_id: "app/room", extras: { headers: {
      sockudo_action: "message.append", sockudo_message_serial: "v1",
      sockudo_version_serial: "v2", sockudo_history_serial: 3, sockudo_version_timestamp_ms: 100,
    } },
  };
  return { client, frame };
}

test("live identities match exact acknowledgements before normalization", () => {
  const { client, frame } = fixture();
  client.assertTranscriptIdentities([frame]);
  for (const field of ["sockudo_message_serial", "sockudo_version_serial", "sockudo_history_serial", "sockudo_action"]) {
    const corrupt = structuredClone(frame);
    corrupt.extras.headers[field] = "wrong";
    assert.throws(() => client.assertTranscriptIdentities([corrupt]), field);
  }
  for (const field of ["serial", "stream_id", "channel"]) {
    assert.throws(() => client.assertTranscriptIdentities([{ ...frame, [field]: "wrong" }]), field);
  }
  assert.throws(() => client.assertTranscriptIdentities([frame, frame]), "duplicate delivery");
});

test("HTTP aggregate identities match exact mutation acknowledgements", () => {
  const { client } = fixture();
  const frame = { event: "history:get_latest", channel: "room", data: {
    event: "sockudo:message.append", channel: "room", action: "append", message_serial: "v1",
    history_serial: 3, delivery_serial: 4, serial: 4, version: { serial: "v2", timestamp_ms: 100 },
  } };
  client.assertTranscriptIdentities([frame]);
  for (const field of ["message_serial", "history_serial", "delivery_serial", "serial", "action"]) {
    const corrupt = structuredClone(frame);
    corrupt.data[field] = "wrong";
    assert.throws(() => client.assertTranscriptIdentities([corrupt]), field);
  }
});

test("mutations preserve original history position and advance version and delivery", () => {
  for (const fields of [{ history_serial: 4 }, { delivery_serial: 4 }, { version_serial: "v1" }, { message_serial: "other" }]) {
    const { client } = fixture();
    assert.throws(() => client.recordAcknowledgement("room", "append", {
      message_serial: "v1", version_serial: "v3", history_serial: 3, delivery_serial: 5, ...fields,
    }));
  }
});

test("normalization preserves runtime metadata and application extras", () => {
  const { frame } = fixture();
  const [normalized] = normalizeTranscript([frame]);
  assert.equal(normalized.extras.headers.sockudo_message_serial, "<sockudo_message_serial>");
  assert.equal(normalized.extras.headers.sockudo_version_serial, "<sockudo_version_serial>");
  assert.equal(normalized.extras.headers.sockudo_version_timestamp_ms, "<timestamp>");
  assert.equal(normalized.extras.headers.sockudo_action, "message.append");
  assert.equal(normalized.data, "fragment");
  assert.deepEqual(Object.keys(normalized), Object.keys(frame));
});
