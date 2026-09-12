<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { NAlert, NButton, NFormItem, NInput, NModal, NSpace, NText } from "naive-ui";
import { api, isAbsoluteExecutablePath } from "./api/tauri";
import { localizedErrorMessage, t } from "./i18n";
import { finishPathSelection, normalizePickerPath, pathPickerRequest } from "./pathPicker";

const path = ref("");
const browsing = ref(false);
const error = ref("");
const normalizedPath = computed(() => normalizePickerPath(path.value));
const valid = computed(() => isAbsoluteExecutablePath(normalizedPath.value));
watch(pathPickerRequest, (request) => {
  path.value = request?.initialPath ?? "";
  error.value = "";
});
function finish(value: string | null): void {
  if (pathPickerRequest.value) finishPathSelection(pathPickerRequest.value.id, value);
}
function confirm(): void {
  if (valid.value && !browsing.value) finish(normalizedPath.value);
}
onBeforeUnmount(() => finish(null));
async function browse(): Promise<void> {
  const request = pathPickerRequest.value;
  if (!request || browsing.value) return;
  browsing.value = true;
  error.value = "";
  try {
    const defaultPath = valid.value ? normalizedPath.value : undefined;
    const selected = request.directory
      ? await api.selectRepositoryDiscoveryDirectory(request.title, defaultPath)
      : await api.selectApplicationExecutable(request.title, defaultPath);
    if (pathPickerRequest.value?.id === request.id && selected) path.value = selected;
  } catch (cause) {
    if (pathPickerRequest.value?.id === request.id) {
      error.value = localizedErrorMessage(typeof cause === "object" && cause !== null && "code" in cause ? String(cause.code) : "command_failed");
    }
  } finally {
    browsing.value = false;
  }
}
</script>

<template>
  <n-modal :show="Boolean(pathPickerRequest)" preset="card" :title="pathPickerRequest?.title" class="path-picker-dialog"
    :mask-closable="!browsing" :closable="!browsing" :close-on-esc="!browsing" @update:show="(shown) => { if (!shown) finish(null); }">
    <n-form-item :label="t(pathPickerRequest?.directory ? 'workspace.pathPicker.folderPath' : 'workspace.dialogs.executableAbsolutePath')"
      :validation-status="path && !valid ? 'warning' : undefined" :feedback="path && !valid ? t('workspace.pathPicker.absolutePath') : undefined">
      <n-input v-model:value="path" autofocus clearable :disabled="browsing" :placeholder="t('workspace.pathPicker.placeholder')" @keyup.enter="confirm" />
    </n-form-item>
    <n-text depth="3">{{ t('workspace.pathPicker.hint') }}</n-text>
    <n-alert v-if="error" type="error" :bordered="false">{{ error }}</n-alert>
    <template #action>
      <n-space justify="space-between">
        <n-button :loading="browsing" :disabled="browsing" @click="browse">{{ t('workspace.pathPicker.browse') }}</n-button>
        <n-space>
          <n-button :disabled="browsing" @click="finish(null)">{{ t('workspace.dialogs.cancel') }}</n-button>
          <n-button type="primary" :disabled="!valid || browsing" @click="confirm">{{ t('workspace.pathPicker.usePath') }}</n-button>
        </n-space>
      </n-space>
    </template>
  </n-modal>
</template>

<style>
.path-picker-dialog {
  width: min(640px, calc(100vw - 32px));
}
</style>
