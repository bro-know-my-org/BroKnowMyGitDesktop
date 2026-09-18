<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { NAlert, NButton, NSpin, useMessage } from "naive-ui";
import { t } from "../../../shared/i18n";
import { exportResourcePreview, getResourcePreview, releaseResourcePreview, resourceUrl, type ResourceComparison, type ResourcePreview } from "../../../shared/api/resources";
import { formatResourceSize, resourceExtension, resourceKind } from "../utils/resourcePreview";
import ImageResourceDiff from "./ImageResourceDiff.vue";
import PdfResourcePreview from "./PdfResourcePreview.vue";
import MediaResourcePreview from "./MediaResourcePreview.vue";
const props = defineProps<{ repositoryPath: string; comparison: ResourceComparison }>();
const preview = shallowRef<ResourcePreview | null>(null);
const loading = ref(false), error = ref(false), exporting = ref<number | null>(null);
const failed = ref([false, false]);
const message = useMessage();
let generation = 0;
const urls = computed(() => preview.value?.sides.map((side) => side?.file && !side.notice ? resourceUrl(side.file) : null) ?? [null, null]);
const kinds = computed(() => preview.value?.sides.map((side) => resourceKind(side?.path ?? props.comparison.path)) ?? []);
const images = computed(() => preview.value?.sides.every((side, i) => !side || kinds.value[i] === "image"));
const sizeDelta = computed(() => (preview.value?.sides[1]?.size ?? 0) - (preview.value?.sides[0]?.size ?? 0));
function release() {
  const previous = preview.value;
  preview.value = null;
  if (previous) void releaseResourcePreview(previous.id).catch(() => {});
}
async function load() {
  const current = ++generation;
  release(); loading.value = true; error.value = false; failed.value = [false, false];
  try {
    const next = await getResourcePreview(props.repositoryPath, props.comparison);
    if (current !== generation) { await releaseResourcePreview(next.id); return; }
    preview.value = next;
  } catch { if (current === generation) error.value = true; }
  finally { if (current === generation) loading.value = false; }
}
async function save(side: number) {
  if (!preview.value || exporting.value !== null) return;
  exporting.value = side;
  try { if (await exportResourcePreview(preview.value.id, side)) message.success(t('repository.resource.exported')); }
  catch { message.error(t('repository.resource.exportFailed')); }
  finally { exporting.value = null; }
}
watch(() => JSON.stringify([props.repositoryPath, props.comparison]), load, { immediate: true });
onBeforeUnmount(() => { generation++; release(); });
</script>

<template>
  <section class="resource-viewer" :aria-label="t('repository.resource.preview')">
    <n-spin v-if="loading" class="resource-loading" />
    <n-alert v-else-if="error" type="error" :bordered="false">
      {{ t('repository.resource.loadFailed') }}
      <n-button size="small" @click="load">{{ t('repository.resource.retry') }}</n-button>
    </n-alert>
    <template v-else-if="preview">
      <div class="resource-summary">{{ t('repository.resource.sizeChange') }}: {{ sizeDelta > 0 ? '+' : sizeDelta < 0 ? '−' : '' }}{{ formatResourceSize(Math.abs(sizeDelta)) }}</div>
      <div class="resource-columns">
        <section v-for="(side, index) in preview.sides" :key="index" class="resource-side">
          <header class="resource-side-header">
            <strong>{{ t(index === 0 ? 'repository.resource.before' : 'repository.resource.after') }}</strong>
            <span v-if="side">{{ formatResourceSize(side.size) }} · {{ resourceExtension(side.path).toUpperCase() || t('repository.resource.binaryFile') }}</span>
            <n-button v-if="side?.exportable" size="tiny" :loading="exporting === index" :disabled="exporting !== null" @click="save(index)">{{ t('repository.resource.export') }}</n-button>
          </header>
          <div v-if="side" class="resource-path" :title="side.path">{{ side.path }}</div>
          <p v-if="!side" class="resource-notice">{{ t(index === 0 ? 'repository.resource.added' : 'repository.resource.deleted') }}</p>
          <p v-else-if="side.notice" class="resource-notice">{{ t(`repository.resource.${side.notice}`) }}</p>
          <template v-else-if="!images && urls[index]">
            <p v-if="failed[index]" class="resource-notice">{{ t('repository.resource.decodeFailed') }}</p>
            <MediaResourcePreview v-else-if="kinds[index] === 'video' || kinds[index] === 'audio'" :url="urls[index]!" :kind="kinds[index] as 'audio' | 'video'" />
            <PdfResourcePreview v-else-if="kinds[index] === 'pdf'" :url="urls[index]!" />
            <img v-else-if="kinds[index] === 'image'" :src="urls[index]!" :alt="side.path" @error="failed[index] = true" />
            <p v-else class="resource-notice">{{ t('repository.resource.binary') }}</p>
          </template>
        </section>
      </div>
      <ImageResourceDiff v-if="images && urls.some(Boolean)" :urls="urls" />
    </template>
  </section>
</template>
