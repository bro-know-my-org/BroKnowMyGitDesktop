pub struct PersistentProfileStore {
    path: PathBuf,
    data: ProfileStore,
}

/// An update may already have replaced the durable store before a final
/// directory sync fails. Callers must keep the new in-memory value in that
/// case; rolling it back would make memory disagree with `profiles.json`.
enum SaveError {
    BeforeReplace(AppError),
    AfterReplace(AppError),
}

/// Distinguishes a mutation that definitely did not reach `profiles.json`
/// from one that crossed the atomic-replace boundary but could not confirm the
/// final directory sync. External compensations (keychain deletion and Git
/// rollback) must only run for the former.
pub(crate) enum MutationFailure {
    Operation(AppError),
    BeforeReplace(AppError),
    AfterReplace(AppError),
}

impl MutationFailure {
    pub(crate) fn committed(&self) -> bool {
        matches!(self, Self::AfterReplace(_))
    }

    pub(crate) fn into_error(self) -> AppError {
        match self {
            Self::Operation(error) | Self::BeforeReplace(error) | Self::AfterReplace(error) => {
                error
            }
        }
    }
}

impl PersistentProfileStore {
    pub fn load(app_data_dir: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(app_data_dir)
            .map_err(|error| AppError::io("创建应用数据目录", error))?;
        let path = app_data_dir.join(STORE_FILE_NAME);
        let data = if path.exists() {
            let content = fs::read(&path).map_err(|error| AppError::io("读取账号配置", error))?;
            let mut data: ProfileStore = serde_json::from_slice(&content).map_err(|_| {
                AppError::new(
                    "profile_store_invalid",
                    "账号配置文件无法解析；为保护现有数据，未自动覆盖它。",
                )
            })?;
            if data.schema_version > default_schema_version() {
                return Err(AppError::new(
                    "profile_store_version_unsupported",
                    "账号配置由更高版本的应用创建，无法安全写入。",
                ));
            }
            data.schema_version = default_schema_version();
            data
        } else {
            empty_profile_store()
        };

        Ok(Self { path, data })
    }

    /// Preserve a malformed store under a unique sibling name, then start
    /// with an empty in-memory store. This lets the application launch while
    /// keeping the original bytes available for recovery or diagnosis.
    pub fn recover_invalid(app_data_dir: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(app_data_dir)
            .map_err(|error| AppError::io("创建应用数据目录", error))?;
        let path = app_data_dir.join(STORE_FILE_NAME);
        if path.exists() {
            let backup = app_data_dir.join(format!("profiles.invalid.{}.json", Uuid::new_v4()));
            fs::rename(&path, backup).map_err(|error| AppError::io("备份损坏的账号配置", error))?;
        }
        Ok(Self {
            path,
            data: empty_profile_store(),
        })
    }

    pub fn data(&self) -> &ProfileStore {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut ProfileStore {
        &mut self.data
    }

    fn save(&self) -> Result<(), SaveError> {
        let serialized = serde_json::to_vec_pretty(&self.data).map_err(|_| {
            SaveError::BeforeReplace(AppError::new(
                "profile_store_serialize_failed",
                "无法保存账号配置",
            ))
        })?;
        let temporary_path = self
            .path
            .with_extension(format!("json.{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> io::Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                // Account metadata is not a secret store, but it includes
                // local repository paths and keychain identifiers. Create
                // the replacement with owner-only permissions so a normal
                // umask cannot make the atomically renamed file world-readable.
                options.mode(0o600);
            }
            let mut temporary = options.open(&temporary_path)?;
            temporary.write_all(&serialized)?;
            temporary.sync_all()
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary_path);
            return Err(SaveError::BeforeReplace(AppError::io(
                "写入账号配置临时文件",
                error,
            )));
        }

        if let Err(error) = replace_file(&temporary_path, &self.path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(SaveError::BeforeReplace(AppError::io(
                "原子更新账号配置",
                error,
            )));
        }
        if let Err(_error) = sync_parent_directory(&self.path) {
            return Err(SaveError::AfterReplace(AppError::with_safe_details(
                "profile_store_sync_failed",
                "账号配置已更新，但无法确认其已同步到磁盘。",
                "请避免立即断电，并在恢复后检查账号配置。",
            )));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub(crate) fn replace_file(temporary_path: &Path, destination_path: &Path) -> io::Result<()> {
    fs::rename(temporary_path, destination_path)
}

#[cfg(windows)]
pub(crate) fn replace_file(temporary_path: &Path, destination_path: &Path) -> io::Result<()> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let temporary: Vec<u16> = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let destination: Vec<u16> = destination_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    // SAFETY: both strings are NUL-terminated UTF-16 paths that remain alive
    // throughout the call. MoveFileExW replaces the existing destination and
    // requests that Windows flush the metadata before returning.
    let replaced = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "profile store has no parent directory",
        )
    })?;
    match fs::File::open(parent)?.sync_all() {
        Ok(()) => Ok(()),
        // Darwin commonly rejects fsync on directory descriptors. The file
        // itself was already synced before its atomic replacement, so this
        // portability limitation must not be surfaced as a failed save.
        #[cfg(target_os = "macos")]
        Err(error) if error.raw_os_error() == Some(libc::EINVAL) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
pub(crate) fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    // Windows uses MOVEFILE_WRITE_THROUGH in `replace_file`; opening a
    // directory through std::fs::File is not portable there.
    Ok(())
}

/// Shared Tauri state. Commands mutate metadata through `mutate`, which persists only after
/// the whole mutation succeeds and restores the in-memory snapshot on a write failure.
#[derive(Clone)]
pub struct AppState {
    profiles: Arc<Mutex<PersistentProfileStore>>,
    active_publishes: Arc<Mutex<HashSet<String>>>,
    active_clones: Arc<Mutex<HashSet<String>>>,
    active_repository_operations: Arc<Mutex<HashSet<String>>>,
    active_account_operations: Arc<Mutex<HashSet<String>>>,
    active_cli_account_imports: Arc<Mutex<HashSet<String>>>,
    account_status_refresh_generation: Arc<Mutex<u64>>,
    clean_untracked_confirmations: Arc<Mutex<HashMap<String, CleanUntrackedConfirmation>>>,
    discard_changes_confirmations: Arc<Mutex<HashMap<String, DiscardChangesConfirmation>>>,
    // An advisory lock held for this AppState's lifetime. It serializes
    // independent desktop processes using the same profile directory.
    _profile_store_lock: Arc<fs::File>,
}
