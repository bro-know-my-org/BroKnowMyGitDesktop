<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { NButton, NSpin } from "naive-ui";
import { t } from "../../../shared/i18n";
import type { PDFDocumentLoadingTask, PDFDocumentProxy, PDFPageProxy, RenderTask } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
const props = defineProps<{ url: string }>();
const document = shallowRef<PDFDocumentProxy | null>(null);
const canvas = ref<HTMLCanvasElement>();
const page = ref(1), busy = ref(false), failed = ref(false);
let loadingTask: PDFDocumentLoadingTask | null = null, rendering: RenderTask | null = null, generation = 0;
async function render() {
  const current = ++generation;
  const previousRender = rendering;
  previousRender?.cancel();
  if (previousRender) await previousRender.promise.catch(() => {});
  if (current !== generation || !document.value) return;
  busy.value = true; failed.value = false;
  let pdfPage: PDFPageProxy | undefined;
  try {
    pdfPage = await document.value.getPage(page.value);
    if (current !== generation) return;
    await nextTick();
    const target = canvas.value;
    if (!target) return;
    const base = pdfPage.getViewport({ scale: 1 });
    const scale = Math.min(1.5, 1400 / Math.max(base.width, base.height));
    const viewport = pdfPage.getViewport({ scale });
    target.width = Math.ceil(viewport.width); target.height = Math.ceil(viewport.height);
    rendering = pdfPage.render({ canvas: target, viewport });
    await rendering.promise;
  } catch { if (current === generation) failed.value = true; }
  finally { pdfPage?.cleanup(); if (current === generation) busy.value = false; }
}
watch(() => props.url, async (url, _, cleanup) => {
  let active = true;
  cleanup(() => { active = false; generation++; rendering?.cancel(); void loadingTask?.destroy(); });
  document.value = null; page.value = 1; busy.value = true; failed.value = false;
  try {
    const pdf = await import("pdfjs-dist");
    if (!active) return;
    pdf.GlobalWorkerOptions.workerSrc = workerUrl;
    const assets = new URL(import.meta.env.DEV ? "/node_modules/pdfjs-dist/" : `${import.meta.env.BASE_URL}pdf-assets/`, window.location.href).href;
    loadingTask = pdf.getDocument({ url, isEvalSupported: false,
      cMapUrl: `${assets}cmaps/`, cMapPacked: true, standardFontDataUrl: `${assets}standard_fonts/`, wasmUrl: `${assets}wasm/` });
    const loaded = await loadingTask.promise;
    if (!active) { await loaded.destroy(); return; }
    document.value = loaded;
    await render();
  } catch { if (active) { failed.value = true; busy.value = false; } }
}, { immediate: true });
watch(page, render);
onBeforeUnmount(() => { generation++; rendering?.cancel(); });
</script>

<template>
  <div class="resource-pdf">
    <div v-if="document" class="resource-toolbar">
      <n-button size="tiny" :disabled="page <= 1" :aria-label="t('repository.resource.previousPage')" @click="page--">‹</n-button>
      <span>{{ page }} / {{ document.numPages }}</span>
      <n-button size="tiny" :disabled="page >= document.numPages" :aria-label="t('repository.resource.nextPage')" @click="page++">›</n-button>
    </div>
    <n-spin v-if="busy" size="small" />
    <p v-if="failed" class="resource-notice">{{ t('repository.resource.pdfFailed') }}</p>
    <canvas v-show="!failed" ref="canvas" :aria-label="t('repository.resource.pdfPage', { page })" />
  </div>
</template>
