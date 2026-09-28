import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, ApiError } from "@/api/client";
import type { DashboardUser } from "@/types/user";

export const useAuthStore = defineStore("auth", () => {
  const user = ref<DashboardUser | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const challenge = ref<string | null>(null);

  const email = computed(() => user.value?.email ?? null);
  const isAdmin = computed(() => user.value?.role === "admin");

  async function bootstrap() {
    try {
      user.value = await api.me();
    } catch {
      user.value = null;
    }
  }

  async function login(loginEmail: string, password: string) {
    loading.value = true;
    error.value = null;
    try {
      challenge.value = null;
      const result = await api.login(loginEmail, password);
      if ("mfa_required" in result) {
        user.value = null;
        challenge.value = result.challenge;
      } else {
        user.value = result;
      }
    } catch (err) {
      error.value = err instanceof ApiError ? err.message : "Login failed";
      throw err;
    } finally {
      loading.value = false;
    }
  }

  async function verifyTotp(code: string) {
    if (!challenge.value) return;
    loading.value = true;
    error.value = null;
    try {
      user.value = await api.verifyTotp(challenge.value, code.trim());
      challenge.value = null;
    } catch (err) {
      error.value = err instanceof ApiError ? err.message : "Verification failed";
      throw err;
    } finally {
      loading.value = false;
    }
  }

  function clearSession() {
    user.value = null;
    challenge.value = null;
    error.value = null;
  }

  async function logout() {
    await api.logout();
    clearSession();
  }

  return { user, email, isAdmin, loading, error, challenge, bootstrap, login, verifyTotp, clearSession, logout };
});
