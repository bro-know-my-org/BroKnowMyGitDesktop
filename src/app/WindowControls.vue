<script setup lang="ts">
import { t } from "../shared/i18n";

defineProps<{ maximized: boolean }>();
defineEmits<{ minimize: []; maximize: []; close: [] }>();
</script>

<template>
  <div class="window-controls" data-window-interactive>
    <button type="button" class="window-control" data-activate-on-release :aria-label="t('workspace.header.minimizeWindow')" :title="t('workspace.header.minimizeWindow')" @click="$emit('minimize')">
      <svg viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M3 8.5h10" /></svg>
    </button>
    <button type="button" class="window-control" data-activate-on-release :aria-label="t(maximized ? 'workspace.header.restoreWindow' : 'workspace.header.maximizeWindow')" :title="t(maximized ? 'workspace.header.restoreWindow' : 'workspace.header.maximizeWindow')" @click="$emit('maximize')">
      <svg v-if="maximized" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M5.5 5.5v-3h8v8h-3" /><rect x="2.5" y="5.5" width="8" height="8" /></svg>
      <svg v-else viewBox="0 0 16 16" fill="none" aria-hidden="true"><rect x="3" y="3" width="10" height="10" /></svg>
    </button>
    <button type="button" class="window-control window-control-close" data-activate-on-release :aria-label="t('workspace.header.closeWindow')" :title="t('workspace.header.closeWindow')" @click="$emit('close')">
      <svg viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="m4 4 8 8m0-8-8 8" /></svg>
    </button>
  </div>
</template>

<style src="./WindowControls.css"></style>
