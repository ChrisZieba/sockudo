<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter, RouterLink, RouterView } from "vue-router";
import {
  Activity,
  Database,
  LayoutGrid,
  LogOut,
  Radio,
  ShieldCheck,
  Users,
} from "lucide-vue-next";
import { useAuthStore } from "@/stores/auth";
import { api } from "@/api/client";

const auth = useAuthStore();
const router = useRouter();
const driver = ref("");
const initials = computed(() => {
  const identity = auth.user?.name || auth.email || "Operator";
  return identity
    .split(/\s+|@/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");
});

onMounted(async () => {
  try {
    const cfg = await api.opsConfig();
    driver.value = cfg.app_manager_driver;
  } catch {
    driver.value = "unknown";
  }
});

async function logout() {
  await auth.logout();
  router.push({ name: "login" });
}
</script>

<template>
  <div class="flex min-h-screen lg:h-screen lg:overflow-hidden">
    <aside
      class="relative hidden w-72 shrink-0 flex-col overflow-hidden border-r border-surface-800/80 bg-surface-950/75 px-4 py-5 backdrop-blur-2xl lg:flex"
    >
      <div
        class="pointer-events-none absolute -left-24 -top-24 h-56 w-56 rounded-full bg-brand-600/15 blur-3xl"
      />

      <div class="relative mb-8 flex items-center gap-3 px-2">
        <div
          class="flex h-10 w-10 items-center justify-center rounded-xl border border-brand-400/25 bg-brand-500/15 text-brand-300 shadow-lg shadow-brand-950/30"
        >
          <Radio class="h-5 w-5" />
        </div>
        <div class="min-w-0">
          <div class="flex items-center gap-2">
            <span class="font-semibold tracking-tight text-surface-50">Sockudo</span>
            <span
              class="rounded-full border border-surface-700/70 bg-surface-800/60 px-1.5 py-0.5 text-[0.55rem] font-semibold uppercase tracking-wider text-surface-500"
            >
              Ops
            </span>
          </div>
          <p class="mt-0.5 text-xs text-surface-500">Realtime control plane</p>
        </div>
      </div>

      <p class="mb-2 px-3 text-[0.62rem] font-semibold uppercase tracking-[0.18em] text-surface-600">
        Workspace
      </p>
      <nav class="relative flex-1 space-y-1">
        <RouterLink
          to="/"
          class="group flex items-center gap-3 rounded-xl px-3 py-3 text-sm text-surface-400 transition hover:bg-surface-800/60 hover:text-surface-100"
          active-class="!bg-brand-500/12 !text-brand-200 ring-1 ring-inset ring-brand-400/15"
        >
          <LayoutGrid class="h-4 w-4 text-surface-500 group-hover:text-surface-300" />
          <span class="font-medium">Applications</span>
        </RouterLink>
        <RouterLink
          to="/metrics"
          class="group flex items-center gap-3 rounded-xl px-3 py-3 text-sm text-surface-400 transition hover:bg-surface-800/60 hover:text-surface-100"
          active-class="!bg-brand-500/12 !text-brand-200 ring-1 ring-inset ring-brand-400/15"
        >
          <Activity class="h-4 w-4 text-surface-500 group-hover:text-surface-300" />
          <span class="font-medium">Observability</span>
        </RouterLink>
        <RouterLink
          v-if="auth.isAdmin"
          to="/users"
          class="group flex items-center gap-3 rounded-xl px-3 py-3 text-sm text-surface-400 transition hover:bg-surface-800/60 hover:text-surface-100"
          active-class="!bg-brand-500/12 !text-brand-200 ring-1 ring-inset ring-brand-400/15"
        >
          <Users class="h-4 w-4 text-surface-500 group-hover:text-surface-300" />
          <span class="font-medium">Team access</span>
        </RouterLink>
      </nav>

      <div
        v-if="driver"
        class="relative mb-3 flex items-center gap-3 rounded-xl border border-surface-800 bg-surface-900/55 px-3 py-3"
      >
        <div
          class="flex h-8 w-8 items-center justify-center rounded-lg"
          :class="
            driver === 'unknown'
              ? 'bg-amber-500/10 text-amber-300'
              : 'bg-emerald-500/10 text-emerald-300'
          "
        >
          <Database class="h-4 w-4" />
        </div>
        <div class="min-w-0">
          <p class="text-[0.62rem] font-semibold uppercase tracking-wider text-surface-600">
            App manager
          </p>
          <p class="truncate font-mono text-xs text-surface-300">{{ driver }}</p>
        </div>
        <span
          class="ml-auto h-2 w-2 rounded-full"
          :class="
            driver === 'unknown'
              ? 'bg-amber-400'
              : 'bg-emerald-400 shadow-[0_0_10px_rgba(52,211,153,0.6)]'
          "
        />
      </div>

      <div
        class="relative flex items-center gap-3 rounded-xl border border-surface-800/80 bg-surface-900/45 p-2"
      >
        <div
          class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-surface-800 text-xs font-semibold text-brand-200"
        >
          {{ initials }}
        </div>
        <div class="min-w-0 flex-1">
          <p class="truncate text-xs font-medium text-surface-200">
            {{ auth.user?.name || auth.email }}
          </p>
          <p class="mt-0.5 flex items-center gap-1 text-[0.62rem] text-surface-600">
            <ShieldCheck v-if="auth.isAdmin" class="h-3 w-3 text-brand-400" />
            {{ auth.isAdmin ? "Administrator" : "Operator" }}
          </p>
        </div>
        <button
          class="icon-button"
          title="Sign out"
          aria-label="Sign out"
          @click="logout"
        >
          <LogOut class="h-3.5 w-3.5" />
        </button>
      </div>
    </aside>

    <main class="min-w-0 flex-1 overflow-y-auto">
      <header
        class="sticky top-0 z-30 border-b border-surface-800/80 bg-surface-950/85 px-4 py-3 backdrop-blur-xl lg:hidden"
      >
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2.5">
            <div
              class="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15 text-brand-300"
            >
              <Radio class="h-4 w-4" />
            </div>
            <div>
              <p class="text-sm font-semibold text-surface-100">Sockudo</p>
              <p class="text-[0.6rem] uppercase tracking-widest text-surface-600">Control plane</p>
            </div>
          </div>
          <div class="flex items-center gap-2">
            <span
              v-if="driver"
              class="hidden items-center gap-1.5 rounded-full border border-surface-800 bg-surface-900 px-2.5 py-1 font-mono text-[0.62rem] text-surface-500 sm:flex"
            >
              <span
                class="h-1.5 w-1.5 rounded-full"
                :class="driver === 'unknown' ? 'bg-amber-400' : 'bg-emerald-400'"
              />
              {{ driver }}
            </span>
            <button
              class="icon-button"
              title="Sign out"
              aria-label="Sign out"
              @click="logout"
            >
              <LogOut class="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <nav
          class="-mb-3 mt-3 grid gap-1 pb-2"
          :class="auth.isAdmin ? 'grid-cols-3' : 'grid-cols-2'"
        >
          <RouterLink
            to="/"
            class="flex min-w-0 items-center justify-center gap-1.5 rounded-lg px-2 py-2 text-[0.68rem] font-medium text-surface-500 sm:text-xs"
            active-class="!bg-brand-500/12 !text-brand-200"
          >
            <LayoutGrid class="h-3.5 w-3.5" />
            Applications
          </RouterLink>
          <RouterLink
            to="/metrics"
            class="flex min-w-0 items-center justify-center gap-1.5 rounded-lg px-2 py-2 text-[0.68rem] font-medium text-surface-500 sm:text-xs"
            active-class="!bg-brand-500/12 !text-brand-200"
          >
            <Activity class="h-3.5 w-3.5" />
            Observability
          </RouterLink>
          <RouterLink
            v-if="auth.isAdmin"
            to="/users"
            class="flex min-w-0 items-center justify-center gap-1.5 rounded-lg px-2 py-2 text-[0.68rem] font-medium text-surface-500 sm:text-xs"
            active-class="!bg-brand-500/12 !text-brand-200"
          >
            <Users class="h-3.5 w-3.5" />
            Team access
          </RouterLink>
        </nav>
      </header>

      <div class="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8 lg:py-8">
        <RouterView />
      </div>
    </main>
  </div>
</template>
