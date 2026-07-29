<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps<{
  lines: string[];
  lineClass: (line: string) => string;
  resetKey: string;
  label: string;
}>();

const diffLineHeight = 19.44;
const diffLineOverscan = 24;
const maximumMeasuredColumns = 10_000;
const scroller = ref<HTMLElement | null>(null);
const visibleStart = ref(0);
const visibleEnd = ref(0);
let viewportAnimationFrame: number | null = null;
let viewportResizeObserver: ResizeObserver | null = null;
let viewerMounted = false;

const visibleLines = computed(() => props.lines
  .slice(visibleStart.value, visibleEnd.value)
  .map((line, offset) => ({ index: visibleStart.value + offset, line })));

const maximumLineColumns = computed(() => {
  let maximum = 0;
  for (const line of props.lines) {
    maximum = Math.max(maximum, Math.min(line.length, maximumMeasuredColumns));
    if (maximum >= maximumMeasuredColumns) break;
  }
  return maximum;
});

const spacerStyle = computed(() => ({
  height: `${props.lines.length * diffLineHeight}px`,
  minWidth: `max(100%, calc(${maximumLineColumns.value}ch + 28px))`,
}));

function updateVisibleLines(): void {
  viewportAnimationFrame = null;
  const element = scroller.value;
  if (!element) return;
  const firstVisible = Math.floor(element.scrollTop / diffLineHeight);
  const visibleCount = Math.ceil(element.clientHeight / diffLineHeight);
  visibleStart.value = Math.max(0, firstVisible - diffLineOverscan);
  visibleEnd.value = Math.min(props.lines.length, firstVisible + visibleCount + diffLineOverscan);
}

function scheduleVisibleLineUpdate(): void {
  if (viewerMounted && viewportAnimationFrame === null) {
    viewportAnimationFrame = window.requestAnimationFrame(updateVisibleLines);
  }
}

watch(() => props.lines.length, () => {
  void nextTick(scheduleVisibleLineUpdate);
});

watch(() => props.resetKey, () => {
  const element = scroller.value;
  if (element) element.scrollTop = 0;
  void nextTick(scheduleVisibleLineUpdate);
});

onMounted(() => {
  viewerMounted = true;
  viewportResizeObserver = new ResizeObserver(scheduleVisibleLineUpdate);
  if (scroller.value) viewportResizeObserver.observe(scroller.value);
  scheduleVisibleLineUpdate();
});

onBeforeUnmount(() => {
  viewerMounted = false;
  viewportResizeObserver?.disconnect();
  if (viewportAnimationFrame !== null) window.cancelAnimationFrame(viewportAnimationFrame);
});
</script>

<template>
  <div
    ref="scroller"
    class="diff-output virtual-diff-output"
    role="region"
    tabindex="0"
    :aria-label="label"
    @scroll="scheduleVisibleLineUpdate"
  >
    <div class="virtual-diff-spacer" :style="spacerStyle">
      <span
        v-for="entry in visibleLines"
        :key="entry.index"
        class="diff-line virtual-diff-line"
        :class="lineClass(entry.line)"
        :style="{ top: `${entry.index * diffLineHeight}px` }"
      >{{ entry.line }}</span>
    </div>
  </div>
</template>
