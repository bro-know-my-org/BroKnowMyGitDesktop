use super::support::repository_root;
use super::*;
use ignore::WalkBuilder;
use notify::{
    event::{EventKind, ModifyKind},
    Config, ErrorKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher,
};
use serde::Serialize;
use std::collections::HashSet;
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
    _watcher: Box<dyn Watcher + Send>,
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
    watch_plan_changed: bool,
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

fn safe_relative_worktree_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn nearest_existing_watch_target(path: &Path) -> Option<PathBuf> {
    let mut candidate = path;
    loop {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        candidate = candidate.parent()?;
    }
}

fn configured_exclude_event_changes_plan(event_path: &Path, exclude_file: &Path) -> bool {
    event_path == exclude_file || exclude_file.starts_with(event_path)
}

fn worktree_watch_directories(
    repository: &Path,
    git_metadata_directory: Option<&Path>,
    configured_exclude_file: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let mut builder = WalkBuilder::new(repository);
    builder
        // Hidden source directories are meaningful to Git; only ignore rules,
        // not a leading dot, may remove them from the watch plan.
        .hidden(false)
        // `.ignore` is a search-tool convention rather than a Git rule.
        .ignore(false)
        // Git never inherits a parent repository's .gitignore rules across a
        // repository boundary.
        .parents(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(true)
        .follow_links(false)
        .current_dir(repository)
        // We need directories only. Filtering files here avoids retaining a
        // complete file list while the walker still evaluates Git ignore rules
        // before deciding whether to descend into each directory.
        .filter_entry(|entry| {
            entry.depth() == 0
                || (entry
                    .file_type()
                    .is_some_and(|file_type| file_type.is_dir())
                    && entry.file_name() != std::ffi::OsStr::new(".git"))
        });
    if let Some(git_metadata_directory) = git_metadata_directory {
        let exclude_file = git_metadata_directory.join("info/exclude");
        if exclude_file.is_file() {
            // `ignore` discovers the ordinary `.git/info/exclude` itself. Add
            // the resolved metadata path explicitly for linked worktrees,
            // whose `.git` entry is a pointer file outside the worktree.
            if builder.add_ignore(exclude_file).is_some() {
                return Err(AppError::new(
                    "repository_watch_failed_transient",
                    "无法读取当前工作树的 Git 忽略规则。",
                ));
            }
        }
    }
    if let Some(exclude_file) = configured_exclude_file {
        if exclude_file.is_file() && builder.add_ignore(exclude_file).is_some() {
            return Err(AppError::new(
                "repository_watch_failed_transient",
                "无法读取 Git 全局忽略规则。",
            ));
        }
    }

    let mut directories = HashSet::new();
    for entry in builder.build() {
        let entry = entry.map_err(|_| {
            AppError::new(
                "repository_watch_failed_transient",
                "无法读取当前仓库的可监听目录。",
            )
        })?;
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        {
            directories.insert(entry.into_path());
        }
    }

    // A tracked file can become ignored after it was committed. Add its
    // existing parent directories explicitly so ignore-aware traversal never
    // makes a tracked path invisible.
    for tracked_path in GitCore::new().tracked_worktree_paths(repository)? {
        let mut parent = tracked_path.parent();
        while let Some(relative_directory) = parent {
            if relative_directory.as_os_str().is_empty() {
                break;
            }
            if safe_relative_worktree_path(relative_directory) {
                let directory = repository.join(relative_directory);
                if directory.is_dir() {
                    directories.insert(directory);
                }
            }
            parent = relative_directory.parent();
        }
    }

    directories.insert(repository.to_path_buf());
    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then(left.cmp(right))
    });
    Ok(directories)
}

fn install_worktree_watch_targets<W: Watcher>(
    watcher: &mut W,
    directories: impl IntoIterator<Item = PathBuf>,
) -> notify::Result<()> {
    for directory in directories {
        watcher.watch(&directory, RecursiveMode::NonRecursive)?;
    }
    Ok(())
}

fn install_auxiliary_watch_targets<W: Watcher>(
    watcher: &mut W,
    configured_exclude_watch_target: Option<&Path>,
    git_directory: Option<&Path>,
    common_directory: Option<&Path>,
) -> notify::Result<()> {
    if let Some(watch_target) = configured_exclude_watch_target {
        watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    }

    if let Some(git_directory) = git_directory {
        watcher.watch(git_directory, RecursiveMode::Recursive)?;
        if let Some(common_directory) = common_directory {
            if common_directory != git_directory {
                watcher.watch(common_directory, RecursiveMode::Recursive)?;
            }
        }
    }
    Ok(())
}

fn install_repository_watcher(
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
    let configured_exclude = GitCore::new().worktree_excludes_file(&repository)?;
    let configured_exclude_file = configured_exclude
        .as_ref()
        .map(|exclude| exclude.path.clone());
    let configured_exclude_rules_file = configured_exclude
        .as_ref()
        .filter(|exclude| exclude.load_rules)
        .map(|exclude| exclude.path.clone());
    let configured_exclude_watch_target = configured_exclude
        .as_ref()
        .map(|exclude| {
            let target = if exclude.watch_externally {
                exclude.path.as_path()
            } else {
                exclude.path.parent().unwrap_or(&exclude.path)
            };
            nearest_existing_watch_target(target).ok_or_else(|| {
                AppError::new(
                    "repository_watch_failed_transient",
                    "无法定位 Git 全局忽略文件的可监听父目录。",
                )
            })
        })
        .transpose()?;
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
    let event_configured_exclude_file = configured_exclude_file.clone();
    let last_event_emitted_at = Arc::new(Mutex::new(Instant::now() - EVENT_EMIT_THROTTLE));
    let last_error_emitted_at = Arc::new(Mutex::new(Instant::now() - ERROR_EMIT_THROTTLE));
    let event_handler = Arc::new(move |result: notify::Result<notify::Event>| {
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
                        watch_plan_changed: false,
                    },
                );
                return;
            }
        };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        let mut history_objects_changed = false;
        let mut stashes_changed = false;
        let mut watch_plan_changed = false;
        let worktree_directory_event = matches!(
            &event.kind,
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
        );
        for path in &event.paths {
            watch_plan_changed |=
                event_configured_exclude_file
                    .as_deref()
                    .is_some_and(|exclude_file| {
                        configured_exclude_event_changes_plan(path, exclude_file)
                    });
            if let Some(path) = event_git_directories
                .iter()
                .find_map(|directory| path.strip_prefix(directory).ok())
            {
                history_objects_changed |= path == Path::new("shallow")
                    || path == Path::new("info/grafts")
                    || path == Path::new("objects/info/alternates")
                    || path == Path::new("packed-refs")
                    || path.starts_with(Path::new("refs/replace"));
                stashes_changed |= path == Path::new("packed-refs")
                    || path == Path::new("refs/stash")
                    || path == Path::new("logs/refs/stash");
                watch_plan_changed |= path == Path::new("info/exclude")
                    || path == Path::new("config")
                    || path == Path::new("config.worktree");
            } else {
                watch_plan_changed |= path.file_name() == Some(std::ffi::OsStr::new(".gitignore"))
                    || (worktree_directory_event && path.is_dir());
            }
        }
        let Ok(mut last_emitted_at) = last_event_emitted_at.lock() else {
            return;
        };
        if !history_objects_changed
            && !stashes_changed
            && !watch_plan_changed
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
                watch_plan_changed,
            },
        );
    });
    let native_event_handler = Arc::clone(&event_handler);
    let mut native_watcher = RecommendedWatcher::new(
        move |result| native_event_handler(result),
        Config::default(),
    )
    .map_err(|error| watcher_installation_error(error, "无法监听当前仓库的文件变动。"))?;

    let git_metadata_directory = common_directory.as_deref().or(git_directory.as_deref());
    let mut watched_directories = HashSet::new();
    let initial_directories = worktree_watch_directories(
        &repository,
        git_metadata_directory,
        configured_exclude_rules_file.as_deref(),
    )?;
    watched_directories.extend(initial_directories.iter().cloned());
    let mut native_error =
        install_worktree_watch_targets(&mut native_watcher, initial_directories).err();
    // The repository root is installed first because the plan is sorted by
    // depth. Re-scan after registration to close the only remaining gap: a
    // directory created after the initial walk but before the root watch was
    // active. Later directory creation is reported by an installed parent.
    if native_error.is_none() {
        for directory in worktree_watch_directories(
            &repository,
            git_metadata_directory,
            configured_exclude_rules_file.as_deref(),
        )? {
            if watched_directories.insert(directory.clone()) {
                if let Err(error) = native_watcher.watch(&directory, RecursiveMode::NonRecursive) {
                    native_error = Some(error);
                    break;
                }
            }
        }
    }
    if native_error.is_none() {
        if let Err(error) = install_auxiliary_watch_targets(
            &mut native_watcher,
            configured_exclude_watch_target.as_deref(),
            git_directory.as_deref(),
            common_directory.as_deref(),
        ) {
            native_error = Some(error);
        }
    }

    let watcher: Box<dyn Watcher + Send> = if let Some(error) = native_error {
        if !matches!(error.kind, ErrorKind::MaxFilesWatch) {
            return Err(watcher_installation_error(
                error,
                "无法监听当前仓库的文件变动。",
            ));
        }
        drop(native_watcher);
        let polling_event_handler = Arc::clone(&event_handler);
        let mut polling_watcher = PollWatcher::new(
            move |result| polling_event_handler(result),
            Config::default().with_poll_interval(Duration::from_secs(5)),
        )
        .map_err(|error| watcher_installation_error(error, "无法启动仓库轮询监听。"))?;
        let polling_directories = worktree_watch_directories(
            &repository,
            git_metadata_directory,
            configured_exclude_rules_file.as_deref(),
        )?;
        let mut watched_directories = polling_directories.iter().cloned().collect::<HashSet<_>>();
        install_worktree_watch_targets(&mut polling_watcher, polling_directories)
            .map_err(|error| watcher_installation_error(error, "无法启动仓库轮询监听。"))?;
        for directory in worktree_watch_directories(
            &repository,
            git_metadata_directory,
            configured_exclude_rules_file.as_deref(),
        )? {
            if watched_directories.insert(directory.clone()) {
                polling_watcher
                    .watch(&directory, RecursiveMode::NonRecursive)
                    .map_err(|error| watcher_installation_error(error, "无法启动仓库轮询监听。"))?;
            }
        }
        install_auxiliary_watch_targets(
            &mut polling_watcher,
            configured_exclude_watch_target.as_deref(),
            git_directory.as_deref(),
            common_directory.as_deref(),
        )
        .map_err(|error| watcher_installation_error(error, "无法启动仓库轮询监听。"))?;
        Box::new(polling_watcher)
    } else {
        Box::new(native_watcher)
    };

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
        // This synthetic event is also the reconciliation barrier for plan
        // changes: the frontend refreshes once more after a replacement
        // watcher is fully installed, so writes made during the refresh-to-
        // reinstall window cannot leave the snapshot stale.
        let _ = app.emit(
            REPOSITORY_CHANGED_EVENT,
            RepositoryChangedPayload {
                repository_path,
                request_generation,
                error_kind: None,
                retryable: false,
                history_objects_changed: false,
                stashes_changed: false,
                watch_plan_changed: false,
            },
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn watch_repository_changes(
    repository_path: String,
    request_generation: u64,
    app: AppHandle,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        install_repository_watcher(repository_path, request_generation, app)
    })
    .await
    .map_err(|_| {
        AppError::new(
            "repository_watch_failed_transient",
            "仓库监听后台任务未能完成。",
        )
    })?
}

#[tauri::command]
pub async fn stop_repository_watcher(request_generation: u64) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut state = repository_watcher_state()
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "仓库文件监听状态不可用，请重启应用。"))?;
        if request_generation >= state.generation {
            state.generation = request_generation;
            state.active = None;
        }
        Ok(())
    })
    .await
    .map_err(|_| AppError::new("command_failed", "停止仓库监听的后台任务未能完成。"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;
    use uuid::Uuid;

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn create(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("temporary directory is created");
            Self(path)
        }
    }

    impl std::ops::Deref for TemporaryDirectory {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AsRef<Path> for TemporaryDirectory {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_configured_exclude_uses_existing_parent_and_detects_created_ancestors() {
        let root = TemporaryDirectory::create("bkm-watcher-exclude");
        let exclude_file = root.join("future/config/ignore");

        assert_eq!(
            nearest_existing_watch_target(&exclude_file),
            Some(root.to_path_buf())
        );
        assert!(configured_exclude_event_changes_plan(
            &root.join("future"),
            &exclude_file
        ));
        assert!(configured_exclude_event_changes_plan(
            &exclude_file,
            &exclude_file
        ));
        assert!(!configured_exclude_event_changes_plan(
            &root.join("unrelated"),
            &exclude_file
        ));
    }

    #[test]
    fn worktree_watch_plan_skips_ignored_trees_and_keeps_tracked_parents() {
        let root = TemporaryDirectory::create("bkm-watcher-plan");
        fs::create_dir_all(root.join("node_modules/package"))
            .expect("ignored dependency tree exists");
        fs::create_dir_all(root.join("target/debug")).expect("ignored build tree exists");
        fs::create_dir_all(root.join("src/nested")).expect("source tree exists");
        fs::create_dir_all(root.join("src/generated/deep")).expect("nested ignored tree exists");
        fs::create_dir_all(root.join("excluded-by-info/deep")).expect("info-excluded tree exists");
        fs::create_dir_all(root.join("excluded-by-config/deep"))
            .expect("configured-excluded tree exists");
        fs::create_dir_all(root.join(".hidden/source")).expect("hidden source tree exists");
        fs::create_dir_all(root.join("tracked-parent/nested")).expect("tracked parent tree exists");
        fs::write(root.join("tracked-parent/nested/kept.txt"), "tracked\n")
            .expect("tracked fixture exists");
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .expect("git init starts")
            .success());
        assert!(Command::new("git")
            .args(["add", "tracked-parent/nested/kept.txt"])
            .current_dir(&root)
            .status()
            .expect("git add starts")
            .success());
        let configured_excludes = root.join("configured-excludes");
        fs::write(&configured_excludes, "excluded-by-config/\n")
            .expect("configured excludes exist");
        assert!(Command::new("git")
            .args([
                "config",
                "core.excludesFile",
                configured_excludes.to_str().expect("test path is UTF-8"),
            ])
            .current_dir(&root)
            .status()
            .expect("git config starts")
            .success());
        fs::write(
            root.join(".gitignore"),
            "node_modules/\ntarget/\ntracked-parent/\n",
        )
        .expect("ignore rules exist");
        fs::write(root.join("src/.gitignore"), "generated/\n").expect("nested ignore rules exist");
        fs::write(root.join(".git/info/exclude"), "excluded-by-info/\n")
            .expect("git exclude rules exist");

        let directories =
            worktree_watch_directories(&root, Some(&root.join(".git")), Some(&configured_excludes))
                .expect("watch plan is built");
        assert!(directories.contains(&root.to_path_buf()));
        assert!(directories.contains(&root.join("src")));
        assert!(directories.contains(&root.join("src/nested")));
        assert!(!directories.contains(&root.join("src/generated")));
        assert!(!directories.contains(&root.join("src/generated/deep")));
        assert!(!directories.contains(&root.join("excluded-by-info")));
        assert!(!directories.contains(&root.join("excluded-by-config")));
        assert!(directories.contains(&root.join(".hidden")));
        assert!(directories.contains(&root.join("tracked-parent")));
        assert!(directories.contains(&root.join("tracked-parent/nested")));
        assert!(!directories.contains(&root.join("node_modules")));
        assert!(!directories.contains(&root.join("node_modules/package")));
        assert!(!directories.contains(&root.join("target")));
        assert!(!directories
            .iter()
            .any(|path| path.starts_with(root.join(".git"))));
    }

    #[test]
    fn resolves_git_directory_file_relative_to_worktree() {
        let root = TemporaryDirectory::create("bkm-watcher");
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
    }

    #[test]
    fn rejects_git_file_target_without_git_metadata_layout() {
        let root = TemporaryDirectory::create("bkm-watcher");
        let worktree = root.join("worktree");
        let unrelated = root.join("unrelated");
        fs::create_dir_all(&worktree).expect("worktree directory is created");
        fs::create_dir_all(&unrelated).expect("unrelated directory is created");
        fs::write(worktree.join(".git"), "gitdir: ../unrelated\n").expect("git file is written");

        assert_eq!(linked_git_directory(&worktree), None);
    }

    #[test]
    fn resolves_linked_worktree_common_git_directory() {
        let root = TemporaryDirectory::create("bkm-watcher");
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
    }
}
