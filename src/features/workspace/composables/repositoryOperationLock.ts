import { computed, ref } from "vue";

export type RepositoryOperationRelease = () => void;

const repositoryOperationsInFlight = new Set<string>();
const repositoryOperationCount = ref(0);

export const repositoryOperationsBusy = computed(() => repositoryOperationCount.value > 0);

export function isRepositoryOperationInFlight(repositoryPath: string): boolean {
  return repositoryOperationsInFlight.has(repositoryPath);
}

export function acquireRepositoryOperation(repositoryPath: string): RepositoryOperationRelease | null {
  if (repositoryOperationsInFlight.has(repositoryPath)) return null;
  repositoryOperationsInFlight.add(repositoryPath);
  repositoryOperationCount.value += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    repositoryOperationsInFlight.delete(repositoryPath);
    repositoryOperationCount.value -= 1;
  };
}
