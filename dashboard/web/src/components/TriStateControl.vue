<script setup lang="ts">
defineProps<{
  modelValue?: boolean;
  label: string;
  hint?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: boolean | undefined];
}>();
</script>

<template>
  <div class="setting-row">
    <div class="min-w-0">
      <p class="text-sm font-medium text-surface-200">{{ label }}</p>
      <p v-if="hint" class="text-xs leading-5 text-surface-500 mt-0.5">{{ hint }}</p>
    </div>
    <div class="segmented shrink-0" role="group" :aria-label="label">
      <button
        type="button"
        :class="{ active: modelValue === undefined }"
        @click="emit('update:modelValue', undefined)"
      >
        Inherit
      </button>
      <button
        type="button"
        :class="{ active: modelValue === true }"
        @click="emit('update:modelValue', true)"
      >
        On
      </button>
      <button
        type="button"
        :class="{ active: modelValue === false }"
        @click="emit('update:modelValue', false)"
      >
        Off
      </button>
    </div>
  </div>
</template>
