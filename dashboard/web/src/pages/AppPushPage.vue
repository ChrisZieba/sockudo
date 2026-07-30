<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  AlertTriangle,
  ArrowLeft,
  Bell,
  ChevronRight,
  Database,
  KeyRound,
  RefreshCw,
  RotateCcw,
  Search,
  Send,
  ShieldCheck,
  Smartphone,
  Trash2,
  Users,
} from "lucide-vue-next";
import {
  api,
  type PushChannelSubscription,
  type PushCredential,
  type PushCredentialRouteProvider,
  type PushDeadLetter,
  type PushDevice,
  type PushProvider,
  type PushPublishAccepted,
  type PushPublishStatus,
  type PushTarget,
} from "@/api/client";
import { useAuthStore } from "@/stores/auth";

type PushSection = "credentials" | "devices" | "delivery" | "dead-letters";
type ApnsMode = "token" | "p12" | "pem";
type BasicTargetType = "channel" | "device" | "client";

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();
const appId = computed(() => route.params.id as string);

const sections: Array<{
  id: PushSection;
  label: string;
  description: string;
}> = [
  {
    id: "credentials",
    label: "Credentials",
    description: "Provider keys",
  },
  {
    id: "devices",
    label: "Devices",
    description: "Registrations",
  },
  {
    id: "delivery",
    label: "Delivery",
    description: "Compose & inspect",
  },
  {
    id: "dead-letters",
    label: "Dead letters",
    description: "Failures & replay",
  },
];

const providers: Array<{
  id: PushCredentialRouteProvider;
  responseId: Exclude<PushProvider, "realtime">;
  short: string;
  label: string;
  description: string;
}> = [
  {
    id: "fcm",
    responseId: "fcm",
    short: "FCM",
    label: "Firebase Cloud Messaging",
    description: "Android and Firebase-backed apps",
  },
  {
    id: "apns",
    responseId: "apns",
    short: "APNs",
    label: "Apple Push Notification service",
    description: "iPhone, iPad, macOS, and Apple platforms",
  },
  {
    id: "webpush",
    responseId: "webPush",
    short: "Web",
    label: "Web Push",
    description: "Browser push with VAPID keys",
  },
  {
    id: "hms",
    responseId: "hms",
    short: "HMS",
    label: "Huawei Mobile Services",
    description: "Huawei devices and AppGallery apps",
  },
  {
    id: "wns",
    responseId: "wns",
    short: "WNS",
    label: "Windows Notification Service",
    description: "Windows and Microsoft Store apps",
  },
];

const activeSection = ref<PushSection>("credentials");
const refreshing = ref(false);
const notice = ref<string | null>(null);

const credentials = ref<PushCredential[]>([]);
const credentialsCursor = ref<string | null>(null);
const credentialsHasMore = ref(false);
const credentialsLoading = ref(false);
const credentialsError = ref<string | null>(null);

const devices = ref<PushDevice[]>([]);
const devicesCursor = ref<string | null>(null);
const devicesHasMore = ref(false);
const devicesLoading = ref(false);
const devicesError = ref<string | null>(null);
const deletingDeviceId = ref<string | null>(null);

const subscriptions = ref<PushChannelSubscription[]>([]);
const subscriptionsCursor = ref<string | null>(null);
const subscriptionsHasMore = ref(false);
const subscriptionsLoading = ref(false);
const subscriptionsError = ref<string | null>(null);
const subscriptionChannel = ref("");
const subscriptionDeviceId = ref("");

const deadLetters = ref<PushDeadLetter[]>([]);
const deadLettersCursor = ref<string | null>(null);
const deadLettersHasMore = ref(false);
const deadLettersLoading = ref(false);
const deadLettersError = ref<string | null>(null);
const deadLetterProvider = ref<"" | PushCredentialRouteProvider>("");
const replayingDeadLetterId = ref<string | null>(null);

const selectedProvider = ref<PushCredentialRouteProvider | null>(null);
const savingCredential = ref(false);
const credentialError = ref<string | null>(null);
const credentialDraft = reactive({
  credentialId: "",
  version: 1,
  fcmJson: "",
  apnsMode: "token" as ApnsMode,
  apnsTeamId: "",
  apnsKeyId: "",
  apnsPrivateKey: "",
  apnsP12: "",
  apnsP12Password: "",
  apnsPem: "",
  webPushPublicKey: "",
  webPushPrivateKey: "",
  hmsAppId: "",
  hmsClientSecret: "",
  wnsPackageSid: "",
  wnsClientSecret: "",
});

const composer = reactive({
  targetType: "channel" as BasicTargetType,
  targetValue: "",
  title: "",
  body: "",
  icon: "",
  sound: "",
  collapseKey: "",
  publishId: "",
  sync: false,
});
const publishing = ref(false);
const publishError = ref<string | null>(null);
const acceptedPublish = ref<PushPublishAccepted | null>(null);

const statusPublishId = ref("");
const publishStatus = ref<PushPublishStatus | null>(null);
const statusLoading = ref(false);
const statusError = ref<string | null>(null);

const selectedProviderDetails = computed(() =>
  providers.find((provider) => provider.id === selectedProvider.value),
);

const statusCounters = computed(() =>
  Object.entries(publishStatus.value?.counters ?? {}).sort(([left], [right]) =>
    left.localeCompare(right),
  ),
);

function messageFrom(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}

function providerLabel(provider?: PushProvider): string {
  if (!provider) return "Unassigned";
  if (provider === "webPush") return "Web Push";
  if (provider === "realtime") return "Realtime";
  return provider.toUpperCase();
}

function credentialCount(
  provider: (typeof providers)[number],
): number {
  return credentials.value.filter(
    (credential) => credential.provider === provider.responseId,
  ).length;
}

function formatDate(timestampMs: number): string {
  if (!timestampMs) return "—";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestampMs));
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function setNotice(message: string): void {
  notice.value = message;
  window.setTimeout(() => {
    if (notice.value === message) notice.value = null;
  }, 6_000);
}

async function loadCredentials(reset = true): Promise<void> {
  if (credentialsLoading.value) return;
  credentialsLoading.value = true;
  credentialsError.value = null;
  try {
    const page = await api.listPushCredentials(appId.value, {
      limit: 50,
      cursor: reset ? undefined : credentialsCursor.value ?? undefined,
    });
    credentials.value = reset
      ? page.items
      : [...credentials.value, ...page.items];
    credentialsCursor.value = page.next_cursor;
    credentialsHasMore.value = page.has_more;
  } catch (error) {
    credentialsError.value = messageFrom(
      error,
      "Failed to load stored credentials",
    );
  } finally {
    credentialsLoading.value = false;
  }
}

async function loadDevices(reset = true): Promise<void> {
  if (devicesLoading.value) return;
  devicesLoading.value = true;
  devicesError.value = null;
  try {
    const page = await api.listPushDevices(appId.value, {
      limit: 50,
      cursor: reset ? undefined : devicesCursor.value ?? undefined,
    });
    devices.value = reset ? page.items : [...devices.value, ...page.items];
    devicesCursor.value = page.next_cursor;
    devicesHasMore.value = page.has_more;
  } catch (error) {
    devicesError.value = messageFrom(
      error,
      "Failed to load device registrations",
    );
  } finally {
    devicesLoading.value = false;
  }
}

async function loadSubscriptions(reset = true): Promise<void> {
  if (subscriptionsLoading.value) return;
  subscriptionsLoading.value = true;
  subscriptionsError.value = null;
  try {
    const page = await api.listPushSubscriptions(appId.value, {
      channel: subscriptionChannel.value.trim() || undefined,
      deviceId: subscriptionDeviceId.value.trim() || undefined,
      limit: 50,
      cursor: reset ? undefined : subscriptionsCursor.value ?? undefined,
    });
    subscriptions.value = reset
      ? page.items
      : [...subscriptions.value, ...page.items];
    subscriptionsCursor.value = page.next_cursor;
    subscriptionsHasMore.value = page.has_more;
  } catch (error) {
    subscriptionsError.value = messageFrom(
      error,
      "Failed to load channel subscriptions",
    );
  } finally {
    subscriptionsLoading.value = false;
  }
}

async function loadDeadLetters(reset = true): Promise<void> {
  if (deadLettersLoading.value) return;
  deadLettersLoading.value = true;
  deadLettersError.value = null;
  try {
    const page = await api.listPushDeadLetters(appId.value, {
      provider: deadLetterProvider.value || undefined,
      limit: 50,
      cursor: reset ? undefined : deadLettersCursor.value ?? undefined,
    });
    deadLetters.value = reset
      ? page.items
      : [...deadLetters.value, ...page.items];
    deadLettersCursor.value = page.next_cursor;
    deadLettersHasMore.value = page.has_more;
  } catch (error) {
    deadLettersError.value = messageFrom(
      error,
      "Failed to load dead letters",
    );
  } finally {
    deadLettersLoading.value = false;
  }
}

async function refreshAll(): Promise<void> {
  refreshing.value = true;
  await Promise.all([
    loadCredentials(true),
    loadDevices(true),
    loadSubscriptions(true),
    loadDeadLetters(true),
  ]);
  refreshing.value = false;
}

function clearCredentialDraft(): void {
  credentialDraft.credentialId = "";
  credentialDraft.version = 1;
  credentialDraft.fcmJson = "";
  credentialDraft.apnsMode = "token";
  credentialDraft.apnsTeamId = "";
  credentialDraft.apnsKeyId = "";
  credentialDraft.apnsPrivateKey = "";
  credentialDraft.apnsP12 = "";
  credentialDraft.apnsP12Password = "";
  credentialDraft.apnsPem = "";
  credentialDraft.webPushPublicKey = "";
  credentialDraft.webPushPrivateKey = "";
  credentialDraft.hmsAppId = "";
  credentialDraft.hmsClientSecret = "";
  credentialDraft.wnsPackageSid = "";
  credentialDraft.wnsClientSecret = "";
  credentialError.value = null;
}

function openCredentialForm(provider: PushCredentialRouteProvider): void {
  clearCredentialDraft();
  selectedProvider.value = provider;
}

function closeCredentialForm(): void {
  clearCredentialDraft();
  selectedProvider.value = null;
}

function credentialMetadata(): {
  credentialId?: string;
  version: number;
} {
  if (
    !Number.isSafeInteger(credentialDraft.version) ||
    credentialDraft.version < 1
  ) {
    throw new Error("Credential version must be a positive integer");
  }
  return {
    credentialId: credentialDraft.credentialId.trim() || undefined,
    version: credentialDraft.version,
  };
}

async function submitCredential(): Promise<void> {
  const provider = selectedProvider.value;
  if (!provider || !auth.isAdmin) return;
  savingCredential.value = true;
  credentialError.value = null;
  try {
    const metadata = credentialMetadata();
    let stored: PushCredential;

    switch (provider) {
      case "fcm": {
        let serviceAccountJson: unknown;
        try {
          serviceAccountJson = JSON.parse(credentialDraft.fcmJson);
        } catch {
          throw new Error("Service account JSON is not valid JSON");
        }
        if (
          !serviceAccountJson ||
          typeof serviceAccountJson !== "object" ||
          Array.isArray(serviceAccountJson)
        ) {
          throw new Error("Service account JSON must be an object");
        }
        stored = await api.storePushCredential(appId.value, "fcm", {
          ...metadata,
          serviceAccountJson: serviceAccountJson as Record<string, unknown>,
        });
        break;
      }
      case "apns": {
        if (
          credentialDraft.apnsMode === "token" &&
          (!credentialDraft.apnsTeamId.trim() ||
            !credentialDraft.apnsKeyId.trim() ||
            !credentialDraft.apnsPrivateKey.trim())
        ) {
          throw new Error(
            "Team ID, key ID, and private key are required for APNs token authentication",
          );
        }
        if (
          credentialDraft.apnsMode === "p12" &&
          !credentialDraft.apnsP12.trim()
        ) {
          throw new Error("A PKCS#12 payload is required");
        }
        if (
          credentialDraft.apnsMode === "pem" &&
          !credentialDraft.apnsPem.trim()
        ) {
          throw new Error("A PEM certificate and key payload is required");
        }
        stored = await api.storePushCredential(appId.value, "apns", {
          ...metadata,
          ...(credentialDraft.apnsMode === "token"
            ? {
                teamId: credentialDraft.apnsTeamId.trim(),
                keyId: credentialDraft.apnsKeyId.trim(),
                privateKey: credentialDraft.apnsPrivateKey.trim(),
              }
            : {}),
          ...(credentialDraft.apnsMode === "p12"
            ? {
                p12: credentialDraft.apnsP12.trim(),
                p12Password:
                  credentialDraft.apnsP12Password.trim() || undefined,
              }
            : {}),
          ...(credentialDraft.apnsMode === "pem"
            ? { pem: credentialDraft.apnsPem.trim() }
            : {}),
        });
        break;
      }
      case "webpush":
        if (
          !credentialDraft.webPushPublicKey.trim() ||
          !credentialDraft.webPushPrivateKey.trim()
        ) {
          throw new Error("Both VAPID public and private keys are required");
        }
        stored = await api.storePushCredential(appId.value, "webpush", {
          ...metadata,
          publicKey: credentialDraft.webPushPublicKey.trim(),
          privateKey: credentialDraft.webPushPrivateKey.trim(),
        });
        break;
      case "hms":
        if (
          !credentialDraft.hmsAppId.trim() ||
          !credentialDraft.hmsClientSecret.trim()
        ) {
          throw new Error("HMS app ID and client secret are required");
        }
        stored = await api.storePushCredential(appId.value, "hms", {
          ...metadata,
          hmsAppId: credentialDraft.hmsAppId.trim(),
          clientSecret: credentialDraft.hmsClientSecret.trim(),
        });
        break;
      case "wns":
        if (
          !credentialDraft.wnsPackageSid.trim() ||
          !credentialDraft.wnsClientSecret.trim()
        ) {
          throw new Error("Package SID and client secret are required");
        }
        stored = await api.storePushCredential(appId.value, "wns", {
          ...metadata,
          packageSid: credentialDraft.wnsPackageSid.trim(),
          clientSecret: credentialDraft.wnsClientSecret.trim(),
        });
        break;
    }

    const label = providerLabel(stored.provider);
    closeCredentialForm();
    await loadCredentials(true);
    setNotice(
      `${label} credential “${stored.credential_id}” version ${stored.version} was stored. Restart the matching provider worker to load it.`,
    );
  } catch (error) {
    credentialError.value = messageFrom(
      error,
      "Failed to store provider credential",
    );
  } finally {
    savingCredential.value = false;
  }
}

async function deleteDevice(device: PushDevice): Promise<void> {
  if (!auth.isAdmin) return;
  if (
    !window.confirm(
      `Delete device registration “${device.id}”? Its channel subscriptions will no longer deliver push notifications.`,
    )
  ) {
    return;
  }
  deletingDeviceId.value = device.id;
  devicesError.value = null;
  try {
    await api.deletePushDevice(appId.value, device.id);
    await Promise.all([loadDevices(true), loadSubscriptions(true)]);
    setNotice(`Device “${device.id}” was removed.`);
  } catch (error) {
    devicesError.value = messageFrom(
      error,
      "Failed to delete device registration",
    );
  } finally {
    deletingDeviceId.value = null;
  }
}

function composerTarget(): PushTarget {
  const value = composer.targetValue.trim();
  if (!value) throw new Error("A recipient value is required");
  switch (composer.targetType) {
    case "channel":
      return { type: "channel", channel: value };
    case "device":
      return { type: "device", device_id: value };
    case "client":
      return { type: "client", client_id: value };
  }
}

async function publishNotification(): Promise<void> {
  if (!auth.isAdmin) return;
  publishing.value = true;
  publishError.value = null;
  acceptedPublish.value = null;
  try {
    const target = composerTarget();
    if (!composer.title.trim() && !composer.body.trim()) {
      throw new Error("Add a notification title or body");
    }
    const accepted = await api.publishPush(appId.value, {
      publishId: composer.publishId.trim() || undefined,
      recipients: [target],
      payload: {
        title: composer.title.trim() || undefined,
        body: composer.body.trim() || undefined,
        icon: composer.icon.trim() || undefined,
        sound: composer.sound.trim() || undefined,
        collapseKey: composer.collapseKey.trim() || undefined,
      },
      sync: composer.sync,
    });
    acceptedPublish.value = accepted;
    statusPublishId.value = accepted.publishId;
    composer.publishId = "";
    composer.title = "";
    composer.body = "";
    composer.icon = "";
    composer.sound = "";
    composer.collapseKey = "";
    setNotice(`Push “${accepted.publishId}” was accepted for delivery.`);
  } catch (error) {
    publishError.value = messageFrom(
      error,
      "Failed to publish notification",
    );
  } finally {
    publishing.value = false;
  }
}

async function lookupPublishStatus(): Promise<void> {
  const publishId = statusPublishId.value.trim();
  if (!publishId) {
    statusError.value = "Enter a publish ID";
    return;
  }
  statusLoading.value = true;
  statusError.value = null;
  try {
    publishStatus.value = await api.getPushPublishStatus(
      appId.value,
      publishId,
    );
  } catch (error) {
    publishStatus.value = null;
    statusError.value = messageFrom(error, "Failed to load publish status");
  } finally {
    statusLoading.value = false;
  }
}

async function replayDeadLetter(deadLetter: PushDeadLetter): Promise<void> {
  if (!auth.isAdmin || !deadLetter.replayable) return;
  if (
    !window.confirm(
      `Replay dead letter “${deadLetter.deadLetterId}” for publish “${deadLetter.publishId}”?`,
    )
  ) {
    return;
  }
  replayingDeadLetterId.value = deadLetter.deadLetterId;
  deadLettersError.value = null;
  try {
    await api.replayPushDeadLetter(appId.value, deadLetter.deadLetterId);
    statusPublishId.value = deadLetter.publishId;
    await loadDeadLetters(true);
    setNotice(
      `Dead letter “${deadLetter.deadLetterId}” was submitted for replay.`,
    );
  } catch (error) {
    deadLettersError.value = messageFrom(
      error,
      "Failed to replay dead letter",
    );
  } finally {
    replayingDeadLetterId.value = null;
  }
}

onMounted(refreshAll);
watch(appId, refreshAll);
</script>

<template>
  <div>
    <button
      class="back-link"
      type="button"
      @click="router.push({ name: 'app-detail', params: { id: appId } })"
    >
      <ArrowLeft class="w-4 h-4" />
      Back to app settings
    </button>

    <header class="page-header">
      <div class="flex items-start gap-4">
        <div
          class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-brand-400/20 bg-brand-500/12 text-brand-300"
        >
          <Bell class="w-6 h-6" />
        </div>
        <div>
          <p class="eyebrow">Application · {{ appId }}</p>
          <h1 class="page-title">Push Manager</h1>
          <p class="page-subtitle">
            Store provider credentials, inspect registered devices, and follow delivery outcomes.
          </p>
        </div>
      </div>
      <div class="flex items-center gap-3">
        <span
          class="status-pill"
          :class="auth.isAdmin ? 'status-positive' : 'bg-surface-800 text-surface-400'"
        >
          {{ auth.isAdmin ? "Admin controls" : "Read only" }}
        </span>
        <button
          class="btn-secondary flex items-center gap-2"
          type="button"
          :disabled="refreshing"
          @click="refreshAll"
        >
          <RefreshCw class="w-4 h-4" :class="{ 'animate-spin': refreshing }" />
          Refresh
        </button>
      </div>
    </header>

    <div v-if="notice" class="alert border-emerald-400/20 bg-emerald-500/10 text-emerald-200 mb-5">
      {{ notice }}
    </div>

    <div v-if="!auth.isAdmin" class="alert alert-warning mb-5 flex items-start gap-3">
      <ShieldCheck class="w-5 h-5 shrink-0 mt-0.5" />
      <p>
        Operators can inspect push state. Uploading credentials, publishing, deleting devices, and
        replaying dead letters require an administrator.
      </p>
    </div>

    <div class="alert alert-warning mb-6">
      <div class="flex items-start gap-3">
        <AlertTriangle class="w-5 h-5 shrink-0 mt-0.5" />
        <div>
          <p class="font-medium text-amber-100">Stored does not mean active</p>
          <p class="mt-1 text-xs leading-5">
            A stored credential only confirms that Sockudo accepted a durable credential record.
            Delivery also requires the matching provider feature and runtime flag, a configured
            credential-encryption key, and a running provider worker. Existing workers load changed
            stored credentials after restart; an upload does not prove provider connectivity.
          </p>
        </div>
      </div>
    </div>

    <div class="grid grid-cols-2 lg:grid-cols-4 gap-3 mb-6">
      <div class="panel p-4">
        <div class="flex items-center justify-between">
          <KeyRound class="w-4 h-4 text-brand-300" />
          <span class="text-[0.65rem] uppercase tracking-wider text-surface-600">loaded</span>
        </div>
        <p class="mt-3 text-2xl font-semibold">{{ formatNumber(credentials.length) }}</p>
        <p class="mt-1 text-xs text-surface-500">Stored credentials</p>
      </div>
      <div class="panel p-4">
        <div class="flex items-center justify-between">
          <Smartphone class="w-4 h-4 text-sky-300" />
          <span class="text-[0.65rem] uppercase tracking-wider text-surface-600">loaded</span>
        </div>
        <p class="mt-3 text-2xl font-semibold">{{ formatNumber(devices.length) }}</p>
        <p class="mt-1 text-xs text-surface-500">Device registrations</p>
      </div>
      <div class="panel p-4">
        <div class="flex items-center justify-between">
          <Users class="w-4 h-4 text-violet-300" />
          <span class="text-[0.65rem] uppercase tracking-wider text-surface-600">loaded</span>
        </div>
        <p class="mt-3 text-2xl font-semibold">{{ formatNumber(subscriptions.length) }}</p>
        <p class="mt-1 text-xs text-surface-500">Channel subscriptions</p>
      </div>
      <div class="panel p-4">
        <div class="flex items-center justify-between">
          <AlertTriangle class="w-4 h-4 text-amber-300" />
          <span class="text-[0.65rem] uppercase tracking-wider text-surface-600">loaded</span>
        </div>
        <p class="mt-3 text-2xl font-semibold">{{ formatNumber(deadLetters.length) }}</p>
        <p class="mt-1 text-xs text-surface-500">Dead letters</p>
      </div>
    </div>

    <nav
      class="panel mb-6 grid grid-cols-4 gap-1 overflow-hidden p-2 sm:flex sm:overflow-x-auto"
      aria-label="Push manager sections"
    >
      <button
        v-for="section in sections"
        :key="section.id"
        type="button"
        :aria-pressed="activeSection === section.id"
        class="min-w-0 flex-1 rounded-xl px-1.5 py-3 text-center transition-colors sm:min-w-max sm:px-4 sm:text-left"
        :class="
          activeSection === section.id
            ? 'bg-brand-500/12 text-brand-200'
            : 'text-surface-400 hover:bg-surface-800/60 hover:text-surface-200'
        "
        @click="activeSection = section.id"
      >
        <span class="block text-[0.68rem] font-medium sm:text-sm">{{ section.label }}</span>
        <span class="mt-0.5 hidden text-[0.68rem] text-surface-600 sm:block">
          {{ section.description }}
        </span>
      </button>
    </nav>

    <section v-if="activeSection === 'credentials'" class="space-y-5">
      <div class="settings-card">
        <div class="card-heading">
          <div>
            <p class="eyebrow">Provider access</p>
            <h2>Stored credentials</h2>
            <p>
              Upload encrypted provider material. Sockudo never returns secret material after it is
              stored.
            </p>
          </div>
          <Database class="w-5 h-5 text-surface-500" />
        </div>

        <p v-if="credentialsError" class="alert alert-error mb-4">{{ credentialsError }}</p>

        <div class="grid md:grid-cols-2 xl:grid-cols-3 gap-3">
          <article
            v-for="provider in providers"
            :key="provider.id"
            class="subcard flex min-h-44 flex-col"
            :class="{ 'border-brand-400/35 bg-brand-500/5': selectedProvider === provider.id }"
          >
            <div class="flex items-start justify-between gap-3">
              <div
                class="flex h-10 w-10 items-center justify-center rounded-xl bg-surface-800 font-semibold text-brand-200"
              >
                {{ provider.short }}
              </div>
              <span
                class="status-pill"
                :class="credentialCount(provider) ? 'status-positive' : 'bg-surface-800 text-surface-500'"
              >
                {{ credentialCount(provider) }} stored
              </span>
            </div>
            <h3 class="mt-4 text-sm font-semibold text-surface-100">{{ provider.label }}</h3>
            <p class="mt-1 flex-1 text-xs leading-5 text-surface-500">
              {{ provider.description }}
            </p>
            <button
              v-if="auth.isAdmin"
              class="mt-4 flex items-center justify-between text-left text-xs font-medium text-brand-300 hover:text-brand-200"
              type="button"
              :aria-label="`Store or rotate ${provider.label} credential`"
              @click="openCredentialForm(provider.id)"
            >
              Store or rotate
              <ChevronRight class="w-4 h-4" />
            </button>
          </article>
        </div>
      </div>

      <form
        v-if="selectedProvider && selectedProviderDetails && auth.isAdmin"
        class="settings-card"
        autocomplete="off"
        @submit.prevent="submitCredential"
      >
        <div class="card-heading">
          <div>
            <p class="eyebrow">Memory-only form</p>
            <h2>{{ selectedProviderDetails.label }}</h2>
            <p>
              Values stay only in this tab and are cleared after a successful upload or when you
              close this form.
            </p>
          </div>
          <button class="text-button" type="button" @click="closeCredentialForm">Close</button>
        </div>

        <p v-if="credentialError" class="alert alert-error mb-5">{{ credentialError }}</p>

        <div class="grid md:grid-cols-2 gap-4 mb-5">
          <label>
            <span class="field-label">Credential ID</span>
            <input
              v-model.trim="credentialDraft.credentialId"
              class="input-field font-mono"
              autocomplete="off"
              :placeholder="`${selectedProvider}-primary`"
            />
            <span class="field-hint">Leave blank to use Sockudo’s provider default ID.</span>
          </label>
          <label>
            <span class="field-label">Version</span>
            <input
              v-model.number="credentialDraft.version"
              class="input-field"
              type="number"
              min="1"
              step="1"
              autocomplete="off"
            />
            <span class="field-hint">Increment this when rotating existing material.</span>
          </label>
        </div>

        <div v-if="selectedProvider === 'fcm'" class="space-y-4">
          <label>
            <span class="field-label">Service account JSON</span>
            <textarea
              v-model="credentialDraft.fcmJson"
              class="input-field min-h-56 resize-y font-mono text-xs"
              autocomplete="off"
              spellcheck="false"
              placeholder='{ "type": "service_account", "project_id": "..." }'
            />
            <span class="field-hint">
              Paste the complete Google service-account object. It is encrypted before durable
              storage.
            </span>
          </label>
        </div>

        <div v-else-if="selectedProvider === 'apns'" class="space-y-4">
          <div>
            <span class="field-label">Authentication material</span>
            <div class="segmented">
              <button
                v-for="mode in (['token', 'p12', 'pem'] as ApnsMode[])"
                :key="mode"
                type="button"
                :class="{ active: credentialDraft.apnsMode === mode }"
                :aria-pressed="credentialDraft.apnsMode === mode"
                @click="credentialDraft.apnsMode = mode"
              >
                {{ mode === "token" ? "Apple .p8 key" : mode.toUpperCase() }}
              </button>
            </div>
          </div>

          <div v-if="credentialDraft.apnsMode === 'token'" class="grid md:grid-cols-2 gap-4">
            <label>
              <span class="field-label">Apple Team ID</span>
              <input
                v-model.trim="credentialDraft.apnsTeamId"
                class="input-field font-mono"
                autocomplete="off"
              />
            </label>
            <label>
              <span class="field-label">Key ID</span>
              <input
                v-model.trim="credentialDraft.apnsKeyId"
                class="input-field font-mono"
                autocomplete="off"
              />
            </label>
            <label class="md:col-span-2">
              <span class="field-label">Private .p8 key</span>
              <textarea
                v-model="credentialDraft.apnsPrivateKey"
                class="input-field min-h-44 resize-y font-mono text-xs"
                autocomplete="off"
                spellcheck="false"
                placeholder="-----BEGIN PRIVATE KEY-----"
              />
            </label>
          </div>

          <div v-else-if="credentialDraft.apnsMode === 'p12'" class="grid md:grid-cols-2 gap-4">
            <label class="md:col-span-2">
              <span class="field-label">PKCS#12 payload</span>
              <textarea
                v-model="credentialDraft.apnsP12"
                class="input-field min-h-36 resize-y font-mono text-xs"
                autocomplete="off"
                spellcheck="false"
                placeholder="Base64-encoded .p12 content"
              />
            </label>
            <label>
              <span class="field-label">Certificate password</span>
              <input
                v-model="credentialDraft.apnsP12Password"
                class="input-field"
                type="password"
                autocomplete="off"
              />
            </label>
          </div>

          <label v-else>
            <span class="field-label">PEM certificate and private key</span>
            <textarea
              v-model="credentialDraft.apnsPem"
              class="input-field min-h-52 resize-y font-mono text-xs"
              autocomplete="off"
              spellcheck="false"
              placeholder="-----BEGIN CERTIFICATE-----"
            />
          </label>

          <p class="alert border-sky-400/15 bg-sky-500/8 text-xs text-sky-200/80">
            The APNs topic and production/sandbox endpoint remain provider-worker runtime settings;
            they are not part of this stored credential record.
          </p>
        </div>

        <div v-else-if="selectedProvider === 'webpush'" class="grid md:grid-cols-2 gap-4">
          <label>
            <span class="field-label">VAPID public key</span>
            <textarea
              v-model="credentialDraft.webPushPublicKey"
              class="input-field min-h-32 resize-y font-mono text-xs"
              autocomplete="off"
              spellcheck="false"
            />
          </label>
          <label>
            <span class="field-label">VAPID private key</span>
            <textarea
              v-model="credentialDraft.webPushPrivateKey"
              class="input-field min-h-32 resize-y font-mono text-xs"
              autocomplete="off"
              spellcheck="false"
            />
          </label>
        </div>

        <div v-else-if="selectedProvider === 'hms'" class="grid md:grid-cols-2 gap-4">
          <label>
            <span class="field-label">HMS App ID</span>
            <input
              v-model.trim="credentialDraft.hmsAppId"
              class="input-field font-mono"
              autocomplete="off"
            />
          </label>
          <label>
            <span class="field-label">Client secret</span>
            <input
              v-model="credentialDraft.hmsClientSecret"
              class="input-field"
              type="password"
              autocomplete="off"
            />
          </label>
        </div>

        <div v-else-if="selectedProvider === 'wns'" class="grid md:grid-cols-2 gap-4">
          <label>
            <span class="field-label">Package SID</span>
            <input
              v-model.trim="credentialDraft.wnsPackageSid"
              class="input-field font-mono"
              autocomplete="off"
            />
          </label>
          <label>
            <span class="field-label">Client secret</span>
            <input
              v-model="credentialDraft.wnsClientSecret"
              class="input-field"
              type="password"
              autocomplete="off"
            />
          </label>
        </div>

        <div class="mt-6 flex flex-wrap items-center gap-3 border-t border-surface-800 pt-5">
          <button class="btn-primary flex items-center gap-2" type="submit" :disabled="savingCredential">
            <KeyRound class="w-4 h-4" />
            {{ savingCredential ? "Storing securely..." : "Store credential" }}
          </button>
          <button class="btn-secondary" type="button" @click="closeCredentialForm">Cancel</button>
          <span class="text-xs text-surface-600">
            Secret material cannot be retrieved after upload.
          </span>
        </div>
      </form>

      <div class="settings-card">
        <div class="card-heading">
          <div>
            <h2>Credential inventory</h2>
            <p>Metadata only. This list does not expose provider secret material or worker state.</p>
          </div>
        </div>

        <div v-if="credentials.length" class="overflow-x-auto">
          <table class="w-full min-w-160 text-sm">
            <thead class="text-left text-xs uppercase tracking-wider text-surface-500">
              <tr>
                <th class="pb-3 font-medium">Provider</th>
                <th class="pb-3 font-medium">Credential ID</th>
                <th class="pb-3 font-medium">Version</th>
                <th class="pb-3 text-right font-medium">Record state</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="credential in credentials"
                :key="`${credential.provider}:${credential.credential_id}`"
                class="border-t border-surface-800"
              >
                <td class="py-4 font-medium text-surface-200">
                  {{ providerLabel(credential.provider) }}
                </td>
                <td class="py-4 font-mono text-xs text-brand-200">
                  {{ credential.credential_id }}
                </td>
                <td class="py-4 text-surface-400">v{{ credential.version }}</td>
                <td class="py-4 text-right">
                  <span class="status-pill status-positive">Stored</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else-if="!credentialsLoading" class="inherit-state">
          No provider credentials are stored for this app.
        </div>
        <div v-else class="inherit-state">Loading stored credentials…</div>

        <button
          v-if="credentialsHasMore"
          class="btn-secondary btn-sm mt-4"
          type="button"
          :disabled="credentialsLoading"
          @click="loadCredentials(false)"
        >
          Load more credentials
        </button>
      </div>
    </section>

    <section v-else-if="activeSection === 'devices'" class="space-y-5">
      <div class="settings-card">
        <div class="card-heading">
          <div>
            <p class="eyebrow">Device inventory</p>
            <h2>Registered devices</h2>
            <p>Provider tokens stay redacted; hashes are shown for operational correlation.</p>
          </div>
          <Smartphone class="w-5 h-5 text-surface-500" />
        </div>

        <p v-if="devicesError" class="alert alert-error mb-4">{{ devicesError }}</p>

        <div v-if="devices.length" class="overflow-x-auto">
          <table class="w-full min-w-240 text-sm">
            <thead class="text-left text-xs uppercase tracking-wider text-surface-500">
              <tr>
                <th class="pb-3 font-medium">Device</th>
                <th class="pb-3 font-medium">Client</th>
                <th class="pb-3 font-medium">Platform</th>
                <th class="pb-3 font-medium">Provider</th>
                <th class="pb-3 font-medium">State</th>
                <th class="pb-3 font-medium">Last active</th>
                <th class="pb-3 text-right font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="device in devices" :key="device.id" class="border-t border-surface-800">
                <td class="py-4">
                  <p class="font-mono text-xs text-brand-200">{{ device.id }}</p>
                  <p class="mt-1 max-w-44 truncate font-mono text-[0.65rem] text-surface-600">
                    {{ device.recipient.tokenHash }}
                  </p>
                </td>
                <td class="py-4 text-surface-400">{{ device.clientId || "—" }}</td>
                <td class="py-4">
                  <p class="text-surface-300">{{ device.platform }}</p>
                  <p class="text-xs text-surface-600">{{ device.formFactor }}</p>
                </td>
                <td class="py-4 text-surface-400">{{ providerLabel(device.recipient.provider) }}</td>
                <td class="py-4">
                  <span
                    class="status-pill"
                    :class="
                      device.pushState === 'ACTIVE'
                        ? 'status-positive'
                        : device.pushState === 'FAILED'
                          ? 'status-negative'
                          : 'bg-amber-500/12 text-amber-300'
                    "
                  >
                    {{ device.pushState }}
                  </span>
                  <p v-if="device.pushFailureCount" class="mt-1 text-[0.65rem] text-surface-600">
                    {{ device.pushFailureCount }} failures
                  </p>
                </td>
                <td class="py-4 text-xs text-surface-500">
                  {{ formatDate(device.lastActiveAtMs) }}
                </td>
                <td class="py-4 text-right">
                  <button
                    v-if="auth.isAdmin"
                    class="icon-button danger"
                    type="button"
                    :disabled="deletingDeviceId === device.id"
                    title="Delete device registration"
                    :aria-label="`Delete device registration ${device.id}`"
                    @click="deleteDevice(device)"
                  >
                    <Trash2 class="w-4 h-4" />
                  </button>
                  <span v-else class="text-xs text-surface-600">Read only</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else-if="!devicesLoading" class="inherit-state">
          No devices are registered for this app.
        </div>
        <div v-else class="inherit-state">Loading device registrations…</div>

        <button
          v-if="devicesHasMore"
          class="btn-secondary btn-sm mt-4"
          type="button"
          :disabled="devicesLoading"
          @click="loadDevices(false)"
        >
          Load more devices
        </button>
      </div>

      <div class="settings-card">
        <div class="card-heading">
          <div>
            <p class="eyebrow">Audience routing</p>
            <h2>Channel subscriptions</h2>
            <p>See which registered devices receive channel-targeted notifications.</p>
          </div>
          <Users class="w-5 h-5 text-surface-500" />
        </div>

        <form class="grid gap-3 md:grid-cols-[1fr_1fr_auto] mb-5" @submit.prevent="loadSubscriptions(true)">
          <label>
            <span class="field-label">Channel</span>
            <input
              v-model="subscriptionChannel"
              class="input-field font-mono"
              placeholder="notifications:user-123"
            />
          </label>
          <label>
            <span class="field-label">Device ID</span>
            <input
              v-model="subscriptionDeviceId"
              class="input-field font-mono"
              placeholder="device-123"
            />
          </label>
          <button
            class="btn-secondary self-end flex items-center justify-center gap-2"
            type="submit"
            :disabled="subscriptionsLoading"
          >
            <Search class="w-4 h-4" />
            Filter
          </button>
        </form>

        <p v-if="subscriptionsError" class="alert alert-error mb-4">
          {{ subscriptionsError }}
        </p>

        <div v-if="subscriptions.length" class="overflow-x-auto">
          <table class="w-full min-w-200 text-sm">
            <thead class="text-left text-xs uppercase tracking-wider text-surface-500">
              <tr>
                <th class="pb-3 font-medium">Channel</th>
                <th class="pb-3 font-medium">Device</th>
                <th class="pb-3 font-medium">Client</th>
                <th class="pb-3 font-medium">Provider</th>
                <th class="pb-3 text-right font-medium">Credential version</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="subscription in subscriptions"
                :key="`${subscription.channel}:${subscription.deviceId}`"
                class="border-t border-surface-800"
              >
                <td class="py-4 font-mono text-xs text-brand-200">
                  {{ subscription.channel }}
                </td>
                <td class="py-4 font-mono text-xs text-surface-300">
                  {{ subscription.deviceId }}
                </td>
                <td class="py-4 text-surface-400">{{ subscription.clientId || "—" }}</td>
                <td class="py-4 text-surface-400">{{ providerLabel(subscription.provider) }}</td>
                <td class="py-4 text-right text-surface-500">
                  {{ subscription.credentialVersion ?? "default" }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else-if="!subscriptionsLoading" class="inherit-state">
          No matching channel subscriptions.
        </div>
        <div v-else class="inherit-state">Loading channel subscriptions…</div>

        <button
          v-if="subscriptionsHasMore"
          class="btn-secondary btn-sm mt-4"
          type="button"
          :disabled="subscriptionsLoading"
          @click="loadSubscriptions(false)"
        >
          Load more subscriptions
        </button>
      </div>
    </section>

    <section v-else-if="activeSection === 'delivery'" class="grid lg:grid-cols-2 gap-5 items-start">
      <div class="settings-card">
        <div class="card-heading">
          <div>
            <p class="eyebrow">Test delivery</p>
            <h2>Compose notification</h2>
            <p>Send one basic notification to a channel, device, or client target.</p>
          </div>
          <Send class="w-5 h-5 text-surface-500" />
        </div>

        <form v-if="auth.isAdmin" class="space-y-4" @submit.prevent="publishNotification">
          <p v-if="publishError" class="alert alert-error">{{ publishError }}</p>

          <div class="grid grid-cols-[9rem_minmax(0,1fr)] gap-3">
            <label>
              <span class="field-label">Target type</span>
              <select v-model="composer.targetType" class="input-field">
                <option value="channel">Channel</option>
                <option value="device">Device</option>
                <option value="client">Client</option>
              </select>
            </label>
            <label>
              <span class="field-label">
                {{ composer.targetType === "channel" ? "Channel" : composer.targetType === "device" ? "Device ID" : "Client ID" }}
              </span>
              <input
                v-model="composer.targetValue"
                class="input-field font-mono"
                :placeholder="
                  composer.targetType === 'channel'
                    ? 'notifications:user-123'
                    : composer.targetType === 'device'
                      ? 'device-123'
                      : 'client-123'
                "
              />
            </label>
          </div>

          <label>
            <span class="field-label">Title</span>
            <input v-model="composer.title" class="input-field" placeholder="Your order is ready" />
          </label>
          <label>
            <span class="field-label">Body</span>
            <textarea
              v-model="composer.body"
              class="input-field min-h-28 resize-y"
              placeholder="Open the app to see the latest update."
            />
          </label>

          <details class="subcard">
            <summary class="cursor-pointer text-sm font-medium text-surface-300">
              Optional delivery fields
            </summary>
            <div class="grid md:grid-cols-2 gap-4 mt-4">
              <label>
                <span class="field-label">Publish ID</span>
                <input v-model="composer.publishId" class="input-field font-mono" />
                <span class="field-hint">Reuse a stable ID for idempotent retries.</span>
              </label>
              <label>
                <span class="field-label">Collapse key</span>
                <input v-model="composer.collapseKey" class="input-field font-mono" />
              </label>
              <label>
                <span class="field-label">Icon</span>
                <input v-model="composer.icon" class="input-field" />
              </label>
              <label>
                <span class="field-label">Sound</span>
                <input v-model="composer.sound" class="input-field" />
              </label>
              <label class="md:col-span-2 flex items-start gap-3 rounded-lg bg-surface-950/30 p-3">
                <input v-model="composer.sync" type="checkbox" class="toggle-input mt-0.5" />
                <span>
                  <span class="block text-sm text-surface-300">Request synchronous fanout</span>
                  <span class="field-hint !mt-0">
                    Sockudo may force large fanouts to remain asynchronous.
                  </span>
                </span>
              </label>
            </div>
          </details>

          <button class="btn-primary w-full flex items-center justify-center gap-2" type="submit" :disabled="publishing">
            <Send class="w-4 h-4" />
            {{ publishing ? "Submitting..." : "Publish notification" }}
          </button>
        </form>

        <div v-else class="inherit-state">
          Publishing is available to administrators. You can still inspect any publish by ID.
        </div>

        <div v-if="acceptedPublish" class="mt-5 rounded-xl border border-emerald-400/20 bg-emerald-500/8 p-4">
          <div class="flex items-start justify-between gap-3">
            <div>
              <p class="text-sm font-medium text-emerald-200">Publish accepted</p>
              <p class="mt-1 font-mono text-xs text-emerald-300/80">
                {{ acceptedPublish.publishId }}
              </p>
            </div>
            <span class="status-pill status-positive">{{ acceptedPublish.status }}</span>
          </div>
          <div class="mt-4 grid grid-cols-2 gap-3 text-xs">
            <div class="rounded-lg bg-surface-950/30 p-3">
              <span class="text-surface-600">Expected recipients</span>
              <p class="mt-1 text-surface-200">{{ formatNumber(acceptedPublish.expectedRecipients) }}</p>
            </div>
            <div class="rounded-lg bg-surface-950/30 p-3">
              <span class="text-surface-600">Fanout regime</span>
              <p class="mt-1 text-surface-200">{{ acceptedPublish.fanoutRegime }}</p>
            </div>
          </div>
          <p v-if="acceptedPublish.renderedPayloads.length" class="mt-3 text-xs text-surface-500">
            Rendered for
            {{
              acceptedPublish.renderedPayloads
                .map((rendered) => providerLabel(rendered.provider))
                .join(", ")
            }}.
          </p>
        </div>
      </div>

      <div class="settings-card lg:sticky lg:top-6">
        <div class="card-heading">
          <div>
            <p class="eyebrow">Durable outcome</p>
            <h2>Publish status</h2>
            <p>Look up acceptance, dispatch, retry, and terminal delivery counters.</p>
          </div>
          <Search class="w-5 h-5 text-surface-500" />
        </div>

        <form class="flex gap-2" @submit.prevent="lookupPublishStatus">
          <input
            v-model="statusPublishId"
            class="input-field min-w-0 font-mono"
            placeholder="publish-id"
          />
          <button class="btn-secondary shrink-0" type="submit" :disabled="statusLoading">
            {{ statusLoading ? "Loading..." : "Look up" }}
          </button>
        </form>

        <p v-if="statusError" class="alert alert-error mt-4">{{ statusError }}</p>

        <div v-if="publishStatus" class="mt-5 space-y-4">
          <div class="subcard">
            <div class="flex items-start justify-between gap-3">
              <div>
                <p class="font-mono text-xs text-brand-200">{{ publishStatus.publishId }}</p>
                <p v-if="publishStatus.fanoutRegime" class="mt-1 text-xs text-surface-600">
                  {{ publishStatus.fanoutRegime }}
                </p>
              </div>
              <span
                class="status-pill"
                :class="
                  ['failed', 'rejected', 'expired'].includes(publishStatus.state.toLowerCase())
                    ? 'status-negative'
                    : ['delivered', 'complete', 'completed'].includes(
                          publishStatus.state.toLowerCase(),
                        )
                      ? 'status-positive'
                      : 'bg-brand-500/12 text-brand-200'
                "
              >
                {{ publishStatus.state }}
              </span>
            </div>
            <p v-if="publishStatus.errorReason" class="alert alert-error mt-4">
              {{ publishStatus.errorReason }}
            </p>
            <p v-if="publishStatus.retryAfterMs" class="mt-3 text-xs text-amber-300">
              Retry suggested after {{ formatNumber(publishStatus.retryAfterMs) }} ms.
            </p>
          </div>

          <div v-if="statusCounters.length" class="grid grid-cols-2 sm:grid-cols-3 gap-2">
            <div
              v-for="[counter, value] in statusCounters"
              :key="counter"
              class="rounded-lg border border-surface-800 bg-surface-950/30 p-3"
            >
              <p class="text-lg font-semibold text-surface-200">{{ formatNumber(value) }}</p>
              <p class="mt-1 break-words text-[0.65rem] text-surface-600">{{ counter }}</p>
            </div>
          </div>

          <button
            class="btn-secondary btn-sm flex items-center gap-2"
            type="button"
            :disabled="statusLoading"
            @click="lookupPublishStatus"
          >
            <RefreshCw class="w-3.5 h-3.5" />
            Refresh status
          </button>
        </div>

        <div v-else-if="!statusError" class="inherit-state mt-5">
          Enter a publish ID to inspect its durable status.
        </div>
      </div>
    </section>

    <section v-else class="settings-card">
      <div class="card-heading">
        <div>
          <p class="eyebrow">Failure recovery</p>
          <h2>Dead letters</h2>
          <p>Inspect failed push work and replay only entries Sockudo marks as replayable.</p>
        </div>
        <AlertTriangle class="w-5 h-5 text-surface-500" />
      </div>

      <div class="mb-5 flex flex-wrap items-end gap-3">
        <label class="w-52">
          <span class="field-label">Provider</span>
          <select v-model="deadLetterProvider" class="input-field" @change="loadDeadLetters(true)">
            <option value="">All providers</option>
            <option v-for="provider in providers" :key="provider.id" :value="provider.id">
              {{ provider.label }}
            </option>
          </select>
        </label>
        <button
          class="btn-secondary flex items-center gap-2"
          type="button"
          :disabled="deadLettersLoading"
          @click="loadDeadLetters(true)"
        >
          <RefreshCw class="w-4 h-4" />
          Refresh failures
        </button>
      </div>

      <p v-if="deadLettersError" class="alert alert-error mb-4">{{ deadLettersError }}</p>

      <div v-if="deadLetters.length" class="space-y-3">
        <article
          v-for="deadLetter in deadLetters"
          :key="deadLetter.deadLetterId"
          class="subcard"
        >
          <div class="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
            <div class="min-w-0">
              <div class="flex flex-wrap items-center gap-2">
                <span class="status-pill status-negative">{{ deadLetter.stage }}</span>
                <span class="status-pill bg-surface-800 text-surface-400">
                  {{ providerLabel(deadLetter.provider) }}
                </span>
                <span
                  class="status-pill"
                  :class="
                    deadLetter.replayable
                      ? 'bg-amber-500/12 text-amber-300'
                      : 'bg-surface-800 text-surface-600'
                  "
                >
                  {{ deadLetter.replayable ? "Replayable" : "Terminal" }}
                </span>
              </div>
              <p class="mt-3 text-sm leading-6 text-surface-300">{{ deadLetter.reason }}</p>
              <dl class="mt-4 grid gap-x-6 gap-y-2 text-xs sm:grid-cols-2">
                <div>
                  <dt class="text-surface-600">Publish</dt>
                  <dd class="mt-0.5 break-all font-mono text-brand-200">
                    {{ deadLetter.publishId }}
                  </dd>
                </div>
                <div>
                  <dt class="text-surface-600">Occurred</dt>
                  <dd class="mt-0.5 text-surface-400">{{ formatDate(deadLetter.occurredAtMs) }}</dd>
                </div>
                <div>
                  <dt class="text-surface-600">Dead-letter ID</dt>
                  <dd class="mt-0.5 break-all font-mono text-surface-400">
                    {{ deadLetter.deadLetterId }}
                  </dd>
                </div>
                <div>
                  <dt class="text-surface-600">Key</dt>
                  <dd class="mt-0.5 break-all font-mono text-surface-400">
                    {{ deadLetter.key }}
                  </dd>
                </div>
              </dl>
            </div>
            <button
              v-if="auth.isAdmin && deadLetter.replayable"
              class="btn-secondary btn-sm shrink-0 flex items-center gap-2"
              type="button"
              :disabled="replayingDeadLetterId === deadLetter.deadLetterId"
              :aria-label="`Replay dead letter ${deadLetter.deadLetterId}`"
              @click="replayDeadLetter(deadLetter)"
            >
              <RotateCcw class="w-4 h-4" />
              {{
                replayingDeadLetterId === deadLetter.deadLetterId
                  ? "Replaying..."
                  : "Replay"
              }}
            </button>
          </div>
        </article>
      </div>
      <div v-else-if="!deadLettersLoading" class="inherit-state">
        No matching dead letters.
      </div>
      <div v-else class="inherit-state">Loading dead letters…</div>

      <button
        v-if="deadLettersHasMore"
        class="btn-secondary btn-sm mt-4"
        type="button"
        :disabled="deadLettersLoading"
        @click="loadDeadLetters(false)"
      >
        Load more dead letters
      </button>
    </section>
  </div>
</template>
