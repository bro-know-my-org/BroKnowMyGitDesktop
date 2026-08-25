import { ref } from "vue";

export type SettingsSection = "general" | "repository" | "about";

export const settingsSection = ref<SettingsSection>("general");
