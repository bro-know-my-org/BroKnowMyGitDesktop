import { computed, ref, type Ref } from "vue";
import type { RepositorySidebarGroup } from "./workspaceControllerSupport";

const storageKey = "broknowmygitdesktop.repository-picker-preferences";
type PreferenceKey = "starred" | "hidden" | "collapsed";
type Preferences = Record<PreferenceKey, string[]>;
type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;

function browserStorage(): PreferenceStorage | null {
  try { return typeof window === "undefined" ? null : window.localStorage; }
  catch { return null; }
}

export function useRepositoryPickerPreferences(
  groups: Ref<RepositorySidebarGroup[]>,
  storage: PreferenceStorage | null = browserStorage(),
) {
  const initial: Preferences = { starred: [], hidden: [], collapsed: [] };
  try {
    const saved = JSON.parse(storage?.getItem(storageKey) ?? "null");
    for (const key of Object.keys(initial) as PreferenceKey[]) {
      if (Array.isArray(saved?.[key])) {
        initial[key] = [...new Set<string>(saved[key].filter((value: unknown) =>
          typeof value === "string" && value.length > 0))];
      }
    }
  } catch { /* Invalid or unavailable storage should not block the picker. */ }
  const preferences = ref(initial);
  const showAllRepositories = ref(false);
  const isStarred = (path: string) => preferences.value.starred.includes(path);
  const isHidden = (path: string) => preferences.value.hidden.includes(path);
  const isCollapsed = (key: string) => preferences.value.collapsed.includes(key);

  function toggle(key: PreferenceKey, value: string): void {
    const values = preferences.value[key];
    preferences.value = { ...preferences.value,
      [key]: values.includes(value) ? values.filter((item) => item !== value) : [...values, value] };
    try { storage?.setItem(storageKey, JSON.stringify(preferences.value)); }
    catch { /* Keep preferences available for this session if storage is disabled. */ }
  }

  const pickerGroups = computed<RepositorySidebarGroup[]>(() => {
    const starred = new Set(preferences.value.starred);
    const hidden = new Set(preferences.value.hidden);
    const visible = groups.value.map((group) => ({ ...group,
      repositories: group.repositories.filter((repository) =>
        showAllRepositories.value || !hidden.has(repository.path)) }));
    const favorites = visible.flatMap((group) => group.repositories)
      .filter((repository) => starred.has(repository.path));
    const regular = visible.map((group) => ({ ...group,
      repositories: group.repositories.filter((repository) => !starred.has(repository.path)) }))
      .filter((group) => group.repositories.length);
    return favorites.length ? [{ key: "starred", label: "", repositories: favorites }, ...regular] : regular;
  });

  return {
    pickerGroups, showAllRepositories, isStarred, isHidden, isCollapsed,
    toggleStar: (path: string) => toggle("starred", path),
    toggleHidden: (path: string) => toggle("hidden", path),
    toggleCollapsed: (key: string) => toggle("collapsed", key),
  };
}
