use super::support::*;
use super::*;

#[tauri::command]
pub fn get_file_diff(
    repository_path: String,
    path: String,
    staged: bool,
) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    let paths = (!path.trim().is_empty()).then_some(path);
    GitCore::new().diff(
        &repository,
        &DiffOptions {
            staged,
            paths: paths.into_iter().collect(),
            context_lines: Some(3),
        },
    )
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
    let git = GitCore::new();
    // Capture the precise file set before producing the display text. Files
    // created afterward can appear in the preview but are never included in
    // the one-shot confirmation token.
    let paths = git
        .status(&repository)?
        .entries
        .into_iter()
        .filter(|entry| entry.index_status.is_empty() && entry.worktree_status == "?")
        .map(|entry| entry.path)
        .collect();
    let preview = git.preview_clean_untracked(&repository)?;
    let confirmation =
        state.issue_clean_untracked_confirmation(&repository.to_string_lossy(), paths)?;
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
    let paths =
        state.take_clean_untracked_confirmation(&repository.to_string_lossy(), &confirmation)?;
    GitCore::new().clean_untracked_paths(&repository, &paths)?;
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
