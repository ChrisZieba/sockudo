<script setup lang="ts">
import { ref } from "vue";
import { useRouter } from "vue-router";
import { Check, Download, KeyRound, LoaderCircle, Save, ShieldCheck, ShieldOff } from "lucide-vue-next";
import QRCode from "qrcode";
import { api, type TotpSetup } from "@/api/client";
import { useAuthStore } from "@/stores/auth";

const auth = useAuthStore();
const router = useRouter();
const name = ref(auth.user?.name ?? "");
const currentPassword = ref("");
const newPassword = ref("");
const confirmPassword = ref("");
const securityPassword = ref("");
const securityCode = ref("");
const setup = ref<TotpSetup | null>(null);
const qrCode = ref("");
const recoveryCodes = ref<string[]>([]);
const busy = ref(false);
const error = ref("");
const notice = ref("");

async function perform(action: () => Promise<void>) {
  busy.value = true;
  error.value = "";
  notice.value = "";
  try {
    await action();
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Unable to save changes";
  } finally {
    busy.value = false;
  }
}

async function saveProfile() {
  if (!auth.user) return;
  const id = auth.user.id;
  await perform(async () => {
    auth.user = await api.updateUser(id, { name: name.value.trim() });
    notice.value = "Profile saved.";
  });
}

async function changePassword() {
  if (!auth.user) return;
  if (newPassword.value !== confirmPassword.value) {
    error.value = "New passwords do not match.";
    return;
  }
  const id = auth.user.id;
  await perform(async () => {
    await api.changePassword(id, { current_password: currentPassword.value, new_password: newPassword.value });
    currentPassword.value = newPassword.value = confirmPassword.value = "";
    auth.clearSession();
    await router.replace({ name: "login", query: { password_changed: "1" } });
  });
}

function clearSecurityFields() {
  setup.value = null;
  qrCode.value = "";
  securityPassword.value = "";
  securityCode.value = "";
}

async function beginSetup() {
  await perform(async () => {
    const result = await api.setupTotp(securityPassword.value);
    // Generate locally so the enrollment secret never reaches a third-party service.
    qrCode.value = await QRCode.toDataURL(result.otpauth_uri, { width: 224, margin: 2 });
    setup.value = result;
    securityCode.value = "";
  });
}

async function enableTotp() {
  await perform(async () => {
    const result = await api.enableTotp(securityPassword.value, securityCode.value.trim());
    auth.user = result.user;
    recoveryCodes.value = result.recovery_codes;
    clearSecurityFields();
    notice.value = "Two-factor authentication enabled. Other sessions have been signed out.";
  });
}

async function disableTotp() {
  if (!confirm("Disable two-factor authentication for your account?")) return;
  await perform(async () => {
    auth.user = await api.disableTotp(securityPassword.value, securityCode.value.trim());
    clearSecurityFields();
    recoveryCodes.value = [];
    notice.value = "Two-factor authentication disabled. Other sessions have been signed out.";
  });
}

async function regenerateCodes() {
  if (!confirm("Replace all recovery codes? Existing recovery codes will stop working.")) return;
  await perform(async () => {
    const result = await api.regenerateRecoveryCodes(securityPassword.value, securityCode.value.trim());
    recoveryCodes.value = result.recovery_codes;
    clearSecurityFields();
    notice.value = "Recovery codes replaced. Other sessions have been signed out.";
  });
}

function downloadCodes() {
  const blob = new Blob([`Sockudo recovery codes\nAccount: ${auth.email}\n\n${recoveryCodes.value.join("\n")}\n`], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "sockudo-recovery-codes.txt";
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
</script>

<template>
  <div class="max-w-3xl">
    <div class="page-header"><h1 class="page-title">Profile and security</h1></div>
    <p v-if="error" role="alert" class="alert alert-error mb-5">{{ error }}</p>
    <p v-if="notice" role="status" class="mb-5 text-sm text-emerald-300">{{ notice }}</p>

    <section class="border-b border-surface-800 pb-8">
      <h2 class="mb-5 text-base font-semibold">Account</h2>
      <form class="space-y-4 max-w-lg" @submit.prevent="saveProfile">
        <div><label for="profile-email" class="field-label">Email address</label><input id="profile-email" :value="auth.email" class="input-field" type="email" readonly /></div>
        <div><label for="profile-name" class="field-label">Display name</label><input id="profile-name" v-model="name" class="input-field" autocomplete="name" maxlength="200" /></div>
        <p class="text-xs capitalize text-surface-400">{{ auth.user?.role }}</p>
        <button class="btn-primary inline-flex items-center gap-2" :disabled="busy"><Save class="h-4 w-4" />Save profile</button>
      </form>
    </section>

    <section class="border-b border-surface-800 py-8">
      <h2 class="mb-5 text-base font-semibold">Change password</h2>
      <form class="space-y-4 max-w-lg" @submit.prevent="changePassword">
        <div><label for="current-password" class="field-label">Current password</label><input id="current-password" v-model="currentPassword" class="input-field" type="password" autocomplete="current-password" required /></div>
        <div><label for="new-password" class="field-label">New password</label><input id="new-password" v-model="newPassword" class="input-field" type="password" autocomplete="new-password" minlength="8" required /></div>
        <div><label for="confirm-password" class="field-label">Confirm new password</label><input id="confirm-password" v-model="confirmPassword" class="input-field" type="password" autocomplete="new-password" minlength="8" required /></div>
        <p class="text-xs text-surface-400">At least 8 characters. All sessions will be signed out.</p>
        <button class="btn-primary inline-flex items-center gap-2" :disabled="busy"><KeyRound class="h-4 w-4" />Change password</button>
      </form>
    </section>

    <section class="py-8">
      <div class="mb-5 flex flex-wrap items-center gap-3"><h2 class="text-base font-semibold">Two-factor authentication</h2><span class="status-pill" :class="auth.user?.totp_enabled ? 'status-positive' : 'status-negative'">{{ auth.user?.totp_enabled ? "Enabled" : "Not enabled" }}</span></div>

      <div v-if="recoveryCodes.length" class="mb-6 space-y-4 border-l-2 border-emerald-500 pl-4">
        <h3 class="font-medium">Recovery codes</h3>
        <p class="text-sm text-surface-400">Keep these codes somewhere safe. Each code works once. They will not be shown again.</p>
        <ul class="grid gap-2 font-mono text-sm sm:grid-cols-2"><li v-for="code in recoveryCodes" :key="code" class="break-all">{{ code }}</li></ul>
        <div class="flex flex-wrap gap-2">
          <button class="btn-secondary inline-flex items-center gap-2" @click="downloadCodes"><Download class="h-4 w-4" />Download codes</button>
          <button class="btn-secondary inline-flex items-center gap-2" @click="recoveryCodes = []"><Check class="h-4 w-4" />I saved my codes</button>
        </div>
      </div>

      <form class="max-w-lg space-y-4" @submit.prevent="auth.user?.totp_enabled ? regenerateCodes() : setup ? enableTotp() : beginSetup()">
        <div><label for="security-password" class="field-label">Current password</label><input id="security-password" v-model="securityPassword" class="input-field" type="password" autocomplete="current-password" required /></div>

        <div v-if="setup" class="space-y-4">
          <img :src="qrCode" alt="Authenticator enrollment QR code" width="224" height="224" class="max-w-full rounded-lg" />
          <div><label for="setup-key" class="field-label">Authenticator setup key</label><input id="setup-key" :value="setup.secret" class="input-field font-mono" readonly /></div>
        </div>

        <div v-if="setup || auth.user?.totp_enabled"><label for="security-code" class="field-label">{{ setup ? "Authenticator code" : "Authenticator or recovery code" }}</label><input id="security-code" v-model="securityCode" class="input-field font-mono" type="text" :inputmode="setup ? 'numeric' : 'text'" autocomplete="one-time-code" :pattern="setup ? '[0-9]{6}' : undefined" :maxlength="setup ? 6 : 64" required /></div>

        <div class="flex flex-wrap gap-2">
          <button class="btn-primary inline-flex items-center gap-2" :disabled="busy"><LoaderCircle v-if="busy" class="h-4 w-4 animate-spin" /><ShieldCheck v-else class="h-4 w-4" />{{ auth.user?.totp_enabled ? "Replace recovery codes" : setup ? "Confirm and enable" : "Set up two-factor authentication" }}</button>
          <button v-if="setup" type="button" class="btn-secondary" :disabled="busy" @click="clearSecurityFields">Cancel</button>
          <button v-if="auth.user?.totp_enabled" type="button" class="btn-danger inline-flex items-center gap-2" :disabled="busy || !securityPassword || !securityCode.trim()" @click="disableTotp"><ShieldOff class="h-4 w-4" />Disable two-factor authentication</button>
        </div>
      </form>
    </section>
  </div>
</template>
