<script setup lang="ts">
import { ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  Activity,
  AppWindow,
  Database,
  LoaderCircle,
  LockKeyhole,
  Radio,
  ShieldCheck,
} from "lucide-vue-next";
import { useAuthStore } from "@/stores/auth";

const auth = useAuthStore();
const router = useRouter();
const route = useRoute();

const email = ref("admin@sockudo.local");
const password = ref("");

async function submit() {
  try {
    await auth.login(email.value, password.value);
    const redirect = (route.query.redirect as string) || "/";
    router.push(redirect);
  } catch {
    // error shown via store
  }
}
</script>

<template>
  <div class="relative min-h-screen overflow-hidden">
    <div
      class="pointer-events-none absolute -left-48 -top-48 h-[34rem] w-[34rem] rounded-full bg-brand-600/15 blur-3xl"
    />
    <div
      class="pointer-events-none absolute -bottom-56 right-0 h-[36rem] w-[36rem] rounded-full bg-sky-500/8 blur-3xl"
    />

    <div class="relative mx-auto grid min-h-screen max-w-7xl lg:grid-cols-[1.05fr_0.95fr]">
      <section class="hidden flex-col justify-between px-10 py-10 lg:flex xl:px-16 xl:py-14">
        <div class="flex items-center gap-3">
          <div
            class="flex h-11 w-11 items-center justify-center rounded-2xl border border-brand-400/25 bg-brand-500/15 text-brand-300 shadow-xl shadow-brand-950/30"
          >
            <Radio class="h-5 w-5" />
          </div>
          <div>
            <p class="font-semibold tracking-tight text-surface-50">Sockudo</p>
            <p class="text-xs text-surface-500">Realtime infrastructure</p>
          </div>
        </div>

        <div class="max-w-xl py-14">
          <p class="eyebrow">Operator control plane</p>
          <h1 class="text-4xl font-semibold leading-tight tracking-tight text-surface-50 xl:text-5xl">
            Run every realtime app from one focused workspace.
          </h1>
          <p class="mt-5 max-w-lg text-base leading-7 text-surface-400">
            Configure application policy, inspect live traffic, and manage delivery infrastructure
            without leaving the dashboard.
          </p>

          <div class="mt-10 grid max-w-lg gap-3 sm:grid-cols-3">
            <div class="rounded-2xl border border-surface-800 bg-surface-900/45 p-4">
              <AppWindow class="h-5 w-5 text-brand-300" />
              <p class="mt-4 text-sm font-medium text-surface-200">App policy</p>
              <p class="mt-1 text-xs leading-5 text-surface-600">Credentials, limits, and channels</p>
            </div>
            <div class="rounded-2xl border border-surface-800 bg-surface-900/45 p-4">
              <Activity class="h-5 w-5 text-emerald-300" />
              <p class="mt-4 text-sm font-medium text-surface-200">Live metrics</p>
              <p class="mt-1 text-xs leading-5 text-surface-600">Operational signals at a glance</p>
            </div>
            <div class="rounded-2xl border border-surface-800 bg-surface-900/45 p-4">
              <Database class="h-5 w-5 text-sky-300" />
              <p class="mt-4 text-sm font-medium text-surface-200">Durable state</p>
              <p class="mt-1 text-xs leading-5 text-surface-600">Database-backed configuration</p>
            </div>
          </div>
        </div>

        <p class="flex items-center gap-2 text-xs text-surface-600">
          <ShieldCheck class="h-4 w-4 text-emerald-400/70" />
          Authenticated operator access
        </p>
      </section>

      <section class="flex min-h-screen items-center justify-center px-4 py-10 sm:px-8 lg:border-l lg:border-surface-800/70">
        <div class="w-full max-w-md">
          <div class="mb-8 flex items-center gap-3 lg:hidden">
            <div
              class="flex h-10 w-10 items-center justify-center rounded-xl bg-brand-500/15 text-brand-300"
            >
              <Radio class="h-5 w-5" />
            </div>
            <div>
              <p class="font-semibold text-surface-100">Sockudo</p>
              <p class="text-xs text-surface-600">Operator dashboard</p>
            </div>
          </div>

          <div class="panel overflow-hidden">
            <div class="border-b border-surface-800/80 px-6 py-6 sm:px-8 sm:py-7">
              <div
                class="mb-5 flex h-11 w-11 items-center justify-center rounded-xl bg-surface-800/80 text-brand-300 ring-1 ring-inset ring-surface-700/60"
              >
                <LockKeyhole class="h-5 w-5" />
              </div>
              <h2 class="text-xl font-semibold tracking-tight text-surface-50">Welcome back</h2>
              <p class="mt-2 text-sm leading-6 text-surface-500">
                Sign in with your operator account to continue.
              </p>
            </div>

            <form class="space-y-5 px-6 py-6 sm:px-8 sm:py-7" @submit.prevent="submit">
              <div>
                <label for="login-email" class="field-label">Email address</label>
                <input
                  id="login-email"
                  v-model="email"
                  type="email"
                  class="input-field"
                  autocomplete="email"
                  placeholder="you@example.com"
                  required
                />
              </div>
              <div>
                <label for="login-password" class="field-label">Password</label>
                <input
                  id="login-password"
                  v-model="password"
                  type="password"
                  class="input-field"
                  autocomplete="current-password"
                  placeholder="Enter your password"
                  required
                />
              </div>

              <p v-if="auth.error" class="alert alert-error" role="alert">{{ auth.error }}</p>

              <button
                type="submit"
                class="btn-primary flex w-full items-center justify-center gap-2 py-2.5"
                :disabled="auth.loading"
              >
                <LoaderCircle v-if="auth.loading" class="h-4 w-4 animate-spin" />
                {{ auth.loading ? "Signing in..." : "Sign in to dashboard" }}
              </button>
            </form>

            <div class="border-t border-surface-800/80 bg-surface-950/25 px-6 py-4 sm:px-8">
              <p class="flex items-center justify-center gap-2 text-[0.68rem] text-surface-600">
                <ShieldCheck class="h-3.5 w-3.5 text-emerald-400/70" />
                Session protected with secure, HTTP-only authentication
              </p>
            </div>
          </div>

          <p class="mt-6 text-center text-xs text-surface-700">
            Sockudo operator workspace
          </p>
        </div>
      </section>
    </div>
  </div>
</template>
