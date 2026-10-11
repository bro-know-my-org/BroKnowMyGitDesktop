<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { NScrollbar } from "naive-ui";
import { calculateVirtualWindow, preserveLogicalScrollTop } from "./window.mjs";

const props = withDefaults(defineProps<{
  itemCount: number;
  itemHeight: number;
  itemKeys: readonly (string | number)[];
  overscanViewports?: number;
  resetKey?: string | number | null;
}>(), {
  overscanViewports: 3,
  resetKey: null,
});

defineSlots<{
  default(props: { index: number; tabIndex: number }): unknown;
}>();

const scrollbar = ref<InstanceType<typeof NScrollbar> | null>(null);
// Virtualization must measure and scroll the inner viewport, not the scrollbar wrapper.
const scroller = computed(() => scrollbar.value?.scrollbarInstRef?.containerRef ?? null);
const scrollTop = ref(0);
const viewportHeight = ref(0);
const activeIndex = ref(0);
const focusedIndex = ref<number | null>(null);
const activeItemIdentity = ref<string | null>(null);
const focusedItemIdentity = ref<string | null>(null);
let resizeObserver: ResizeObserver | null = null;

const virtualWindow = computed(() => calculateVirtualWindow({
  itemCount: props.itemCount,
  itemHeight: props.itemHeight,
  scrollTop: scrollTop.value,
  viewportHeight: viewportHeight.value,
  overscanViewports: props.overscanViewports,
}));

const visibleIndexes = computed(() => {
  const indexes = Array.from(
    { length: virtualWindow.value.end - virtualWindow.value.start },
    (_, offset) => virtualWindow.value.start + offset,
  );
  if (
    focusedIndex.value !== null
    && (focusedIndex.value < virtualWindow.value.start || focusedIndex.value >= virtualWindow.value.end)
  ) {
    indexes.push(focusedIndex.value);
    indexes.sort((left, right) => left - right);
  }
  return indexes;
});

const spacerStyle = computed(() => ({ height: `${props.itemCount * props.itemHeight}px` }));
const resetNamespace = computed(() => (
  props.resetKey === null ? "null" : `${typeof props.resetKey}:${String(props.resetKey)}`
));

function updateViewport(): void {
  const element = scroller.value;
  if (!element) return;
  scrollTop.value = element.scrollTop;
  viewportHeight.value = element.clientHeight;
  if (focusedIndex.value === null && props.itemCount > 0) {
    const firstVisible = Math.min(props.itemCount - 1, Math.floor(element.scrollTop / props.itemHeight));
    const visibleCount = Math.max(1, Math.ceil(element.clientHeight / props.itemHeight));
    if (activeIndex.value < firstVisible || activeIndex.value >= firstVisible + visibleCount) {
      setActiveIndex(firstVisible);
    }
  }
}

function itemStyle(index: number): Record<string, string> {
  return {
    height: `${props.itemHeight}px`,
    transform: `translateY(${index * props.itemHeight}px)`,
  };
}

function itemIdentity(index: number): string {
  const key = props.itemKeys[index];
  return key === undefined
    ? `fallback-index:${index}`
    : `item-key:${typeof key}:${String(key)}`;
}

function virtualItemKey(index: number): string {
  return `${resetNamespace.value}\u0000${itemIdentity(index)}`;
}

function setActiveIndex(index: number): void {
  activeIndex.value = index;
  activeItemIdentity.value = props.itemCount > 0 ? itemIdentity(index) : null;
}

function scrollToIndex(index: number, align: "nearest" | "start" | "end" = "nearest"): void {
  const element = scroller.value;
  if (!element || props.itemCount === 0) return;
  const targetIndex = Math.min(props.itemCount - 1, Math.max(0, index));
  const itemTop = targetIndex * props.itemHeight;
  const itemBottom = itemTop + props.itemHeight;
  const viewportBottom = element.scrollTop + element.clientHeight;
  if (align === "start" || itemTop < element.scrollTop) element.scrollTop = itemTop;
  else if (align === "end" || itemBottom > viewportBottom) element.scrollTop = itemBottom - element.clientHeight;
  updateViewport();
}

function focusIndex(index: number, align: "nearest" | "start" | "end" = "nearest"): void {
  const targetIndex = Math.min(props.itemCount - 1, Math.max(0, index));
  setActiveIndex(targetIndex);
  scrollToIndex(targetIndex, align);
  void nextTick(() => {
    scroller.value
      ?.querySelector<HTMLElement>(`[data-virtual-index="${targetIndex}"] [data-virtual-list-focus]`)
      ?.focus();
  });
}

function handleKeydown(event: KeyboardEvent): void {
  if (event.altKey || event.ctrlKey || event.metaKey || props.itemCount === 0) return;
  const target = event.target;
  if (!(target instanceof HTMLElement)) return;
  const item = target.closest<HTMLElement>(".fixed-virtual-list-item");
  if (!item || target !== item.querySelector<HTMLElement>("[data-virtual-list-focus]")) return;
  const pageSize = Math.max(1, Math.floor(viewportHeight.value / props.itemHeight));
  let nextIndex: number | null = null;
  let align: "nearest" | "start" | "end" = "nearest";
  if (event.key === "ArrowDown") nextIndex = activeIndex.value + 1;
  else if (event.key === "ArrowUp") nextIndex = activeIndex.value - 1;
  else if (event.key === "PageDown") nextIndex = activeIndex.value + pageSize;
  else if (event.key === "PageUp") nextIndex = activeIndex.value - pageSize;
  else if (event.key === "Home") {
    nextIndex = 0;
    align = "start";
  } else if (event.key === "End") {
    nextIndex = props.itemCount - 1;
    align = "end";
  }
  if (nextIndex === null) return;
  event.preventDefault();
  focusIndex(nextIndex, align);
}

function handleFocusIn(event: FocusEvent): void {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return;
  const item = target.closest<HTMLElement>(".fixed-virtual-list-item");
  const index = Number(item?.dataset.virtualIndex);
  if (!Number.isInteger(index)) return;
  setActiveIndex(index);
  focusedIndex.value = index;
  focusedItemIdentity.value = itemIdentity(index);
}

function handleFocusOut(): void {
  void nextTick(() => {
    if (!scroller.value?.contains(document.activeElement)) {
      focusedIndex.value = null;
      focusedItemIdentity.value = null;
      updateViewport();
    }
  });
}

watch(() => props.resetKey, () => {
  if (scroller.value) scroller.value.scrollTop = 0;
  scrollTop.value = 0;
  setActiveIndex(0);
  focusedIndex.value = null;
  focusedItemIdentity.value = null;
});

watch([() => [...props.itemKeys], () => props.itemCount], () => {
  const count = Math.max(0, props.itemCount);
  const identities = Array.from({ length: count }, (_, index) => itemIdentity(index));
  const activeMatch = activeItemIdentity.value === null ? -1 : identities.indexOf(activeItemIdentity.value);
  setActiveIndex(activeMatch >= 0 ? activeMatch : Math.min(Math.max(0, count - 1), activeIndex.value));
  const focusedMatch = focusedItemIdentity.value === null ? -1 : identities.indexOf(focusedItemIdentity.value);
  if (focusedMatch >= 0) {
    focusedIndex.value = focusedMatch;
  } else {
    focusedIndex.value = null;
    focusedItemIdentity.value = null;
  }
  void nextTick(updateViewport);
}, { immediate: true });

watch(() => props.itemHeight, (itemHeight, previousItemHeight) => {
  const element = scroller.value;
  if (!element || previousItemHeight <= 0) {
    void nextTick(updateViewport);
    return;
  }
  const adjustedScrollTop = preserveLogicalScrollTop(element.scrollTop, previousItemHeight, itemHeight);
  void nextTick(() => {
    element.scrollTop = adjustedScrollTop;
    updateViewport();
  });
});

onMounted(() => {
  resizeObserver = new ResizeObserver(updateViewport);
  if (scroller.value) resizeObserver.observe(scroller.value);
  updateViewport();
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
});

defineExpose({ scrollToIndex });
</script>

<template>
  <NScrollbar
    ref="scrollbar"
    class="fixed-virtual-list"
    role="list"
    @focusin="handleFocusIn"
    @focusout="handleFocusOut"
    @keydown="handleKeydown"
    @scroll="updateViewport"
  >
    <div class="fixed-virtual-list-spacer" :style="spacerStyle">
      <div
        v-for="index in visibleIndexes"
        :key="virtualItemKey(index)"
        class="fixed-virtual-list-item"
        role="listitem"
        :aria-posinset="index + 1"
        :aria-setsize="itemCount"
        :data-virtual-index="index"
        :style="itemStyle(index)"
      >
        <slot :index="index" :tab-index="activeIndex === index ? 0 : -1" />
      </div>
    </div>
  </NScrollbar>
</template>

<style scoped>
/* NScrollbar's rendered root does not carry this component's scope attribute. */
:global(.fixed-virtual-list) {
  box-sizing: border-box;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

:global(.fixed-virtual-list > .n-scrollbar-container) {
  overscroll-behavior: contain;
}

.fixed-virtual-list-spacer {
  position: relative;
  min-width: 0;
}

.fixed-virtual-list-item {
  position: absolute;
  inset: 0 0 auto;
  min-width: 0;
}
</style>
