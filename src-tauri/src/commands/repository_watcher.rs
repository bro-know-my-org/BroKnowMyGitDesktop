use super::support::repository_root;
use super::*;
use notify::{event::EventKind, Config, ErrorKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const REPOSITORY_CHANGED_EVENT: &str = "repository-changed";
const REPOSITORY_WATCH_ERROR_EVENT: &str = "repository-watch-error";
const MAX_GIT_FILE_BYTES: u64 = 4096;
const EVENT_EMIT_THROTTLE: Duration = Duration::from_millis(100);
const ERROR_EMIT_THROTTLE: Duration = Duration::from_secs(1);

struct ActiveRepositoryWatcher {
    repository_path: String,
    _watcher: RecommendedWatcher,
}

#[derive(Default)]
struct RepositoryWatcherState {
    generation: u64,
    active: Option<ActiveRepositoryWatcher>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryChangedPayload {
    repository_path: String,
    request_generation: u64,
    error_kind: Option<&'static str>,
    retryable: bool,
    history_objects_changed: bool,
    stashes_changed: bool,
}

static REPOSITORY_WATCHER_STATE: OnceLock<Mutex<RepositoryWatcherState>> = OnceLock::new();

fn repository_watcher_state() -> &'static Mutex<RepositoryWatcherState> {
    REPOSITORY_WATCHER_STATE.get_or_init(|| Mutex::new(RepositoryWatcherState::default()))
}

fn watcher_error_metadata(error: &notify::Error) -> (&'static str, bool) {
    match &error.kind {
        ErrorKind::MaxFilesWatch => ("watch_limit_reached", false),
        ErrorKind::InvalidConfig(_) => ("invalid_configuration", false),
        ErrorKind::PathNotFound => ("path_unavailable", true),
        ErrorKind::WatchNotFound => ("watch_invalidated", true),
        ErrorKind::Io(_) => ("io_error", true),
        ErrorKind::Generic(_) => ("backend_error", true),
    }
}

fn watcher_installation_error(error: notify::Error, message: &'static str) -> AppError {
    let (_, retryable) = watcher_error_metadata(&error);
    AppError::new(
        if retryable {
            "repository_watch_failed_transient"
        } else {
            "repository_watch_failed_permanent"
        },
        message,
    )
}

fn valid_git_directory(path: PathBuf) -> Option<PathBuf> {
    let has_head = path.join("HEAD").is_file();
    let has_object_database = path.join("objects").is_dir();
    let has_common_directory = path.join("commondir").is_file();
    (has_head && (has_object_database || has_common_directory)).then_some(path)
}

fn common_git_directory(git_directory: &Path) -> Option<PathBuf> {
    let pointer = git_directory.join("commondir");
    let metadata = fs::metadata(&pointer).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_GIT_FILE_BYTES {
        return None;
    }
    let contents = fs::read_to_string(pointer).ok()?;
    let path = PathBuf::from(contents.trim());
    if path.as_os_str().is_empty() {
        return None;
    }
    let path = if path.is_absolute() {
        path
    } else {
        git_directory.join(path)
    };
    fs::canonicalize(path)
        .ok()
        .filter(|path| path.is_dir())
        .and_then(valid_git_directory)
}

fn linked_git_directory(repository: &Path) -> Option<PathBuf> {
    let dot_git = repository.join(".git");
    if dot_git.is_dir() {
        return fs::canonicalize(dot_git).ok().and_then(valid_git_directory);
    }

    let metadata = fs::metadata(&dot_git).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_GIT_FILE_BYTES {
        return None;
    }
    let contents = fs::read_to_string(dot_git).ok()?;
    let path = contents.trim().strip_prefix("gitdir:")?.trim();
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        repository.join(path)
    };
    fs::canonicalize(path)
        .ok()
        .filter(|path| path.is_dir())
        .and_then(valid_git_directory)
}

#[tauri::command]
pub fn watch_repository_changes(
    repository_path: String,
    request_generation: u64,
    app: AppHandle,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let repository_path = repository.to_string_lossy().into_owned();
    let previous_watcher = {
        let mut state = repository_watcher_state()
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "仓库文件监听状态不可用，请重启应用。"))?;
        if request_generation < state.generation {
            return Ok(());
        }
        state.generation = request_generation;
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.repository_path != repository_path)
        {
            state.active.take()
        } else {
            None
        }
    };
    drop(previous_watcher);
    let git_directory = linked_git_directory(&repository);
    let common_directory = git_directory.as_deref().and_then(common_git_directory);
    let mut event_git_directories = Vec::new();
    if let Some(path) = git_directory.as_ref() {
        event_git_directories.push(path.clone());
    }
    if let Some(path) = common_directory.as_ref() {
        if !event_git_directories.contains(path) {
            event_git_directories.push(path.clone());
        }
    }
    let event_repository_path = repository_path.clone();
    let event_app = app.clone();
    let last_event_emitted_at = Arc::new(Mutex::new(Instant::now() - EVENT_EMIT_THROTTLE));
    let last_error_emitted_at = Arc::new(Mutex::new(Instant::now() - ERROR_EMIT_THROTTLE));
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<notify::Event>| {
            let event = match result {
                Ok(event) => event,
                Err(error) => {
                    let Ok(mut last_emitted_at) = last_error_emitted_at.lock() else {
                        return;
                    };
                    if last_emitted_at.elapsed() < ERROR_EMIT_THROTTLE {
                        return;
                    }
                    *last_emitted_at = Instant::now();
                    let (error_kind, retryable) = watcher_error_metadata(&error);
                    let _ = event_app.emit(
                        REPOSITORY_WATCH_ERROR_EVENT,
                        RepositoryChangedPayload {
                            repository_path: event_repository_path.clone(),
                            request_generation,
                            error_kind: Some(error_kind),
                            retryable,
                            history_objects_changed: false,
                            stashes_changed: false,
                        },
                    );
                    return;
                }
            };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let git_relative_paths = event.paths.iter().filter_map(|path| {
                event_git_directories
                    .iter()
                    .find_map(|directory| path.strip_prefix(directory).ok())
            });
            let mut history_objects_changed = false;
            let mut stashes_changed = false;
            for path in git_relative_paths {
                history_objects_changed |= path == Path::new("shallow")
                    || path == Path::new("info/grafts")
                    || path == Path::new("objects/info/alternates")
                    || path == Path::new("packed-refs")
                    || path.starts_with(Path::new("refs/replace"));
                stashes_changed |= path == Path::new("packed-refs")
                    || path == Path::new("refs/stash")
                    || path == Path::new("logs/refs/stash");
            }
            let Ok(mut last_emitted_at) = last_event_emitted_at.lock() else {
                return;
            };
            if !history_objects_changed
                && !stashes_changed
                && last_emitted_at.elapsed() < EVENT_EMIT_THROTTLE
            {
                return;
            }
            *last_emitted_at = Instant::now();
            let _ = event_app.emit(
                REPOSITORY_CHANGED_EVENT,
                RepositoryChangedPayload {
                    repository_path: event_repository_path.clone(),
                    request_generation,
                    error_kind: None,
                    retryable: false,
                    history_objects_changed,
                    stashes_changed,
                },
            );
        },
        Config::default(),
    )
    .map_err(|error| watcher_installation_error(error, "无法监听当前仓库的文件变动。"))?;

    watcher
        .watch(&repository, RecursiveMode::Recursive)
        .map_err(|error| watcher_installation_error(error, "无法监听当前仓库的文件变动。"))?;

    if let Some(git_directory) = git_directory {
        if !git_directory.starts_with(&repository) {
            watcher
                .watch(&git_directory, RecursiveMode::Recursive)
                .map_err(|error| {
                    watcher_installation_error(error, "无法监听当前仓库的 Git 状态变动。")
                })?;
        }
        if let Some(common_directory) = common_directory {
            if common_directory != git_directory && !common_directory.starts_with(&repository) {
                watcher
                    .watch(&common_directory, RecursiveMode::Recursive)
                    .map_err(|error| {
                        watcher_installation_error(error, "无法监听当前仓库的共享 Git 状态变动。")
                    })?;
            }
        }
    }

    let mut state = repository_watcher_state()
        .lock()
        .map_err(|_| AppError::new("state_poisoned", "仓库文件监听状态不可用，请重启应用。"))?;
    let installed = state.generation == request_generation;
    if installed {
        state.active = Some(ActiveRepositoryWatcher {
            repository_path: repository_path.clone(),
            _watcher: watcher,
        });
    }
    drop(state);
    if installed {
        let _ = app.emit(
            REPOSITORY_CHANGED_EVENT,
            RepositoryChangedPayload {
                repository_path,
                request_generation,
                error_kind: None,
                retryable: false,
                history_objects_changed: false,
                stashes_changed: false,
            },
        );
    }
    Ok(())
}

#[tauri::command]
pub fn stop_repository_watcher(request_generation: u64) -> Result<(), AppError> {
    let mut state = repository_watcher_state()
        .lock()
        .map_err(|_| AppError::new("state_poisoned", "仓库文件监听状态不可用，请重启应用。"))?;
    if request_generation >= state.generation {
        state.generation = request_generation;
        state.active = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;

    #[test]
    fn resolves_git_directory_file_relative_to_worktree() {
        let root = std::env::temp_dir().join(format!("bkm-watcher-{}", Uuid::new_v4()));
        let worktree = root.join("worktree");
        let git_directory = root.join("metadata");
        fs::create_dir_all(&worktree).expect("worktree directory is created");
        fs::create_dir_all(&git_directory).expect("git directory is created");
        fs::write(git_directory.join("HEAD"), "ref: refs/heads/main\n")
            .expect("git HEAD is written");
        fs::write(git_directory.join("commondir"), "..\n")
            .expect("git common directory pointer is written");
        let mut dot_git = fs::File::create(worktree.join(".git")).expect("git file is created");
        writeln!(dot_git, "gitdir: ../metadata").expect("git file is written");

        assert_eq!(
            linked_git_directory(&worktree),
            Some(fs::canonicalize(&git_directory).expect("git directory is canonicalized"))
        );

        fs::remove_dir_all(root).expect("temporary watcher fixture is removed");
    }

    #[test]
    fn rejects_git_file_target_without_git_metadata_layout() {
        let root = std::env::temp_dir().join(format!("bkm-watcher-{}", Uuid::new_v4()));
        let worktree = root.join("worktree");
        let unrelated = root.join("unrelated");
        fs::create_dir_all(&worktree).expect("worktree directory is created");
        fs::create_dir_all(&unrelated).expect("unrelated directory is created");
        fs::write(worktree.join(".git"), "gitdir: ../unrelated\n").expect("git file is written");

        assert_eq!(linked_git_directory(&worktree), None);

        fs::remove_dir_all(root).expect("temporary watcher fixture is removed");
    }

    #[test]
    fn resolves_linked_worktree_common_git_directory() {
        let root = std::env::temp_dir().join(format!("bkm-watcher-{}", Uuid::new_v4()));
        let git_directory = root.join("metadata/worktrees/feature");
        let common_directory = root.join("metadata");
        fs::create_dir_all(git_directory.as_path()).expect("worktree git directory is created");
        fs::create_dir_all(common_directory.join("objects"))
            .expect("common object database is created");
        fs::write(common_directory.join("HEAD"), "ref: refs/heads/main\n")
            .expect("common HEAD is written");
        fs::write(git_directory.join("HEAD"), "ref: refs/heads/feature\n")
            .expect("worktree HEAD is written");
        fs::write(git_directory.join("commondir"), "../..\n")
            .expect("common directory pointer is written");

        assert_eq!(
            common_git_directory(&git_directory),
            Some(fs::canonicalize(&common_directory).expect("common directory is canonicalized"))
        );

        fs::remove_dir_all(root).expect("temporary watcher fixture is removed");
    }
}
