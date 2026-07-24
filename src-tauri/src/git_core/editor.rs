use super::*;
use crate::process_tree::ProcessTreeHandle;

#[cfg(windows)]
use crate::process_tree::terminate_process_tree;

const MAX_EXTERNAL_EDITOR_CONFIGURATION_COMMAND_UNITS: usize = 16_000;
const MAX_EXTERNAL_EDITOR_LAUNCH_COMMAND_UNITS: usize = 30_000;

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

pub(super) fn validate_external_editor_launch_size<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), AppError> {
    if external_editor_command_units(values) > MAX_EXTERNAL_EDITOR_LAUNCH_COMMAND_UNITS {
        return Err(AppError::new(
            "external_editor_arguments_invalid",
            "要在外部编辑器中打开的路径过长。",
        ));
    }
    Ok(())
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
    let canonical = fs::canonicalize(path)
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
    let executable = canonical
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
