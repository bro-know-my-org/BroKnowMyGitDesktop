import { ref } from "vue";

export type SettingsSection = "general" | "about";

export const settingsSection = ref<SettingsSection>("general");
