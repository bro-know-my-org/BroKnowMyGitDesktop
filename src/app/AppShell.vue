<script setup lang="ts">
import { NLayout } from "naive-ui";
import RepositorySidebar from "../features/workspace/components/RepositorySidebar.vue";
import WorkspaceDialogs from "../features/workspace/components/WorkspaceDialogs.vue";
import WorkspaceHeader from "../features/workspace/components/WorkspaceHeader.vue";
import WorkspaceContent from "./WorkspaceContent.vue";
</script>

<template>
  <n-layout has-sider class="application-shell">
    <RepositorySidebar />
    <n-layout class="application-main">
      <WorkspaceHeader />
      <WorkspaceContent />
    </n-layout>
  </n-layout>
  <WorkspaceDialogs />
</template>

