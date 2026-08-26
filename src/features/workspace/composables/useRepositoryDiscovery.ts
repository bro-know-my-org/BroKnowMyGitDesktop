import { computed, ref, type Ref } from "vue";
import type {
  RepositoryDiscoveryResult,
  RepositoryImportResult,
  RepositoryRecord,
} from "../../../shared/types/domain";

interface RepositoryDiscoveryOperations {
  selectDirectory: () => Promise<string | null>;
  discoverRepositories: (rootPath: string) => Promise<RepositoryDiscoveryResult>;
  addRepositories: (repositoryPaths: string[]) => Promise<RepositoryImportResult>;
}

interface RepositoryDiscoveryMessages {
  pickerFailed: () => string;
  scanFailed: () => string;
  addFailed: () => string;
  noRepositoriesFound: () => string;
  addedRepositories: (count: number) => string;
  partiallyAddedRepositories: (added: number, selected: number) => string;
  importPersistenceWarning: (added: number, selected: number) => string;
  importCommittedRefreshFailed: (added: number, selected: number) => string;
  selectionLimitReached: (limit: number) => string;
}

interface RepositoryDiscoveryDependencies {
  repositories: Ref<RepositoryRecord[]>;
  replaceRepositoryRecord: (repository: RepositoryRecord) => void;
  loadRepositories: () => Promise<boolean>;
  operations: RepositoryDiscoveryOperations;
  messages: RepositoryDiscoveryMessages;
  reportError: (error: unknown, action: string) => void;
  showSuccess: (content: string) => void;
  showWarning: (content: string) => void;
}

export function useRepositoryDiscovery({
  repositories,
  replaceRepositoryRecord,
  loadRepositories,
  operations,
  messages,
  reportError,
  showSuccess,
  showWarning,
}: RepositoryDiscoveryDependencies) {
  const discoverModalShown = ref(false);
  const discoveryRootPath = ref("");
  const repositoryImportLimit = ref(0);
  const discoveredRepositories = ref<RepositoryRecord[]>([]);
  const selectedDiscoveredRepositoryPaths = ref<string[]>([]);
  const directoryPickerBusy = ref(false);
  const discoveryScanning = ref(false);
  const discoveryAdding = ref(false);
  let requestGeneration = 0;
  let disposed = false;

  const discoveryBusy = computed(() => discoveryScanning.value || discoveryAdding.value);
  const existingRepositoryPaths = computed(() =>
    new Set(repositories.value.map((repository) => repository.path)));
  const selectedDiscoveredRepositoryPathSet = computed(() =>
    new Set(selectedDiscoveredRepositoryPaths.value));
  const selectableDiscoveredRepositories = computed(() => {
    return discoveredRepositories.value.filter((repository) =>
      !existingRepositoryPaths.value.has(repository.path));
  });
  const selectedDiscoveredRepositoryCount = computed(() => {
    const selectablePaths = new Set(selectableDiscoveredRepositories.value.map((repository) => repository.path));
    return selectedDiscoveredRepositoryPaths.value.filter((path) => selectablePaths.has(path)).length;
  });
  const selectableDiscoveredRepositoryCount = computed(() =>
    selectableDiscoveredRepositories.value.length);
  const selectableRepositoryTargetCount = computed(() =>
    Math.min(selectableDiscoveredRepositories.value.length, repositoryImportLimit.value));
  const allDiscoveredRepositoriesSelected = computed(() =>
    selectableRepositoryTargetCount.value > 0
    && selectedDiscoveredRepositoryCount.value === selectableRepositoryTargetCount.value);
  const someDiscoveredRepositoriesSelected = computed(() =>
    selectedDiscoveredRepositoryCount.value > 0 && !allDiscoveredRepositoriesSelected.value);

  function resetRepositoryDiscovery(): void {
    discoveryRootPath.value = "";
    repositoryImportLimit.value = 0;
    discoveredRepositories.value = [];
    selectedDiscoveredRepositoryPaths.value = [];
  }

  function finishRepositoryDiscovery(): void {
    discoverModalShown.value = false;
    resetRepositoryDiscovery();
  }

  function closeRepositoryDiscovery(): void {
    if (discoveryBusy.value) return;
    requestGeneration += 1;
    finishRepositoryDiscovery();
  }

  function disposeRepositoryDiscovery(): void {
    disposed = true;
    requestGeneration += 1;
    directoryPickerBusy.value = false;
    discoveryScanning.value = false;
    discoveryAdding.value = false;
    finishRepositoryDiscovery();
  }

  function updateRepositoryDiscoveryShown(shown: boolean): void {
    if (shown) discoverModalShown.value = true;
    else closeRepositoryDiscovery();
  }

  function discoveredRepositoryAlreadyAdded(repositoryPath: string): boolean {
    return existingRepositoryPaths.value.has(repositoryPath);
  }

  function discoveredRepositorySelected(repositoryPath: string): boolean {
    return selectedDiscoveredRepositoryPathSet.value.has(repositoryPath);
  }

  function discoveredRepositorySelectionDisabled(repositoryPath: string): boolean {
    return discoveryBusy.value
      || repositoryImportLimit.value === 0
      || discoveredRepositoryAlreadyAdded(repositoryPath)
      || (
        !discoveredRepositorySelected(repositoryPath)
        && selectedDiscoveredRepositoryCount.value >= repositoryImportLimit.value
      );
  }

  function toggleDiscoveredRepositorySelection(repositoryPath: string): void {
    if (discoveredRepositorySelectionDisabled(repositoryPath)) return;
    updateDiscoveredRepositorySelection(
      repositoryPath,
      !discoveredRepositorySelected(repositoryPath),
    );
  }

  function updateDiscoveredRepositorySelection(repositoryPath: string, checked: boolean): void {
    if (
      discoveryBusy.value
      || (checked && (
        repositoryImportLimit.value === 0
        || discoveredRepositoryAlreadyAdded(repositoryPath)
      ))
    ) return;
    const selectablePaths = new Set(selectableDiscoveredRepositories.value.map((repository) => repository.path));
    const selectedPaths = new Set(
      selectedDiscoveredRepositoryPaths.value.filter((path) => selectablePaths.has(path)),
    );
    if (checked) {
      if (selectedPaths.has(repositoryPath)) return;
      if (selectedPaths.size >= repositoryImportLimit.value) {
        showWarning(messages.selectionLimitReached(repositoryImportLimit.value));
        return;
      }
      selectedPaths.add(repositoryPath);
    } else selectedPaths.delete(repositoryPath);
    selectedDiscoveredRepositoryPaths.value = [...selectedPaths];
  }

  function updateAllDiscoveredRepositorySelections(checked: boolean): void {
    selectedDiscoveredRepositoryPaths.value = checked
      ? selectableDiscoveredRepositories.value
        .slice(0, repositoryImportLimit.value)
        .map((repository) => repository.path)
      : [];
  }

  function reportRepositoryImport(result: RepositoryImportResult, selectedCount: number): void {
    if (result.persistenceWarning) {
      showWarning(messages.importPersistenceWarning(result.repositories.length, selectedCount));
    } else if (result.rejectedCount > 0) {
      showWarning(messages.partiallyAddedRepositories(result.repositories.length, selectedCount));
    } else {
      showSuccess(messages.addedRepositories(result.repositories.length));
    }
  }

  function retainUnimportedSelections(result: RepositoryImportResult): void {
    const importedPaths = new Set(result.repositories.map((repository) => repository.path));
    selectedDiscoveredRepositoryPaths.value = selectedDiscoveredRepositoryPaths.value
      .filter((path) => !importedPaths.has(path) && !existingRepositoryPaths.value.has(path))
      .slice(0, result.remainingCapacity);
  }

  async function openRepositoryDiscovery(): Promise<void> {
    if (disposed || directoryPickerBusy.value || discoveryBusy.value) return;
    const generation = ++requestGeneration;
    directoryPickerBusy.value = true;
    let rootPath: string | null;
    try {
      rootPath = await operations.selectDirectory();
    } catch (error) {
      if (generation === requestGeneration) reportError(error, messages.pickerFailed());
      return;
    } finally {
      if (generation === requestGeneration) directoryPickerBusy.value = false;
    }
    if (generation !== requestGeneration || !rootPath) return;

    resetRepositoryDiscovery();
    discoveryRootPath.value = rootPath;
    discoverModalShown.value = true;
    discoveryScanning.value = true;
    try {
      const result = await operations.discoverRepositories(rootPath);
      if (generation !== requestGeneration) return;
      if (!result.repositories.length) {
        finishRepositoryDiscovery();
        showSuccess(messages.noRepositoriesFound());
        return;
      }
      discoveredRepositories.value = result.repositories;
      repositoryImportLimit.value = result.importLimit;
      selectedDiscoveredRepositoryPaths.value = result.repositories
        .filter((repository) => !existingRepositoryPaths.value.has(repository.path))
        .slice(0, result.importLimit)
        .map((repository) => repository.path);
    } catch (error) {
      if (generation !== requestGeneration) return;
      finishRepositoryDiscovery();
      reportError(error, messages.scanFailed());
    } finally {
      if (generation === requestGeneration) discoveryScanning.value = false;
    }
  }

  async function addSelectedDiscoveredRepositories(): Promise<void> {
    const selectablePaths = new Set(selectableDiscoveredRepositories.value.map((repository) => repository.path));
    const repositoryPaths = selectedDiscoveredRepositoryPaths.value.filter((path) => selectablePaths.has(path));
    if (disposed || !repositoryPaths.length || discoveryAdding.value) return;
    const generation = ++requestGeneration;
    discoveryAdding.value = true;
    try {
      const result = await operations.addRepositories(repositoryPaths);
      if (disposed || generation !== requestGeneration) return;
      repositoryImportLimit.value = result.remainingCapacity;
      const refreshed = await loadRepositories();
      if (disposed || generation !== requestGeneration) return;
      if (!refreshed) {
        for (const repository of result.repositories) replaceRepositoryRecord(repository);
        retainUnimportedSelections(result);
        if (result.persistenceWarning) {
          showWarning(messages.importPersistenceWarning(
            result.repositories.length,
            repositoryPaths.length,
          ));
        }
        showWarning(messages.importCommittedRefreshFailed(
          result.repositories.length,
          repositoryPaths.length,
        ));
        return;
      }
      if (result.rejectedCount > 0) {
        retainUnimportedSelections(result);
        reportRepositoryImport(result, repositoryPaths.length);
        return;
      }
      finishRepositoryDiscovery();
      reportRepositoryImport(result, repositoryPaths.length);
    } catch (error) {
      if (!disposed && generation === requestGeneration) reportError(error, messages.addFailed());
    } finally {
      if (!disposed && generation === requestGeneration) discoveryAdding.value = false;
    }
  }

  return {
    discoverModalShown,
    discoveryRootPath,
    repositoryImportLimit,
    discoveredRepositories,
    selectedDiscoveredRepositoryPaths,
    directoryPickerBusy,
    discoveryScanning,
    discoveryAdding,
    discoveryBusy,
    selectableDiscoveredRepositoryCount,
    selectedDiscoveredRepositoryCount,
    allDiscoveredRepositoriesSelected,
    someDiscoveredRepositoriesSelected,
    openRepositoryDiscovery,
    closeRepositoryDiscovery,
    disposeRepositoryDiscovery,
    updateRepositoryDiscoveryShown,
    discoveredRepositoryAlreadyAdded,
    discoveredRepositorySelected,
    discoveredRepositorySelectionDisabled,
    toggleDiscoveredRepositorySelection,
    updateDiscoveredRepositorySelection,
    updateAllDiscoveredRepositorySelections,
    addSelectedDiscoveredRepositories,
  };
}
