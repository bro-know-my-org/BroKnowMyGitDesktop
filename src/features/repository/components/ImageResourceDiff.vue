<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { NButton, NSelect, NSlider } from "naive-ui";
import { t } from "../../../shared/i18n";
import { pixelDifference } from "../utils/resourcePreview";
const props = defineProps<{ urls: (string | null)[]; sideIndices?: number[] }>();
type Mode = "sideBySide" | "wipe" | "overlay" | "difference";
const mode = ref<Mode>("sideBySide"), zoom = ref<number | null>(null), amount = ref(50);
const images = shallowRef<(HTMLImageElement | null)[]>([null, null]);
const failures = ref([false, false]), differenceFailed = ref(false);
const host = ref<HTMLElement>(), canvas = ref<HTMLCanvasElement>();
const availableWidth = ref(600);
let observer: ResizeObserver | undefined, generation = 0, differenceGeneration = 0, syncing = false;
const options = computed(() => (["sideBySide", "wipe", "overlay", "difference"] as Mode[]).map((value) => ({
  value, label: t(`repository.resource.${value}`), disabled: value !== "sideBySide" && !images.value.every(Boolean),
})));
const visibleIndices = computed(() => props.sideIndices ?? props.urls.flatMap((url, index) => url ? [index] : []));
const width = computed(() => Math.max(1, ...images.value.map((image) => image?.naturalWidth ?? 0)));
const height = computed(() => Math.max(1, ...images.value.map((image) => image?.naturalHeight ?? 0)));
const fitScale = computed(() => Math.max(0.01, Math.min(1, (availableWidth.value / (mode.value === "sideBySide" ? Math.max(1, visibleIndices.value.length) : 1) - 24) / width.value, 460 / height.value)));
const scale = computed(() => zoom.value === null ? fitScale.value : zoom.value / 100);
const zoomPercent = computed(() => Math.round(scale.value * 100));
const stageStyle = computed(() => ({ width: `${width.value * scale.value}px`, height: `${height.value * scale.value}px` }));
const imageStyle = (index: number) => ({ width: `${(images.value[index]?.naturalWidth ?? 0) * scale.value}px`, height: `${(images.value[index]?.naturalHeight ?? 0) * scale.value}px` });
async function load() {
  const current = ++generation;
  images.value = [null, null]; failures.value = [false, false]; mode.value = "sideBySide";
  const loaded = await Promise.all(props.urls.map(async (url, index) => {
    if (!url) return null;
    const image = new Image();
    image.crossOrigin = "anonymous";
    image.src = url;
    try { await image.decode(); return image; }
    catch { if (current === generation) failures.value[index] = true; return null; }
  }));
  if (current === generation) images.value = loaded;
}
async function drawDifference() {
  const current = ++differenceGeneration;
  differenceFailed.value = false;
  if (mode.value !== "difference" || !images.value.every(Boolean)) return;
  await nextTick();
  if (current !== differenceGeneration) return;
  const target = canvas.value;
  if (!target) return;
  if (width.value * height.value > 4_000_000) { differenceFailed.value = true; return; }
  const frames: HTMLCanvasElement[] = [];
  try {
    // Freeze both source frames once; animated resources must not advance
    // between the asynchronous tiles below.
    for (const image of images.value) {
      const frame = document.createElement("canvas");
      frame.width = image!.naturalWidth; frame.height = image!.naturalHeight;
      const context = frame.getContext("2d");
      if (!context) throw new Error("Canvas unavailable");
      context.drawImage(image!, 0, 0); frames.push(frame);
    }
    target.width = width.value; target.height = height.value;
    const context = target.getContext("2d");
    const tile = document.createElement("canvas");
    const scratch = tile.getContext("2d", { willReadFrequently: true });
    if (!context || !scratch) throw new Error("Canvas unavailable");
    // Only a 512px tile's buffers are live at once. Yield between tiles so large
    // images never run one full-frame pixel loop on the UI thread.
    for (let y = 0; y < target.height; y += 512) {
      for (let x = 0; x < target.width; x += 512) {
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
        if (current !== differenceGeneration) return;
        tile.width = Math.min(512, target.width - x); tile.height = Math.min(512, target.height - y);
        const read = (image: HTMLCanvasElement) => {
          scratch.clearRect(0, 0, tile.width, tile.height);
          scratch.drawImage(image, -x, -y);
          return scratch.getImageData(0, 0, tile.width, tile.height).data;
        };
        const pixels = pixelDifference(read(frames[0]), read(frames[1]));
        const result = scratch.createImageData(tile.width, tile.height);
        result.data.set(pixels); context.putImageData(result, x, y);
      }
    }
  } catch { if (current === differenceGeneration) differenceFailed.value = true; }
  finally { frames.forEach((frame) => { frame.width = 0; frame.height = 0; }); }
}
function syncScroll(event: Event) {
  if (syncing) return;
  syncing = true;
  const source = event.target as HTMLElement;
  host.value?.querySelectorAll<HTMLElement>(".resource-image-scroll").forEach((pane) => {
    if (pane !== source) { pane.scrollLeft = source.scrollLeft; pane.scrollTop = source.scrollTop; }
  });
  requestAnimationFrame(() => { syncing = false; });
}
async function wheelZoom(event: WheelEvent) {
  if (!event.deltaY || !images.value.some(Boolean)) return;
  event.preventDefault();
  const pane = event.currentTarget as HTMLElement;
  const bounds = pane.getBoundingClientRect();
  const x = event.clientX - bounds.left, y = event.clientY - bounds.top;
  const before = scale.value;
  const anchorX = (pane.scrollLeft + x) / before, anchorY = (pane.scrollTop + y) / before;
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? pane.clientHeight : 1;
  const delta = Math.max(-100, Math.min(100, event.deltaY * unit));
  // Allow returning to a very small fit scale without a backwards jump.
  const after = Math.max(Math.min(0.25, fitScale.value), Math.min(8, before * Math.exp(-delta / 500)));
  zoom.value = after * 100;
  await nextTick();
  if (!pane.isConnected) return;
  host.value?.querySelectorAll<HTMLElement>(".resource-image-scroll").forEach((target) => {
    target.scrollLeft = anchorX * after - x;
    target.scrollTop = anchorY * after - y;
  });
}
watch(() => props.urls, load, { immediate: true });
watch([mode, images], drawDifference);
onMounted(() => {
  observer = new ResizeObserver(([entry]) => { availableWidth.value = Math.max(80, entry.contentRect.width); });
  if (host.value) observer.observe(host.value);
});
onBeforeUnmount(() => { generation++; differenceGeneration++; observer?.disconnect(); });
</script>

<template>
  <div ref="host" class="resource-image-diff">
    <div class="resource-toolbar">
      <n-select v-if="visibleIndices.length === 2" v-model:value="mode" :options="options" :aria-label="t('repository.resource.compareMode')" class="resource-mode" size="small" />
      <n-button size="small" :disabled="zoomPercent <= 25" :aria-label="t('repository.resource.zoomOut')" @click="zoom = Math.max(25, (Math.ceil(zoomPercent / 25) - 1) * 25)">−</n-button>
      <span>{{ zoomPercent }}%</span>
      <n-button size="small" :disabled="zoomPercent >= 800" :aria-label="t('repository.resource.zoomIn')" @click="zoom = Math.min(800, (Math.floor(zoomPercent / 25) + 1) * 25)">+</n-button>
      <n-button size="small" @click="zoom = null">{{ t('repository.resource.fit') }}</n-button>
      <n-button size="small" @click="zoom = 100">1:1</n-button>
      <span class="resource-pixel-label">{{ t('repository.resource.nearest') }}</span>
    </div>
    <div v-if="mode === 'wipe' || mode === 'overlay'" class="resource-toolbar">
      <label :for="`resource-${mode}`">{{ t(mode === 'wipe' ? 'repository.resource.wipePosition' : 'repository.resource.opacity') }}</label>
      <n-slider :id="`resource-${mode}`" v-model:value="amount" :aria-label="t(mode === 'wipe' ? 'repository.resource.wipePosition' : 'repository.resource.opacity')" class="resource-slider" />
    </div>
    <div class="resource-columns resource-dimensions" :class="{ 'resource-columns-single': visibleIndices.length === 1 }">
      <span v-for="index in visibleIndices" :key="index">{{ images[index] ? `${images[index]!.naturalWidth} × ${images[index]!.naturalHeight} px` : failures[index] ? t('repository.resource.decodeFailed') : '—' }}</span>
    </div>
    <div v-if="mode === 'sideBySide'" class="resource-columns" :class="{ 'resource-columns-single': visibleIndices.length === 1 }">
      <div v-for="index in visibleIndices" :key="index" class="resource-image-scroll" tabindex="0" @scroll="syncScroll" @wheel="wheelZoom">
        <div :style="stageStyle">
          <img v-if="urls[index] && images[index]" class="resource-checker" :src="urls[index]!" :style="imageStyle(index)" :alt="t(index === 0 ? 'repository.resource.before' : 'repository.resource.after')" />
        </div>
      </div>
    </div>
    <div v-else class="resource-image-scroll" tabindex="0" @wheel="wheelZoom">
      <div class="resource-image-stage resource-checker" :style="stageStyle">
        <template v-if="mode !== 'difference'">
          <img :src="urls[0]!" :style="imageStyle(0)" :alt="t('repository.resource.before')" />
          <div class="resource-image-overlay" :class="{ 'resource-checker': mode === 'wipe' }" :style="{ ...stageStyle, opacity: mode === 'overlay' ? amount / 100 : 1, clipPath: mode === 'wipe' ? `inset(0 ${100 - amount}% 0 0)` : undefined }">
            <img :src="urls[1]!" :style="imageStyle(1)" :alt="t('repository.resource.after')" />
          </div>
        </template>
        <canvas v-else ref="canvas" :style="stageStyle" :aria-label="t('repository.resource.difference')" />
      </div>
    </div>
    <p v-if="mode === 'difference'" class="resource-notice">{{ t(differenceFailed ? 'repository.resource.differenceFailed' : 'repository.resource.differenceHelp') }}</p>
  </div>
</template>
