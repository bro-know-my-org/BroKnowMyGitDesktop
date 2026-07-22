<script setup lang="ts">
import { provide } from "vue";
import AppShell from "./app/AppShell.vue";
import { workspaceControllerKey } from "./features/workspace/composables/workspaceContext";
import { useWorkspaceController } from "./features/workspace/composables/useWorkspaceController";

provide(workspaceControllerKey, useWorkspaceController());
</script>

<template>
  <AppShell />
</template>
