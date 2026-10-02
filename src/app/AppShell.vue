<script setup lang="ts">
import { NLayout } from "naive-ui";
import WorkspaceDialogs from "../features/workspace/components/WorkspaceDialogs.vue";
import WorkspaceHeader from "../features/workspace/components/WorkspaceHeader.vue";
import WorkspaceContent from "./WorkspaceContent.vue";
import PathPicker from "../shared/PathPicker.vue";
import WindowControls from "./WindowControls.vue";
import { resizeDirections, useWindowChrome } from "./useWindowChrome";

const { enabled, focused, maximized, fullscreen, minimize, toggleMaximize, close, startTitlebarGesture, startResize } = useWindowChrome();
</script>

<template>
  <n-layout class="application-shell">
    <n-layout class="application-main">
      <div class="window-toolbar" :class="{ 'window-toolbar-custom': enabled, 'window-toolbar-inactive': enabled && !focused }" @mousedown="startTitlebarGesture">
        <WorkspaceHeader :custom-titlebar="enabled" />
        <div v-if="enabled && !fullscreen" class="window-drag-space" aria-hidden="true" />
        <WindowControls v-if="enabled && !fullscreen" :maximized="maximized" @minimize="minimize" @maximize="toggleMaximize" @close="close" />
      </div>
      <WorkspaceContent />
    </n-layout>
  </n-layout>
  <WorkspaceDialogs />
  <PathPicker />
  <template v-if="enabled && !maximized && !fullscreen">
    <div v-for="direction in resizeDirections" :key="direction" class="window-resize-handle" :data-direction="direction" aria-hidden="true" @mousedown.stop="startResize($event, direction)" />
  </template>
</template>

<style src="./AppShell.css"></style>
