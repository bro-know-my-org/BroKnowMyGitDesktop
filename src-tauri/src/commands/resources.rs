use super::support::repository_root;
use super::*;
use crate::git_core::resources::{ResourceComparison, ResourceSide, PREVIEW_LIMIT};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri_plugin_dialog::DialogExt;

struct ResourceSession {
    created: Instant,
    _directory: tempfile::TempDir,
    repository: PathBuf,
    sides: [Option<ResourceSide>; 2],
    files: [Option<PathBuf>; 2],
}

pub struct ResourcePreviews {
    directory: tempfile::TempDir,
    sessions: Mutex<HashMap<String, Arc<ResourceSession>>>,
    copying: Arc<tokio::sync::Semaphore>,
    exporting: Arc<tokio::sync::Semaphore>,
    epoch: AtomicU64,
}

impl ResourcePreviews {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            directory: tempfile::Builder::new().prefix("bkm-previews-").tempdir()?,
            sessions: Mutex::new(HashMap::new()),
            copying: Arc::new(tokio::sync::Semaphore::new(2)),
            exporting: Arc::new(tokio::sync::Semaphore::new(1)),
            epoch: AtomicU64::new(0),
        })
    }
    pub fn clear(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            self.epoch.fetch_add(1, Ordering::SeqCst);
            sessions.clear();
        }
    }
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn evict_oldest(sessions: &mut HashMap<String, Arc<ResourceSession>>) {
    if let Some(id) = sessions
        .iter()
        .min_by_key(|(_, session)| session.created)
        .map(|(id, _)| id.clone())
    {
        sessions.remove(&id);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePreviewSide {
    path: String,
    size: u64,
    file: Option<String>,
    notice: Option<String>,
    exportable: bool,
}

#[derive(Serialize)]
pub struct ResourcePreview {
    id: String,
    sides: [Option<ResourcePreviewSide>; 2],
}

#[tauri::command]
pub async fn get_resource_preview(
    repository_path: String,
    comparison: ResourceComparison,
    app: tauri::AppHandle,
) -> Result<ResourcePreview, AppError> {
    use tauri::Manager;
    let epoch = app.state::<ResourcePreviews>().epoch.load(Ordering::SeqCst);
    let permit = app
        .state::<ResourcePreviews>()
        .copying
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::new("resource_unavailable", "资源预览不可用。"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let state = app.state::<ResourcePreviews>();
        {
            let _sessions = state
                .sessions
                .lock()
                .map_err(|_| AppError::new("resource_unavailable", "资源预览不可用。"))?;
            if state.epoch.load(Ordering::SeqCst) != epoch {
                return Err(AppError::new("resource_unavailable", "资源预览已过期。"));
            }
        }
        let repository = repository_root(&repository_path)?;
        let git = GitCore::new();
        let sides = git.resource_sides(&repository, &comparison)?;
        let directory = tempfile::Builder::new()
            .prefix("resource-")
            .tempdir_in(state.path())
            .map_err(|error| AppError::io("创建资源预览", error))?;
        let mut files: [Option<PathBuf>; 2] = [None, None];
        let mut previews: [Option<ResourcePreviewSide>; 2] = [None, None];
        for (index, side) in sides.iter().enumerate() {
            let Some(side) = side else {
                continue;
            };
            let mut notice = side.notice.map(str::to_owned);
            if side.source.is_some() {
                if side.size > PREVIEW_LIMIT {
                    notice = Some("large".into());
                } else {
                    let extension = Path::new(&side.path)
                        .extension()
                        .and_then(|value| value.to_str())
                        .filter(|value| {
                            value.len() <= 16 && value.bytes().all(|b| b.is_ascii_alphanumeric())
                        })
                        .unwrap_or("bin");
                    let path = directory.path().join(format!("{index}.{extension}"));
                    let mut file = std::fs::File::create(&path)
                        .map_err(|error| AppError::io("创建资源预览", error))?;
                    git.copy_resource(&repository, side, &mut file)?;
                    // LFS pointers are useful metadata, never pretend they are decoded media.
                    if side.size < 1024 {
                        let bytes = std::fs::read(&path)
                            .map_err(|error| AppError::io("读取资源预览", error))?;
                        if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
                            notice = Some("lfs".into());
                        }
                    }
                    if !app.asset_protocol_scope().is_allowed(&path) {
                        return Err(AppError::new(
                            "resource_unavailable",
                            "资源预览路径未获授权。",
                        ));
                    }
                    files[index] = Some(path);
                }
            }
            previews[index] = Some(ResourcePreviewSide {
                path: side.path.clone(),
                size: side.size,
                file: files[index]
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                notice,
                exportable: side.source.is_some(),
            });
        }
        let id = Uuid::new_v4().to_string();
        let session = Arc::new(ResourceSession {
            created: Instant::now(),
            _directory: directory,
            repository,
            sides,
            files,
        });
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| AppError::new("resource_unavailable", "资源预览不可用。"))?;
        if state.epoch.load(Ordering::SeqCst) != epoch {
            return Err(AppError::new("resource_unavailable", "资源预览已过期。"));
        }
        // Evict only after successful creation, atomically with insertion.
        while sessions.len() >= 6 {
            evict_oldest(&mut sessions);
        }
        sessions.insert(id.clone(), session);
        Ok(ResourcePreview {
            id,
            sides: previews,
        })
    })
    .await
    .map_err(|_| AppError::new("command_failed", "资源预览后台任务未能完成。"))?
}

#[tauri::command]
pub fn release_resource_preview(id: String, state: State<'_, ResourcePreviews>) {
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.remove(&id);
    }
}

#[tauri::command]
pub async fn export_resource_preview(
    id: String,
    side: usize,
    app: tauri::AppHandle,
) -> Result<bool, AppError> {
    use tauri::Manager;
    // Keep one pending save dialog/snapshot lease across navigation without
    // consuming the permits used to load new previews.
    let export_permit = app
        .state::<ResourcePreviews>()
        .exporting
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::new("resource_unavailable", "请先完成当前资源导出。"))?;
    let session = app
        .state::<ResourcePreviews>()
        .sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(&id).cloned())
        .ok_or_else(|| AppError::new("resource_unavailable", "资源预览已过期。"))?;
    let source = session
        .sides
        .get(side)
        .and_then(Option::as_ref)
        .filter(|source| source.source.is_some())
        .ok_or_else(|| AppError::new("resource_unavailable", "此版本不存在。"))?;
    let name = Path::new(&source.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let dialog_app = app.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_file_name(name)
            .blocking_save_file()
    })
    .await
    .map_err(|_| AppError::new("command_failed", "资源导出对话框未能完成。"))?;
    let Some(destination) = destination else {
        return Ok(false);
    };
    let destination = destination
        .into_path()
        .map_err(|_| AppError::new("resource_unavailable", "导出路径不可用。"))?;
    // An explicitly requested export owns its snapshot even if the user navigates
    // away. Only disk copying, never time spent in a dialog, consumes a permit.
    let permit = app
        .state::<ResourcePreviews>()
        .copying
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::new("resource_unavailable", "资源导出不可用。"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let _export_permit = export_permit;
        let source = session.sides[side].as_ref().unwrap();
        let parent = destination
            .parent()
            .ok_or_else(|| AppError::new("resource_unavailable", "导出路径不可用。"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| AppError::io("导出资源", error))?;
        if let Some(snapshot) = &session.files[side] {
            let mut input =
                std::fs::File::open(snapshot).map_err(|error| AppError::io("读取资源", error))?;
            std::io::copy(&mut input, temporary.as_file_mut())
                .map_err(|error| AppError::io("导出资源", error))?;
        } else {
            GitCore::new().copy_resource(&session.repository, source, temporary.as_file_mut())?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| AppError::io("导出资源", error))?;
        temporary
            .persist(destination)
            .map_err(|error| AppError::io("保存资源", error.error))?;
        Ok(true)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "资源导出后台任务未能完成。"))?
}
