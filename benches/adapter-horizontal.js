import crypto from "k6/crypto";
import execution from "k6/execution";
import http from "k6/http";
import ws from "k6/ws";
import { Counter, Trend } from "k6/metrics";

function positiveInteger(name, fallback) {
  const value = Number(__ENV[name] ?? fallback);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer, got ${value}`);
  }
  return value;
}

const subscriberCount = positiveInteger("SUBSCRIBERS", 900);
const channelCount = positiveInteger("CHANNELS", 100);
const messagesPerSecond = positiveInteger("MESSAGES_PER_SECOND", 1000);
const warmupSeconds = positiveInteger("WARMUP_SECONDS", 10);
const durationSeconds = positiveInteger("DURATION_SECONDS", 20);
const drainSeconds = positiveInteger("DRAIN_SECONDS", 12);
const totalSeconds = warmupSeconds + durationSeconds + drainSeconds;

const appId = __ENV.APP_ID ?? "bench-app";
const appKey = __ENV.APP_KEY ?? "bench-key";
const appSecret = __ENV.APP_SECRET ?? "bench-secret";
const eventName = __ENV.EVENT_NAME ?? "adapter-benchmark";
const wsUrls = (__ENV.WS_URLS ??
  `ws://127.0.0.1:6001/app/${appKey},ws://127.0.0.1:6002/app/${appKey},ws://127.0.0.1:6003/app/${appKey}`
)
  .split(",")
  .map((value) => value.trim());
const httpUrls = (__ENV.HTTP_URLS ??
  "http://127.0.0.1:6001,http://127.0.0.1:6002,http://127.0.0.1:6003"
)
  .split(",")
  .map((value) => value.trim());

if (wsUrls.length !== httpUrls.length || wsUrls.length === 0) {
  throw new Error("WS_URLS and HTTP_URLS must contain the same non-zero number of nodes");
}
if (subscriberCount % (channelCount * wsUrls.length) !== 0) {
  throw new Error(
    `SUBSCRIBERS (${subscriberCount}) must be divisible by CHANNELS * nodes (${channelCount * wsUrls.length})`,
  );
}

const deliveriesPerPublish = subscriberCount / channelCount;
const subscriptionReady = new Counter("subscription_ready");
const websocketErrors = new Counter("websocket_errors");
const publishSuccess = new Counter("publish_success");
const publishFailure = new Counter("publish_failure");
const deliveryMessages = new Counter("delivery_messages");
const deliveryLatency = new Trend("delivery_latency_ms", true);
const publishLatency = new Trend("publish_latency_ms", true);

export const options = {
  summaryTrendStats: ["avg", "min", "med", "p(90)", "p(95)", "p(99)", "max"],
  scenarios: {
    subscribers: {
      executor: "per-vu-iterations",
      exec: "subscribers",
      vus: subscriberCount,
      iterations: 1,
      maxDuration: `${totalSeconds + 15}s`,
      gracefulStop: "5s",
    },
    publishers: {
      executor: "constant-arrival-rate",
      exec: "publishers",
      rate: messagesPerSecond,
      timeUnit: "1s",
      duration: `${durationSeconds}s`,
      startTime: `${warmupSeconds}s`,
      preAllocatedVUs: Math.max(100, Math.ceil(messagesPerSecond / 5)),
      maxVUs: Math.max(500, messagesPerSecond),
      gracefulStop: `${drainSeconds}s`,
    },
  },
};

function signedPublishUrl(baseUrl, body) {
  const path = `/apps/${appId}/events`;
  const authParams = {
    auth_key: appKey,
    auth_timestamp: Math.floor(Date.now() / 1000).toString(),
    auth_version: "1.0",
    body_md5: crypto.md5(body, "hex"),
  };
  const queryForSignature = Object.keys(authParams)
    .sort()
    .map((key) => `${key}=${authParams[key]}`)
    .join("&");
  const signature = crypto.hmac(
    "sha256",
    appSecret,
    `POST\n${path}\n${queryForSignature}`,
    "hex",
  );
  return `${baseUrl}${path}?${queryForSignature}&auth_signature=${signature}`;
}

export function publishers() {
  const sequence = execution.scenario.iterationInTest;
  const channelIndex = sequence % channelCount;
  const nodeIndex = sequence % httpUrls.length;
  const sentAt = Date.now();
  const body = JSON.stringify({
    channel: `benchmark-${channelIndex}`,
    name: eventName,
    data: JSON.stringify({ sequence, sent_at: sentAt }),
  });
  const response = http.post(signedPublishUrl(httpUrls[nodeIndex], body), body, {
    headers: { "Content-Type": "application/json" },
    tags: { name: "publish_event" },
  });

  publishLatency.add(response.timings.duration);
  if (response.status === 200) {
    publishSuccess.add(1);
  } else {
    publishFailure.add(1, { status: String(response.status) });
  }
}

export function subscribers() {
  // This scenario has one iteration per VU, so iterationInTest is a dense
  // scenario-local index. A global VU ID can have gaps when publisher VUs are
  // preallocated concurrently, which would skew channel distribution.
  const subscriberIndex = execution.scenario.iterationInTest;
  const channel = `benchmark-${subscriberIndex % channelCount}`;
  const nodeIndex = subscriberIndex % wsUrls.length;
  const response = ws.connect(wsUrls[nodeIndex], null, (socket) => {
    socket.on("open", () => {
      socket.setInterval(() => {
        socket.send(JSON.stringify({ event: "pusher:ping", data: "{}" }));
      }, 30000);
    });

    socket.on("message", (rawMessage) => {
      let message;
      try {
        message = JSON.parse(rawMessage);
      } catch (_error) {
        websocketErrors.add(1, { kind: "invalid_json" });
        return;
      }

      if (message.event === "pusher:connection_established") {
        socket.send(JSON.stringify({
          event: "pusher:subscribe",
          data: { channel },
        }));
        return;
      }

      if (message.event === "pusher_internal:subscription_succeeded") {
        subscriptionReady.add(1);
        return;
      }

      if (message.event === eventName) {
        let data;
        try {
          data = typeof message.data === "string" ? JSON.parse(message.data) : message.data;
        } catch (_error) {
          websocketErrors.add(1, { kind: "invalid_event_data" });
          return;
        }
        deliveryMessages.add(1);
        deliveryLatency.add(Date.now() - data.sent_at);
      }
    });

    socket.on("error", () => websocketErrors.add(1, { kind: "socket" }));
    socket.setTimeout(() => socket.close(), totalSeconds * 1000);
  });

  if (!response || response.status !== 101) {
    websocketErrors.add(1, { kind: "handshake" });
  }
}

export function handleSummary(data) {
  const success = data.metrics.publish_success?.values?.count ?? 0;
  const received = data.metrics.delivery_messages?.values?.count ?? 0;
  const expected = success * deliveriesPerPublish;
  const deliveryRatio = expected === 0 ? 0 : received / expected;
  const compact = {
    workload: {
      nodes: wsUrls.length,
      subscribers: subscriberCount,
      channels: channelCount,
      messages_per_second: messagesPerSecond,
      deliveries_per_publish: deliveriesPerPublish,
      warmup_seconds: warmupSeconds,
      duration_seconds: durationSeconds,
      drain_seconds: drainSeconds,
    },
    outcome: {
      publish_success: success,
      publish_failure: data.metrics.publish_failure?.values?.count ?? 0,
      dropped_iterations: data.metrics.dropped_iterations?.values?.count ?? 0,
      subscriptions_ready: data.metrics.subscription_ready?.values?.count ?? 0,
      websocket_errors: data.metrics.websocket_errors?.values?.count ?? 0,
      delivery_messages: received,
      expected_deliveries: expected,
      delivery_ratio: deliveryRatio,
    },
    delivery_latency_ms: data.metrics.delivery_latency_ms?.values ?? {},
    publish_latency_ms: data.metrics.publish_latency_ms?.values ?? {},
    http_req_duration_ms: data.metrics.http_req_duration?.values ?? {},
  };

  return {
    stdout: `${JSON.stringify(compact, null, 2)}\n`,
    [__ENV.RESULT_PATH ?? "adapter-benchmark-result.json"]: `${JSON.stringify(compact, null, 2)}\n`,
  };
}
