<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import {
  AppWindow,
  ArrowRight,
  Gauge,
  KeyRound,
  Plus,
  RadioTower,
  RefreshCw,
  Trash2,
  UsersRound,
  Webhook,
  X,
} from "lucide-vue-next";
import { api, type App, type StatsResponse } from "@/api/client";

const router = useRouter();
const apps = ref<App[]>([]);
const stats = ref<StatsResponse | null>(null);
const loading = ref(true);
const error = ref<string | null>(null);
const showCreate = ref(false);
const enabledApps = computed(() => apps.value.filter((app) => app.enabled).length);

const form = ref({
  id: "",
  key: "",
  secret: "",
  enabled: true,
});

onMounted(load);

async function load() {
  loading.value = true;
  error.value = null;
  try {
    const [appList, statsData] = await Promise.all([
      api.listApps(),
      api.stats().catch(() => null),
    ]);
    apps.value = appList;
    stats.value = statsData;
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Failed to load apps";
  } finally {
    loading.value = false;
  }
}

function connectionsFor(appId: string) {
  return stats.value?.apps.find((a) => a.app_id === appId)?.connections ?? 0;
}

function appInitials(appId: string) {
  return appId
    .split(/[-_\s]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");
}

function webhookCount(app: App) {
  return app.webhook_count ?? app.policy.webhooks?.length ?? 0;
}

async function createApp() {
  try {
    const created = await api.createApp({
      id: form.value.id.trim(),
      key: form.value.key.trim(),
      secret: form.value.secret.trim() || undefined,
      enabled: form.value.enabled,
    });
    showCreate.value = false;
    form.value = { id: "", key: "", secret: "", enabled: true };
    router.push({ name: "app-detail", params: { id: created.id } });
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Create failed";
  }
}

async function removeApp(app: App) {
  if (!confirm(`Delete app "${app.id}"?`)) return;
  try {
    await api.deleteApp(app.id);
    await load();
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Delete failed";
  }
}
</script>

<template>
  <div>
    <div class="page-header">
      <div>
        <p class="eyebrow">Application manager</p>
        <h1 class="page-title">Applications</h1>
        <p class="page-subtitle">
          Configure credentials, delivery policies, limits, and integrations for every realtime
          app.
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button
          class="btn-secondary flex items-center gap-2"
          aria-label="Refresh applications"
          :disabled="loading"
          @click="load"
        >
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" />
          <span class="hidden sm:inline">Refresh</span>
        </button>
        <button class="btn-primary flex items-center gap-2" @click="showCreate = !showCreate">
          <Plus class="h-4 w-4" />
          New app
        </button>
      </div>
    </div>

    <div class="mb-6 grid grid-cols-2 gap-3 lg:grid-cols-4 lg:gap-4">
      <div class="panel relative overflow-hidden p-4 sm:p-5">
        <AppWindow class="absolute right-4 top-4 h-5 w-5 text-brand-400/50" />
        <p class="text-xs font-medium text-surface-500">Registered apps</p>
        <p class="mt-2 text-2xl font-semibold tracking-tight text-surface-50">{{ apps.length }}</p>
        <p class="mt-1 text-[0.68rem] text-surface-600">{{ enabledApps }} currently enabled</p>
      </div>
      <div class="panel relative overflow-hidden p-4 sm:p-5">
        <RadioTower class="absolute right-4 top-4 h-5 w-5 text-emerald-400/50" />
        <p class="text-xs font-medium text-surface-500">Live connections</p>
        <p class="mt-2 text-2xl font-semibold tracking-tight text-surface-50">
          {{ stats?.totals.connections ?? "—" }}
        </p>
        <p class="mt-1 flex items-center gap-1.5 text-[0.68rem] text-surface-600">
          <span
            class="h-1.5 w-1.5 rounded-full"
            :class="stats ? 'bg-emerald-400' : 'bg-surface-600'"
          />
          {{ stats ? "Runtime connected" : "Runtime unavailable" }}
        </p>
      </div>
      <div class="panel relative overflow-hidden p-4 sm:p-5">
        <UsersRound class="absolute right-4 top-4 h-5 w-5 text-sky-400/50" />
        <p class="text-xs font-medium text-surface-500">Users online</p>
        <p class="mt-2 text-2xl font-semibold tracking-tight text-surface-50">
          {{ stats?.totals.users ?? "—" }}
        </p>
        <p class="mt-1 text-[0.68rem] text-surface-600">Across all applications</p>
      </div>
      <div class="panel relative overflow-hidden p-4 sm:p-5">
        <Gauge class="absolute right-4 top-4 h-5 w-5 text-amber-400/50" />
        <p class="text-xs font-medium text-surface-500">Memory used</p>
        <p class="mt-2 text-2xl font-semibold tracking-tight text-surface-50">
          {{ stats ? `${stats.memory.percent.toFixed(1)}%` : "—" }}
        </p>
        <div class="mt-2 h-1 overflow-hidden rounded-full bg-surface-800">
          <div
            class="h-full rounded-full bg-gradient-to-r from-brand-500 to-sky-400 transition-all"
            :style="{ width: `${Math.min(stats?.memory.percent ?? 0, 100)}%` }"
          />
        </div>
      </div>
    </div>

    <form v-if="showCreate" class="panel mb-6 overflow-hidden" @submit.prevent="createApp">
      <div
        class="flex items-start justify-between gap-4 border-b border-surface-800 bg-surface-900/40 px-5 py-4 sm:px-6"
      >
        <div>
          <p class="text-sm font-semibold text-surface-100">Create application</p>
          <p class="mt-1 text-xs text-surface-500">
            Give the app a stable identifier and API key. Its secret can be generated for you.
          </p>
        </div>
        <button
          type="button"
          class="icon-button"
          aria-label="Close create form"
          @click="showCreate = false"
        >
          <X class="h-4 w-4" />
        </button>
      </div>
      <div class="grid gap-5 px-5 py-5 sm:px-6 md:grid-cols-2">
        <div>
          <label for="new-app-id" class="field-label">App ID</label>
          <input
            id="new-app-id"
            v-model="form.id"
            class="input-field font-mono"
            placeholder="customer-portal"
            required
          />
          <span class="field-hint">Used in API routes and operational reporting.</span>
        </div>
        <div>
          <label for="new-app-key" class="field-label">App key</label>
          <input
            id="new-app-key"
            v-model="form.key"
            class="input-field font-mono"
            placeholder="customer-portal-key"
            required
          />
          <span class="field-hint">Public identifier used by realtime clients.</span>
        </div>
        <div>
          <label for="new-app-secret" class="field-label">Secret</label>
          <div class="relative">
            <KeyRound
              class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-surface-600"
            />
            <input
              id="new-app-secret"
              v-model="form.secret"
              class="input-field pl-9 font-mono"
              placeholder="Auto-generate a secure secret"
            />
          </div>
          <span class="field-hint">Optional. Leave blank to generate one automatically.</span>
        </div>
        <label
          class="flex cursor-pointer items-center justify-between gap-4 rounded-xl border border-surface-800 bg-surface-950/30 px-4 py-3"
        >
          <span>
            <span class="block text-sm font-medium text-surface-200">Enable immediately</span>
            <span class="mt-1 block text-xs text-surface-500">
              Accept connections as soon as the app is created.
            </span>
          </span>
          <input v-model="form.enabled" type="checkbox" class="toggle-input" />
        </label>
      </div>
      <div
        class="flex justify-end gap-2 border-t border-surface-800 bg-surface-950/25 px-5 py-4 sm:px-6"
      >
        <button type="button" class="btn-secondary" @click="showCreate = false">Cancel</button>
        <button type="submit" class="btn-primary flex items-center gap-2">
          <Plus class="h-4 w-4" />
          Create application
        </button>
      </div>
    </form>

    <div v-if="error" class="alert alert-error mb-5 flex items-start justify-between gap-4">
      <span>{{ error }}</span>
      <button class="text-red-200/70 hover:text-red-100" aria-label="Dismiss error" @click="error = null">
        <X class="h-4 w-4" />
      </button>
    </div>

    <div v-if="loading" class="panel overflow-hidden">
      <div class="border-b border-surface-800 px-5 py-4">
        <div class="h-4 w-36 animate-pulse rounded bg-surface-800" />
      </div>
      <div v-for="index in 4" :key="index" class="flex items-center gap-4 border-b border-surface-800/70 px-5 py-4 last:border-0">
        <div class="h-11 w-11 animate-pulse rounded-xl bg-surface-800" />
        <div class="flex-1 space-y-2">
          <div class="h-3 w-32 animate-pulse rounded bg-surface-800" />
          <div class="h-2.5 w-48 animate-pulse rounded bg-surface-800/70" />
        </div>
      </div>
    </div>

    <div v-else-if="apps.length === 0" class="empty-state">
      <div
        class="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-brand-500/10 text-brand-300"
      >
        <AppWindow class="h-5 w-5" />
      </div>
      <p class="font-medium text-surface-200">No applications yet</p>
      <p class="mx-auto mt-1 max-w-sm text-xs leading-5 text-surface-500">
        Create your first app to issue credentials, accept realtime connections, and configure
        delivery policies.
      </p>
      <button class="btn-primary mt-5 inline-flex items-center gap-2" @click="showCreate = true">
        <Plus class="h-4 w-4" />
        Create first app
      </button>
    </div>

    <div v-else>
      <div class="panel hidden overflow-hidden md:block">
        <div class="flex items-center justify-between border-b border-surface-800 px-5 py-4">
          <div>
            <p class="text-sm font-semibold text-surface-200">Application registry</p>
            <p class="mt-0.5 text-xs text-surface-600">
              {{ apps.length }} {{ apps.length === 1 ? "application" : "applications" }} configured
            </p>
          </div>
          <span class="status-pill status-positive">
            {{ enabledApps }} active
          </span>
        </div>
        <table class="w-full text-sm">
          <thead class="bg-surface-950/25 text-left">
          <tr>
              <th class="px-5 py-3 text-[0.68rem] font-semibold uppercase tracking-wider text-surface-600">
                Application
              </th>
              <th class="px-4 py-3 text-[0.68rem] font-semibold uppercase tracking-wider text-surface-600">
                Status
              </th>
              <th class="px-4 py-3 text-[0.68rem] font-semibold uppercase tracking-wider text-surface-600">
                Connections
              </th>
              <th class="px-4 py-3 text-[0.68rem] font-semibold uppercase tracking-wider text-surface-600">
                Webhooks
              </th>
              <th class="px-5 py-3 text-right text-[0.68rem] font-semibold uppercase tracking-wider text-surface-600">
                Actions
              </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="app in apps"
            :key="app.id"
              class="border-t border-surface-800/80 transition-colors hover:bg-surface-800/25"
          >
              <td class="px-5 py-4">
                <div class="flex min-w-0 items-center gap-3">
                  <div class="app-avatar">{{ appInitials(app.id) }}</div>
                  <div class="min-w-0">
                    <p class="truncate font-medium text-surface-100">{{ app.id }}</p>
                    <p class="mt-1 flex items-center gap-1.5 truncate font-mono text-[0.68rem] text-surface-600">
                      <KeyRound class="h-3 w-3 shrink-0" />
                      {{ app.key }}
                    </p>
                  </div>
                </div>
              </td>
              <td class="px-4 py-4">
              <span
                  class="status-pill"
                  :class="app.enabled ? 'status-positive' : 'status-negative'"
              >
                  {{ app.enabled ? "Enabled" : "Disabled" }}
              </span>
            </td>
              <td class="px-4 py-4">
                <span class="inline-flex items-center gap-2 text-surface-300">
                  <span
                    class="h-1.5 w-1.5 rounded-full"
                    :class="connectionsFor(app.id) > 0 ? 'bg-emerald-400' : 'bg-surface-700'"
                  />
                  {{ connectionsFor(app.id) }}
                </span>
              </td>
              <td class="px-4 py-4">
                <span class="inline-flex items-center gap-2 text-surface-400">
                  <Webhook class="h-3.5 w-3.5 text-surface-600" />
                  {{ webhookCount(app) }}
                </span>
              </td>
              <td class="px-5 py-4">
                <div class="flex justify-end gap-2">
                <button
                    class="btn-secondary btn-sm flex items-center gap-1.5"
                  @click="router.push({ name: 'app-detail', params: { id: app.id } })"
                >
                    Configure
                    <ArrowRight class="h-3 w-3" />
                </button>
                <button
                    class="icon-button danger"
                    :aria-label="`Delete ${app.id}`"
                  @click="removeApp(app)"
                >
                    <Trash2 class="h-3.5 w-3.5" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
      </div>

      <div class="space-y-3 md:hidden">
        <article v-for="app in apps" :key="app.id" class="panel p-4">
          <div class="flex items-start gap-3">
            <div class="app-avatar">{{ appInitials(app.id) }}</div>
            <div class="min-w-0 flex-1">
              <div class="flex items-start justify-between gap-2">
                <div class="min-w-0">
                  <h2 class="truncate text-sm font-semibold text-surface-100">{{ app.id }}</h2>
                  <p class="mt-1 truncate font-mono text-[0.68rem] text-surface-600">{{ app.key }}</p>
                </div>
                <span
                  class="status-pill"
                  :class="app.enabled ? 'status-positive' : 'status-negative'"
                >
                  {{ app.enabled ? "Enabled" : "Disabled" }}
                </span>
              </div>
              <div class="mt-4 grid grid-cols-2 gap-2">
                <div class="rounded-lg bg-surface-950/40 px-3 py-2">
                  <p class="text-[0.62rem] uppercase tracking-wide text-surface-600">Connections</p>
                  <p class="mt-1 text-sm font-medium text-surface-300">
                    {{ connectionsFor(app.id) }}
                  </p>
                </div>
                <div class="rounded-lg bg-surface-950/40 px-3 py-2">
                  <p class="text-[0.62rem] uppercase tracking-wide text-surface-600">Webhooks</p>
                  <p class="mt-1 text-sm font-medium text-surface-300">{{ webhookCount(app) }}</p>
                </div>
              </div>
            </div>
          </div>
          <div class="mt-4 flex gap-2 border-t border-surface-800 pt-3">
            <button
              class="btn-secondary flex flex-1 items-center justify-center gap-2"
              @click="router.push({ name: 'app-detail', params: { id: app.id } })"
            >
              Configure
              <ArrowRight class="h-3.5 w-3.5" />
            </button>
            <button
              class="icon-button danger h-auto w-10"
              :aria-label="`Delete ${app.id}`"
              @click="removeApp(app)"
            >
              <Trash2 class="h-3.5 w-3.5" />
            </button>
          </div>
        </article>
      </div>
    </div>
  </div>
</template>
