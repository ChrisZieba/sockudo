<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  Activity,
  Gauge,
  Pause,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Trash2,
  X,
} from "lucide-vue-next";
import MetricChart from "@/components/MetricChart.vue";
import {
  api,
  type MetricFamily,
  type MetricSample,
  type MetricsResponse,
} from "@/api/client";

type ViewMode = "value" | "rate" | "average";

interface PanelConfig {
  id: string;
  metric: string;
  title: string;
  view: ViewMode;
  color: string;
}

interface Point {
  at: number;
  value: number;
}

interface StoredMetricsState {
  version: 1;
  interval: number;
  appFilter: string;
  panels: PanelConfig[];
  history: Record<string, Point[]>;
}

const STORAGE_KEY = "sockudo.metrics.workbench.v1";
const MAX_POINTS = 180;
const MAX_POINT_AGE_MS = 6 * 60 * 60 * 1_000;
const COLORS = ["#748ffc", "#38bdf8", "#34d399", "#fbbf24", "#f472b6", "#a78bfa"];

const families = ref<MetricFamily[]>([]);
const panels = ref<PanelConfig[]>([]);
const history = ref<Record<string, Point[]>>({});
const currentValues = ref<Record<string, number | undefined>>({});
const intervalSeconds = ref(15);
const appFilter = ref("*");
const loading = ref(true);
const refreshing = ref(false);
const error = ref<string | null>(null);
const lastScrape = ref<string | null>(null);
const showPicker = ref(false);
const pickerSearch = ref("");
const seededDefaults = ref(false);
const isHidden = ref(false);
const previousCounters = new Map<string, { at: number; value: number }>();
let timer: ReturnType<typeof setTimeout> | null = null;
let inFlight = false;

const availableApps = computed(() => {
  const ids = new Set<string>();
  for (const family of families.value) {
    for (const sample of family.samples) {
      const id = sample.labels.app_id ?? sample.labels.app;
      if (id) ids.add(id);
    }
  }
  return [...ids].sort();
});

const filteredFamilies = computed(() => {
  const query = pickerSearch.value.trim().toLowerCase();
  return families.value
    .filter((family) => {
      if (!query) return true;
      return (
        family.name.toLowerCase().includes(query) ||
        family.help?.toLowerCase().includes(query) ||
        categoryFor(family.name).toLowerCase().includes(query)
      );
    })
    .sort((left, right) => {
      const category = categoryFor(left.name).localeCompare(categoryFor(right.name));
      return category || left.name.localeCompare(right.name);
    });
});

onMounted(() => {
  restore();
  isHidden.value = document.hidden;
  document.addEventListener("visibilitychange", visibilityChanged);
  void load();
});

onUnmounted(() => {
  if (timer) clearTimeout(timer);
  document.removeEventListener("visibilitychange", visibilityChanged);
});

watch(intervalSeconds, () => {
  saveState();
  schedule();
});

watch(appFilter, () => {
  previousCounters.clear();
  history.value = Object.fromEntries(panels.value.map((panel) => [panel.id, []]));
  currentValues.value = {};
  saveState();
  void load();
});

function restore() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    const stored = JSON.parse(raw) as StoredMetricsState;
    if (stored.version !== 1 || !Array.isArray(stored.panels)) return;
    const cutoff = Date.now() - MAX_POINT_AGE_MS;
    panels.value = stored.panels;
    intervalSeconds.value = [5, 15, 30, 60].includes(stored.interval)
      ? stored.interval
      : 15;
    appFilter.value = stored.appFilter || "*";
    history.value = Object.fromEntries(
      Object.entries(stored.history ?? {}).map(([id, points]) => [
        id,
        points
          .filter(
            (point) =>
              Number.isFinite(point.at) &&
              Number.isFinite(point.value) &&
              point.at >= cutoff,
          )
          .slice(-MAX_POINTS),
      ]),
    );
    seededDefaults.value = true;
  } catch {
    localStorage.removeItem(STORAGE_KEY);
  }
}

function saveState() {
  const state: StoredMetricsState = {
    version: 1,
    interval: intervalSeconds.value,
    appFilter: appFilter.value,
    panels: panels.value,
    history: history.value,
  };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
}

function visibilityChanged() {
  isHidden.value = document.hidden;
  if (!document.hidden) void load();
}

function schedule() {
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => void load(), intervalSeconds.value * 1_000);
}

async function load() {
  if (inFlight) return;
  if (document.hidden) {
    schedule();
    return;
  }
  inFlight = true;
  refreshing.value = true;
  try {
    const response = await api.metrics();
    families.value = normalizeFamilies(response);
    lastScrape.value = response.scraped_at ?? new Date().toISOString();
    seedPanels();
    samplePanels(Date.now());
    error.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Metrics unavailable";
  } finally {
    inFlight = false;
    loading.value = false;
    refreshing.value = false;
    schedule();
  }
}

function normalizeFamilies(response: MetricsResponse): MetricFamily[] {
  if (response.families?.length) return response.families;
  const grouped = new Map<string, MetricSample[]>();
  for (const sample of response.samples) {
    const samples = grouped.get(sample.name) ?? [];
    samples.push(sample);
    grouped.set(sample.name, samples);
  }
  return [...grouped.entries()].map(([name, samples]) => ({
    name,
    type: name.endsWith("_total") ? "counter" : "untyped",
    samples,
  }));
}

function seedPanels() {
  if (seededDefaults.value) return;
  const defaults: Array<[string, string, ViewMode]> = [
    ["connected", "Connected sockets", "value"],
    ["ws_messages_received_total", "WebSocket messages in", "rate"],
    ["ws_messages_sent_total", "WebSocket messages out", "rate"],
    ["http_calls_received_total", "HTTP API calls", "rate"],
    ["connection_errors_total", "Connection errors", "rate"],
    ["process_resident_memory_bytes", "Resident memory", "value"],
    ["tokio_active_tasks", "Tokio active tasks", "value"],
    ["push_publish_accepted_total", "Push accepted", "rate"],
  ];

  const used = new Set<string>();
  for (const [suffix, title, view] of defaults) {
    const family = families.value
      .filter((candidate) => candidate.name.endsWith(suffix))
      .sort((left, right) => left.name.length - right.name.length)[0];
    if (!family || used.has(family.name)) continue;
    used.add(family.name);
    panels.value.push({
      id: crypto.randomUUID(),
      metric: family.name,
      title,
      view,
      color: COLORS[panels.value.length % COLORS.length],
    });
  }
  if (panels.value.length === 0) {
    for (const family of families.value.slice(0, 6)) addPanel(family, false);
  }
  seededDefaults.value = true;
  saveState();
}

function samplesFor(family: MetricFamily): MetricSample[] {
  if (appFilter.value === "*") return family.samples;
  const isAppScoped = family.samples.some(
    (sample) => sample.labels.app_id !== undefined || sample.labels.app !== undefined,
  );
  if (!isAppScoped) return family.samples;
  return family.samples.filter(
    (sample) =>
      sample.labels.app_id === appFilter.value || sample.labels.app === appFilter.value,
  );
}

function panelScopeLabel(panel: PanelConfig) {
  if (appFilter.value === "*") return "All app label values";
  const family = families.value.find((candidate) => candidate.name === panel.metric);
  const isAppScoped = family?.samples.some(
    (sample) => sample.labels.app_id !== undefined || sample.labels.app !== undefined,
  );
  return isAppScoped ? `app = ${appFilter.value}` : "Global metric (not app-labelled)";
}

function rawValue(family: MetricFamily, view: ViewMode): number | undefined {
  const samples = samplesFor(family);
  if (samples.length === 0) return 0;
  if (view === "average" || family.type === "histogram" || family.type === "summary") {
    const sum = samples
      .filter((sample) => sample.name === `${family.name}_sum`)
      .reduce((total, sample) => total + sample.value, 0);
    const count = samples
      .filter((sample) => sample.name === `${family.name}_count`)
      .reduce((total, sample) => total + sample.value, 0);
    return count > 0 ? sum / count : 0;
  }
  const primary = samples.filter(
    (sample) =>
      sample.name === family.name ||
      sample.name === `${family.name}_total` ||
      (!sample.name.endsWith("_bucket") &&
        !sample.name.endsWith("_sum") &&
        !sample.name.endsWith("_count")),
  );
  return primary.reduce((total, sample) => total + sample.value, 0);
}

function samplePanel(panel: PanelConfig, now: number) {
  const family = families.value.find((candidate) => candidate.name === panel.metric);
  if (!family) {
    currentValues.value[panel.id] = undefined;
    return;
  }
  const raw = rawValue(family, panel.view);
  if (raw === undefined) return;

  let value: number | undefined = raw;
  if (panel.view === "rate") {
    const previousKey = `${panel.id}:${appFilter.value}`;
    const previous = previousCounters.get(previousKey);
    previousCounters.set(previousKey, { at: now, value: raw });
    if (!previous || now <= previous.at) value = undefined;
    else {
      const delta = raw >= previous.value ? raw - previous.value : raw;
      value = delta / ((now - previous.at) / 1_000);
    }
  }
  currentValues.value[panel.id] = value;
  if (value === undefined || !Number.isFinite(value)) return;
  const cutoff = now - MAX_POINT_AGE_MS;
  const points = (history.value[panel.id] ?? [])
    .filter((point) => point.at >= cutoff)
    .concat({ at: now, value })
    .slice(-MAX_POINTS);
  history.value[panel.id] = points;
}

function samplePanels(now: number) {
  for (const panel of panels.value) samplePanel(panel, now);
  saveState();
}

function addPanel(family: MetricFamily, closePicker = true) {
  const view: ViewMode =
    family.type === "counter"
      ? "rate"
      : family.type === "histogram" || family.type === "summary"
        ? "average"
        : "value";
  const panel: PanelConfig = {
    id: crypto.randomUUID(),
    metric: family.name,
    title: prettyMetricName(family.name),
    view,
    color: COLORS[panels.value.length % COLORS.length],
  };
  panels.value.push(panel);
  history.value[panel.id] = [];
  if (closePicker) showPicker.value = false;
  saveState();
  samplePanel(panel, Date.now());
}

function removePanel(id: string) {
  panels.value = panels.value.filter((panel) => panel.id !== id);
  delete history.value[id];
  delete currentValues.value[id];
  previousCounters.delete(`${id}:${appFilter.value}`);
  saveState();
}

function changeView(panel: PanelConfig) {
  history.value[panel.id] = [];
  delete currentValues.value[panel.id];
  previousCounters.delete(`${panel.id}:${appFilter.value}`);
  saveState();
  samplePanel(panel, Date.now());
}

function resetDashboard() {
  if (!confirm("Reset the metrics dashboard to its discovered defaults?")) return;
  panels.value = [];
  history.value = {};
  currentValues.value = {};
  previousCounters.clear();
  seededDefaults.value = false;
  localStorage.removeItem(STORAGE_KEY);
  seedPanels();
  samplePanels(Date.now());
}

function categoryFor(name: string) {
  if (name.includes("_push_")) return "Push";
  if (name.includes("_history") || name.includes("_recovery")) return "History & recovery";
  if (name.includes("_ai_")) return "AI transport";
  if (name.includes("_queue_")) return "Queue";
  if (
    name.startsWith("process_") ||
    name.includes("_process_") ||
    name.includes("_tokio_")
  ) {
    return "Runtime";
  }
  if (name.includes("_annotation")) return "Annotations";
  if (name.includes("_presence")) return "Presence";
  if (name.includes("_delta")) return "Delta compression";
  return "Realtime";
}

function prettyMetricName(name: string) {
  return name
    .replace(/^sockudo_/, "")
    .replace(/_total$/, "")
    .split("_")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function formatValue(panel: PanelConfig, value?: number) {
  if (value === undefined) return "—";
  if (panel.metric.endsWith("_bytes")) {
    const units = ["B", "KiB", "MiB", "GiB"];
    let amount = value;
    let unit = 0;
    while (Math.abs(amount) >= 1024 && unit < units.length - 1) {
      amount /= 1024;
      unit += 1;
    }
    return `${amount.toFixed(amount >= 10 ? 1 : 2)} ${units[unit]}`;
  }
  const formatted = new Intl.NumberFormat(undefined, {
    notation: Math.abs(value) >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: Math.abs(value) < 10 ? 2 : 1,
  }).format(value);
  return panel.view === "rate" ? `${formatted}/s` : formatted;
}

function lastScrapeLabel() {
  if (!lastScrape.value) return "Not scraped yet";
  const date = new Date(lastScrape.value);
  return Number.isNaN(date.getTime()) ? "Just now" : date.toLocaleTimeString();
}
</script>

<template>
  <div>
    <header class="page-header">
      <div>
        <div class="flex items-center gap-2 text-brand-300 mb-2">
          <Activity class="w-4 h-4" />
          <span class="eyebrow !mb-0">Observability workbench</span>
        </div>
        <h1 class="page-title">Metrics</h1>
        <p class="page-subtitle">
          Build a live dashboard from every Prometheus family exported by Sockudo.
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <select
          v-model.number="intervalSeconds"
          class="input-field !w-auto"
          aria-label="Scrape interval"
        >
          <option :value="5">5 second scrape</option>
          <option :value="15">15 second scrape</option>
          <option :value="30">30 second scrape</option>
          <option :value="60">60 second scrape</option>
        </select>
        <button class="btn-secondary flex items-center gap-2" :disabled="refreshing" @click="load">
          <RefreshCw class="w-4 h-4" :class="{ 'animate-spin': refreshing }" />
          Refresh
        </button>
        <button class="btn-primary flex items-center gap-2" @click="showPicker = true">
          <Plus class="w-4 h-4" />
          Add panel
        </button>
      </div>
    </header>

    <div v-if="error" class="alert alert-warning mb-5">
      {{ error }} — ensure the Sockudo metrics listener is reachable. Existing chart history is
      preserved.
    </div>

    <section class="panel p-4 mb-5">
      <div class="flex flex-col lg:flex-row lg:items-center justify-between gap-4">
        <div class="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
          <span class="inline-flex items-center gap-2 text-surface-300">
            <span
              class="w-2 h-2 rounded-full"
              :class="error ? 'bg-amber-400' : 'bg-emerald-400 animate-pulse'"
            />
            {{ error ? "Datasource degraded" : "Scraping live" }}
          </span>
          <span class="text-surface-500">{{ families.length }} metric families discovered</span>
          <span class="text-surface-500">Last scrape {{ lastScrapeLabel() }}</span>
          <span v-if="isHidden" class="inline-flex items-center gap-1 text-surface-500">
            <Pause class="w-3 h-3" />
            Paused while hidden
          </span>
        </div>
        <div class="flex items-center gap-2">
          <label class="text-xs text-surface-500">App</label>
          <select
            v-model="appFilter"
            class="input-field !w-auto min-w-36"
            aria-label="Filter metrics by app"
          >
            <option value="*">All apps</option>
            <option v-for="id in availableApps" :key="id" :value="id">{{ id }}</option>
          </select>
          <button class="icon-button" title="Reset dashboard" @click="resetDashboard">
            <Settings2 class="w-4 h-4" />
          </button>
        </div>
      </div>
    </section>

    <div v-if="loading && families.length === 0" class="empty-state">
      Discovering Prometheus metrics…
    </div>

    <div v-else-if="panels.length === 0" class="empty-state">
      <Gauge class="w-8 h-8 text-surface-600 mx-auto mb-3" />
      <p class="text-surface-300 font-medium">Your workbench is empty</p>
      <p class="mt-1">Add any observed Sockudo, runtime, queue, history, AI, or push metric.</p>
      <button class="btn-primary mt-4" @click="showPicker = true">Add first panel</button>
    </div>

    <div v-else class="grid md:grid-cols-2 xl:grid-cols-3 gap-4">
      <article v-for="panel in panels" :key="panel.id" class="metric-panel">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <p class="text-sm font-medium text-surface-200 truncate">{{ panel.title }}</p>
            <p class="font-mono text-[0.65rem] text-surface-600 truncate mt-1">
              {{ panel.metric }}
            </p>
          </div>
          <div class="flex items-center gap-1 shrink-0">
            <select
              v-model="panel.view"
              class="panel-select"
              :aria-label="`View mode for ${panel.title}`"
              @change="changeView(panel)"
            >
              <option value="value">Value</option>
              <option value="rate">Rate</option>
              <option value="average">Average</option>
            </select>
            <button
              class="icon-button !w-7 !h-7 danger"
              :aria-label="`Remove ${panel.title} panel`"
              @click="removePanel(panel.id)"
            >
              <Trash2 class="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
        <p class="text-3xl font-semibold tracking-tight text-surface-50 mt-5">
          {{ formatValue(panel, currentValues[panel.id]) }}
        </p>
        <p class="text-[0.68rem] text-surface-600 mt-1">
          {{ panelScopeLabel(panel) }}
        </p>
        <MetricChart
          :id="panel.id"
          :points="history[panel.id] ?? []"
          :color="panel.color"
          class="mt-3"
        />
      </article>
    </div>

    <div v-if="showPicker" class="modal-backdrop" @click.self="showPicker = false">
      <section class="metric-picker panel">
        <div class="flex items-start justify-between gap-4 p-5 border-b border-surface-800">
          <div>
            <p class="eyebrow">Panel library</p>
            <h2 class="text-lg font-semibold">Add a Prometheus metric</h2>
            <p class="text-xs text-surface-500 mt-1">
              Includes core, runtime, history, queue, AI transport, and push families.
            </p>
          </div>
          <button
            class="icon-button"
            aria-label="Close metric library"
            @click="showPicker = false"
          >
            <X class="w-4 h-4" />
          </button>
        </div>
        <div class="p-4 border-b border-surface-800">
          <div class="relative">
            <Search class="absolute left-3 top-2.5 w-4 h-4 text-surface-500" />
            <input
              v-model="pickerSearch"
              class="input-field !pl-9"
              autofocus
              placeholder="Search metric name, description, or category…"
            />
          </div>
        </div>
        <div class="metric-family-list">
          <button
            v-for="family in filteredFamilies"
            :key="family.name"
            class="metric-family-row"
            @click="addPanel(family)"
          >
            <span class="min-w-0">
              <span class="block font-mono text-xs text-brand-200 truncate">
                {{ family.name }}
              </span>
              <span class="block text-xs text-surface-500 truncate mt-1">
                {{ family.help || "No exporter description" }}
              </span>
            </span>
            <span class="flex items-center gap-2 shrink-0">
              <span class="metric-tag">{{ categoryFor(family.name) }}</span>
              <span class="metric-tag">{{ family.type ?? "untyped" }}</span>
              <Plus class="w-4 h-4 text-surface-500" />
            </span>
          </button>
          <div v-if="filteredFamilies.length === 0" class="empty-state !border-0 !rounded-none">
            No matching metric families.
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.metric-panel {
  border: 1px solid rgb(51 65 85 / 0.45);
  border-radius: 1rem;
  background: rgb(15 23 42 / 0.72);
  padding: 1rem;
  box-shadow: 0 16px 50px rgb(2 6 23 / 0.15);
  overflow: hidden;
}

.panel-select {
  border: 1px solid rgb(51 65 85 / 0.7);
  border-radius: 0.45rem;
  background: rgb(30 41 59 / 0.75);
  color: #94a3b8;
  padding: 0.28rem 0.4rem;
  font-size: 0.65rem;
  outline: none;
}

.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  padding: 1rem;
  background: rgb(2 6 23 / 0.78);
  backdrop-filter: blur(10px);
}

.metric-picker {
  width: min(48rem, 100%);
  max-height: min(44rem, calc(100vh - 2rem));
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.metric-family-list {
  overflow-y: auto;
  padding: 0.5rem;
}

.metric-family-row {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  border-radius: 0.7rem;
  padding: 0.75rem;
  text-align: left;
  cursor: pointer;
}

.metric-family-row:hover {
  background: rgb(30 41 59 / 0.65);
}

.metric-tag {
  border: 1px solid rgb(51 65 85 / 0.6);
  border-radius: 999px;
  padding: 0.2rem 0.45rem;
  color: #64748b;
  font-size: 0.6rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
</style>
