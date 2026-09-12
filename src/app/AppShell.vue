<script setup lang="ts">
import { NLayout } from "naive-ui";
import WorkspaceDialogs from "../features/workspace/components/WorkspaceDialogs.vue";
import WorkspaceHeader from "../features/workspace/components/WorkspaceHeader.vue";
import WorkspaceContent from "./WorkspaceContent.vue";
import PathPicker from "../shared/PathPicker.vue";
</script>

<template>
  <n-layout class="application-shell">
    <n-layout class="application-main">
      <WorkspaceHeader />
      <WorkspaceContent />
    </n-layout>
  </n-layout>
  <WorkspaceDialogs />
  <PathPicker />
</template>

<style src="./AppShell.css"></style>
