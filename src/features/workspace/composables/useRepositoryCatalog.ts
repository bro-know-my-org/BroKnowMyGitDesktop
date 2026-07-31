import { computed, reactive, ref, type Ref } from "vue";
import { api } from "../../../shared/api/tauri";
import { appLocale, t } from "../../../shared/i18n";
import type { RepositoryRecord, RepositorySnapshot } from "../../../shared/types/domain";
import {
  initialRepositoryGrouping,
  type RepositoryGrouping,
  type RepositorySidebarGroup,
} from "./workspaceControllerSupport";

interface RepositoryCatalogDependencies {
  snapshot: Ref<RepositorySnapshot | null>;
  mutationBusy: Ref<boolean>;
  reportError: (error: unknown, action: string) => void;
  showSuccess: (content: string) => void;
}

export function useRepositoryCatalog({
  snapshot,
  mutationBusy,
  reportError,
  showSuccess,
}: RepositoryCatalogDependencies) {
  const repositories = ref<RepositoryRecord[]>([]);
  const repositoryGrouping = ref<RepositoryGrouping>(initialRepositoryGrouping());
  const repositoriesLoading = ref(false);
  const repositoryGroupModalShown = ref(false);
  const repositoryAliasModalShown = ref(false);
  const repositoryGroupTarget = ref<RepositoryRecord | null>(null);
  const repositoryAliasTarget = ref<RepositoryRecord | null>(null);
  const repositoryGroupForm = reactive({ name: "" });
  const repositoryAliasForm = reactive({ name: "" });
  let repositoriesLoadPromise: Promise<void> | null = null;
  let repositoriesReloadPending = false;
  let repositoriesRefreshOwnersPending = false;

  function compareRepositoryRecords(left: RepositoryRecord, right: RepositoryRecord): number {
    const leftLabel = left.alias?.trim() || left.name;
    const rightLabel = right.alias?.trim() || right.name;
    return leftLabel.localeCompare(rightLabel, appLocale.value, { sensitivity: "base" })
      || left.path.localeCompare(right.path, appLocale.value);
  }

  const repositoryGroups = computed<RepositorySidebarGroup[]>(() => {
    const sorted = [...repositories.value].sort(compareRepositoryRecords);

    if (repositoryGrouping.value === "manual") {
      const groups = new Map<string, RepositorySidebarGroup>();
      const ungrouped: RepositoryRecord[] = [];
      for (const repository of sorted) {
        const name = repository.group?.trim();
        if (!name) {
          ungrouped.push(repository);
          continue;
        }
        const key = name.toLocaleLowerCase();
        const group = groups.get(key);
        if (group) group.repositories.push(repository);
        else groups.set(key, { key: `manual:${key}`, label: name, repositories: [repository] });
      }
      const namedGroups = [...groups.values()].sort((left, right) =>
        left.label.localeCompare(right.label, appLocale.value));
      return ungrouped.length
        ? [...namedGroups, { key: "manual:ungrouped", label: t("workspace.controller.ungrouped"), repositories: ungrouped }]
        : namedGroups;
    }

    if (repositoryGrouping.value === "owner") {
      const groups = new Map<string, RepositorySidebarGroup>();
      const unknown: RepositoryRecord[] = [];
      for (const repository of sorted) {
        const host = repository.host?.trim();
        const namespace = repository.namespace?.trim();
        if (!host || !namespace) {
          unknown.push(repository);
          continue;
        }
        const key = `${host.toLocaleLowerCase()}/${namespace.toLocaleLowerCase()}`;
        const group = groups.get(key);
        if (group) group.repositories.push(repository);
        else groups.set(key, { key: `owner:${key}`, label: `${host} / ${namespace}`, repositories: [repository] });
      }
      const ownerGroups = [...groups.values()].sort((left, right) =>
        left.label.localeCompare(right.label, appLocale.value));
      return unknown.length
        ? [...ownerGroups, { key: "owner:unknown", label: t("workspace.controller.unrecognizedOwner"), repositories: unknown }]
        : ownerGroups;
    }

    const now = Date.now();
    const today = new Date(now);
    today.setHours(0, 0, 0, 0);
    const todayStart = today.getTime();
    const weekStart = now - 7 * 24 * 60 * 60 * 1000;
    const buckets: RepositorySidebarGroup[] = [
      { key: "recent:today", label: t("workspace.controller.today"), repositories: [] },
      { key: "recent:week", label: t("workspace.controller.last7Days"), repositories: [] },
      { key: "recent:older", label: t("workspace.controller.earlier"), repositories: [] },
      { key: "recent:never", label: t("workspace.controller.notOpenedYet"), repositories: [] },
    ];
    for (const repository of sorted) {
      const openedAt = repository.lastOpenedAt;
      if (typeof openedAt !== "number" || !Number.isFinite(openedAt)) buckets[3].repositories.push(repository);
      else if (openedAt >= todayStart) buckets[0].repositories.push(repository);
      else if (openedAt >= weekStart) buckets[1].repositories.push(repository);
      else buckets[2].repositories.push(repository);
    }
    for (const bucket of buckets) {
      bucket.repositories.sort((left, right) =>
        (right.lastOpenedAt ?? 0) - (left.lastOpenedAt ?? 0) || compareRepositoryRecords(left, right));
    }
    return buckets.filter((bucket) => bucket.repositories.length);
  });

  function replaceRepositoryRecord(repository: RepositoryRecord): void {
    const index = repositories.value.findIndex((item) => item.path === repository.path);
    if (index === -1) repositories.value = [...repositories.value, repository];
    else repositories.value = repositories.value.map((item) =>
      item.path === repository.path ? repository : item);
  }

  function openRepositoryGroupEditor(repository: RepositoryRecord): void {
    repositoryGroupTarget.value = repository;
    repositoryGroupForm.name = repository.group || "";
    repositoryGroupModalShown.value = true;
  }

  async function saveRepositoryGroup(): Promise<void> {
    const target = repositoryGroupTarget.value;
    if (!target) return;
    mutationBusy.value = true;
    try {
      const name = repositoryGroupForm.name.trim();
      const updated = await api.setRepositoryGroup(target.path, name || null);
      replaceRepositoryRecord(updated);
      if (snapshot.value?.repository.path === updated.path) {
        snapshot.value = { ...snapshot.value, repository: updated };
      }
      repositoryGroupModalShown.value = false;
      showSuccess(name
        ? t("workspace.controller.repositoryGroupSaved")
        : t("workspace.controller.repositoryMovedToUngrouped"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotSaveRepositoryGroup"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function openRepositoryAliasEditor(repository: RepositoryRecord): void {
    repositoryAliasTarget.value = repository;
    repositoryAliasForm.name = repository.alias || "";
    repositoryAliasModalShown.value = true;
  }

  async function saveRepositoryAlias(): Promise<void> {
    const target = repositoryAliasTarget.value;
    if (!target) return;
    mutationBusy.value = true;
    try {
      const name = repositoryAliasForm.name.trim();
      const updated = await api.setRepositoryAlias(target.path, name || null);
      replaceRepositoryRecord(updated);
      if (snapshot.value?.repository.path === updated.path) {
        snapshot.value = { ...snapshot.value, repository: updated };
      }
      repositoryAliasModalShown.value = false;
      showSuccess(name
        ? t("workspace.controller.repositoryAliasSaved")
        : t("workspace.controller.repositoryAliasRemoved"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotSaveRepositoryAlias"));
    } finally {
      mutationBusy.value = false;
    }
  }

  async function loadRepositories(refreshOwners = repositoryGrouping.value === "owner"): Promise<void> {
    if (repositoriesLoadPromise) {
      repositoriesReloadPending = true;
      repositoriesRefreshOwnersPending ||= refreshOwners;
      return repositoriesLoadPromise;
    }
    repositoriesLoadPromise = (async () => {
      repositoriesLoading.value = true;
      let shouldRefreshOwners = refreshOwners;
      try {
        do {
          repositoriesReloadPending = false;
          repositoriesRefreshOwnersPending = false;
          try {
            repositories.value = await api.listRepositories(shouldRefreshOwners);
          } catch (error) {
            reportError(error, t("workspace.controller.couldNotLoadRepositories"));
            if (!repositoriesReloadPending) break;
          }
          shouldRefreshOwners = repositoriesRefreshOwnersPending;
        } while (repositoriesReloadPending);
      } finally {
        repositoriesLoading.value = false;
        repositoriesLoadPromise = null;
      }
    })();
    return repositoriesLoadPromise;
  }

  return {
    repositories,
    repositoryGrouping,
    repositoriesLoading,
    repositoryGroups,
    repositoryGroupModalShown,
    repositoryAliasModalShown,
    repositoryGroupTarget,
    repositoryAliasTarget,
    repositoryGroupForm,
    repositoryAliasForm,
    compareRepositoryRecords,
    replaceRepositoryRecord,
    openRepositoryGroupEditor,
    saveRepositoryGroup,
    openRepositoryAliasEditor,
    saveRepositoryAlias,
    loadRepositories,
  };
}
