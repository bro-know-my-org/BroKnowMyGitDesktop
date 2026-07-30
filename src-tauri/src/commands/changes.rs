use super::support::*;
use super::*;

#[tauri::command]
pub async fn get_file_diff(
    repository_path: String,
    path: String,
    staged: bool,
) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        let command_started = std::time::Instant::now();
        let repository = repository_root(&repository_path)?;
        #[cfg(debug_assertions)]
        let repository_resolved = std::time::Instant::now();
        // Git permits filenames containing only whitespace. Only an actual empty
        // IPC value requests a repository-wide diff.
        let paths = (!path.is_empty()).then_some(path);
        let result = GitCore::new().diff(
            &repository,
            &DiffOptions {
                staged,
                paths: paths.into_iter().collect(),
                context_lines: Some(3),
            },
        );
        #[cfg(debug_assertions)]
        eprintln!(
            "[BKM performance] get_file_diff: repository_root={}ms git_diff={}ms total={}ms",
            repository_resolved
                .duration_since(command_started)
                .as_millis(),
            repository_resolved.elapsed().as_millis(),
            command_started.elapsed().as_millis(),
        );
        result
    })
    .await
    .map_err(|_| AppError::new("command_failed", "文件差异后台任务未能完成。"))?
}

#[tauri::command]
pub fn get_file_blame(repository_path: String, path: String) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().blame(&repository, &path)
}

#[tauri::command]
pub fn list_reflog(repository_path: String, limit: Option<usize>) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().reflog(&repository, limit.unwrap_or(100))
}

#[tauri::command]
pub fn preview_clean_untracked(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<CleanUntrackedPreview, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    // Generate the display text from the exact leaf paths captured in the
    // confirmation token. A broad `git clean -nd` can collapse an untracked
    // directory into one display line while status lists its children, which
    // would make the confirmation authorize paths the user did not see.
    let paths = git
        .status(&repository)?
        .entries
        .into_iter()
        .filter(|entry| entry.index_status.is_empty() && entry.worktree_status == "?")
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    // The frontend intentionally treats an empty preview as a no-op. Do not
    // ask the fingerprint validator to authorize an empty path list: that
    // input is invalid for destructive operations, but perfectly valid for a
    // read-only preview.
    if paths.is_empty() {
        return Ok(CleanUntrackedPreview {
            preview: String::new(),
            confirmation: String::new(),
        });
    }
    // Capture identities before generating the preview. A separate process
    // cannot replace a reviewed path during the preview and have that new
    // object become the one authorized by this confirmation token.
    let fingerprints = git.capture_clean_untracked_path_fingerprints(&repository, &paths)?;
    let preview = git.preview_clean_untracked_paths(&repository, &paths)?;
    // The preview command itself can take long enough for a filesystem change
    // to occur. Refuse to issue a token unless it still represents the exact
    // filesystem objects captured before rendering the display text.
    git.validate_clean_untracked_path_fingerprints(&repository, &fingerprints)?;
    let confirmation =
        state.issue_clean_untracked_confirmation(&repository.to_string_lossy(), fingerprints)?;
    Ok(CleanUntrackedPreview {
        preview,
        confirmation,
    })
}

#[tauri::command]
pub fn clean_untracked(
    repository_path: String,
    confirmation: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let fingerprints =
        state.take_clean_untracked_confirmation(&repository.to_string_lossy(), &confirmation)?;
    let git = GitCore::new();
    git.clean_untracked_paths_with_fingerprints(&repository, &fingerprints)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn get_external_editor(
    state: State<'_, AppState>,
) -> Result<Option<ExternalEditorConfig>, AppError> {
    Ok(state.snapshot()?.external_editor)
}

/// Saves an app-local external editor command. This does not read or modify
/// Git's editor or mergetool configuration, which may be scoped to an
/// untrusted repository.
#[tauri::command]
pub fn save_external_editor(
    editor: Option<ExternalEditorConfig>,
    state: State<'_, AppState>,
) -> Result<Option<ExternalEditorConfig>, AppError> {
    let editor = editor.map(normalize_external_editor_config).transpose()?;
    state.mutate(|store| {
        store.external_editor = editor.clone();
        Ok(editor)
    })
}

/// Starts the configured external editor with every currently unresolved
/// conflict file. The editor is not awaited; the user resolves files outside
/// the app, returns here, refreshes, stages, and then continues the operation.
#[tauri::command]
pub fn open_conflicted_files_in_external_editor(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let editor = state.snapshot()?.external_editor.ok_or_else(|| {
        AppError::new(
            "external_editor_not_configured",
            "请先配置用于解决冲突的外部编辑器。",
        )
    })?;
    GitCore::new().open_conflicted_files_in_external_editor(&repository, &editor)
}
