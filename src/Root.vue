<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import { NConfigProvider, NDialogProvider, NGlobalStyle, NMessageProvider, NNotificationProvider, darkTheme, type GlobalThemeOverrides } from "naive-ui";
import App from "./App.vue";
import {
  interfaceFontFamily,
  resolvedInterfaceFontSize,
  resolvedInterfaceFontWeight,
} from "./shared/fonts";
import { naiveUiDateLocale, naiveUiLocale } from "./shared/i18n";
import { isDarkTheme, themePalette } from "./shared/theme";

// Keep in sync with --app-* tokens in src/styles/tokens.css.
const lightCommonOverrides: GlobalThemeOverrides["common"] = {
  popoverColor: "#ffffff",
  modalColor: "#ffffff",
  cardColor: "#ffffff",
};

const darkCommonOverrides: GlobalThemeOverrides["common"] = {
  // Deepest layer, semi-transparent so CSS backdrop-filter can blur through.
  popoverColor: "rgba(28, 28, 30, 0.88)",
  modalColor: "#2c2c2e",
  cardColor: "#2c2c2e",
  tableColor: "#2c2c2e",
  inputColor: "#1c1c1e",
  actionColor: "#2c2c2e",
  hoverColor: "rgba(255, 255, 255, 0.08)",
  dividerColor: "rgba(255, 255, 255, 0.08)",
  borderColor: "rgba(255, 255, 255, 0.12)",
  bodyColor: "#1c1c1e",
  boxShadow1: "0 1px 2px rgba(0, 0, 0, 0.3)",
  boxShadow2: "0 2px 8px rgba(0, 0, 0, 0.3)",
  boxShadow3: "0 2px 8px rgba(0, 0, 0, 0.3)",
};

const lightCardOverrides: GlobalThemeOverrides["Card"] = {
  color: "#ffffff",
  borderColor: "#e6eaf0",
  boxShadow: "0 1px 2px rgba(23, 35, 55, 0.04)",
};

const darkCardOverrides: GlobalThemeOverrides["Card"] = {
  color: "#2c2c2e",
  borderColor: "rgba(255, 255, 255, 0.08)",
  borderRadius: "12px",
  boxShadow: "0 1px 2px rgba(0, 0, 0, 0.3)",
};

const lightPopoverOverrides: GlobalThemeOverrides["Popover"] = {
  borderRadius: "12px",
};

const darkPopoverOverrides: GlobalThemeOverrides["Popover"] = {
  borderRadius: "12px",
  boxShadow: "0 2px 8px rgba(0, 0, 0, 0.3)",
};

const themeOverrides = computed<GlobalThemeOverrides>(() => ({
  common: {
    primaryColor: themePalette.value.primary,
    primaryColorHover: themePalette.value.hover,
    primaryColorPressed: themePalette.value.pressed,
    primaryColorSuppl: themePalette.value.hover,
    borderRadius: "8px",
    borderRadiusSmall: "6px",
    fontSize: `${resolvedInterfaceFontSize.value}px`,
    fontFamily: interfaceFontFamily.value,
    fontWeight: String(resolvedInterfaceFontWeight.value),
    fontWeightStrong: String(Math.min(900, resolvedInterfaceFontWeight.value + 200)),
    ...(isDarkTheme.value ? darkCommonOverrides : lightCommonOverrides),
  },
  Card: {
    ...(isDarkTheme.value ? darkCardOverrides : lightCardOverrides),
  },
  Popover: {
    ...(isDarkTheme.value ? darkPopoverOverrides : lightPopoverOverrides),
  },
  Dropdown: {
    borderRadius: "12px",
  },
  InternalSelectMenu: {
    borderRadius: "12px",
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
      <n-notification-provider placement="bottom-right">
        <n-message-provider placement="bottom-right">
          <n-global-style />
          <App />
        </n-message-provider>
      </n-notification-provider>
    </n-dialog-provider>
  </n-config-provider>
</template>
