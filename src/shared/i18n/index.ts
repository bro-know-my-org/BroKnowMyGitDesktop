import { computed, ref } from "vue";
import { dateEnUS, dateZhCN, enUS, zhCN } from "naive-ui";

export type AppLocale = "zh-CN" | "en-US";

const localeStorageKey = "broknowmygitdesktop.locale";

function initialLocale(): AppLocale {
  if (typeof window === "undefined") return "zh-CN";
  try {
    const saved = window.localStorage.getItem(localeStorageKey);
    if (saved === "zh-CN" || saved === "en-US") return saved;
  } catch {
    // A disabled storage area must not prevent the UI from starting.
  }
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en-US";
}

export const appLocale = ref<AppLocale>(initialLocale());

function applyDocumentLocale(locale: AppLocale): void {
  if (typeof document !== "undefined") document.documentElement.lang = locale;
}

applyDocumentLocale(appLocale.value);

export function setAppLocale(locale: AppLocale): void {
  appLocale.value = locale;
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(localeStorageKey, locale);
    } catch {
      // The current session can still use the selected locale.
    }
  }
  applyDocumentLocale(locale);
}

/**
 * Small, dependency-free translation helper. Keeping the Chinese and English
 * source strings adjacent makes product copy review straightforward while the
 * locale state keeps every Vue render reactive.
 */
export function tr(zh: string, en: string, values: Record<string, string | number> = {}): string {
  const source = appLocale.value === "zh-CN" ? zh : en;
  return source.replace(/\{(\w+)\}/gu, (_, key: string) => String(values[key] ?? `{${key}}`));
}

const englishErrors: Record<string, string> = {
  account_not_found: "The selected account could not be found.",
  account_selection_required: "Select one available account that matches this remote.",
  account_status_refresh_failed: "Account authentication status could not be refreshed.",
  adapter_endpoint_mismatch: "This Git service adapter does not apply to the selected endpoint.",
  adapter_context_mismatch: "The remote, Git service endpoint, and account must belong together.",
  askpass_configuration_invalid: "The internal Git askpass configuration is invalid.",
  askpass_configuration_missing: "The internal Git askpass configuration is missing.",
  askpass_output_failed: "The internal Git askpass helper could not write its response.",
  askpass_program_required: "The internal Git askpass helper is unavailable.",
  askpass_prompt_missing: "The internal Git askpass helper did not receive an authentication prompt.",
  askpass_prompt_host_mismatch: "The internal Git askpass helper rejected a credential prompt for a different host.",
  askpass_prompt_unsupported: "The internal Git askpass helper received an unsupported prompt.",
  authentication_method_mismatch: "This authentication method is not available for the selected endpoint.",
  binding_endpoint_mismatch: "The selected account does not belong to this remote endpoint.",
  builtin_endpoint_immutable: "Built-in endpoints cannot be overwritten.",
  already_git_repository: "The selected folder is already a Git repository.",
  branch_already_exists: "A branch with this name already exists.",
  branch_not_found: "The selected branch could not be found.",
  browser_demo_unsupported: "This destructive operation is not simulated in the browser demo.",
  branch_name_required: "Enter a branch name.",
  cannot_delete_current_branch: "The current branch cannot be deleted.",
  cannot_remove_active_worktree: "Switch to another worktree before removing the current one.",
  cannot_remove_main_worktree: "The main worktree cannot be removed.",
  cli_account_import_failed: "The current CLI account login could not be read.",
  cli_account_import_invalid: "This endpoint cannot use the selected CLI account import method.",
  cli_auth_unavailable: "The current CLI session has no usable authenticated account.",
  cli_import_not_supported: "This endpoint does not support importing an account from gh or glab.",
  cli_token_unavailable: "The current CLI session cannot provide Git credentials.",
  clone_repository_identity_unavailable: "The clone URL does not identify a supported HTTPS or SSH repository.",
  command_failed: "The operation could not be completed.",
  invalid_commit_message: "Enter a commit message.",
  commit_message_required: "Enter a commit message.",
  credential_in_remote_url: "Remote URLs must not contain credentials or tokens.",
  credential_resolution_mismatch: "The resolved credential strategy does not match the selected account.",
  discard_failed: "The selected changes could not be discarded.",
  endpoint_association_required: "Associate this remote with a matching Git service endpoint first.",
  endpoint_git_host_conflict: "Another endpoint already uses this Git host or port. Choose a unique Git authority.",
  endpoint_in_use_immutable: "Move or remove accounts and remote bindings before changing this endpoint's platform type or Git host.",
  endpoint_not_found: "The selected Git service endpoint could not be found.",
  gh_unavailable: "GitHub CLI (gh) is not available.",
  git_authentication_failed: "Authentication failed. Check the remote binding, SSH agent, or Git credential helper.",
  git_command_failed: "Git could not complete the operation. Check the repository state and Git configuration.",
  git_merge_conflict: "Git reported a merge conflict. Resolve it before continuing.",
  git_operation_in_progress: "Finish or abort the current Git operation before starting another one.",
  git_operation_not_in_progress: "That Git operation is no longer in progress.",
  git_network_error: "Check the network connection, proxy, and remote service status.",
  git_network_timeout: "The Git network operation timed out. Check the network, authentication, and remote service, then try again.",
  git_parse_error: "Git returned data that could not be parsed.",
  git_remote_not_found: "Check the remote URL, access permissions, and account binding.",
  git_unavailable: "The system Git executable could not be started.",
  github_adapter_auth_unsupported: "GitHub service information requires a binding that matches the current gh identity.",
  github_api_failed: "The GitHub request failed. Check gh authentication and repository access.",
  github_api_invalid_request: "The GitHub request could not be prepared.",
  github_api_invalid_response: "GitHub returned an unreadable response.",
  github_api_page_limit: "There are too many open pull requests to load safely. Narrow the list in GitHub and try again.",
  github_api_pipe_timeout: "GitHub CLI output did not close in time. Check the network and try again.",
  github_api_response_too_large: "GitHub CLI returned too much data to process safely.",
  github_api_timeout: "The GitHub request timed out. Check the network and gh login, then try again.",
  github_publish_auth_unsupported: "Publishing requires a matching signed-in gh CLI account.",
  github_publish_in_progress: "This repository is already being published to GitHub. Wait for the current operation to finish.",
  github_ported_endpoint_unsupported: "GitHub platform features do not yet support an endpoint with an explicit port. Use PAT, SSH, or the system Git credential helper for Git operations.",
  github_publish_reconciliation_failed: "A prior GitHub publish needs manual remote/account reconciliation.",
  github_publish_failed: "GitHub repository publishing failed. Check gh authentication, repository name, and network access.",
  io_error: "The requested local operation could not be completed.",
  invalid_account: "The account profile is invalid.",
  invalid_account_login: "The account login is required and cannot contain spaces.",
  invalid_askpass_configuration: "The internal Git askpass configuration is invalid.",
  invalid_clone_depth: "Clone depth must be greater than zero.",
  invalid_clone_destination: "The clone destination is invalid.",
  invalid_endpoint: "The Git service endpoint is invalid.",
  invalid_git_revision: "The Git revision is invalid or no longer exists.",
  invalid_git_reference: "The Git reference is invalid.",
  invalid_git_patch: "The selected diff hunk is invalid or too large.",
  invalid_history_limit: "History limit must be between 1 and 1000.",
  invalid_init_destination: "The new repository destination is invalid.",
  invalid_cherry_pick: "Select between 1 and 100 commits to cherry-pick.",
  invalid_keychain_reference: "The account's system keychain reference is invalid.",
  invalid_lfs_pattern: "The Git LFS tracking pattern is invalid.",
  invalid_worktree_config: "Choose a new branch or an existing revision for the worktree.",
  invalid_worktree_path: "The worktree path must be a valid absolute path.",
  invalid_pathspec: "Select at least one valid file path.",
  invalid_personal_access_token: "The personal access token is invalid.",
  invalid_pull_request_body: "The pull request description is invalid or too long.",
  invalid_pull_request_branch: "The pull request head and base branches must be valid.",
  invalid_pull_request_title: "Enter a valid pull request title without line breaks.",
  invalid_refspec: "The Git refspec is invalid.",
  invalid_repository_name: "The repository name is invalid.",
  invalid_remote_name: "The remote name is invalid.",
  invalid_remote_url: "The remote URL is invalid.",
  invalid_repository_root: "The repository discovery path is invalid.",
  invalid_squash_count: "Choose between 2 and 1000 recent commits while keeping an earlier base commit.",
  invalid_tag_message: "An annotated tag requires a message.",
  keychain_delete_failed: "The token could not be removed from the system keychain.",
  keychain_pat_unavailable: "The personal access token is unavailable in the system keychain.",
  keychain_unavailable: "The system keychain is unavailable.",
  keychain_write_failed: "The token could not be saved to the system keychain.",
  lfs_unavailable: "Git LFS is not available on this system.",
  not_a_git_repository: "The selected path is not a Git working tree.",
  no_commit_to_amend: "This branch has no commit to amend yet.",
  nothing_to_commit: "There are no staged changes to commit.",
  nothing_to_discard: "There are no changes left to discard.",
  nothing_to_stash: "There are no changes to stash.",
  path_required: "A path is required.",
  pat_not_supported: "This endpoint has not enabled app-managed personal access tokens.",
  pat_requires_secure_input: "Use the secure PAT input flow to add a personal access token.",
  profile_store_invalid: "The account profile store could not be parsed and was left unchanged.",
  profile_store_in_use: "Another BroKnowMyGitDesktop process is using the account profile. Close the other instance first.",
  profile_store_version_unsupported: "The account profile store was written by a newer app version and was safely backed up.",
  profile_store_serialize_failed: "The account profile store could not be saved.",
  profile_store_sync_failed: "The account profile store was updated, but its final disk sync could not be confirmed. Avoid powering off immediately and check it after recovery.",
  remote_not_found: "The specified remote could not be found in this repository.",
  remote_binding_reconciliation_failed: "The remote changed but its account binding could not be saved or restored. Rebind it before network operations.",
  remote_exists: "A remote with this name already exists.",
  repository_not_found: "The repository could not be found.",
  repository_already_has_remote: "This repository already has a remote and cannot use the publish flow.",
  repository_group_invalid: "A repository group must be at most 80 characters and cannot contain control characters.",
  repository_operation_in_progress: "This repository is running a network operation or updating a remote. Try again shortly.",
  repository_slug_unavailable: "The remote URL does not contain a recognizable repository path.",
  state_poisoned: "Application account state is unavailable. Restart the app.",
  stash_not_found: "The selected stash entry could not be found.",
  squash_recovery_failed: "Squashing commits failed and the original branch state could not be restored automatically. Check the repository history and staging area.",
  token_required: "Enter a personal access token.",
  unsafe_git_config: "This repository's local Git configuration contains unsafe credential or network settings. Remove them before a network operation.",
  unsafe_remote_url: "The HTTP remote URL contains credentials. Remove them before continuing.",
  untracked_diff_unavailable: "The untracked file could not be read safely for a diff on this platform.",
  unsupported_remote_transport: "Account authentication supports HTTPS and SSH Git URLs only.",
  unexpected_error: "An unexpected error occurred.",
  nothing_to_publish: "Create at least one local commit before publishing.",
  working_tree_not_clean: "Commit, stash, or discard current changes before this operation.",
  worktree_not_found: "The selected worktree could not be found.",
};

/** Translate stable backend error codes without ever exposing sensitive details. */
export function localizedErrorMessage(code: string, fallback: string): string {
  if (appLocale.value !== "en-US") return fallback;
  return Object.prototype.hasOwnProperty.call(englishErrors, code) ? englishErrors[code] : fallback;
}

export const naiveUiLocale = computed(() => (appLocale.value === "zh-CN" ? zhCN : enUS));
export const naiveUiDateLocale = computed(() => (appLocale.value === "zh-CN" ? dateZhCN : dateEnUS));

