import { onScopeDispose, ref, watch, type Ref } from "vue";
import { api } from "../../../shared/api/tauri";
import type { CodeHostRepositoryInfo } from "../../../shared/types/domain";

export function useCloneRepositoryList(
  accountId: Ref<string | null>,
  reportError: (error: unknown) => void,
  list = api.listGitHubCloneRepositories,
) {
  const repositories = ref<CodeHostRepositoryInfo[]>([]);
  const loading = ref(false);
  const failed = ref(false);
  const hasMore = ref(false);
  let page = 0;
  let generation = 0;
  onScopeDispose(() => { generation += 1; });
  async function loadRepositories(reset = false): Promise<void> {
    const id = accountId.value;
    const requestId = ++generation;
    const isCurrent = () => requestId === generation && id === accountId.value;
    if (reset) {
      repositories.value = [];
      page = 0;
      hasMore.value = false;
    }
    failed.value = false;
    loading.value = Boolean(id);
    if (!id) return;
    try {
      const next = await list(id, page + 1);
      if (!isCurrent()) return;
      repositories.value = [...new Map([...repositories.value, ...next].map((item) => [item.fullName, item])).values()];
      page += 1;
      hasMore.value = next.length === 100 && page < 1000;
    } catch (error) {
      if (!isCurrent()) return;
      failed.value = true;
      reportError(error);
    } finally {
      if (isCurrent()) loading.value = false;
    }
  }
  watch(accountId, () => { void loadRepositories(true); }, { immediate: true });
  return { repositories, loading, failed, hasMore, loadRepositories };
}
