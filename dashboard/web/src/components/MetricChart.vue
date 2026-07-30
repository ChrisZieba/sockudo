<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  id: string;
  points: Array<{ at: number; value: number }>;
  color?: string;
}>();

const width = 420;
const height = 118;
const padding = 7;

const domain = computed(() => {
  if (!props.points.length) return { min: 0, max: 1 };
  const values = props.points.map((point) => point.value);
  let min = Math.min(...values);
  let max = Math.max(...values);
  if (min === max) {
    const offset = Math.max(Math.abs(min) * 0.1, 1);
    min -= offset;
    max += offset;
  }
  return { min, max };
});

const coordinates = computed(() =>
  props.points.map((point, index) => {
    const x =
      props.points.length <= 1
        ? width / 2
        : padding + (index / (props.points.length - 1)) * (width - padding * 2);
    const y =
      height -
      padding -
      ((point.value - domain.value.min) / (domain.value.max - domain.value.min)) *
        (height - padding * 2);
    return { ...point, x, y };
  }),
);

const linePath = computed(() =>
  coordinates.value
    .map((point, index) => `${index === 0 ? "M" : "L"}${point.x},${point.y}`)
    .join(" "),
);

const areaPath = computed(() => {
  if (!coordinates.value.length) return "";
  const first = coordinates.value[0];
  const last = coordinates.value.at(-1)!;
  return `${linePath.value} L${last.x},${height} L${first.x},${height} Z`;
});
</script>

<template>
  <div class="metric-chart">
    <svg
      v-if="points.length"
      :viewBox="`0 0 ${width} ${height}`"
      preserveAspectRatio="none"
      role="img"
      aria-label="Metric history"
    >
      <defs>
        <linearGradient :id="`fill-${id}`" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" :stop-color="color ?? '#748ffc'" stop-opacity="0.28" />
          <stop offset="100%" :stop-color="color ?? '#748ffc'" stop-opacity="0" />
        </linearGradient>
      </defs>
      <line v-for="line in [0.25, 0.5, 0.75]" :key="line" x1="0" :y1="height * line" :x2="width" :y2="height * line" />
      <path :d="areaPath" :fill="`url(#fill-${id})`" />
      <path :d="linePath" fill="none" :stroke="color ?? '#748ffc'" />
      <circle
        v-if="coordinates.length"
        :cx="coordinates.at(-1)!.x"
        :cy="coordinates.at(-1)!.y"
        r="3"
        :fill="color ?? '#748ffc'"
      />
    </svg>
    <div v-else class="flex h-full items-center justify-center text-xs text-surface-600">
      Waiting for samples
    </div>
  </div>
</template>

<style scoped>
.metric-chart {
  height: 7.4rem;
  width: 100%;
}

svg {
  display: block;
  height: 100%;
  width: 100%;
  overflow: visible;
}

line {
  stroke: rgb(51 65 85 / 0.45);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
}

path {
  stroke-width: 1.8;
  vector-effect: non-scaling-stroke;
}

path[fill] {
  stroke: none;
}

circle {
  vector-effect: non-scaling-stroke;
}
</style>
