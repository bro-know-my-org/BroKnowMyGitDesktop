<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import { NConfigProvider, NDialogProvider, NGlobalStyle, NMessageProvider, darkTheme, type GlobalThemeOverrides } from "naive-ui";
import App from "./App.vue";
import { naiveUiDateLocale, naiveUiLocale } from "./shared/i18n";
import { isDarkTheme } from "./shared/theme";

const lightThemeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#16845f",
    primaryColorHover: "#1d9a70",
    primaryColorPressed: "#0f6d4d",
    borderRadius: "0px",
    fontSize: "14px",
  },
  Card: {
    borderColor: "#e6eaf0",
    boxShadow: "0 1px 2px rgba(23, 35, 55, 0.025)",
  },
};

const darkThemeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#2da875",
    primaryColorHover: "#4cc58e",
    primaryColorPressed: "#20815a",
    borderRadius: "0px",
    fontSize: "14px",
  },
  Card: {
    borderColor: "#30363d",
    boxShadow: "0 1px 2px rgba(0, 0, 0, 0.22)",
  },
};

const themeOverrides = computed(() => (isDarkTheme.value ? darkThemeOverrides : lightThemeOverrides));

function preventNativeContextMenu(event: MouseEvent): void {
  event.preventDefault();
}

onMounted(() => {
  document.addEventListener("contextmenu", preventNativeContextMenu);
});

onBeforeUnmount(() => {
  document.removeEventListener("contextmenu", preventNativeContextMenu);
});
</script>

<template>
  <n-config-provider :locale="naiveUiLocale" :date-locale="naiveUiDateLocale" :theme="isDarkTheme ? darkTheme : null" :theme-overrides="themeOverrides">
    <n-dialog-provider>
      <n-message-provider placement="bottom-right">
        <n-global-style />
        <App />
      </n-message-provider>
    </n-dialog-provider>
  </n-config-provider>
</template>
