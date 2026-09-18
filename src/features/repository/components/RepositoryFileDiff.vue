<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NButton } from "naive-ui";
import { t } from "../../../shared/i18n";
import type { ResourceComparison } from "../../../shared/api/resources";
import { isResourceDiff, resourceExtension } from "../utils/resourcePreview";
import ResourceDiffViewer from "./ResourceDiffViewer.vue";
const props = defineProps<{ repositoryPath: string | null; comparison: ResourceComparison | null; diffText: string; loading: boolean }>();
const source = ref(false);
const resource = computed(() => props.comparison && isResourceDiff(props.comparison.path, props.diffText, props.comparison.previousPath));
const svg = computed(() => [props.comparison?.path, props.comparison?.previousPath].some((path) => resourceExtension(path ?? "") === "svg"));
watch(() => JSON.stringify([props.repositoryPath, props.comparison]), () => { source.value = false; });
</script>

<template>
  <div class="resource-file-diff">
    <div v-if="resource && svg" class="resource-toolbar">
      <n-button size="small" :type="source ? 'default' : 'primary'" @click="source = false">{{ t('repository.resource.preview') }}</n-button>
      <n-button size="small" :type="source ? 'primary' : 'default'" @click="source = true">{{ t('repository.resource.source') }}</n-button>
    </div>
    <ResourceDiffViewer v-if="resource && !source && !loading && comparison && repositoryPath"
      :key="JSON.stringify([repositoryPath, comparison])" :repository-path="repositoryPath" :comparison="comparison" />
    <slot v-else />
  </div>
</template>

<style src="./ResourceDiffViewer.css"></style>
