<script setup lang="ts">
import { provide } from "vue";
import AppShell from "./app/AppShell.vue";
import { workspaceControllerKey } from "./features/workspace/composables/workspaceContext";
import { useWorkspaceController } from "./features/workspace/composables/useWorkspaceController";
import { markPerformance, measurePerformance } from "./shared/performance";

markPerformance("workspace:controller-create:start");
const workspaceController = useWorkspaceController();
markPerformance("workspace:controller-create:end");
measurePerformance(
  "Workspace controller creation",
  "workspace:controller-create:start",
  "workspace:controller-create:end",
);
provide(workspaceControllerKey, workspaceController);
</script>

<template>
  <AppShell />
</template>
