import assert from "node:assert/strict";
import test from "node:test";
import { ref } from "vue";
import type { RepositoryRecord } from "../../../shared/types/domain";
import { useRepositoryDiscovery } from "./useRepositoryDiscovery";

function repository(path: string): RepositoryRecord {
  return {
    path,
    name: path.split("/").filter(Boolean).pop() ?? "repository",
    alias: null,
    group: null,
    host: null,
    namespace: null,
    lastOpenedAt: null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((promiseResolve) => {
    resolve = promiseResolve;
  });
  return { promise, resolve };
}

function discoveryHarness(overrides: {
  existing?: RepositoryRecord[];
  selectDirectory?: () => Promise<string | null>;
  discoverRepositories?: (rootPath: string) => Promise<{ repositories: RepositoryRecord[]; importLimit: number }>;
  addRepositories?: (repositoryPaths: string[]) => Promise<{
    repositories: RepositoryRecord[];
    rejectedCount: number;
    remainingCapacity: number;
  }>;
  loadRepositories?: () => Promise<boolean>;
} = {}) {
  const errors: string[] = [];
  const successes: string[] = [];
  const warnings: string[] = [];
  const repositories = ref(overrides.existing ?? []);
  const flow = useRepositoryDiscovery({
    repositories,
    replaceRepositoryRecord: (repository) => {
      const existingIndex = repositories.value.findIndex((candidate) => candidate.path === repository.path);
      repositories.value = existingIndex === -1
        ? [...repositories.value, repository]
        : repositories.value.map((candidate) => candidate.path === repository.path ? repository : candidate);
    },
    loadRepositories: overrides.loadRepositories ?? (async () => true),
    operations: {
      selectDirectory: overrides.selectDirectory ?? (async () => "/projects"),
      discoverRepositories: overrides.discoverRepositories
        ?? (async () => ({ repositories: [], importLimit: 200 })),
      addRepositories: overrides.addRepositories
        ?? (async () => ({ repositories: [], rejectedCount: 0, remainingCapacity: 200 })),
    },
    messages: {
      pickerFailed: () => "picker failed",
      scanFailed: () => "scan failed",
      addFailed: () => "add failed",
      noRepositoriesFound: () => "none found",
      addedRepositories: (count) => `added ${count}`,
      partiallyAddedRepositories: (added, selected) => `added ${added} of ${selected}`,
      importPersistenceWarning: (added, selected) => `durability warning ${added} of ${selected}`,
      importCommittedRefreshFailed: (added, selected) => `committed ${added} of ${selected}, refresh failed`,
      selectionLimitReached: (limit) => `limit ${limit}`,
    },
    reportError: (_error, action) => errors.push(action),
    showSuccess: (content) => successes.push(content),
    showWarning: (content) => warnings.push(content),
  });
  return { flow, repositories, errors, successes, warnings };
}

test("picker cancellation has no scan side effect", async () => {
  let scans = 0;
  const { flow } = discoveryHarness({
    selectDirectory: async () => null,
    discoverRepositories: async () => {
      scans += 1;
      return { repositories: [], importLimit: 200 };
    },
  });

  await flow.openRepositoryDiscovery();

  assert.equal(scans, 0);
  assert.equal(flow.discoverModalShown.value, false);
});

test("native picker failures are reported separately from scan failures", async () => {
  const { flow, errors } = discoveryHarness({
    selectDirectory: async () => {
      throw new Error("dialog unavailable");
    },
  });

  await flow.openRepositoryDiscovery();

  assert.deepEqual(errors, ["picker failed"]);
});

test("overlapping open requests share one native picker", async () => {
  const picker = deferred<string | null>();
  let pickerCalls = 0;
  let scans = 0;
  const { flow } = discoveryHarness({
    selectDirectory: () => {
      pickerCalls += 1;
      return picker.promise;
    },
    discoverRepositories: async () => {
      scans += 1;
      return { repositories: [], importLimit: 200 };
    },
  });

  const firstOpen = flow.openRepositoryDiscovery();
  await Promise.resolve();
  await flow.openRepositoryDiscovery();
  picker.resolve("/projects");
  await firstOpen;

  assert.equal(pickerCalls, 1);
  assert.equal(scans, 1);
});

test("empty and failed scans close the preview with the right feedback", async (context) => {
  await context.test("empty", async () => {
    const { flow, successes } = discoveryHarness();
    await flow.openRepositoryDiscovery();
    assert.equal(flow.discoverModalShown.value, false);
    assert.deepEqual(successes, ["none found"]);
  });

  await context.test("failure", async () => {
    const { flow, errors } = discoveryHarness({
      discoverRepositories: async () => {
        throw new Error("scan unavailable");
      },
    });
    await flow.openRepositoryDiscovery();
    assert.equal(flow.discoverModalShown.value, false);
    assert.deepEqual(errors, ["scan failed"]);
  });
});

test("new repositories are selected by default and select-all is tri-state", async () => {
  const existing = repository("/projects/existing");
  const first = repository("/projects/first");
  const second = repository("/projects/second");
  const { flow } = discoveryHarness({
    existing: [existing],
    discoverRepositories: async () => ({ repositories: [existing, first, second], importLimit: 200 }),
  });

  await flow.openRepositoryDiscovery();

  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, [first.path, second.path]);
  assert.equal(flow.allDiscoveredRepositoriesSelected.value, true);
  flow.updateDiscoveredRepositorySelection(first.path, false);
  assert.equal(flow.someDiscoveredRepositoriesSelected.value, true);
  flow.updateAllDiscoveredRepositorySelections(false);
  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 0);
  flow.updateAllDiscoveredRepositorySelections(true);
  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, [first.path, second.path]);
});

test("large scans cap default and select-all choices at the backend import limit", async () => {
  const candidates = Array.from({ length: 205 }, (_, index) => repository(`/projects/repository-${index}`));
  const { flow, warnings } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: candidates, importLimit: 200 }),
  });

  await flow.openRepositoryDiscovery();

  assert.equal(flow.repositoryImportLimit.value, 200);
  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 200);
  flow.updateAllDiscoveredRepositorySelections(false);
  flow.updateAllDiscoveredRepositorySelections(true);
  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 200);
  assert.equal(flow.discoveredRepositorySelectionDisabled(candidates[0].path), false);
  assert.equal(flow.discoveredRepositorySelectionDisabled(candidates[200].path), true);
  flow.updateDiscoveredRepositorySelection(candidates[200].path, true);
  assert.deepEqual(warnings, ["limit 200"]);
});

test("a candidate added elsewhere no longer consumes the selection limit", async () => {
  const candidates = Array.from({ length: 201 }, (_, index) => repository(`/projects/repository-${index}`));
  const { flow, repositories, warnings } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: candidates, importLimit: 200 }),
  });
  await flow.openRepositoryDiscovery();

  repositories.value = [candidates[0]];
  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 199);
  flow.updateDiscoveredRepositorySelection(candidates[200].path, true);

  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 200);
  assert.deepEqual(warnings, []);
});

test("direct checkbox updates cannot select busy or already-added repositories", async () => {
  const candidate = repository("/projects/candidate");
  const scan = deferred<{ repositories: RepositoryRecord[]; importLimit: number }>();
  const { flow, repositories } = discoveryHarness({ discoverRepositories: () => scan.promise });

  const opening = flow.openRepositoryDiscovery();
  await Promise.resolve();
  flow.updateDiscoveredRepositorySelection(candidate.path, true);
  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, []);

  scan.resolve({ repositories: [candidate], importLimit: 200 });
  await opening;
  flow.updateDiscoveredRepositorySelection(candidate.path, false);
  repositories.value = [candidate];
  flow.updateDiscoveredRepositorySelection(candidate.path, true);

  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, []);
});

test("close requests cannot bypass an active scan and clear state when idle", async () => {
  const scan = deferred<{ repositories: RepositoryRecord[]; importLimit: number }>();
  const candidate = repository("/projects/candidate");
  const { flow } = discoveryHarness({ discoverRepositories: () => scan.promise });

  const opening = flow.openRepositoryDiscovery();
  await Promise.resolve();
  flow.updateRepositoryDiscoveryShown(false);
  assert.equal(flow.discoverModalShown.value, true);

  scan.resolve({ repositories: [candidate], importLimit: 200 });
  await opening;
  flow.updateRepositoryDiscoveryShown(false);
  assert.equal(flow.discoverModalShown.value, false);
  assert.deepEqual(flow.discoveredRepositories.value, []);
});

test("add failure keeps the reviewed selection open", async () => {
  const candidate = repository("/projects/candidate");
  const { flow, errors } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: [candidate], importLimit: 200 }),
    addRepositories: async () => {
      throw new Error("write unavailable");
    },
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.equal(flow.discoverModalShown.value, true);
  assert.deepEqual(errors, ["add failed"]);
});

test("successful add closes only after an authoritative catalog refresh", async () => {
  const candidate = repository("/projects/candidate");
  let refreshes = 0;
  const { flow, successes } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: [candidate], importLimit: 200 }),
    addRepositories: async () => ({ repositories: [candidate], rejectedCount: 0, remainingCapacity: 199 }),
    loadRepositories: async () => {
      refreshes += 1;
      return true;
    },
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.equal(refreshes, 1);
  assert.equal(flow.discoverModalShown.value, false);
  assert.deepEqual(successes, ["added 1"]);
});

test("a committed import with an unconfirmed disk sync reports a durability warning", async () => {
  const candidate = repository("/projects/candidate");
  const { flow, successes, warnings } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: [candidate], importLimit: 200 }),
    addRepositories: async () => ({
      repositories: [candidate],
      rejectedCount: 0,
      remainingCapacity: 199,
      persistenceWarning: true,
    }),
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.equal(flow.discoverModalShown.value, false);
  assert.deepEqual(successes, []);
  assert.deepEqual(warnings, ["durability warning 1 of 1"]);
});

test("failed catalog refresh keeps the preview open and reports one combined warning", async () => {
  const candidate = repository("/projects/candidate");
  const { flow, successes, warnings } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: [candidate], importLimit: 200 }),
    addRepositories: async () => ({ repositories: [candidate], rejectedCount: 0, remainingCapacity: 42 }),
    loadRepositories: async () => false,
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.equal(flow.discoverModalShown.value, true);
  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 0);
  assert.equal(flow.discoveredRepositoryAlreadyAdded(candidate.path), true);
  assert.equal(flow.repositoryImportLimit.value, 42);
  assert.deepEqual(successes, []);
  assert.deepEqual(warnings, ["committed 1 of 1, refresh failed"]);
});

test("failed refresh trims unresolved selections to the new remaining capacity", async () => {
  const candidates = [
    repository("/projects/first"),
    repository("/projects/second"),
    repository("/projects/third"),
  ];
  const { flow, warnings } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: candidates, importLimit: 3 }),
    addRepositories: async () => ({
      repositories: [candidates[0]],
      rejectedCount: 2,
      remainingCapacity: 1,
    }),
    loadRepositories: async () => false,
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, [candidates[1].path]);
  assert.equal(flow.selectedDiscoveredRepositoryCount.value, 1);
  assert.deepEqual(warnings, ["committed 1 of 3, refresh failed"]);
});

test("partial authoritative import keeps only unresolved paths selected", async () => {
  const candidates = [
    repository("/projects/first"),
    repository("/projects/second"),
    repository("/projects/third"),
  ];
  const { flow, repositories, warnings } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: candidates, importLimit: 3 }),
    addRepositories: async () => ({
      repositories: [candidates[0]],
      rejectedCount: 2,
      remainingCapacity: 1,
    }),
    loadRepositories: async () => {
      repositories.value = [candidates[0]];
      return true;
    },
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.equal(flow.discoverModalShown.value, true);
  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, [candidates[1].path]);
  assert.equal(flow.discoveredRepositoryAlreadyAdded(candidates[0].path), true);
  assert.deepEqual(warnings, ["added 1 of 3"]);
});

test("concurrently added paths do not consume retry selection capacity", async () => {
  const candidates = [
    repository("/projects/imported"),
    repository("/projects/concurrent"),
    repository("/projects/rejected"),
  ];
  const { flow, repositories } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: candidates, importLimit: 3 }),
    addRepositories: async () => ({
      repositories: [candidates[0]],
      rejectedCount: 2,
      remainingCapacity: 1,
    }),
    loadRepositories: async () => {
      repositories.value = [candidates[0], candidates[1]];
      return true;
    },
  });
  await flow.openRepositoryDiscovery();

  await flow.addSelectedDiscoveredRepositories();

  assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, [candidates[2].path]);
});

test("disposing invalidates an in-flight import without late feedback", async () => {
  const candidate = repository("/projects/candidate");
  const imported = deferred<{ repositories: RepositoryRecord[]; rejectedCount: number; remainingCapacity: number }>();
  const { flow, repositories, successes, warnings, errors } = discoveryHarness({
    discoverRepositories: async () => ({ repositories: [candidate], importLimit: 200 }),
    addRepositories: () => imported.promise,
  });
  await flow.openRepositoryDiscovery();

  const adding = flow.addSelectedDiscoveredRepositories();
  flow.disposeRepositoryDiscovery();
  imported.resolve({ repositories: [candidate], rejectedCount: 0, remainingCapacity: 199 });
  await adding;

  assert.deepEqual(repositories.value, []);
  assert.deepEqual(successes, []);
  assert.deepEqual(warnings, []);
  assert.deepEqual(errors, []);
});

test("disposing invalidates an in-flight native picker", async (context) => {
  await context.test("late selection", async () => {
    const picker = deferred<string | null>();
    let scans = 0;
    const { flow, errors } = discoveryHarness({
      selectDirectory: () => picker.promise,
      discoverRepositories: async () => {
        scans += 1;
        return { repositories: [], importLimit: 200 };
      },
    });

    const opening = flow.openRepositoryDiscovery();
    await Promise.resolve();
    assert.equal(flow.directoryPickerBusy.value, true);
    flow.disposeRepositoryDiscovery();
    picker.resolve("/projects");
    await opening;

    assert.equal(scans, 0);
    assert.equal(flow.discoverModalShown.value, false);
    assert.equal(flow.directoryPickerBusy.value, false);
    assert.equal(flow.discoveryRootPath.value, "");
    assert.equal(flow.repositoryImportLimit.value, 0);
    assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, []);
    assert.deepEqual(errors, []);
  });

  await context.test("late rejection", async () => {
    let rejectPicker!: (error: Error) => void;
    const picker = new Promise<string | null>((_resolve, reject) => {
      rejectPicker = reject;
    });
    const { flow, errors } = discoveryHarness({ selectDirectory: () => picker });

    const opening = flow.openRepositoryDiscovery();
    await Promise.resolve();
    assert.equal(flow.directoryPickerBusy.value, true);
    flow.disposeRepositoryDiscovery();
    rejectPicker(new Error("dialog unavailable"));
    await opening;

    assert.equal(flow.discoverModalShown.value, false);
    assert.equal(flow.directoryPickerBusy.value, false);
    assert.deepEqual(errors, []);
  });
});

test("disposing invalidates an in-flight repository scan", async (context) => {
  await context.test("late result", async () => {
    const scan = deferred<{ repositories: RepositoryRecord[]; importLimit: number }>();
    const candidate = repository("/projects/candidate");
    const { flow, successes, errors } = discoveryHarness({
      discoverRepositories: () => scan.promise,
    });

    const opening = flow.openRepositoryDiscovery();
    await Promise.resolve();
    assert.equal(flow.discoveryScanning.value, true);
    flow.disposeRepositoryDiscovery();
    scan.resolve({ repositories: [candidate], importLimit: 200 });
    await opening;

    assert.equal(flow.discoverModalShown.value, false);
    assert.equal(flow.discoveryScanning.value, false);
    assert.equal(flow.discoveryRootPath.value, "");
    assert.equal(flow.repositoryImportLimit.value, 0);
    assert.deepEqual(flow.discoveredRepositories.value, []);
    assert.deepEqual(flow.selectedDiscoveredRepositoryPaths.value, []);
    assert.deepEqual(successes, []);
    assert.deepEqual(errors, []);
  });

  await context.test("late rejection", async () => {
    let rejectScan!: (error: Error) => void;
    const scan = new Promise<{ repositories: RepositoryRecord[]; importLimit: number }>(
      (_resolve, reject) => {
        rejectScan = reject;
      },
    );
    const { flow, errors } = discoveryHarness({ discoverRepositories: () => scan });

    const opening = flow.openRepositoryDiscovery();
    await Promise.resolve();
    assert.equal(flow.discoveryScanning.value, true);
    flow.disposeRepositoryDiscovery();
    rejectScan(new Error("scan unavailable"));
    await opening;

    assert.equal(flow.discoverModalShown.value, false);
    assert.equal(flow.discoveryScanning.value, false);
    assert.deepEqual(errors, []);
  });
});
