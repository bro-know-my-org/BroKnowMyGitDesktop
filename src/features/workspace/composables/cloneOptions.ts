import type { CloneOptions } from "../../../shared/types/domain";

export function defaultCloneOptionsForm() {
  return {
    shallow: false,
    depth: 1 as number | null,
    branch: "",
    singleBranch: false,
    noTags: false,
    recurseSubmodules: false,
    shallowSubmodules: false,
    partial: false,
    noCheckout: false,
  };
}

export function validCloneDepth(form: ReturnType<typeof defaultCloneOptionsForm>): boolean {
  return !form.shallow || (form.depth !== null && Number.isInteger(form.depth) && form.depth > 0 && form.depth <= 2147483647);
}

export function buildCloneOptions(form: ReturnType<typeof defaultCloneOptionsForm>): CloneOptions {
  return {
    depth: form.shallow ? form.depth ?? undefined : undefined,
    branch: form.branch.trim() || undefined,
    singleBranch: form.singleBranch,
    noTags: form.noTags,
    recurseSubmodules: form.recurseSubmodules && !form.noCheckout,
    shallowSubmodules: form.shallowSubmodules && form.recurseSubmodules && !form.noCheckout,
    filter: form.partial ? "blob:none" : undefined,
    noCheckout: form.noCheckout,
  };
}
