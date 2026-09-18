<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from "vue";
import { NSpin } from "naive-ui";
import { t } from "../../../shared/i18n";
const props = defineProps<{ url: string; kind: "audio" | "video" }>();
const source = ref(props.url), failed = ref(false), loading = ref(false), duration = ref(0);
let controller = new AbortController(), generation = 0, blobUrl: string | null = null, triedBlob = false;
function dispose() {
  controller.abort();
  if (blobUrl) URL.revokeObjectURL(blobUrl);
  blobUrl = null;
}
watch(() => props.url, (url) => {
  generation++; dispose(); controller = new AbortController(); triedBlob = false;
  source.value = url; failed.value = false; loading.value = false; duration.value = 0;
});
async function mediaError() {
  if (loading.value) return;
  if (triedBlob) { failed.value = true; return; }
  triedBlob = true; loading.value = true;
  const current = generation;
  try {
    // WebKitGTK can fetch asset: URLs but its media pipeline cannot stream
    // that custom scheme. Use a browser-owned Blob only when direct loading fails.
    const response = await fetch(props.url, { signal: controller.signal });
    if (!response.ok) throw new Error("Media unavailable");
    const blob = await response.blob();
    if (current !== generation) return;
    blobUrl = URL.createObjectURL(blob); source.value = blobUrl;
  } catch { if (current === generation) failed.value = true; }
  finally { if (current === generation) loading.value = false; }
}
function metadata(event: Event) {
  const value = (event.target as HTMLMediaElement).duration;
  duration.value = Number.isFinite(value) ? value : 0;
}
onBeforeUnmount(() => { generation++; dispose(); });
</script>

<template>
  <div class="resource-media">
    <n-spin v-if="loading" size="small" />
    <p v-if="failed" class="resource-notice">{{ t('repository.resource.decodeFailed') }}</p>
    <component :is="kind" v-else :key="source" :src="source" controls preload="metadata"
      @error="mediaError" @loadedmetadata="metadata" @durationchange="metadata" />
    <p v-if="duration">{{ t('repository.resource.duration', { seconds: duration.toFixed(1) }) }}</p>
  </div>
</template>
