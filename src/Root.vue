<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import { NConfigProvider, NDialogProvider, NGlobalStyle, NMessageProvider, darkTheme, type GlobalThemeOverrides } from "naive-ui";
import App from "./App.vue";
import { naiveUiDateLocale, naiveUiLocale } from "./shared/i18n";
import { isDarkTheme, themePalette } from "./shared/theme";

const lightCardOverrides: GlobalThemeOverrides["Card"] = {
  borderColor: "#e6eaf0",
  boxShadow: "0 1px 2px rgba(23, 35, 55, 0.025)",
};

const darkCardOverrides: GlobalThemeOverrides["Card"] = {
  borderColor: "#30363d",
  boxShadow: "0 1px 2px rgba(0, 0, 0, 0.22)",
};

const themeOverrides = computed<GlobalThemeOverrides>(() => ({
  common: {
    primaryColor: themePalette.value.primary,
    primaryColorHover: themePalette.value.hover,
    primaryColorPressed: themePalette.value.pressed,
    primaryColorSuppl: themePalette.value.hover,
    borderRadius: "0px",
    fontSize: "14px",
  },
  Card: {
    ...(isDarkTheme.value ? darkCardOverrides : lightCardOverrides),
  },
  Button: {
    textColorPrimary: themePalette.value.primaryText,
    textColorHoverPrimary: themePalette.value.hoverText,
    textColorPressedPrimary: themePalette.value.pressedText,
    textColorFocusPrimary: themePalette.value.hoverText,
  },
}));

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
