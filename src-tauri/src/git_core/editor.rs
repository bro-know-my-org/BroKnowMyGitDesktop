use super::*;

/// Normalizes the app-local external editor configuration. The UI supplies
/// one argument per line; retaining them as individual values means paths
/// with spaces and editor arguments never need shell parsing.
pub fn normalize_external_editor_config(
    editor: ExternalEditorConfig,
) -> Result<ExternalEditorConfig, AppError> {
    let executable = normalize_external_editor_value(&editor.executable, "外部编辑器可执行文件")?;
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
    Ok(ExternalEditorConfig {
        executable,
        arguments,
    })
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

pub(super) fn terminate_git_network_child(child: &mut std::process::Child, process_id: u32) {
    terminate_git_network_process_group(process_id);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
pub(super) fn configure_git_network_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: the child only creates its own process group before exec. It
    // lets a timeout terminate Git together with SSH, hooks, and helpers it
    // started rather than leaving a descendant behind.
    unsafe {
        command.pre_exec(|| {
            // SAFETY: setpgid is async-signal-safe and uses only the child PID.
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
pub(super) fn configure_git_network_process_group(_command: &mut Command) {}

#[cfg(all(not(unix), not(windows)))]
pub(super) fn configure_git_network_process_group(_command: &mut Command) {}

#[cfg(unix)]
pub(super) fn terminate_git_network_process_group(process_id: u32) {
    // SAFETY: a negative PID targets the dedicated child process group.
    unsafe {
        let _ = libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
pub(super) fn terminate_git_network_process_group(process_id: u32) {
    let process_id = process_id.to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", process_id.as_str(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(not(unix), not(windows)))]
pub(super) fn terminate_git_network_process_group(_process_id: u32) {}
