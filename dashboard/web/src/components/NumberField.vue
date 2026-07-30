<script setup lang="ts">
defineProps<{
  modelValue?: number;
  label: string;
  hint?: string;
  min?: number;
  step?: number;
  placeholder?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: number | undefined];
}>();

function update(event: Event) {
  const raw = (event.target as HTMLInputElement).value.trim();
  emit("update:modelValue", raw === "" ? undefined : Number(raw));
}
</script>

<template>
  <label class="block">
    <span class="field-label">{{ label }}</span>
    <input
      :value="modelValue ?? ''"
      type="number"
      class="input-field"
      :min="min ?? 0"
      :step="step ?? 1"
      :placeholder="placeholder"
      @input="update"
    />
    <span v-if="hint" class="field-hint">{{ hint }}</span>
  </label>
</template>
