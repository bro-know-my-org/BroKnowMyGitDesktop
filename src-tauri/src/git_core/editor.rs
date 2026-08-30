use super::*;
use crate::process_tree::ProcessTreeHandle;

#[cfg(windows)]
use crate::process_tree::terminate_process_tree;

const MAX_EXTERNAL_EDITOR_CONFIGURATION_COMMAND_UNITS: usize = 16_000;
pub(super) const MAX_EXTERNAL_EDITOR_LAUNCH_COMMAND_UNITS: usize = 30_000;
const MAX_REPOSITORY_EDITORS: usize = 16;
const MAX_REPOSITORY_EDITOR_NAME_LENGTH: usize = 80;

pub fn normalize_repository_editor_config(
    editor: RepositoryEditorConfig,
) -> Result<RepositoryEditorConfig, AppError> {
    let editor = normalize_repository_editor_config_structure(editor)?;
    #[cfg(target_os = "macos")]
    if Path::new(&editor.executable)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return normalize_macos_repository_bundle(editor);
    }
    let launch = normalize_external_editor_config(ExternalEditorConfig {
        executable: editor.executable,
        arguments: editor.arguments,
    })
    .map_err(|_| AppError::new("repository_editor_config_invalid", "IDE 配置无效。"))?;
    Ok(RepositoryEditorConfig {
        name: editor.name,
        executable: launch.executable,
        arguments: launch.arguments,
    })
}

#[cfg(target_os = "macos")]
fn normalize_macos_repository_bundle(
    editor: RepositoryEditorConfig,
) -> Result<RepositoryEditorConfig, AppError> {
    let bundle_path = Path::new(&editor.executable);
    if !editor.arguments.is_empty() {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "macOS 应用包不支持附加参数。",
        ));
    }
    // Validate the bundle and its declared executable before handing it to
    // LaunchServices. `open -a <bundle> <repository>` preserves Cocoa's
    // normal open-document/open-folder event and existing-instance behavior.
    let bundle_executable = resolve_external_editor_executable_path(bundle_path)?;
    let metadata = fs::metadata(&bundle_executable)
        .map_err(|_| AppError::new("external_editor_config_invalid", "macOS 应用启动文件无效。"))?;
    if !metadata.is_file() {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "macOS 应用启动文件无效。",
        ));
    }
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "macOS 应用启动文件不可执行。",
        ));
    }
    let bundle = fs::canonicalize(bundle_path)
        .map_err(|_| AppError::new("external_editor_config_invalid", "macOS 应用不存在。"))?;
    let bundle = bundle
        .into_os_string()
        .into_string()
        .map_err(|_| AppError::new("external_editor_config_invalid", "macOS 应用路径无效。"))?;
    Ok(RepositoryEditorConfig {
        name: editor.name,
        executable: bundle,
        arguments: Vec::new(),
    })
}

pub(crate) fn repository_editor_launch_config(
    editor: &RepositoryEditorConfig,
) -> ExternalEditorConfig {
    #[cfg(target_os = "macos")]
    if Path::new(&editor.executable)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return ExternalEditorConfig {
            executable: "/usr/bin/open".to_owned(),
            arguments: vec!["-a".to_owned(), editor.executable.clone()],
        };
    }
    ExternalEditorConfig {
        executable: editor.executable.clone(),
        arguments: editor.arguments.clone(),
    }
}

pub(crate) fn normalize_repository_editor_config_structure(
    editor: RepositoryEditorConfig,
) -> Result<RepositoryEditorConfig, AppError> {
    let name = editor.name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_REPOSITORY_EDITOR_NAME_LENGTH
        || name.chars().any(char::is_control)
    {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "IDE 名称无效。",
        ));
    }
    let executable = normalize_external_editor_value(&editor.executable, "IDE 可执行文件")?;
    if !Path::new(&executable).is_absolute() {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "IDE 可执行文件必须使用绝对路径。",
        ));
    }
    if editor.arguments.len() > MAX_EXTERNAL_EDITOR_ARGUMENTS {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "IDE 附加参数过多。",
        ));
    }
    let arguments = editor
        .arguments
        .iter()
        .map(|argument| normalize_external_editor_value(argument, "IDE 参数"))
        .collect::<Result<Vec<_>, _>>()?;
    if external_editor_command_units(
        std::iter::once(executable.as_str()).chain(arguments.iter().map(String::as_str)),
    ) > MAX_EXTERNAL_EDITOR_CONFIGURATION_COMMAND_UNITS
    {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "IDE 启动参数过长。",
        ));
    }
    Ok(RepositoryEditorConfig {
        name: name.to_owned(),
        executable,
        arguments,
    })
}

pub fn normalize_repository_editor_configs(
    editors: Vec<RepositoryEditorConfig>,
) -> Result<Vec<RepositoryEditorConfig>, AppError> {
    let editors = normalize_repository_editor_configs_structure(editors)?;
    editors
        .into_iter()
        .map(normalize_repository_editor_config)
        .collect()
}

pub(crate) fn normalize_repository_editor_configs_structure(
    editors: Vec<RepositoryEditorConfig>,
) -> Result<Vec<RepositoryEditorConfig>, AppError> {
    if editors.len() > MAX_REPOSITORY_EDITORS {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "最多只能配置 16 个 IDE。",
        ));
    }
    let editors = editors
        .into_iter()
        .map(normalize_repository_editor_config_structure)
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = HashSet::new();
    if editors
        .iter()
        .any(|editor| !names.insert(editor.name.to_lowercase()))
    {
        return Err(AppError::new(
            "repository_editor_config_invalid",
            "IDE 名称不能重复。",
        ));
    }
    Ok(editors)
}

/// Normalizes the app-local external editor configuration. The UI supplies
/// one argument per line; retaining them as individual values means paths
/// with spaces and editor arguments never need shell parsing.
pub fn normalize_external_editor_config(
    editor: ExternalEditorConfig,
) -> Result<ExternalEditorConfig, AppError> {
    let executable = normalize_external_editor_executable(&editor.executable)?;
    if editor.arguments.len() > MAX_EXTERNAL_EDITOR_ARGUMENTS {
        return Err(AppError::new(
            "external_editor_arguments_invalid",
            "外部编辑器参数过多。",
        ));
    }
    let arguments = editor
        .arguments
        .iter()
        .map(|argument| normalize_external_editor_value(argument, "外部编辑器参数"))
        .collect::<Result<Vec<_>, _>>()?;
    if external_editor_command_units(
        std::iter::once(executable.as_str()).chain(arguments.iter().map(String::as_str)),
    ) > MAX_EXTERNAL_EDITOR_CONFIGURATION_COMMAND_UNITS
    {
        return Err(AppError::new(
            "external_editor_arguments_invalid",
            "外部编辑器参数过长。",
        ));
    }
    Ok(ExternalEditorConfig {
        executable,
        arguments,
    })
}

/// Reserve two UTF-16 units for surrounding quotes and pessimistically allow
/// every input unit to need one escape. This remains below Windows' command
/// line ceiling even when the configured editor is later given conflict paths.
pub(super) fn external_editor_command_units<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> usize {
    values
        .into_iter()
        .map(|value| 2 + value.encode_utf16().count().saturating_mul(2))
        .sum()
}

fn normalize_external_editor_executable(value: &str) -> Result<String, AppError> {
    let value = normalize_external_editor_value(value, "外部编辑器可执行文件")?;
    let path = Path::new(&value);
    if !path.is_absolute() {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "外部编辑器配置无效。",
        ));
    }
    let resolved_path = resolve_external_editor_executable_path(path)?;
    let canonical = fs::canonicalize(&resolved_path)
        .map_err(|_| AppError::new("external_editor_config_invalid", "外部编辑器配置无效。"))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|_| AppError::new("external_editor_config_invalid", "外部编辑器配置无效。"))?;
    if !metadata.is_file() {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "外部编辑器配置无效。",
        ));
    }
    #[cfg(windows)]
    if !matches!(
        canonical.extension().and_then(|extension| extension.to_str()),
        Some(extension)
            if extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("com")
    ) {
        // `Command` launches `.bat` and `.cmd` through `cmd.exe`. Conflict
        // filenames are repository-controlled, so never route them through a
        // command interpreter even when the editor itself was user-selected.
        return Err(AppError::new(
            "external_editor_config_invalid",
            "外部编辑器配置无效。",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(AppError::new(
                "external_editor_config_invalid",
                "外部编辑器配置无效。",
            ));
        }
    }
    #[cfg(target_os = "linux")]
    let launch_path = if resolved_path.parent() == Some(Path::new("/snap/bin"))
        && resolved_path.file_name().is_some()
        && canonical == Path::new("/usr/bin/snap")
    {
        // Snap dispatches applications by argv[0]. `/snap/bin/<app>` is a
        // root-managed link to the common launcher, so preserve that identity.
        resolved_path
    } else {
        canonical
    };
    #[cfg(not(target_os = "linux"))]
    let launch_path = canonical;
    let executable = launch_path
        .into_os_string()
        .into_string()
        .map_err(|_| AppError::new("external_editor_config_invalid", "外部编辑器配置无效。"))?;
    if executable.len() > MAX_EXTERNAL_EDITOR_VALUE_LENGTH
        || executable.chars().any(char::is_control)
    {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "外部编辑器配置无效。",
        ));
    }
    Ok(executable)
}

#[cfg(not(target_os = "macos"))]
fn resolve_external_editor_executable_path(path: &Path) -> Result<PathBuf, AppError> {
    Ok(path.to_path_buf())
}

#[cfg(target_os = "macos")]
fn resolve_external_editor_executable_path(path: &Path) -> Result<PathBuf, AppError> {
    if !path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return Ok(path.to_path_buf());
    }
    let bundle = fs::canonicalize(path)
        .map_err(|_| AppError::new("external_editor_config_invalid", "macOS 应用不存在。"))?;
    if !bundle.is_dir() {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "macOS 应用包无效。",
        ));
    }
    let info = read_bounded_plist(&bundle.join("Contents/Info.plist"))
        .map_err(|_| AppError::new("external_editor_config_invalid", "macOS 应用包无效。"))?;
    let executable_name = info
        .as_dictionary()
        .and_then(|dictionary| dictionary.get("CFBundleExecutable"))
        .and_then(plist::Value::as_string)
        .ok_or_else(|| AppError::new("external_editor_config_invalid", "macOS 应用包无效。"))?;
    let executable_name_path = Path::new(executable_name);
    if executable_name_path.components().count() != 1
        || !matches!(
            executable_name_path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "macOS 应用启动文件无效。",
        ));
    }
    let executable = fs::canonicalize(bundle.join("Contents/MacOS").join(executable_name))
        .map_err(|_| {
            AppError::new(
                "external_editor_config_invalid",
                "macOS 应用启动文件不存在。",
            )
        })?;
    if !executable.starts_with(&bundle) {
        return Err(AppError::new(
            "external_editor_config_invalid",
            "macOS 应用启动文件位于应用包外。",
        ));
    }
    Ok(executable)
}

#[cfg(target_os = "macos")]
pub(crate) fn read_bounded_plist(path: &Path) -> Result<plist::Value, std::io::Error> {
    use std::io::{Cursor, Error, ErrorKind, Read};
    use std::os::unix::fs::OpenOptionsExt;

    const MAX_PLIST_BYTES: u64 = 1024 * 1024;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_PLIST_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "plist is too large"));
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "plist is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PLIST_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PLIST_BYTES as usize {
        return Err(Error::new(ErrorKind::InvalidData, "plist is too large"));
    }
    plist::Value::from_reader(Cursor::new(bytes))
        .map_err(|error| Error::new(ErrorKind::InvalidData, error))
}

pub(super) fn normalize_external_editor_value(
    value: &str,
    label: &'static str,
) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_EXTERNAL_EDITOR_VALUE_LENGTH
        || value.chars().any(char::is_control)
    {
        let _ = label;
        return Err(AppError::new(
            "external_editor_config_invalid",
            "外部编辑器配置无效。",
        ));
    }
    Ok(value.to_owned())
}

pub(super) fn is_conflicted_status_entry(entry: &GitStatusEntry) -> bool {
    matches!(
        (entry.index_status.as_str(), entry.worktree_status.as_str()),
        ("D", "D") | ("A", "U") | ("U", "D") | ("U", "A") | ("D", "U") | ("A", "A") | ("U", "U")
    )
}

pub(super) fn terminate_git_network_child(
    child: &mut std::process::Child,
    process_id: u32,
    process_tree: Option<&ProcessTreeHandle>,
) {
    #[cfg(unix)]
    {
        let _ = process_tree;
        terminate_git_network_process_group(process_id);
    }
    #[cfg(windows)]
    {
        let _ = process_id;
        if let Some(process_tree) = process_tree {
            terminate_process_tree(process_tree);
        }
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = (process_id, process_tree);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The direct child has already exited, so its numeric PID must not be reused
/// for cleanup. Windows retains a Job Object handle. On Unix, deliberately do
/// not signal the old process group: a descendant that left the group can hold
/// a pipe open after the group becomes empty, allowing that numeric PGID to be
/// recycled for an unrelated process group.
pub(super) fn terminate_remaining_git_network_process_tree(
    process_tree: &ProcessTreeHandle,
    process_id: u32,
) {
    #[cfg(unix)]
    {
        let _ = (process_tree, process_id);
    }
    #[cfg(windows)]
    {
        let _ = process_id;
        terminate_process_tree(process_tree);
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = (process_tree, process_id);
    }
}

#[cfg(unix)]
pub(super) fn configure_git_network_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Delegate the post-fork setup to the standard library rather than
    // running Rust code in a `pre_exec` closure of this multithreaded process.
    command.process_group(0);
}

#[cfg(windows)]
pub(super) fn configure_git_network_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    // A Job Object can only be attached after Command has created the
    // process. Keep that short interval safe: no Git code (or descendants)
    // can run until runtime.rs has successfully attached the Job Object and
    // explicitly resumed this initial thread.
    command.creation_flags(CREATE_SUSPENDED);
}

#[cfg(all(not(unix), not(windows)))]
pub(super) fn configure_git_network_process_group(_command: &mut Command) {}

#[cfg(windows)]
pub(super) fn resume_git_network_process(process_id: u32) -> std::io::Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    // SAFETY: the snapshot has no borrowed inputs and is closed before this
    // function returns.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    // A just-created suspended process has only its initial thread. Open it
    // by the kernel-provided process ID, then release the snapshot before
    // resuming Git.
    let thread = if unsafe { Thread32First(snapshot, &mut entry) } == 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: this handle was returned by CreateToolhelp32Snapshot.
        unsafe {
            let _ = CloseHandle(snapshot);
        }
        return Err(error);
    } else {
        loop {
            if entry.th32OwnerProcessID == process_id {
                // SAFETY: Toolhelp returned this thread ID from the live
                // snapshot. The handle is closed after ResumeThread below.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !thread.is_null() {
                    break Some(thread);
                }
            }
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
                break None;
            }
        }
    };
    // SAFETY: this handle was returned by CreateToolhelp32Snapshot.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    let thread = thread.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "无法定位 Git 子进程的初始线程",
        )
    })?;
    // ResumeThread returns u32::MAX on failure; any other value is the
    // previous suspend count and means the process can now run inside its Job.
    let resumed = unsafe { ResumeThread(thread) };
    let error = if resumed == u32::MAX {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    // SAFETY: OpenThread returned this owned handle above.
    unsafe {
        let _ = CloseHandle(thread);
    }
    error.map_or(Ok(()), Err)
}

#[cfg(not(windows))]
pub(super) fn resume_git_network_process(_process_id: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn terminate_git_network_process_group(process_id: u32) {
    // SAFETY: a negative PID targets the dedicated child process group.
    unsafe {
        let _ = libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}
