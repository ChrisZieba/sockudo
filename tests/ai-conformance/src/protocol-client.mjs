import crypto from "node:crypto";
import assert from "node:assert/strict";

export class AitProtocolClient {
  constructor(options = {}) {
    this.baseUrl = options.baseUrl ?? process.env.SOCKUDO_BASE_URL ?? "http://127.0.0.1:6001";
    this.wsUrl =
      options.wsUrl ??
      process.env.SOCKUDO_WS_URL ??
      "ws://127.0.0.1:6001/app/app-key?protocol=2&client=ait-conformance&version=0";
    this.appId = options.appId ?? process.env.SOCKUDO_APP_ID ?? "app-id";
    this.key = options.key ?? process.env.SOCKUDO_APP_KEY ?? "app-key";
    this.secret = options.secret ?? process.env.SOCKUDO_APP_SECRET ?? "app-secret";
    this.timeoutMs = Number(process.env.AIT_CONFORMANCE_TIMEOUT_MS ?? 5000);
    this.acknowledgements = new Map();
    this.latestByMessage = new Map();
    this.deliveryByChannel = new Map();
  }

  async connect() {
    if (typeof WebSocket !== "function") {
      throw new Error("Node global WebSocket is required; run with Node >=22");
    }
    const socket = new WebSocket(this.wsUrl);
    const transcript = [];
    socket.addEventListener("message", (event) => {
      transcript.push(parseFrame(event.data));
    });
    await waitForEvent(socket, "open", this.timeoutMs);
    await waitUntil(
      () => transcript.some((frame) => frame.event === "sockudo:connection_established"),
      this.timeoutMs,
      "connection_established",
    );
    return new AitWsSession(socket, transcript, this.timeoutMs, this.key, this.secret);
  }

  async publish({ name, channel, data, extras, messageId, idempotencyKey }) {
    const result = await this.signedJson("POST", "/events", {
      name,
      channel,
      data,
      extras,
      ...(messageId ? { message_id: messageId } : {}),
      ...(idempotencyKey ? { idempotency_key: idempotencyKey } : {}),
    });
    this.recordAcknowledgement(channel, "create", result.channels?.[channel]);
    return result;
  }

  async append({ channel, messageSerial, data, extras, opId }) {
    const result = await this.signedJson(
      "POST",
      `/channels/${encodeURIComponent(channel)}/messages/${encodeURIComponent(messageSerial)}/append`,
      {
        data,
        extras,
        ...(opId ? { op_id: opId } : {}),
      },
    );
    assert.equal(result.message_serial, messageSerial);
    assert.equal(result.action, "append");
    assert.equal(result.accepted, true);
    assert.equal(result.status, "applied");
    this.recordAcknowledgement(channel, "append", result);
    return result;
  }

  async update({ channel, messageSerial, data, extras, opId }) {
    const result = await this.signedJson(
      "POST",
      `/channels/${encodeURIComponent(channel)}/messages/${encodeURIComponent(messageSerial)}/update`,
      {
        data,
        extras,
        ...(opId ? { op_id: opId } : {}),
      },
    );
    assert.equal(result.message_serial, messageSerial);
    assert.equal(result.action, "update");
    assert.equal(result.accepted, true);
    assert.equal(result.status, "applied");
    this.recordAcknowledgement(channel, "update", result);
    return result;
  }

  async getMessage({ channel, messageSerial }) {
    return this.signedJson(
      "GET",
      `/channels/${encodeURIComponent(channel)}/messages/${encodeURIComponent(messageSerial)}`,
    );
  }

  recordAcknowledgement(channel, action, acknowledgement) {
    assert(acknowledgement, "publish must acknowledge its version identity");
    const { message_serial, version_serial, history_serial, delivery_serial } = acknowledgement;
    assert.equal(typeof message_serial, "string");
    assert(message_serial.length > 0);
    assert.equal(typeof version_serial, "string");
    assert(version_serial.length > 0);
    assert(Number.isSafeInteger(history_serial) && history_serial > 0);
    assert(Number.isSafeInteger(delivery_serial) && delivery_serial > 0);
    assert(delivery_serial > (this.deliveryByChannel.get(channel) ?? 0), "delivery serial must increase");
    const previous = this.latestByMessage.get(message_serial);
    if (action === "create") {
      assert.equal(previous, undefined);
      assert.equal(version_serial, message_serial);
    } else {
      assert(previous, "mutation must preserve an existing message identity");
      assert.equal(history_serial, previous.history_serial);
      assert(version_serial > previous.version_serial, "version serial must increase");
    }
    assert(!this.acknowledgements.has(version_serial), "version identities must be unique");
    const identity = { channel, action, message_serial, version_serial, history_serial, delivery_serial };
    this.acknowledgements.set(version_serial, identity);
    this.latestByMessage.set(message_serial, identity);
    this.deliveryByChannel.set(channel, delivery_serial);
  }

  assertTranscriptIdentities(frames) {
    const lastDelivery = new Map();
    for (const frame of frames) {
      if (frame.event === "history:get_latest") {
        const item = frame.data;
        const expected = this.acknowledgements.get(item.version?.serial);
        assert(expected, "history version must match a mutation acknowledgement");
        for (const field of ["channel", "action", "message_serial", "history_serial", "delivery_serial"]) {
          assert.equal(item[field], expected[field], `history ${field}`);
        }
        assert.equal(item.serial, expected.delivery_serial);
        assert.equal(item.event, `sockudo:message.${expected.action}`);
        assert(Number.isSafeInteger(item.version.timestamp_ms) && item.version.timestamp_ms > 0);
        continue;
      }
      if (!frame.event.startsWith("ai-") && !frame.event.startsWith("sockudo:message.")) continue;
      const headers = frame.extras?.headers;
      const expected = this.acknowledgements.get(headers?.sockudo_version_serial);
      assert(expected, "live version must match its HTTP acknowledgement");
      assert.equal(frame.channel, expected.channel);
      assert.equal(headers.sockudo_action, `message.${expected.action}`);
      assert.equal(headers.sockudo_message_serial, expected.message_serial);
      assert.equal(headers.sockudo_history_serial, expected.history_serial);
      assert.equal(frame.serial, expected.delivery_serial);
      assert.equal(frame.stream_id, `${this.appId}/${frame.channel}`);
      assert.equal(typeof frame.message_id, "string");
      assert(frame.message_id.length > 0);
      assert(Number.isSafeInteger(headers.sockudo_version_timestamp_ms) && headers.sockudo_version_timestamp_ms > 0);
      assert(frame.serial > (lastDelivery.get(frame.channel) ?? 0), "live delivery serial must increase");
      lastDelivery.set(frame.channel, frame.serial);
      if (expected.action !== "create") assert.equal(frame.event, `sockudo:message.${expected.action}`);
    }
  }

  async signedJson(method, path, body) {
    const fullPath = `/apps/${this.appId}${path}`;
    const bodyText = body === undefined ? undefined : JSON.stringify(body);
    const query = {
      auth_key: this.key,
      auth_timestamp: `${Math.floor(Date.now() / 1000)}`,
      auth_version: "1.0",
    };
    if (bodyText !== undefined) {
      query.body_md5 = crypto.createHash("md5").update(bodyText).digest("hex");
    }
    const canonicalQuery = Object.keys(query)
      .map((key) => [key.toLowerCase(), query[key]])
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, value]) => `${key}=${value}`)
      .join("&");
    const signature = crypto
      .createHmac("sha256", this.secret)
      .update(`${method}\n${fullPath}\n${canonicalQuery}`)
      .digest("hex");

    const url = new URL(`${this.baseUrl}${fullPath}`);
    for (const [key, value] of Object.entries(query)) {
      url.searchParams.set(key, value);
    }
    url.searchParams.set("auth_signature", signature);

    const response = await fetch(url, {
      method,
      headers: bodyText === undefined ? undefined : { "content-type": "application/json" },
      body: bodyText,
    });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${text}`);
    }
    return text ? JSON.parse(text) : {};
  }
}

export class AitWsSession {
  constructor(socket, transcript, timeoutMs, key, secret) {
    this.socket = socket;
    this.transcript = transcript;
    this.timeoutMs = timeoutMs;
    this.key = key;
    this.secret = secret;
  }

  send(event, data, channel) {
    this.socket.send(JSON.stringify({ event, data: JSON.stringify(data), channel }));
  }

  subscribe(channel, extra = {}) {
    let auth = {};
    if (channel.startsWith("private-") || channel.startsWith("presence-")) {
      const established = this.transcript.find((frame) => frame.event === "sockudo:connection_established");
      const socketId = established?.data?.socket_id;
      if (!socketId) throw new Error("connection has no socket ID for channel authentication");
      const channelData = extra.channel_data;
      const input = `${socketId}:${channel}${channelData === undefined ? "" : `:${channelData}`}`;
      const signature = crypto.createHmac("sha256", this.secret).update(input).digest("hex");
      auth = { auth: `${this.key}:${signature}` };
    }
    this.send("pusher:subscribe", { channel, ...auth, ...extra });
  }

  async waitForEvent(predicate, label) {
    return waitUntil(
      () => this.transcript.find((frame) => predicate(frame)),
      this.timeoutMs,
      label,
    );
  }

  close() {
    this.socket.close();
  }
}

export function aiExtras(transport, codec = {}) {
  return {
    ai: {
      transport,
      codec,
    },
  };
}

export function normalizeTranscript(frames) {
  return frames.map((frame) => normalizeValue(frame)).filter((frame) => frame.event !== "sockudo:connection_established");
}

function normalizeValue(value) {
  if (Array.isArray(value)) {
    return value.map(normalizeValue);
  }
  if (value && typeof value === "object") {
    const normalized = {};
    for (const [key, raw] of Object.entries(value)) {
      if (key === "socket_id") {
        normalized[key] = "<socket>";
      } else if (key === "channel" && typeof raw === "string") {
        normalized[key] = normalizeChannel(raw);
      } else if (["message_id", "message_serial", "version_serial", "sockudo_message_serial", "sockudo_version_serial"].includes(key)) {
        normalized[key] = typeof raw === "string" && raw.length > 0 ? `<${key}>` : raw;
      } else if (key === "stream_id") {
        normalized[key] = "<stream_id>";
      } else if (key.endsWith("_serial") || key === "serial") {
        normalized[key] = typeof raw === "number" ? "<serial>" : normalizeValue(raw);
      } else if (key === "timestamp_ms" || key === "timestamp" || key === "sockudo_version_timestamp_ms") {
        normalized[key] = "<timestamp>";
      } else if (key === "version" && raw && typeof raw === "object") {
        normalized[key] = { ...normalizeValue(raw), serial: "<version_serial>", timestamp_ms: "<timestamp>" };
      } else {
        normalized[key] = normalizeValue(raw);
      }
    }
    return normalized;
  }
  return value;
}

function normalizeChannel(value) {
  const runId = process.env.AIT_CONFORMANCE_RUN_ID ?? "golden";
  return value.endsWith(`-${runId}`) ? `${value.slice(0, -runId.length)}golden` : value;
}

function parseFrame(data) {
  const frame = JSON.parse(String(data));
  if (typeof frame.data === "string") {
    try {
      frame.data = JSON.parse(frame.data);
    } catch {
      // Pusher-compatible events may carry an arbitrary string payload.
    }
  }
  return frame;
}

function waitForEvent(target, event, timeoutMs) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`timed out waiting for ${event}`)), timeoutMs);
    target.addEventListener(event, (value) => {
      clearTimeout(timer);
      resolve(value);
    }, { once: true });
    target.addEventListener("error", (value) => {
      clearTimeout(timer);
      reject(new Error(`websocket error while waiting for ${event}: ${value.message ?? "unknown"}`));
    }, { once: true });
  });
}

async function waitUntil(check, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = check();
    if (value) {
      return value;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`timed out waiting for ${label}`);
}
