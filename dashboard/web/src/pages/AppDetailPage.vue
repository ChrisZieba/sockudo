<script setup lang="ts">
import { computed, onMounted, ref, toRaw, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  ArrowLeft,
  BellRing,
  Check,
  ChevronRight,
  KeyRound,
  Plus,
  RefreshCw,
  Save,
  Trash2,
} from "lucide-vue-next";
import NumberField from "@/components/NumberField.vue";
import TriStateControl from "@/components/TriStateControl.vue";
import {
  api,
  type App,
  type ChannelNamespace,
  type Webhook,
} from "@/api/client";

type Section = "general" | "limits" | "channels" | "reliability" | "webhooks";
type OptionalPolicy = "idempotency" | "connection_recovery" | "history" | "presence_history";

const route = useRoute();
const router = useRouter();
const appId = computed(() => route.params.id as string);
const activeSection = ref<Section>("general");
const app = ref<App | null>(null);
const loading = ref(true);
const saving = ref(false);
const saved = ref(false);
const error = ref<string | null>(null);
const revealSecret = ref(false);

const sections: Array<{ id: Section; label: string; description: string }> = [
  { id: "general", label: "General", description: "Credentials and app capabilities" },
  { id: "limits", label: "Limits", description: "Traffic, payload, and presence guardrails" },
  { id: "channels", label: "Channels", description: "Origins, namespaces, and delta rules" },
  { id: "reliability", label: "Reliability", description: "Recovery, history, and idempotency" },
  { id: "webhooks", label: "Webhooks", description: "Event delivery and retry policies" },
];

const origins = computed({
  get: () => app.value?.policy.channels.allowed_origins?.join(", ") ?? "",
  set: (value: string) => {
    if (!app.value) return;
    app.value.policy.channels.allowed_origins = value
      .split(",")
      .map((origin) => origin.trim())
      .filter(Boolean);
  },
});

const deltaRules = computed(() =>
  Object.entries(app.value?.policy.channels.channel_delta_compression ?? {}),
);
const newDeltaPattern = ref("");
const newDeltaMode = ref<"inherit" | "disabled" | "fossil" | "xdelta3">("fossil");

const showWebhookForm = ref(false);
const editingWebhookIndex = ref<number | null>(null);
const webhookTarget = ref<"url" | "lambda">("url");
const webhookHeaders = ref("{}");
const webhookForm = ref<Webhook>(newWebhook());

const eventTypeOptions = [
  "channel_occupied",
  "channel_vacated",
  "subscription_count",
  "member_added",
  "member_removed",
  "member_updated",
  "client_event",
  "cache_miss",
  "ai_turn_started",
  "ai_turn_ended",
  "ai_cancel_requested",
  "ai_stream_orphaned",
  "message_version_created",
  "annotation_created",
  "annotation_deleted",
];

onMounted(load);
watch(appId, load);

function newWebhook(): Webhook {
  return {
    url: "",
    event_types: ["channel_occupied"],
    retry: { enabled: true, max_attempts: 3 },
  };
}

async function load() {
  loading.value = true;
  error.value = null;
  try {
    app.value = await api.getApp(appId.value);
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Failed to load app";
  } finally {
    loading.value = false;
  }
}

async function saveApp() {
  if (!app.value) return;
  saving.value = true;
  saved.value = false;
  error.value = null;
  try {
    app.value = await api.updateApp(appId.value, {
      key: app.value.key,
      enabled: app.value.enabled,
      policy: app.value.policy,
      replace_policy: true,
    });
    revealSecret.value = false;
    saved.value = true;
    window.setTimeout(() => (saved.value = false), 2_000);
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Save failed";
  } finally {
    saving.value = false;
  }
}

async function toggleSecret() {
  if (!app.value) return;
  try {
    revealSecret.value = !revealSecret.value;
    const fresh = await api.getApp(appId.value, revealSecret.value);
    app.value.secret = fresh.secret;
  } catch (err) {
    revealSecret.value = false;
    error.value = err instanceof Error ? err.message : "Secret lookup failed";
  }
}

async function rotateSecret() {
  if (!confirm("Rotate this app secret? Existing signed API calls will stop working.")) return;
  try {
    app.value = await api.rotateSecret(appId.value);
    revealSecret.value = true;
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Rotate failed";
  }
}

function enableMessageRateLimit() {
  if (!app.value) return;
  app.value.policy.limits.message_rate_limit = {
    enabled: false,
    max_attempts: 60,
    decay_seconds: 60,
    terminate_on_limit: false,
  };
}

function addNamespace() {
  if (!app.value) return;
  const namespaces = (app.value.policy.channels.channel_namespaces ??= []);
  namespaces.push({ name: `namespace-${namespaces.length + 1}` });
}

function removeNamespace(index: number) {
  app.value?.policy.channels.channel_namespaces?.splice(index, 1);
}

function addDeltaRule() {
  if (!app.value || !newDeltaPattern.value.trim()) return;
  const rules = (app.value.policy.channels.channel_delta_compression ??= {});
  rules[newDeltaPattern.value.trim()] = newDeltaMode.value;
  newDeltaPattern.value = "";
}

function updateDeltaRule(pattern: string, mode: string) {
  if (!app.value || mode === "advanced") return;
  const rules = (app.value.policy.channels.channel_delta_compression ??= {});
  rules[pattern] = mode as "inherit" | "disabled" | "fossil" | "xdelta3";
}

function removeDeltaRule(pattern: string) {
  if (!app.value?.policy.channels.channel_delta_compression) return;
  delete app.value.policy.channels.channel_delta_compression[pattern];
}

function addPolicyOverride(section: OptionalPolicy) {
  if (!app.value) return;
  if (section === "idempotency") app.value.policy.idempotency = {};
  if (section === "connection_recovery") app.value.policy.connection_recovery = {};
  if (section === "history") app.value.policy.history = {};
  if (section === "presence_history") app.value.policy.presence_history = {};
}

function removePolicyOverride(section: OptionalPolicy) {
  if (!app.value) return;
  delete app.value.policy[section];
}

function resetWebhookForm() {
  webhookForm.value = newWebhook();
  webhookTarget.value = "url";
  webhookHeaders.value = "{}";
  editingWebhookIndex.value = null;
  showWebhookForm.value = false;
}

function openNewWebhook() {
  resetWebhookForm();
  showWebhookForm.value = true;
}

function editWebhook(index: number) {
  const hook = app.value?.policy.webhooks?.[index];
  if (!hook) return;
  editingWebhookIndex.value = index;
  webhookForm.value = structuredClone(toRaw(hook));
  webhookForm.value.retry ??= { enabled: true, max_attempts: 3 };
  webhookTarget.value = hook.lambda || hook.lambda_function ? "lambda" : "url";
  webhookHeaders.value = JSON.stringify(hook.headers ?? {}, null, 2);
  showWebhookForm.value = true;
}

async function saveWebhook() {
  error.value = null;
  try {
    const parsedHeaders = JSON.parse(webhookHeaders.value || "{}");
    if (!parsedHeaders || Array.isArray(parsedHeaders) || typeof parsedHeaders !== "object") {
      throw new Error("Webhook headers must be a JSON object");
    }
    webhookForm.value.headers =
      Object.keys(parsedHeaders).length > 0 ? parsedHeaders : undefined;

    if (webhookTarget.value === "url") {
      delete webhookForm.value.lambda;
      delete webhookForm.value.lambda_function;
    } else {
      delete webhookForm.value.url;
    }

    const updated =
      editingWebhookIndex.value === null
        ? await api.createWebhook(appId.value, webhookForm.value)
        : await api.updateWebhook(
            appId.value,
            editingWebhookIndex.value,
            webhookForm.value,
          );
    app.value = updated;
    revealSecret.value = false;
    resetWebhookForm();
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Webhook save failed";
  }
}

async function deleteWebhook(index: number) {
  if (!confirm("Delete this webhook?")) return;
  try {
    app.value = await api.deleteWebhook(appId.value, index);
    revealSecret.value = false;
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Delete failed";
  }
}

function toggleEventType(type: string) {
  const selected = new Set(webhookForm.value.event_types);
  if (selected.has(type)) selected.delete(type);
  else selected.add(type);
  webhookForm.value.event_types = [...selected];
}

function webhookDestination(hook: Webhook) {
  if (hook.url) return hook.url;
  if (hook.lambda) return `${hook.lambda.function_name} · ${hook.lambda.region}`;
  return hook.lambda_function ?? "Lambda target";
}
</script>

<template>
  <div>
    <button class="back-link" @click="router.push({ name: 'apps' })">
      <ArrowLeft class="w-4 h-4" />
      Applications
    </button>

    <div v-if="loading" class="empty-state">Loading application configuration…</div>

    <template v-else-if="app">
      <header class="page-header">
        <div class="min-w-0">
          <div class="flex items-center gap-3">
            <div class="app-avatar">{{ app.id.slice(0, 2).toUpperCase() }}</div>
            <div class="min-w-0">
              <div class="flex items-center gap-2">
                <h1 class="page-title font-mono truncate">{{ app.id }}</h1>
                <span :class="app.enabled ? 'status-positive' : 'status-negative'" class="status-pill">
                  {{ app.enabled ? "Enabled" : "Disabled" }}
                </span>
              </div>
              <p class="page-subtitle">Application policy and delivery configuration</p>
            </div>
          </div>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <button
            class="btn-secondary flex items-center gap-2"
            @click="router.push({ name: 'app-push', params: { id: app.id } })"
          >
            <BellRing class="w-4 h-4" />
            Push manager
          </button>
          <button class="btn-primary flex items-center gap-2" :disabled="saving" @click="saveApp">
            <Check v-if="saved" class="w-4 h-4" />
            <Save v-else class="w-4 h-4" />
            {{ saved ? "Saved" : saving ? "Saving…" : "Save changes" }}
          </button>
        </div>
      </header>

      <div v-if="error" class="alert alert-error mb-5">{{ error }}</div>

      <div class="app-settings-layout">
        <nav class="settings-nav panel">
          <button
            v-for="section in sections"
            :key="section.id"
            type="button"
            :class="{ active: activeSection === section.id }"
            @click="activeSection = section.id"
          >
            <span>
              <strong>{{ section.label }}</strong>
              <small>{{ section.description }}</small>
            </span>
            <ChevronRight class="w-4 h-4" />
          </button>
        </nav>

        <div class="min-w-0 space-y-5">
          <template v-if="activeSection === 'general'">
            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Identity</p>
                  <h2>API credentials</h2>
                  <p>Use these values for realtime clients and signed server requests.</p>
                </div>
                <KeyRound class="w-5 h-5 text-brand-300" />
              </div>
              <div class="grid md:grid-cols-2 gap-4">
                <label>
                  <span class="field-label">App ID</span>
                  <input :value="app.id" class="input-field font-mono" disabled />
                </label>
                <label>
                  <span class="field-label">App key</span>
                  <input v-model.trim="app.key" class="input-field font-mono" />
                </label>
                <label class="md:col-span-2">
                  <span class="field-label">App secret</span>
                  <div class="flex flex-col sm:flex-row gap-2">
                    <input
                      :value="app.secret"
                      class="input-field font-mono flex-1"
                      readonly
                      autocomplete="off"
                    />
                    <button class="btn-secondary" type="button" @click="toggleSecret">
                      {{ revealSecret ? "Hide" : "Reveal" }}
                    </button>
                    <button
                      class="btn-secondary flex items-center justify-center gap-2"
                      type="button"
                      @click="rotateSecret"
                    >
                      <RefreshCw class="w-4 h-4" />
                      Rotate
                    </button>
                  </div>
                </label>
              </div>
              <div class="alert alert-warning mt-5">
                App changes are read from the configured app-manager database. Running Sockudo
                nodes may retain cached policy until their database cache TTL expires.
              </div>
            </section>

            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Capabilities</p>
                  <h2>Application features</h2>
                  <p>Choose which client-originated protocol features this app can use.</p>
                </div>
              </div>
              <label class="setting-row">
                <span>
                  <span class="block text-sm font-medium text-surface-200">Application enabled</span>
                  <span class="block text-xs text-surface-500 mt-1">
                    Disabled apps reject new authenticated connections and API calls.
                  </span>
                </span>
                <input v-model="app.enabled" type="checkbox" class="toggle-input" />
              </label>
              <label class="setting-row">
                <span>
                  <span class="block text-sm font-medium text-surface-200">Client messages</span>
                  <span class="block text-xs text-surface-500 mt-1">
                    Allow clients to publish client-prefixed events.
                  </span>
                </span>
                <input
                  v-model="app.policy.features.enable_client_messages"
                  type="checkbox"
                  class="toggle-input"
                />
              </label>
              <TriStateControl
                v-model="app.policy.features.enable_user_authentication"
                label="User authentication"
                hint="Enable authenticated user sign-in and user-scoped features."
              />
              <TriStateControl
                v-model="app.policy.features.enable_watchlist_events"
                label="Watchlist events"
                hint="Emit user watchlist state changes to compatible clients."
              />
            </section>
          </template>

          <template v-else-if="activeSection === 'limits'">
            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Capacity</p>
                  <h2>Connection and request limits</h2>
                  <p>Blank optional values inherit the server-wide configuration.</p>
                </div>
              </div>
              <div class="grid md:grid-cols-2 gap-4">
                <label>
                  <span class="field-label">Max connections</span>
                  <input
                    v-model.number="app.policy.limits.max_connections"
                    type="number"
                    min="0"
                    class="input-field"
                  />
                </label>
                <NumberField
                  v-model="app.policy.limits.max_backend_events_per_second"
                  label="Backend events / second"
                  hint="Signed HTTP API publishes."
                />
                <label>
                  <span class="field-label">Client events / second</span>
                  <input
                    v-model.number="app.policy.limits.max_client_events_per_second"
                    type="number"
                    min="0"
                    class="input-field"
                  />
                </label>
                <NumberField
                  v-model="app.policy.limits.max_read_requests_per_second"
                  label="Read requests / second"
                />
                <NumberField
                  v-model="app.policy.limits.decay_seconds"
                  label="Client-event window (seconds)"
                  hint="Sliding window used by the client-event limiter."
                />
              </div>
              <label class="setting-row mt-4">
                <span>
                  <span class="block text-sm font-medium text-surface-200">
                    Terminate when client-event limit is exceeded
                  </span>
                  <span class="block text-xs text-surface-500 mt-1">
                    Otherwise only the over-limit event is rejected.
                  </span>
                </span>
                <input
                  v-model="app.policy.limits.terminate_on_limit"
                  type="checkbox"
                  class="toggle-input"
                />
              </label>
            </section>

            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Protocol guardrails</p>
                  <h2>Channels and event payloads</h2>
                </div>
              </div>
              <div class="grid md:grid-cols-2 gap-4">
                <NumberField
                  v-model="app.policy.limits.max_channel_name_length"
                  label="Channel name length"
                />
                <NumberField
                  v-model="app.policy.limits.max_event_name_length"
                  label="Event name length"
                />
                <NumberField
                  v-model="app.policy.limits.max_event_channels_at_once"
                  label="Channels per event"
                />
                <NumberField
                  v-model="app.policy.limits.max_event_payload_in_kb"
                  label="Event payload (KiB)"
                />
                <NumberField
                  v-model="app.policy.limits.max_event_batch_size"
                  label="Events per batch"
                />
              </div>
            </section>

            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Presence</p>
                  <h2>Presence limits</h2>
                </div>
              </div>
              <div class="grid md:grid-cols-2 gap-4">
                <NumberField
                  v-model="app.policy.limits.max_presence_members_per_channel"
                  label="Members per channel"
                />
                <NumberField
                  v-model="app.policy.limits.max_presence_member_size_in_kb"
                  label="Member data (KiB)"
                />
              </div>
            </section>

            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Abuse protection</p>
                  <h2>All-message rate limiter</h2>
                  <p>Applies to every inbound WebSocket message, not just client events.</p>
                </div>
                <button
                  v-if="!app.policy.limits.message_rate_limit"
                  class="btn-secondary btn-sm"
                  @click="enableMessageRateLimit"
                >
                  Add override
                </button>
                <button
                  v-else
                  class="btn-secondary btn-sm"
                  @click="delete app.policy.limits.message_rate_limit"
                >
                  Use server defaults
                </button>
              </div>
              <div v-if="app.policy.limits.message_rate_limit" class="space-y-4">
                <label class="setting-row">
                  <span class="text-sm font-medium text-surface-200">Enabled</span>
                  <input
                    v-model="app.policy.limits.message_rate_limit.enabled"
                    type="checkbox"
                    class="toggle-input"
                  />
                </label>
                <div class="grid md:grid-cols-2 gap-4">
                  <NumberField
                    v-model="app.policy.limits.message_rate_limit.max_attempts"
                    label="Maximum messages"
                  />
                  <NumberField
                    v-model="app.policy.limits.message_rate_limit.decay_seconds"
                    label="Window (seconds)"
                  />
                </div>
                <label class="setting-row">
                  <span class="text-sm font-medium text-surface-200">Terminate connection</span>
                  <input
                    v-model="app.policy.limits.message_rate_limit.terminate_on_limit"
                    type="checkbox"
                    class="toggle-input"
                  />
                </label>
              </div>
              <div v-else class="inherit-state">Using the server-wide message limiter.</div>
            </section>
          </template>

          <template v-else-if="activeSection === 'channels'">
            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Access</p>
                  <h2>Origins and annotations</h2>
                </div>
              </div>
              <label>
                <span class="field-label">Allowed origins</span>
                <input
                  v-model="origins"
                  class="input-field font-mono"
                  placeholder="https://app.example.com, https://*.example.com"
                />
                <span class="field-hint">Comma-separated patterns. Use * only for trusted development.</span>
              </label>
              <div class="mt-4">
                <TriStateControl
                  v-model="app.policy.channels.annotations_enabled"
                  label="Message annotations"
                  hint="Allow reactions and other annotation records on compatible channels."
                />
              </div>
            </section>

            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Namespaces</p>
                  <h2>Channel policy overrides</h2>
                  <p>Match names like namespace:room and override permissions or retention.</p>
                </div>
                <button class="btn-secondary btn-sm flex items-center gap-1" @click="addNamespace">
                  <Plus class="w-3.5 h-3.5" />
                  Add namespace
                </button>
              </div>

              <div
                v-if="!app.policy.channels.channel_namespaces?.length"
                class="inherit-state"
              >
                No namespace-specific policy. All channels inherit the app settings.
              </div>
              <div v-else class="space-y-4">
                <div
                  v-for="(namespace, index) in app.policy.channels.channel_namespaces"
                  :key="index"
                  class="subcard"
                >
                  <div class="flex items-center justify-between gap-3 mb-4">
                    <p class="font-mono text-sm text-brand-300">
                      {{ namespace.name || `namespace-${index + 1}` }}
                    </p>
                    <button
                      class="icon-button danger"
                      :aria-label="`Remove namespace ${namespace.name || index + 1}`"
                      @click="removeNamespace(index)"
                    >
                      <Trash2 class="w-4 h-4" />
                    </button>
                  </div>
                  <div class="grid md:grid-cols-2 gap-4">
                    <label>
                      <span class="field-label">Namespace</span>
                      <input v-model.trim="namespace.name" class="input-field font-mono" />
                    </label>
                    <label>
                      <span class="field-label">Channel regex</span>
                      <input
                        v-model.trim="namespace.channel_name_pattern"
                        class="input-field font-mono"
                        placeholder="Optional"
                      />
                    </label>
                    <NumberField
                      v-model="namespace.max_channel_name_length"
                      label="Maximum channel length"
                    />
                  </div>
                  <div class="grid lg:grid-cols-2 gap-x-5 mt-4">
                    <TriStateControl
                      v-model="namespace.allow_subscribe_for_client"
                      label="Client subscribe"
                    />
                    <TriStateControl
                      v-model="namespace.allow_publish_for_client"
                      label="Client publish"
                    />
                    <TriStateControl
                      v-model="namespace.allow_presence_for_client"
                      label="Client presence"
                    />
                    <TriStateControl
                      v-model="namespace.allow_user_limited_channels"
                      label="User-limited channels"
                    />
                    <TriStateControl
                      v-model="namespace.annotations_enabled"
                      label="Annotations"
                    />
                  </div>
                  <div class="grid lg:grid-cols-2 gap-4 mt-4">
                    <div class="nested-policy">
                      <div class="flex justify-between gap-2 mb-3">
                        <p class="text-sm font-medium">History override</p>
                        <button
                          v-if="!namespace.history"
                          class="text-button"
                          :aria-label="`Add history override for ${namespace.name || `namespace ${index + 1}`}`"
                          @click="namespace.history = {}"
                        >
                          Add
                        </button>
                        <button
                          v-else
                          class="text-button"
                          :aria-label="`Remove history override for ${namespace.name || `namespace ${index + 1}`}`"
                          @click="delete namespace.history"
                        >
                          Remove
                        </button>
                      </div>
                      <template v-if="namespace.history">
                        <TriStateControl
                          v-model="namespace.history.rewind_enabled"
                          label="Rewind"
                        />
                        <NumberField
                          v-model="namespace.history.retention_window_seconds"
                          label="Retention (seconds)"
                        />
                        <NumberField
                          v-model="namespace.history.max_messages_per_channel"
                          label="Messages per channel"
                        />
                        <NumberField
                          v-model="namespace.history.max_bytes_per_channel"
                          label="Bytes per channel"
                        />
                      </template>
                      <p v-else class="text-xs text-surface-500">Inherits app history.</p>
                    </div>
                    <div class="nested-policy">
                      <div class="flex justify-between gap-2 mb-3">
                        <p class="text-sm font-medium">Presence history override</p>
                        <button
                          v-if="!namespace.presence_history"
                          class="text-button"
                          :aria-label="`Add presence history override for ${namespace.name || `namespace ${index + 1}`}`"
                          @click="namespace.presence_history = {}"
                        >
                          Add
                        </button>
                        <button
                          v-else
                          class="text-button"
                          :aria-label="`Remove presence history override for ${namespace.name || `namespace ${index + 1}`}`"
                          @click="delete namespace.presence_history"
                        >
                          Remove
                        </button>
                      </div>
                      <template v-if="namespace.presence_history">
                        <TriStateControl
                          v-model="namespace.presence_history.enabled"
                          label="Enabled"
                        />
                        <NumberField
                          v-model="namespace.presence_history.retention_window_seconds"
                          label="Retention (seconds)"
                        />
                        <NumberField
                          v-model="namespace.presence_history.max_events_per_channel"
                          label="Events per channel"
                        />
                        <NumberField
                          v-model="namespace.presence_history.max_bytes_per_channel"
                          label="Bytes per channel"
                        />
                      </template>
                      <p v-else class="text-xs text-surface-500">Inherits app presence history.</p>
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Protocol V2</p>
                  <h2>Delta compression rules</h2>
                  <p>Set the delivery algorithm by exact channel or channel pattern.</p>
                </div>
              </div>
              <div v-if="deltaRules.length" class="space-y-2 mb-4">
                <div v-for="[pattern, config] in deltaRules" :key="pattern" class="rule-row">
                  <code>{{ pattern }}</code>
                  <select
                    class="input-field !w-auto"
                    :aria-label="`Compression mode for ${pattern}`"
                    :value="typeof config === 'string' ? config : 'advanced'"
                    @change="updateDeltaRule(pattern, ($event.target as HTMLSelectElement).value)"
                  >
                    <option value="inherit">Inherit</option>
                    <option value="disabled">Disabled</option>
                    <option value="fossil">Fossil</option>
                    <option value="xdelta3">Xdelta3</option>
                    <option v-if="typeof config === 'object'" value="advanced">
                      Advanced conflation
                    </option>
                  </select>
                  <button
                    class="icon-button danger"
                    :aria-label="`Remove delta rule ${pattern}`"
                    @click="removeDeltaRule(pattern)"
                  >
                    <Trash2 class="w-4 h-4" />
                  </button>
                </div>
              </div>
              <div class="grid sm:grid-cols-[1fr_150px_auto] gap-2">
                <input
                  v-model.trim="newDeltaPattern"
                  class="input-field font-mono"
                  placeholder="channel or pattern"
                />
                <select
                  v-model="newDeltaMode"
                  class="input-field"
                  aria-label="New delta compression mode"
                >
                  <option value="inherit">Inherit</option>
                  <option value="disabled">Disabled</option>
                  <option value="fossil">Fossil</option>
                  <option value="xdelta3">Xdelta3</option>
                </select>
                <button class="btn-secondary" @click="addDeltaRule">Add rule</button>
              </div>
            </section>
          </template>

          <template v-else-if="activeSection === 'reliability'">
            <section
              v-for="section in ([
                ['idempotency', 'Idempotency', 'Suppress duplicate signed publishes.'],
                ['connection_recovery', 'Connection recovery', 'Keep a bounded hot replay buffer.'],
                ['history', 'Durable history', 'Persist messages for history and rewind.'],
                ['presence_history', 'Presence history', 'Persist member transition records.'],
              ] as const)"
              :key="section[0]"
              class="settings-card"
            >
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Reliability policy</p>
                  <h2>{{ section[1] }}</h2>
                  <p>{{ section[2] }}</p>
                </div>
                <button
                  v-if="!app.policy[section[0]]"
                  class="btn-secondary btn-sm"
                  :aria-label="`Add ${section[1]} override`"
                  @click="addPolicyOverride(section[0])"
                >
                  Add override
                </button>
                <button
                  v-else
                  class="btn-secondary btn-sm"
                  :aria-label="`Use server defaults for ${section[1]}`"
                  @click="removePolicyOverride(section[0])"
                >
                  Use server defaults
                </button>
              </div>

              <div v-if="!app.policy[section[0]]" class="inherit-state">
                This app inherits the server-wide {{ section[1].toLowerCase() }} configuration.
              </div>

              <template v-else-if="section[0] === 'idempotency' && app.policy.idempotency">
                <TriStateControl v-model="app.policy.idempotency.enabled" label="Enabled" />
                <NumberField
                  v-model="app.policy.idempotency.ttl_seconds"
                  label="Deduplication TTL (seconds)"
                />
              </template>

              <template
                v-else-if="
                  section[0] === 'connection_recovery' && app.policy.connection_recovery
                "
              >
                <TriStateControl v-model="app.policy.connection_recovery.enabled" label="Enabled" />
                <div class="grid md:grid-cols-2 gap-4 mt-4">
                  <NumberField
                    v-model="app.policy.connection_recovery.buffer_ttl_seconds"
                    label="Buffer TTL (seconds)"
                  />
                  <NumberField
                    v-model="app.policy.connection_recovery.max_buffer_size"
                    label="Maximum buffered messages"
                  />
                </div>
              </template>

              <template v-else-if="section[0] === 'history' && app.policy.history">
                <TriStateControl v-model="app.policy.history.enabled" label="Enabled" />
                <TriStateControl v-model="app.policy.history.rewind_enabled" label="Rewind" />
                <div class="grid md:grid-cols-2 gap-4 mt-4">
                  <NumberField
                    v-model="app.policy.history.retention_window_seconds"
                    label="Retention (seconds)"
                  />
                  <NumberField
                    v-model="app.policy.history.max_messages_per_channel"
                    label="Messages per channel"
                  />
                  <NumberField
                    v-model="app.policy.history.max_bytes_per_channel"
                    label="Bytes per channel"
                  />
                </div>
              </template>

              <template
                v-else-if="section[0] === 'presence_history' && app.policy.presence_history"
              >
                <TriStateControl v-model="app.policy.presence_history.enabled" label="Enabled" />
                <div class="grid md:grid-cols-2 gap-4 mt-4">
                  <NumberField
                    v-model="app.policy.presence_history.retention_window_seconds"
                    label="Retention (seconds)"
                  />
                  <NumberField
                    v-model="app.policy.presence_history.max_events_per_channel"
                    label="Events per channel"
                  />
                  <NumberField
                    v-model="app.policy.presence_history.max_bytes_per_channel"
                    label="Bytes per channel"
                  />
                </div>
              </template>
            </section>
          </template>

          <template v-else>
            <section class="settings-card">
              <div class="card-heading">
                <div>
                  <p class="eyebrow">Delivery</p>
                  <h2>Webhooks</h2>
                  <p>Forward selected realtime events to HTTP or AWS Lambda targets.</p>
                </div>
                <button class="btn-primary btn-sm flex items-center gap-1" @click="openNewWebhook">
                  <Plus class="w-3.5 h-3.5" />
                  Add webhook
                </button>
              </div>

              <div v-if="showWebhookForm" class="subcard mb-5 space-y-4">
                <div>
                  <span class="field-label">Destination</span>
                  <div class="segmented w-fit">
                    <button
                      :class="{ active: webhookTarget === 'url' }"
                      :aria-pressed="webhookTarget === 'url'"
                      @click="webhookTarget = 'url'"
                    >
                      HTTP URL
                    </button>
                    <button
                      :class="{ active: webhookTarget === 'lambda' }"
                      :aria-pressed="webhookTarget === 'lambda'"
                      @click="webhookTarget = 'lambda'"
                    >
                      AWS Lambda
                    </button>
                  </div>
                </div>
                <label v-if="webhookTarget === 'url'">
                  <span class="field-label">URL</span>
                  <input
                    v-model.trim="webhookForm.url"
                    class="input-field"
                    placeholder="https://hooks.example.com/sockudo"
                  />
                </label>
                <div v-else class="grid md:grid-cols-2 gap-4">
                  <label>
                    <span class="field-label">Function name</span>
                    <input
                      :value="webhookForm.lambda?.function_name ?? ''"
                      class="input-field"
                      @input="
                        webhookForm.lambda = {
                          function_name: ($event.target as HTMLInputElement).value,
                          region: webhookForm.lambda?.region ?? '',
                        }
                      "
                    />
                  </label>
                  <label>
                    <span class="field-label">AWS region</span>
                    <input
                      :value="webhookForm.lambda?.region ?? ''"
                      class="input-field"
                      placeholder="eu-west-1"
                      @input="
                        webhookForm.lambda = {
                          function_name: webhookForm.lambda?.function_name ?? '',
                          region: ($event.target as HTMLInputElement).value,
                        }
                      "
                    />
                  </label>
                </div>

                <div>
                  <span class="field-label">Event types</span>
                  <div class="flex flex-wrap gap-2">
                    <button
                      v-for="type in eventTypeOptions"
                      :key="type"
                      type="button"
                      class="event-chip"
                      :class="{ active: webhookForm.event_types.includes(type) }"
                      :aria-pressed="webhookForm.event_types.includes(type)"
                      @click="toggleEventType(type)"
                    >
                      {{ type }}
                    </button>
                  </div>
                </div>

                <div class="grid md:grid-cols-2 gap-4">
                  <label>
                    <span class="field-label">Channel prefix</span>
                    <input
                      :value="webhookForm.filter?.channel_prefix ?? ''"
                      class="input-field"
                      @input="
                        webhookForm.filter = {
                          ...webhookForm.filter,
                          channel_prefix: ($event.target as HTMLInputElement).value || undefined,
                        }
                      "
                    />
                  </label>
                  <label>
                    <span class="field-label">Channel suffix</span>
                    <input
                      :value="webhookForm.filter?.channel_suffix ?? ''"
                      class="input-field"
                      @input="
                        webhookForm.filter = {
                          ...webhookForm.filter,
                          channel_suffix: ($event.target as HTMLInputElement).value || undefined,
                        }
                      "
                    />
                  </label>
                  <label>
                    <span class="field-label">Channel regex</span>
                    <input
                      :value="webhookForm.filter?.channel_pattern ?? ''"
                      class="input-field font-mono"
                      @input="
                        webhookForm.filter = {
                          ...webhookForm.filter,
                          channel_pattern: ($event.target as HTMLInputElement).value || undefined,
                        }
                      "
                    />
                  </label>
                  <label>
                    <span class="field-label">Namespace</span>
                    <input
                      :value="webhookForm.filter?.channel_namespace ?? ''"
                      class="input-field"
                      @input="
                        webhookForm.filter = {
                          ...webhookForm.filter,
                          channel_namespace:
                            ($event.target as HTMLInputElement).value || undefined,
                        }
                      "
                    />
                  </label>
                  <label>
                    <span class="field-label">Namespaces (comma-separated)</span>
                    <input
                      :value="webhookForm.filter?.channel_namespaces?.join(', ') ?? ''"
                      class="input-field"
                      @input="
                        webhookForm.filter = {
                          ...webhookForm.filter,
                          channel_namespaces: ($event.target as HTMLInputElement).value
                            .split(',')
                            .map((value) => value.trim())
                            .filter(Boolean),
                        }
                      "
                    />
                  </label>
                </div>

                <div class="grid md:grid-cols-2 gap-4">
                  <label class="setting-row md:col-span-2 !py-2">
                    <span class="text-sm font-medium text-surface-200">Retry delivery</span>
                    <input
                      v-model="webhookForm.retry!.enabled"
                      type="checkbox"
                      class="toggle-input"
                    />
                  </label>
                  <label>
                    <span class="field-label">Request timeout (ms)</span>
                    <input
                      v-model.number="webhookForm.request_timeout_ms"
                      type="number"
                      min="0"
                      class="input-field"
                    />
                  </label>
                  <label>
                    <span class="field-label">Retry attempts</span>
                    <input
                      v-model.number="webhookForm.retry!.max_attempts"
                      type="number"
                      min="0"
                      class="input-field"
                    />
                  </label>
                  <label>
                    <span class="field-label">Initial backoff (ms)</span>
                    <input
                      v-model.number="webhookForm.retry!.initial_backoff_ms"
                      type="number"
                      min="0"
                      class="input-field"
                    />
                  </label>
                  <label>
                    <span class="field-label">Maximum backoff (ms)</span>
                    <input
                      v-model.number="webhookForm.retry!.max_backoff_ms"
                      type="number"
                      min="0"
                      class="input-field"
                    />
                  </label>
                  <label>
                    <span class="field-label">Maximum retry time (ms)</span>
                    <input
                      v-model.number="webhookForm.retry!.max_elapsed_time_ms"
                      type="number"
                      min="0"
                      class="input-field"
                    />
                  </label>
                </div>
                <label>
                  <span class="field-label">Headers (JSON)</span>
                  <textarea
                    v-model="webhookHeaders"
                    rows="4"
                    class="input-field font-mono resize-y"
                    placeholder='{ "X-Service-Key": "..." }'
                  />
                  <span class="field-hint">
                    Header values may contain credentials. They remain in the app policy database.
                  </span>
                </label>
                <div class="flex gap-2">
                  <button class="btn-primary btn-sm" @click="saveWebhook">Save webhook</button>
                  <button class="btn-secondary btn-sm" @click="resetWebhookForm">Cancel</button>
                </div>
              </div>

              <div v-if="!app.policy.webhooks?.length" class="inherit-state">
                No webhooks configured.
              </div>
              <div v-else class="space-y-3">
                <div
                  v-for="(hook, index) in app.policy.webhooks"
                  :key="index"
                  class="webhook-row"
                >
                  <div class="min-w-0">
                    <p class="font-mono text-sm text-brand-300 truncate">
                      {{ webhookDestination(hook) }}
                    </p>
                    <p class="text-xs text-surface-500 mt-1">
                      {{ hook.event_types.length }} event types
                      <span v-if="hook.retry?.enabled !== false">
                        · retries {{ hook.retry?.max_attempts ?? "default" }}
                      </span>
                    </p>
                  </div>
                  <div class="flex gap-2 shrink-0">
                    <button
                      class="btn-secondary btn-sm"
                      :aria-label="`Edit webhook ${webhookDestination(hook)}`"
                      @click="editWebhook(index)"
                    >
                      Edit
                    </button>
                    <button
                      class="icon-button danger"
                      :aria-label="`Delete webhook ${webhookDestination(hook)}`"
                      @click="deleteWebhook(index)"
                    >
                      <Trash2 class="w-4 h-4" />
                    </button>
                  </div>
                </div>
              </div>
            </section>
          </template>
        </div>
      </div>
    </template>
  </div>
</template>
