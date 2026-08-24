use super::support::repository_root;
use crate::app_error::AppError;
use crate::domain::TerminalConfig;
use crate::persistence::AppState;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;

#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::{
    env, fs,
    io::Read,
    os::fd::AsRawFd,
    process::{Output, Stdio},
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{
    sync::{mpsc, OnceLock},
    thread,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use std::ffi::{c_char, c_void, CStr};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFontFamily {
    family: String,
    monospaced: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFontPreferences {
    interface_family: Option<String>,
    code_family: Option<String>,
    interface_size: Option<f64>,
    code_size: Option<f64>,
    interface_weight: Option<u16>,
    code_weight: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemTerminalInfo {
    executable: String,
    arguments: Vec<String>,
}

#[derive(Debug)]
struct FontPreference {
    family: String,
    size: Option<f64>,
    weight: Option<u16>,
}

#[tauri::command]
pub async fn get_system_font_preferences() -> Result<SystemFontPreferences, AppError> {
    tauri::async_runtime::spawn_blocking(platform_font_preferences)
        .await
        .map_err(|_| AppError::new("command_failed", "系统字体偏好读取任务未能完成。"))
}

#[tauri::command]
pub async fn list_system_fonts() -> Result<Vec<SystemFontFamily>, AppError> {
    tauri::async_runtime::spawn_blocking(collect_system_fonts)
        .await
        .map_err(|_| AppError::new("command_failed", "系统字体列表读取任务未能完成。"))
}

#[tauri::command]
pub fn open_external_url(app: AppHandle, url: String) -> Result<(), AppError> {
    let url = validated_external_url(&url)?;
    app.opener()
        .open_url(url.to_string(), None::<String>)
        .map_err(|_| AppError::new("command_failed", "无法调用系统默认浏览器。"))
}

#[tauri::command]
pub async fn open_repository_in_file_manager(
    app: AppHandle,
    repository_path: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let opener_path = if let Some(repository) = repository.to_str() {
            repository.to_owned()
        } else {
            let requested_path = std::fs::canonicalize(&repository_path).map_err(|_| {
                AppError::new("command_failed", "无法解析要在文件管理器中打开的仓库路径。")
            })?;
            if requested_path != repository {
                return Err(AppError::new(
                    "command_failed",
                    "系统文件管理器无法表示该仓库路径。",
                ));
            }
            repository_path
        };
        app.opener()
            .open_path(opener_path, None::<String>)
            .map_err(|_| AppError::new("command_failed", "无法调用系统文件管理器。"))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "文件管理器后台任务未能完成。"))?
}

#[tauri::command]
pub async fn open_repository_in_terminal(repository_path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        platform_open_terminal(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "终端后台任务未能完成。"))?
}

#[tauri::command]
pub async fn get_preferred_terminal(
    state: State<'_, AppState>,
) -> Result<Option<TerminalConfig>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.preferred_terminal())
        .await
        .map_err(|_| AppError::new("command_failed", "首选终端读取任务未能完成。"))?
}

#[tauri::command]
pub async fn save_preferred_terminal(
    terminal: Option<TerminalConfig>,
    state: State<'_, AppState>,
) -> Result<Option<TerminalConfig>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let terminal = terminal.map(normalize_terminal_config).transpose()?;
        state.mutate(|store| {
            store.preferred_terminal = terminal.clone();
            Ok(terminal)
        })
    })
    .await
    .map_err(|_| AppError::new("command_failed", "首选终端保存任务未能完成。"))?
}

#[tauri::command]
pub async fn get_system_terminal() -> Result<Option<SystemTerminalInfo>, AppError> {
    tauri::async_runtime::spawn_blocking(platform_system_terminal)
        .await
        .map_err(|_| AppError::new("command_failed", "系统终端检测任务未能完成。"))
        .map(|terminal| {
            terminal.map(|terminal| SystemTerminalInfo {
                executable: terminal.executable,
                arguments: terminal.arguments,
            })
        })
}

#[tauri::command]
pub async fn open_repository_in_preferred_terminal(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let terminal = state.preferred_terminal()?.ok_or_else(|| {
            AppError::new(
                "preferred_terminal_not_configured",
                "请先在设置中配置首选终端。",
            )
        })?;
        let terminal = normalize_terminal_config(terminal)?;
        let repository = repository_root(&repository_path)?;
        platform_open_preferred_terminal(&repository, &terminal)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "首选终端后台任务未能完成。"))?
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalLaunchSpec {
    executable: String,
    arguments: Vec<String>,
}

const REPOSITORY_ARGUMENT_PLACEHOLDER: &str = "{repository}";

fn preferred_terminal_command(repository: &Path, terminal: &TerminalConfig) -> Command {
    let mut command = Command::new(&terminal.executable);
    for argument in &terminal.arguments {
        if argument == REPOSITORY_ARGUMENT_PLACEHOLDER {
            command.arg(repository);
        } else {
            command.arg(argument);
        }
    }
    command.current_dir(repository);
    command
}

#[cfg(target_os = "windows")]
fn spawn_terminal(command: &mut Command) -> Result<(), AppError> {
    let child = command
        .spawn()
        .map_err(|_| AppError::new("command_failed", "无法调用系统终端。"))?;
    drop(child);
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
static TERMINAL_REAPER_SENDER: OnceLock<Option<mpsc::Sender<std::process::Child>>> =
    OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn terminal_reaper_sender() -> Option<mpsc::Sender<std::process::Child>> {
    TERMINAL_REAPER_SENDER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<std::process::Child>();
            if thread::Builder::new()
                .name("bkm-terminal-reaper".into())
                .spawn(move || {
                    let mut children = Vec::<std::process::Child>::new();
                    loop {
                        if children.is_empty() {
                            match receiver.recv() {
                                Ok(child) => children.push(child),
                                Err(_) => break,
                            }
                        } else {
                            match receiver.recv_timeout(Duration::from_millis(100)) {
                                Ok(child) => children.push(child),
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        while let Ok(child) = receiver.try_recv() {
                            children.push(child);
                        }
                        children.retain_mut(|child| match child.try_wait() {
                            Ok(None) => true,
                            Ok(Some(_)) => false,
                            Err(_) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                false
                            }
                        });
                    }
                })
                .is_err()
            {
                return None;
            }
            Some(sender)
        })
        .as_ref()
        .cloned()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reap_terminal_child(mut child: std::process::Child) -> Result<(), AppError> {
    let Some(sender) = terminal_reaper_sender() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(AppError::new("command_failed", "无法监控终端进程。"));
    };
    if let Err(error) = sender.send(child) {
        child = error.0;
        let _ = child.kill();
        let _ = child.wait();
        return Err(AppError::new("command_failed", "无法监控终端进程。"));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const TERMINAL_STARTUP_PROBE: Duration = Duration::from_millis(50);

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn observe_terminal_startup(
    child: std::process::Child,
    error_message: &'static str,
) -> Result<(), AppError> {
    let deadline = Instant::now() + TERMINAL_STARTUP_PROBE;
    let mut child = child;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(AppError::new("command_failed", error_message)),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => return reap_terminal_child(child),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::new("command_failed", error_message));
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn platform_open_terminal(repository: &Path) -> Result<(), AppError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let terminal = platform_system_terminal()
        .ok_or_else(|| AppError::new("command_failed", "没有找到可用的系统终端。"))?;
    let mut command = Command::new(terminal.executable);
    command
        .args(terminal.arguments)
        .current_dir(repository)
        .creation_flags(CREATE_NEW_CONSOLE);
    spawn_terminal(&mut command)
}

#[cfg(target_os = "windows")]
fn platform_system_terminal() -> Option<TerminalLaunchSpec> {
    let executable = std::env::var_os("COMSPEC")
        .map(std::path::PathBuf::from)
        .and_then(|path| std::fs::canonicalize(path).ok())
        .filter(|path| path.is_file() && !windows_path_requires_command_processor(path))
        .or_else(|| {
            std::env::var_os("SystemRoot")
                .map(|root| std::path::PathBuf::from(root).join("System32/cmd.exe"))
                .and_then(|path| std::fs::canonicalize(path).ok())
                .filter(|path| path.is_file() && !windows_path_requires_command_processor(path))
        })?;
    Some(TerminalLaunchSpec {
        executable: executable.into_os_string().into_string().ok()?,
        arguments: Vec::new(),
    })
}

#[cfg(target_os = "windows")]
fn platform_open_preferred_terminal(
    repository: &Path,
    terminal: &TerminalConfig,
) -> Result<(), AppError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let mut command = preferred_terminal_command(repository, terminal);
    command.creation_flags(CREATE_NEW_CONSOLE);
    spawn_terminal(&mut command)
}

#[cfg(target_os = "macos")]
fn platform_open_terminal(repository: &Path) -> Result<(), AppError> {
    let status = Command::new("open")
        .args(["-a", "Terminal"])
        .arg(repository)
        .status()
        .map_err(|_| AppError::new("command_failed", "无法调用系统终端。"))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::new("command_failed", "无法调用系统终端。"))
    }
}

#[cfg(target_os = "macos")]
fn platform_system_terminal() -> Option<TerminalLaunchSpec> {
    Some(TerminalLaunchSpec {
        executable: "/usr/bin/open".to_string(),
        arguments: vec![
            "-a".to_string(),
            "Terminal".to_string(),
            REPOSITORY_ARGUMENT_PLACEHOLDER.to_string(),
        ],
    })
}

#[cfg(target_os = "macos")]
fn platform_open_preferred_terminal(
    repository: &Path,
    terminal: &TerminalConfig,
) -> Result<(), AppError> {
    let child = preferred_terminal_command(repository, terminal)
        .spawn()
        .map_err(|_| AppError::new("command_failed", "无法调用首选终端。"))?;
    observe_terminal_startup(child, "无法调用首选终端。")
}

#[cfg(target_os = "linux")]
fn platform_open_terminal(repository: &Path) -> Result<(), AppError> {
    let mut found_terminal = false;
    for terminal in linux_system_terminal_candidates()
        .into_iter()
        .filter_map(resolve_terminal_launch_spec)
    {
        found_terminal = true;
        if spawn_linux_terminal(repository, &terminal, "无法调用系统终端。").is_ok() {
            return Ok(());
        }
    }
    Err(if found_terminal {
        AppError::new("command_failed", "无法调用系统终端。")
    } else {
        AppError::new("command_failed", "没有找到可用的系统终端。")
    })
}

#[cfg(target_os = "linux")]
fn platform_system_terminal() -> Option<TerminalLaunchSpec> {
    linux_system_terminal_candidates()
        .into_iter()
        .find_map(resolve_terminal_launch_spec)
}

#[cfg(target_os = "linux")]
fn platform_open_preferred_terminal(
    repository: &Path,
    terminal: &TerminalConfig,
) -> Result<(), AppError> {
    let child = preferred_terminal_command(repository, terminal)
        .spawn()
        .map_err(|_| AppError::new("command_failed", "无法调用首选终端。"))?;
    observe_terminal_startup(child, "无法调用首选终端。")
}

#[cfg(target_os = "linux")]
fn spawn_linux_terminal(
    repository: &Path,
    terminal: &TerminalLaunchSpec,
    error_message: &'static str,
) -> Result<(), AppError> {
    match Command::new(&terminal.executable)
        .args(&terminal.arguments)
        .current_dir(repository)
        .spawn()
    {
        Ok(child) => observe_terminal_startup(child, error_message),
        Err(_) => Err(AppError::new("command_failed", error_message)),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_open_terminal(_repository: &Path) -> Result<(), AppError> {
    Err(AppError::new(
        "command_failed",
        "当前平台不支持打开系统终端。",
    ))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_system_terminal() -> Option<TerminalLaunchSpec> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn platform_open_preferred_terminal(
    _repository: &Path,
    _terminal: &TerminalConfig,
) -> Result<(), AppError> {
    Err(AppError::new(
        "command_failed",
        "当前平台不支持打开首选终端。",
    ))
}

fn normalize_terminal_config(terminal: TerminalConfig) -> Result<TerminalConfig, AppError> {
    const MAX_ARGUMENTS: usize = 32;
    const MAX_VALUE_LENGTH: usize = 4096;
    const MAX_TOTAL_LENGTH: usize = 16_000;

    let executable = terminal.executable.as_str();
    let path = Path::new(executable);
    if executable.is_empty()
        || executable.len() > MAX_VALUE_LENGTH
        || executable.chars().any(char::is_control)
        || !path.is_absolute()
    {
        return Err(AppError::new(
            "preferred_terminal_config_invalid",
            "首选终端配置无效。",
        ));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| AppError::new("preferred_terminal_config_invalid", "首选终端配置无效。"))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|_| AppError::new("preferred_terminal_config_invalid", "首选终端配置无效。"))?;
    if !metadata.is_file() {
        return Err(AppError::new(
            "preferred_terminal_config_invalid",
            "首选终端配置无效。",
        ));
    }
    #[cfg(target_os = "windows")]
    if windows_path_requires_command_processor(&canonical) {
        return Err(AppError::new(
            "preferred_terminal_config_invalid",
            "首选终端配置无效。",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(AppError::new(
                "preferred_terminal_config_invalid",
                "首选终端配置无效。",
            ));
        }
    }
    #[cfg(target_os = "macos")]
    let is_system_open = std::fs::canonicalize("/usr/bin/open")
        .ok()
        .is_some_and(|trusted_open| canonical == trusted_open);
    let executable = canonical
        .into_os_string()
        .into_string()
        .map_err(|_| AppError::new("preferred_terminal_config_invalid", "首选终端配置无效。"))?;
    if terminal.arguments.len() > MAX_ARGUMENTS {
        return Err(AppError::new(
            "preferred_terminal_arguments_invalid",
            "首选终端参数过多。",
        ));
    }
    let arguments = terminal.arguments;
    #[cfg(target_os = "macos")]
    if is_system_open {
        let repository_argument_indices = arguments
            .iter()
            .enumerate()
            .filter_map(|(index, argument)| {
                (argument == REPOSITORY_ARGUMENT_PLACEHOLDER).then_some(index)
            })
            .collect::<Vec<_>>();
        if repository_argument_indices.len() != 1 {
            return Err(AppError::new(
                "preferred_terminal_arguments_invalid",
                "macOS 首选终端参数必须且只能包含一个仓库路径占位符。",
            ));
        }
        let repository_argument_index = repository_argument_indices[0];
        let option_end = arguments
            .iter()
            .position(|argument| argument == "--")
            .unwrap_or(arguments.len());
        if !repository_placeholder_is_open_target(&arguments, repository_argument_index)
            || arguments[..option_end]
                .iter()
                .position(|argument| argument == "--args")
                .is_some_and(|index| index < repository_argument_index)
        {
            return Err(AppError::new(
                "preferred_terminal_arguments_invalid",
                "macOS 仓库路径占位符不能位于 --args 之后。",
            ));
        }
        let launcher_option_end = arguments[..option_end]
            .iter()
            .position(|argument| argument == "--args")
            .unwrap_or(option_end);
        if arguments[..launcher_option_end].iter().any(|argument| {
            argument == "--wait-apps"
                || (argument.starts_with('-')
                    && !argument.starts_with("--")
                    && argument[1..].contains('W'))
        }) {
            return Err(AppError::new(
                "preferred_terminal_arguments_invalid",
                "macOS 首选终端不能等待应用退出。",
            ));
        }
    }
    if arguments
        .iter()
        .any(|argument| argument.len() > MAX_VALUE_LENGTH || argument.chars().any(char::is_control))
        || std::iter::once(executable.len())
            .chain(arguments.iter().map(String::len))
            .sum::<usize>()
            > MAX_TOTAL_LENGTH
    {
        return Err(AppError::new(
            "preferred_terminal_arguments_invalid",
            "首选终端参数无效。",
        ));
    }
    Ok(TerminalConfig {
        executable,
        arguments,
    })
}

#[cfg(any(test, target_os = "windows"))]
fn windows_path_requires_command_processor(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
        })
}

#[cfg(any(test, target_os = "macos"))]
fn repository_placeholder_is_open_target(arguments: &[String], repository_index: usize) -> bool {
    let mut index = 0;
    while index < repository_index {
        let argument = &arguments[index];
        if argument == "--" {
            return true;
        }
        if matches!(
            argument.as_str(),
            "-a" | "-b" | "-s" | "--arch" | "--env" | "--stdin" | "--stdout" | "--stderr"
        ) {
            if index + 1 == repository_index {
                return false;
            }
            index += 2;
            continue;
        }
        index += 1;
    }
    true
}

fn terminal_specific_arguments_override_working_directory(
    terminal_name: &str,
    arguments: &[String],
) -> bool {
    match terminal_name {
        "kitty" => arguments.iter().any(|argument| {
            argument == "-d"
                || (argument.starts_with("-d") && !argument.starts_with("--") && argument.len() > 2)
        }),
        "foot" | "footclient" => arguments.iter().any(|argument| {
            argument == "-D"
                || (argument.starts_with("-D") && !argument.starts_with("--") && argument.len() > 2)
        }),
        "urxvt" | "urxvtc" | "rxvt" | "rxvt-unicode" => {
            arguments.iter().any(|argument| argument == "-cd")
        }
        "wt" | "wt.exe" | "windowsterminal" | "windowsterminal.exe" => {
            arguments.iter().any(|argument| {
                argument == "-d"
                    || argument == "--startingDirectory"
                    || argument.starts_with("--startingDirectory=")
            })
        }
        _ => false,
    }
}

fn terminal_launcher_arguments<'a>(executable: &str, arguments: &'a [String]) -> &'a [String] {
    let executable_name = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase());
    let has_execute_boundary = matches!(
        executable_name.as_deref(),
        Some(
            "xterm"
                | "x-terminal-emulator"
                | "gnome-terminal"
                | "konsole"
                | "xfce4-terminal"
                | "mate-terminal"
                | "alacritty"
                | "foot"
                | "footclient"
                | "kitty"
                | "wezterm"
                | "wt"
                | "wt.exe"
        )
    );
    let boundary = arguments
        .iter()
        .position(|argument| {
            argument == "--"
                || (has_execute_boundary
                    && matches!(argument.as_str(), "-e" | "--execute" | "--command"))
        })
        .unwrap_or(arguments.len());
    &arguments[..boundary]
}

#[cfg(target_os = "linux")]
fn discovered_terminal_embeds_child_command(executable: &str, arguments: &[String]) -> bool {
    discovered_terminal_embeds_child_command_inner(executable, arguments, 0)
}

#[cfg(target_os = "linux")]
fn discovered_terminal_embeds_child_command_inner(
    executable: &str,
    arguments: &[String],
    depth: usize,
) -> bool {
    if depth >= 32 {
        return true;
    }
    if is_env_executable(executable) {
        return env_wrapped_command(arguments).is_none_or(|(command, command_arguments)| {
            discovered_terminal_embeds_child_command_inner(command, command_arguments, depth + 1)
        });
    }

    let executable_name = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase());
    if executable_name.as_deref() == Some("flatpak") {
        let Some(run_arguments) = flatpak_run_arguments(arguments) else {
            return false;
        };
        let Some(ref_index) = flatpak_application_ref_index(run_arguments) else {
            return true;
        };
        let command = flatpak_application_command(run_arguments, ref_index)
            .unwrap_or(&run_arguments[ref_index]);
        return discovered_terminal_embeds_child_command_inner(
            command,
            &run_arguments[ref_index + 1..],
            depth + 1,
        );
    }

    arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--" | "-e" | "--execute" | "--command"))
}

#[cfg(target_os = "linux")]
fn linux_system_terminal_candidates() -> Vec<TerminalLaunchSpec> {
    let kdeglobals = linux_desktop_is_kde().then(read_kdeglobals).flatten();
    let terminal_environment = env::var("TERMINAL").ok();
    build_linux_terminal_candidates(kdeglobals.as_deref(), terminal_environment.as_deref())
}

#[cfg(target_os = "linux")]
fn build_linux_terminal_candidates(
    kdeglobals: Option<&str>,
    terminal_environment: Option<&str>,
) -> Vec<TerminalLaunchSpec> {
    let mut candidates = vec![terminal_launch_spec("xdg-terminal-exec")];
    if let Some(kdeglobals) = kdeglobals {
        candidates.extend(kde_terminal_launch_specs(kdeglobals));
    }
    if let Some(terminal) = terminal_environment.and_then(command_line_terminal_launch_spec) {
        candidates.push(terminal);
    }
    candidates.extend(
        [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
            "mate-terminal",
            "kitty",
            "wezterm",
            "alacritty",
            "foot",
            "xterm",
        ]
        .into_iter()
        .map(terminal_launch_spec),
    );
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| {
        seen.insert((candidate.executable.clone(), candidate.arguments.clone()))
    });
    candidates
}

#[cfg(target_os = "linux")]
fn terminal_launch_spec(executable: &str) -> TerminalLaunchSpec {
    TerminalLaunchSpec {
        executable: executable.to_string(),
        arguments: Vec::new(),
    }
}

#[cfg(target_os = "linux")]
fn linux_desktop_is_kde() -> bool {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    env::var_os("KDE_FULL_SESSION").is_some()
        || desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("kde"))
}

#[cfg(target_os = "linux")]
fn resolve_terminal_launch_spec(terminal: TerminalLaunchSpec) -> Option<TerminalLaunchSpec> {
    let path = env::var_os("PATH");
    resolve_terminal_launch_spec_with_path(terminal, path.as_deref())
}

#[cfg(target_os = "linux")]
fn resolve_terminal_launch_spec_with_path(
    mut terminal: TerminalLaunchSpec,
    path: Option<&std::ffi::OsStr>,
) -> Option<TerminalLaunchSpec> {
    let resolved = resolve_terminal_executable_with_path(&terminal.executable, path)?;
    terminal.executable = resolved.into_os_string().into_string().ok()?;
    resolve_env_wrapped_terminal_commands(&mut terminal, path)?;
    if discovered_terminal_embeds_child_command(&terminal.executable, &terminal.arguments)
        || terminal_arguments_override_working_directory(&terminal.executable, &terminal.arguments)
        || wrapper_arguments_override_working_directory(&terminal.executable, &terminal.arguments)
    {
        return None;
    }
    Some(terminal)
}

#[cfg(target_os = "linux")]
fn resolve_terminal_executable_with_path(
    executable: &str,
    path: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let executable = Path::new(executable);
    let resolved = if executable.is_absolute() {
        fs::canonicalize(executable).ok()?
    } else if executable.components().count() > 1 {
        return None;
    } else {
        path.into_iter()
            .flat_map(env::split_paths)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(executable))
            .find_map(|candidate| {
                let resolved = fs::canonicalize(candidate).ok()?;
                terminal_path_is_executable(&resolved).then_some(resolved)
            })?
    };
    if !resolved.is_absolute() || !terminal_path_is_executable(&resolved) {
        return None;
    }
    Some(resolved)
}

#[cfg(target_os = "linux")]
fn resolve_env_wrapped_terminal_commands(
    terminal: &mut TerminalLaunchSpec,
    path: Option<&std::ffi::OsStr>,
) -> Option<()> {
    let mut executable = terminal.executable.clone();
    let mut argument_offset = 0;
    for _ in 0..32 {
        if !is_env_executable(&executable) {
            return Some(());
        }
        let relative_index = env_wrapped_command_index(&terminal.arguments[argument_offset..])?;
        let command_index = argument_offset + relative_index;
        let resolved =
            resolve_terminal_executable_with_path(&terminal.arguments[command_index], path)?;
        let resolved = resolved.into_os_string().into_string().ok()?;
        terminal.arguments[command_index] = resolved.clone();
        executable = resolved;
        argument_offset = command_index + 1;
    }
    None
}

#[cfg(target_os = "linux")]
fn terminal_path_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn kde_terminal_launch_specs(contents: &str) -> Vec<TerminalLaunchSpec> {
    kde_terminal_candidates(
        kde_general_value(contents, "TerminalApplication")
            .and_then(command_line_terminal_launch_spec),
        kde_general_value(contents, "TerminalService").and_then(resolve_terminal_desktop_service),
    )
}

#[cfg(target_os = "linux")]
fn kde_terminal_candidates(
    application: Option<TerminalLaunchSpec>,
    service: Option<TerminalLaunchSpec>,
) -> Vec<TerminalLaunchSpec> {
    application.into_iter().chain(service).collect()
}

#[cfg(target_os = "linux")]
fn resolve_terminal_desktop_service(service: &str) -> Option<TerminalLaunchSpec> {
    let service = service.trim();
    if service.is_empty()
        || service.len() > 255
        || service.chars().any(char::is_control)
        || service
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
    {
        return None;
    }
    let file_name = if service.ends_with(".desktop") {
        service.to_string()
    } else {
        format!("{service}.desktop")
    };
    for data_directory in xdg_data_directories() {
        let desktop_file = data_directory.join("applications").join(&file_name);
        if !desktop_file.is_file() {
            continue;
        }
        return read_bounded_text_file(&desktop_file, 1024 * 1024)
            .and_then(|contents| desktop_entry_terminal_launch_spec(&contents));
    }
    None
}

#[cfg(target_os = "linux")]
fn xdg_data_directories() -> Vec<PathBuf> {
    xdg_data_directories_from(
        env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
        env::var_os("XDG_DATA_DIRS"),
    )
}

#[cfg(target_os = "linux")]
fn xdg_data_directories_from(
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
    data_directories: Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(data_home) = data_home
        .filter(|value| !value.as_os_str().is_empty() && value.is_absolute())
        .or_else(|| {
            home.filter(|value| !value.as_os_str().is_empty() && value.is_absolute())
                .map(|home| home.join(".local/share"))
        })
    {
        directories.push(data_home);
    }
    let system_directories = data_directories
        .filter(|value| !value.is_empty())
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    directories.extend(
        system_directories
            .into_iter()
            .filter(|directory| directory.is_absolute()),
    );
    directories
}

#[cfg(target_os = "linux")]
fn desktop_entry_terminal_launch_spec(contents: &str) -> Option<TerminalLaunchSpec> {
    let exec = ini_section_value(contents, "[Desktop Entry]", "Exec")?.trim();
    let mut values = parse_desktop_exec(exec)?;
    let executable = sanitize_terminal_executable(values.first()?)?;
    values.remove(0);
    if discovered_terminal_embeds_child_command(&executable, &values)
        || wrapper_arguments_override_working_directory(&executable, &values)
    {
        return None;
    }
    Some(TerminalLaunchSpec {
        executable,
        arguments: values,
    })
}

#[cfg(target_os = "linux")]
fn command_line_terminal_launch_spec(value: &str) -> Option<TerminalLaunchSpec> {
    let mut values = parse_terminal_command_line(value)?;
    let executable = sanitize_terminal_executable(values.first()?)?;
    values.remove(0);
    if values
        .iter()
        .any(|argument| argument.len() > 4096 || argument.chars().any(char::is_control))
        || discovered_terminal_embeds_child_command(&executable, &values)
        || terminal_arguments_override_working_directory(&executable, &values)
        || wrapper_arguments_override_working_directory(&executable, &values)
    {
        return None;
    }
    Some(TerminalLaunchSpec {
        executable,
        arguments: values,
    })
}

#[cfg(target_os = "linux")]
fn wrapper_arguments_override_working_directory(executable: &str, arguments: &[String]) -> bool {
    wrapper_arguments_override_working_directory_inner(executable, arguments, 0)
}

#[cfg(target_os = "linux")]
fn wrapper_arguments_override_working_directory_inner(
    executable: &str,
    arguments: &[String],
    depth: usize,
) -> bool {
    if !is_env_executable(executable) {
        return false;
    }
    if depth >= 32 {
        return true;
    }

    let mut index = 0;
    let mut options_ended = false;
    while index < arguments.len() {
        let argument = &arguments[index];
        if !options_ended {
            if argument == "--" {
                options_ended = true;
                index += 1;
                continue;
            }
            if argument == "-C"
                || argument == "--chdir"
                || (argument.starts_with("-C") && argument.len() > 2)
                || argument.starts_with("--chdir=")
            {
                return true;
            }
            if argument == "-S"
                || (argument.starts_with("-S") && argument.len() > 2)
                || argument == "--split-string"
                || argument.starts_with("--split-string=")
                || argument.starts_with("--c")
                || argument.starts_with("--s")
            {
                return true;
            }
            if matches!(argument.as_str(), "-u" | "--unset" | "-a" | "--argv0") {
                index += 2;
                continue;
            }
            if argument.starts_with("--unset=") || argument.starts_with("--argv0=") {
                index += 1;
                continue;
            }
            if argument.starts_with("--u") || argument.starts_with("--a") {
                return true;
            }
            if argument.starts_with('-') {
                index += 1;
                continue;
            }
        }
        if argument
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'))
        {
            index += 1;
            continue;
        }
        return is_env_executable(argument)
            && wrapper_arguments_override_working_directory_inner(
                argument,
                &arguments[index + 1..],
                depth + 1,
            );
    }
    false
}

#[cfg(target_os = "linux")]
fn is_env_executable(executable: &str) -> bool {
    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("env")
}

#[cfg(target_os = "linux")]
fn terminal_arguments_override_working_directory(executable: &str, arguments: &[String]) -> bool {
    terminal_arguments_override_working_directory_inner(executable, arguments, 0)
}

#[cfg(target_os = "linux")]
fn terminal_arguments_override_working_directory_inner(
    executable: &str,
    arguments: &[String],
    depth: usize,
) -> bool {
    if depth >= 32 {
        return true;
    }
    let launcher_arguments = terminal_launcher_arguments(executable, arguments);
    let common_override = launcher_arguments.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "--working-directory"
                | "--default-working-directory"
                | "--directory"
                | "--workdir"
                | "--cwd"
                | "--chdir"
        ) || [
            "--working-directory=",
            "--default-working-directory=",
            "--directory=",
            "--workdir=",
            "--cwd=",
            "--chdir=",
        ]
        .iter()
        .any(|prefix| argument.starts_with(prefix))
    });
    if common_override {
        return true;
    }

    let executable_name = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase());
    let terminal_specific_override = terminal_specific_arguments_override_working_directory(
        executable_name.as_deref().unwrap_or_default(),
        launcher_arguments,
    );
    if terminal_specific_override {
        return true;
    }

    if executable_name.as_deref() == Some("flatpak")
        && flatpak_run_arguments(arguments).is_some_and(|run_arguments| {
            flatpak_application_ref_index(run_arguments).is_some_and(|ref_index| {
                let application_arguments = &run_arguments[ref_index + 1..];
                application_arguments.iter().any(|argument| {
                    argument == "-d"
                        || argument == "-D"
                        || argument == "-cd"
                        || (argument.starts_with("-d")
                            && !argument.starts_with("--")
                            && argument.len() > 2)
                        || (argument.starts_with("-D")
                            && !argument.starts_with("--")
                            && argument.len() > 2)
                }) || flatpak_env_command_overrides_working_directory(run_arguments, ref_index)
            })
        })
    {
        return true;
    }

    is_env_executable(executable)
        && env_wrapped_command(arguments).is_some_and(|(command, command_arguments)| {
            terminal_arguments_override_working_directory_inner(
                command,
                command_arguments,
                depth + 1,
            )
        })
}

#[cfg(target_os = "linux")]
fn flatpak_env_command_overrides_working_directory(arguments: &[String], ref_index: usize) -> bool {
    flatpak_application_command(arguments, ref_index).is_some_and(is_env_executable)
        && wrapper_arguments_override_working_directory("env", &arguments[ref_index + 1..])
}

#[cfg(target_os = "linux")]
fn flatpak_application_command(arguments: &[String], ref_index: usize) -> Option<&str> {
    let mut command = None;
    let mut index = 0;
    while index < ref_index {
        let argument = &arguments[index];
        if argument == "--" {
            break;
        }
        if let Some((option, value)) = argument.split_once('=') {
            if option == "--command" {
                command = Some(value);
            }
            index += 1;
            continue;
        }
        if argument == "--command" {
            let Some(value) = arguments.get(index + 1) else {
                return None;
            };
            command = Some(value.as_str());
            index += 2;
            continue;
        }
        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        index += 1;
    }
    command
}

#[cfg(target_os = "linux")]
fn flatpak_application_ref_index(arguments: &[String]) -> Option<usize> {
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--" {
            return (index + 1 < arguments.len()).then_some(index + 1);
        }
        if matches!(
            argument.as_str(),
            "--arch"
                | "--branch"
                | "--command"
                | "--cwd"
                | "--runtime"
                | "--runtime-version"
                | "--commit"
                | "--runtime-commit"
                | "--sandbox-expose"
                | "--sandbox-expose-ro"
                | "--env"
                | "--env-fd"
                | "--unset-env"
                | "--own-name"
                | "--talk-name"
                | "--no-talk-name"
                | "--system-own-name"
                | "--system-talk-name"
                | "--system-no-talk-name"
                | "--filesystem"
                | "--nofilesystem"
                | "--socket"
                | "--nosocket"
                | "--device"
                | "--nodevice"
                | "--allow"
                | "--disallow"
                | "--share"
                | "--unshare"
                | "--persist"
                | "--add-policy"
                | "--remove-policy"
                | "--parent-pid"
                | "--instance-id-fd"
                | "--app-path"
                | "--usr-path"
        ) {
            arguments.get(index + 1)?;
            index += 2;
            continue;
        }
        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(index);
    }
    None
}

#[cfg(target_os = "linux")]
fn flatpak_run_arguments(arguments: &[String]) -> Option<&[String]> {
    let mut index = 0;
    let mut options_ended = false;
    while index < arguments.len() {
        let argument = &arguments[index];
        if !options_ended {
            if argument == "--" {
                options_ended = true;
                index += 1;
                continue;
            }
            if matches!(argument.as_str(), "--installation" | "--arch") {
                arguments.get(index + 1)?;
                index += 2;
                continue;
            }
            if argument.starts_with('-') {
                index += 1;
                continue;
            }
        }
        return (argument == "run").then_some(&arguments[index + 1..]);
    }
    None
}

#[cfg(target_os = "linux")]
fn env_wrapped_command(arguments: &[String]) -> Option<(&str, &[String])> {
    let index = env_wrapped_command_index(arguments)?;
    Some((&arguments[index], &arguments[index + 1..]))
}

#[cfg(target_os = "linux")]
fn env_wrapped_command_index(arguments: &[String]) -> Option<usize> {
    let mut index = 0;
    let mut options_ended = false;
    while index < arguments.len() {
        let argument = &arguments[index];
        if !options_ended {
            if argument == "--" {
                options_ended = true;
                index += 1;
                continue;
            }
            if matches!(
                argument.as_str(),
                "-C" | "--chdir" | "-u" | "--unset" | "-a" | "--argv0" | "-S" | "--split-string"
            ) {
                index += 2;
                continue;
            }
            if argument.starts_with("--c") || argument.starts_with("--s") {
                return None;
            }
            if argument.starts_with("--unset=") || argument.starts_with("--argv0=") {
                index += 1;
                continue;
            }
            if argument.starts_with("--u") || argument.starts_with("--a") {
                return None;
            }
            if argument.starts_with('-') {
                index += 1;
                continue;
            }
        }
        if argument
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'))
        {
            index += 1;
            continue;
        }
        return Some(index);
    }
    None
}

#[cfg(target_os = "linux")]
fn parse_desktop_exec(value: &str) -> Option<Vec<String>> {
    let values = parse_desktop_exec_words(value)?;
    let mut normalized = Vec::new();
    for value in values {
        if matches!(
            value.as_str(),
            "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k"
        ) {
            if normalized
                .last()
                .is_some_and(|argument: &String| desktop_option_requires_value(argument))
            {
                return None;
            }
            continue;
        }
        let value = decode_desktop_percent_escapes(&value)?;
        if value.is_empty() {
            continue;
        }
        if value.len() > 4096 || value.chars().any(char::is_control) {
            return None;
        }
        normalized.push(value);
    }
    if normalized.is_empty()
        || terminal_arguments_override_working_directory(&normalized[0], &normalized[1..])
    {
        return None;
    }
    Some(normalized)
}

#[cfg(target_os = "linux")]
fn desktop_option_requires_value(argument: &str) -> bool {
    matches!(
        argument,
        "-T" | "--title"
            | "-c"
            | "--config"
            | "-o"
            | "--override"
            | "-d"
            | "-D"
            | "-cd"
            | "--working-directory"
            | "--default-working-directory"
            | "--directory"
            | "--workdir"
            | "--cwd"
            | "--chdir"
            | "--class"
            | "--name"
            | "--session"
            | "--instance-group"
            | "--listen-on"
            | "--start-as"
            | "--watcher"
            | "-e"
            | "--execute"
            | "--command"
    )
}

#[cfg(target_os = "linux")]
fn decode_desktop_percent_escapes(value: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            decoded.push(character);
            continue;
        }
        if characters.next()? != '%' {
            return None;
        }
        decoded.push('%');
    }
    Some(decoded)
}

#[cfg(target_os = "linux")]
fn parse_desktop_exec_words(value: &str) -> Option<Vec<String>> {
    if value.len() > 16_000
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return None;
    }
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut token_started = false;
    for character in value.chars() {
        if escaped {
            current.push(match character {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                '"' => '"',
                '`' => '`',
                '$' => '$',
                _ => return None,
            });
            escaped = false;
            token_started = true;
        } else if character == '\\' {
            escaped = true;
            token_started = true;
        } else if character == '"' {
            in_quotes = !in_quotes;
            token_started = true;
        } else if character.is_whitespace() && !in_quotes {
            if token_started {
                values.push(std::mem::take(&mut current));
                token_started = false;
            }
        } else {
            current.push(character);
            token_started = true;
        }
    }
    if escaped || in_quotes {
        return None;
    }
    if token_started {
        values.push(current);
    }
    (!values.is_empty()).then_some(values)
}

#[cfg(target_os = "linux")]
fn parse_terminal_command_line(value: &str) -> Option<Vec<String>> {
    if value.len() > 16_000
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return None;
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut values = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut token_started = false;
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match quote {
            Quote::None if character.is_whitespace() => {
                if token_started {
                    values.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            Quote::None if character == '\'' => {
                quote = Quote::Single;
                token_started = true;
            }
            Quote::None if character == '"' => {
                quote = Quote::Double;
                token_started = true;
            }
            Quote::None if character == '\\' => {
                current.push(characters.next()?);
                token_started = true;
            }
            Quote::Single if character == '\'' => quote = Quote::None,
            Quote::Double if character == '"' => quote = Quote::None,
            Quote::Double if character == '\\' => {
                let next = characters.next()?;
                if matches!(next, '"' | '\\' | '$' | '`') {
                    current.push(next);
                } else {
                    current.push('\\');
                    current.push(next);
                }
                token_started = true;
            }
            _ => {
                current.push(character);
                token_started = true;
            }
        }
    }
    if quote != Quote::None {
        return None;
    }
    if token_started {
        values.push(current);
    }
    (!values.is_empty()).then_some(values)
}

#[cfg(target_os = "linux")]
fn sanitize_terminal_executable(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || (value.chars().any(char::is_whitespace) && !Path::new(value).is_absolute())
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(target_os = "linux")]
fn kde_general_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    ini_section_value(contents, "[General]", key)
}

#[cfg(target_os = "linux")]
fn ini_section_value<'a>(contents: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_general_section = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_general_section = line.eq_ignore_ascii_case(section);
            continue;
        }
        if !in_general_section || line.starts_with('#') {
            continue;
        }
        let Some((candidate_key, value)) = line.split_once('=') else {
            continue;
        };
        if candidate_key.trim() == key {
            return Some(value.trim());
        }
    }
    None
}

fn validated_external_url(value: &str) -> Result<Url, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 {
        return Err(AppError::new("command_failed", "网页地址无效。"));
    }
    let url = Url::parse(value).map_err(|_| AppError::new("command_failed", "网页地址无效。"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::new(
            "command_failed",
            "只允许打开不含凭据的 HTTP 或 HTTPS 网页。",
        ));
    }
    Ok(url)
}

fn collect_system_fonts() -> Vec<SystemFontFamily> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    let mut families = BTreeMap::<String, (String, bool)>::new();
    for face in database.faces() {
        for (family, _) in &face.families {
            let family = family.trim();
            if family.is_empty() {
                continue;
            }
            let key = family.to_lowercase();
            let entry = families
                .entry(key)
                .or_insert_with(|| (family.to_string(), false));
            entry.1 |= face.monospaced;
        }
    }

    let mut result = families
        .into_values()
        .map(|(family, monospaced)| SystemFontFamily { family, monospaced })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        left.family
            .to_lowercase()
            .cmp(&right.family.to_lowercase())
            .then_with(|| left.family.cmp(&right.family))
    });
    result
}

#[cfg(target_os = "linux")]
fn platform_font_preferences() -> SystemFontPreferences {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let kde_session = env::var_os("KDE_FULL_SESSION").is_some()
        || desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("kde"));

    if kde_session {
        if let Some(preferences) = kde_font_preferences() {
            return preferences;
        }
    }

    let installed_families = installed_font_family_keys();
    let interface = gsettings_font_preference("font-name", &installed_families);
    let code = gsettings_font_preference("monospace-font-name", &installed_families);
    SystemFontPreferences {
        interface_family: interface
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("sans-serif")),
        code_family: code
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("monospace")),
        interface_size: interface.as_ref().and_then(|preference| preference.size),
        code_size: code.as_ref().and_then(|preference| preference.size),
        interface_weight: interface.as_ref().and_then(|preference| preference.weight),
        code_weight: code.as_ref().and_then(|preference| preference.weight),
    }
}

#[cfg(target_os = "windows")]
fn platform_font_preferences() -> SystemFontPreferences {
    use std::{ffi::c_void, mem::size_of};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
    };

    let mut metrics = NONCLIENTMETRICSW::default();
    metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    let loaded = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&mut metrics as *mut NONCLIENTMETRICSW).cast::<c_void>(),
            0,
        ) != 0
    };
    let interface_family = loaded
        .then(|| utf16_font_family(&metrics.lfMessageFont.lfFaceName))
        .flatten()
        .or_else(|| Some("Segoe UI".to_string()));
    SystemFontPreferences {
        interface_family,
        // Windows has no user-configurable global fixed-pitch UI font.
        // Consolas is the native system code-font fallback available across
        // all currently supported Windows versions.
        code_family: Some("Consolas".to_string()),
        interface_size: loaded.then_some(metrics.lfMessageFont.lfHeight.unsigned_abs() as f64),
        code_size: Some(13.0),
        interface_weight: loaded.then_some(metrics.lfMessageFont.lfWeight.clamp(100, 900) as u16),
        code_weight: Some(400),
    }
}

#[cfg(target_os = "windows")]
fn utf16_font_family(value: &[u16]) -> Option<String> {
    let length = value.iter().position(|character| *character == 0)?;
    let family = String::from_utf16(&value[..length]).ok()?;
    sanitize_font_family(&family)
}

#[cfg(target_os = "macos")]
fn platform_font_preferences() -> SystemFontPreferences {
    const USER_FIXED_PITCH_FONT: u32 = 1;
    const SYSTEM_FONT: u32 = 2;

    let native_interface_family = core_text_ui_font_family(SYSTEM_FONT);
    let interface_family = native_interface_family.map(|family| {
        if family.starts_with('.') {
            // CoreText intentionally exposes the system UI family through a
            // private name. WebKit's public token is the supported way to use it.
            "-apple-system".to_string()
        } else {
            family
        }
    });
    SystemFontPreferences {
        interface_family,
        code_family: core_text_ui_font_family(USER_FIXED_PITCH_FONT),
        interface_size: core_text_ui_font_size(SYSTEM_FONT),
        code_size: core_text_ui_font_size(USER_FIXED_PITCH_FONT),
        interface_weight: Some(400),
        code_weight: Some(400),
    }
}

#[cfg(target_os = "macos")]
fn core_text_ui_font_size(font_type: u32) -> Option<f64> {
    type CoreFoundationRef = *const c_void;

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateUIFontForLanguage(
            font_type: u32,
            size: f64,
            language: CoreFoundationRef,
        ) -> CoreFoundationRef;
        fn CTFontGetSize(font: CoreFoundationRef) -> f64;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CoreFoundationRef);
    }

    let font = unsafe { CTFontCreateUIFontForLanguage(font_type, 0.0, std::ptr::null()) };
    if font.is_null() {
        return None;
    }
    let size = unsafe { CTFontGetSize(font) };
    unsafe { CFRelease(font) };
    (size > 0.0).then_some(size)
}

#[cfg(target_os = "macos")]
fn core_text_ui_font_family(font_type: u32) -> Option<String> {
    type CoreFoundationRef = *const c_void;

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateUIFontForLanguage(
            font_type: u32,
            size: f64,
            language: CoreFoundationRef,
        ) -> CoreFoundationRef;
        fn CTFontCopyFamilyName(font: CoreFoundationRef) -> CoreFoundationRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CoreFoundationRef);
        fn CFStringGetLength(value: CoreFoundationRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            value: CoreFoundationRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
    }

    const UTF8_ENCODING: u32 = 0x0800_0100;
    let font = unsafe { CTFontCreateUIFontForLanguage(font_type, 0.0, std::ptr::null()) };
    if font.is_null() {
        return None;
    }
    let family = unsafe { CTFontCopyFamilyName(font) };
    if family.is_null() {
        unsafe { CFRelease(font) };
        return None;
    }
    let length = unsafe { CFStringGetLength(family) };
    let maximum_size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) };
    let result = if maximum_size < 0 || maximum_size > 4096 {
        None
    } else {
        let mut buffer = vec![0_u8; maximum_size as usize + 1];
        let copied = unsafe {
            CFStringGetCString(
                family,
                buffer.as_mut_ptr().cast::<c_char>(),
                buffer.len() as isize,
                UTF8_ENCODING,
            )
        };
        if copied == 0 {
            None
        } else {
            let value = unsafe { CStr::from_ptr(buffer.as_ptr().cast::<c_char>()) };
            value.to_str().ok().and_then(sanitize_font_family)
        }
    };
    unsafe {
        CFRelease(family);
        CFRelease(font);
    }
    result
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn platform_font_preferences() -> SystemFontPreferences {
    SystemFontPreferences::default()
}

#[cfg(target_os = "linux")]
fn read_kdeglobals() -> Option<String> {
    let config_path = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?
        .join("kdeglobals");
    read_bounded_text_file(&config_path, 1024 * 1024)
}

#[cfg(target_os = "linux")]
fn read_bounded_text_file(path: &Path, maximum_bytes: u64) -> Option<String> {
    let mut contents = String::new();
    let mut file = fs::File::open(path).ok()?;
    file.by_ref()
        .take(maximum_bytes + 1)
        .read_to_string(&mut contents)
        .ok()?;
    if contents.len() as u64 > maximum_bytes {
        return None;
    }
    Some(contents)
}

#[cfg(target_os = "linux")]
fn kde_font_preferences() -> Option<SystemFontPreferences> {
    let contents = read_kdeglobals()?;
    let interface = kde_general_font_preference(&contents, "font");
    let code = kde_general_font_preference(&contents, "fixed");
    if interface.is_none() && code.is_none() {
        return None;
    }
    Some(SystemFontPreferences {
        interface_family: interface
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("sans-serif")),
        code_family: code
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("monospace")),
        interface_size: interface.as_ref().and_then(|preference| preference.size),
        code_size: code.as_ref().and_then(|preference| preference.size),
        interface_weight: interface.as_ref().and_then(|preference| preference.weight),
        code_weight: code.as_ref().and_then(|preference| preference.weight),
    })
}

#[cfg(target_os = "linux")]
fn kde_general_font_preference(contents: &str, key: &str) -> Option<FontPreference> {
    let mut in_general_section = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_general_section = line.eq_ignore_ascii_case("[General]");
            continue;
        }
        if !in_general_section || line.starts_with('#') {
            continue;
        }
        let Some((candidate_key, value)) = line.split_once('=') else {
            continue;
        };
        if candidate_key.trim() == key {
            let fields = value.split(',').map(str::trim).collect::<Vec<_>>();
            let family = sanitize_font_family(fields.first().copied().unwrap_or_default())?;
            let size = fields
                .get(1)
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|size| *size > 0.0)
                .map(|points| points * 4.0 / 3.0);
            let weight = fields
                .get(4)
                .and_then(|value| value.parse::<u16>().ok())
                .map(css_weight_from_qt);
            return Some(FontPreference {
                family,
                size,
                weight,
            });
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn gsettings_font_preference(
    key: &str,
    installed_families: &BTreeSet<String>,
) -> Option<FontPreference> {
    let mut command = Command::new("gsettings");
    command.args(["get", "org.gnome.desktop.interface", key]);
    let output = command_output_with_timeout(command, Duration::from_secs(2))?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let unquoted = raw
        .trim()
        .trim_matches(|character| character == '\'' || character == '"');
    let (description, size) = unquoted
        .rsplit_once(' ')
        .filter(|(_, size)| size.parse::<f64>().is_ok())
        .map_or((unquoted, None), |(description, size)| {
            (
                description,
                size.parse::<f64>().ok().map(|points| points * 4.0 / 3.0),
            )
        });
    let (family, style) = split_pango_family_and_style(description, installed_families);
    let style = style.to_ascii_lowercase();
    let weight = if style.contains("extra bold") || style.contains("extrabold") {
        800
    } else if style.contains("black") || style.contains("heavy") {
        900
    } else if style.contains("semi bold") || style.contains("demi bold") {
        600
    } else if style.contains("bold") {
        700
    } else if style.contains("medium") {
        500
    } else if style.contains("light") {
        300
    } else if style.contains("thin") {
        100
    } else {
        400
    };
    Some(FontPreference {
        family: sanitize_font_family(family)?,
        size,
        weight: Some(weight),
    })
}

#[cfg(target_os = "linux")]
fn css_weight_from_qt(weight: u16) -> u16 {
    match weight {
        0..=6 => 100,
        7..=18 => 200,
        19..=37 => 300,
        38..=53 => 400,
        54..=60 => 500,
        61..=68 => 600,
        69..=78 => 700,
        79..=84 => 800,
        _ => 900,
    }
}

#[cfg(target_os = "linux")]
fn fontconfig_family(generic: &str) -> Option<String> {
    let mut command = Command::new("fc-match");
    command.args(["--format=%{family}", generic]);
    let output = command_output_with_timeout(command, Duration::from_secs(2))?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    sanitize_font_family(raw.split(',').next().unwrap_or_default())
}

#[cfg(target_os = "linux")]
fn command_output_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    const MAX_OUTPUT_BYTES: usize = 4096;
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let descriptor = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let mut output = Vec::new();
    let deadline = Instant::now() + timeout;
    loop {
        if !read_bounded_nonblocking(&mut stdout, &mut output, MAX_OUTPUT_BYTES) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !read_bounded_nonblocking(&mut stdout, &mut output, MAX_OUTPUT_BYTES) {
                    return None;
                }
                return Some(Output {
                    status,
                    stdout: output,
                    stderr: Vec::new(),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn read_bounded_nonblocking(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    maximum_bytes: usize,
) -> bool {
    let mut buffer = [0_u8; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return true,
            Ok(length) => {
                if output.len() + length > maximum_bytes {
                    return false;
                }
                output.extend_from_slice(&buffer[..length]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return true,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
}

#[cfg(target_os = "linux")]
fn split_pango_family_and_style<'a>(
    description: &'a str,
    installed_families: &BTreeSet<String>,
) -> (&'a str, String) {
    if let Some((family, style)) = description.split_once(',') {
        return (family.trim(), style.trim().to_string());
    }

    let mut family = description.trim();
    if installed_families.contains(&family.to_lowercase()) {
        return (family, String::new());
    }
    let mut styles = Vec::new();
    const STYLE_SUFFIXES: &[&str] = &[
        "Extra Bold",
        "Ultra Bold",
        "Semi Bold",
        "Demi Bold",
        "Extra Light",
        "Ultra Light",
        "Semi Light",
        "Demi Light",
        "ExtraBold",
        "UltraBold",
        "SemiBold",
        "DemiBold",
        "Condensed",
        "Expanded",
        "Oblique",
        "Italic",
        "Thin",
        "Light",
        "Book",
        "Regular",
        "Normal",
        "Medium",
        "Bold",
        "Heavy",
        "Black",
    ];
    loop {
        let lowercase_family = family.to_ascii_lowercase();
        let Some(style) = STYLE_SUFFIXES.iter().find(|style| {
            family.len() > style.len()
                && lowercase_family.ends_with(&style.to_ascii_lowercase())
                && family.as_bytes()[family.len() - style.len() - 1].is_ascii_whitespace()
        }) else {
            break;
        };
        styles.push(*style);
        family = family[..family.len() - style.len()].trim_end();
        if installed_families.contains(&family.to_lowercase()) {
            styles.reverse();
            return (family, styles.join(" "));
        }
    }
    (description.trim(), String::new())
}

#[cfg(target_os = "linux")]
fn installed_font_family_keys() -> BTreeSet<String> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    database
        .faces()
        .flat_map(|face| face.families.iter())
        .filter_map(|(family, _)| sanitize_font_family(family))
        .map(|family| family.to_lowercase())
        .collect()
}

fn sanitize_font_family(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_string())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{
        build_linux_terminal_candidates, desktop_entry_terminal_launch_spec,
        kde_general_font_preference, kde_terminal_candidates, kde_terminal_launch_specs,
        resolve_terminal_launch_spec_with_path, split_pango_family_and_style,
        xdg_data_directories_from,
    };
    use std::{collections::BTreeSet, ffi::OsStr, fs, os::unix::fs::PermissionsExt, path::PathBuf};
    use uuid::Uuid;

    #[test]
    fn reads_kde_font_families_from_general_section() {
        let config = r#"
[Icons]
font=Wrong Font,9
[General]
font=JetBrains Mono,10,-1,5,81,0,0,0,0,0,ExtraBold
fixed=Noto Sans Mono,11,-1,5,50,0,0,0,0,0
"#;

        let interface = kde_general_font_preference(config, "font").unwrap();
        assert_eq!(interface.family, "JetBrains Mono");
        assert_eq!(interface.size, Some(40.0 / 3.0));
        assert_eq!(interface.weight, Some(800));

        let code = kde_general_font_preference(config, "fixed").unwrap();
        assert_eq!(code.family, "Noto Sans Mono");
        assert_eq!(code.size, Some(44.0 / 3.0));
        assert_eq!(code.weight, Some(400));
    }

    #[test]
    fn reads_kde_default_terminal_from_general_section() {
        let config = r#"
[Wrong]
TerminalApplication=konsole
[General]
TerminalApplication=kitty
TerminalService=kitty.desktop
"#;

        let terminal = kde_terminal_launch_specs(config).remove(0);
        assert_eq!(terminal.executable, "kitty");
        assert!(terminal.arguments.is_empty());
    }

    #[test]
    fn empty_xdg_data_home_uses_the_home_directory_fallback() {
        for ignored_data_home in [PathBuf::new(), PathBuf::from("relative/share")] {
            let directories = xdg_data_directories_from(
                Some(ignored_data_home),
                Some(PathBuf::from("/home/example")),
                None,
            );

            assert_eq!(directories[0], PathBuf::from("/home/example/.local/share"));
            assert_eq!(directories[1], PathBuf::from("/usr/local/share"));
            assert_eq!(directories[2], PathBuf::from("/usr/share"));
        }
    }

    #[test]
    fn relative_home_does_not_create_a_relative_desktop_data_directory() {
        let directories =
            xdg_data_directories_from(None, Some(PathBuf::from("relative-home")), None);

        assert_eq!(
            directories,
            [
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ]
        );
    }

    #[test]
    fn parses_kde_terminal_application_arguments_without_a_shell() {
        let config = r#"
[General]
TerminalApplication=kitty --single-instance
TerminalService=invalid-bkm-terminal-service.desktop
"#;

        let terminal = kde_terminal_launch_specs(config).remove(0);
        assert_eq!(terminal.executable, "kitty");
        assert_eq!(terminal.arguments, ["--single-instance"]);
    }

    #[test]
    fn missing_kde_application_does_not_suppress_service_candidate() {
        let service = super::terminal_launch_spec("kitty");

        assert_eq!(
            kde_terminal_candidates(None, Some(service.clone())),
            [service]
        );
    }

    #[test]
    fn reads_terminal_executable_from_a_desktop_entry() {
        let desktop_entry = r#"
[Other]
Exec=wrong-terminal
[Desktop Entry]
Name=Example
Exec="/opt/Terminal App/bin/terminal" --new-window
"#;

        let terminal = desktop_entry_terminal_launch_spec(desktop_entry).unwrap();
        assert_eq!(terminal.executable, "/opt/Terminal App/bin/terminal");
        assert_eq!(terminal.arguments, ["--new-window"]);
    }

    #[test]
    fn preserves_desktop_entry_wrapper_arguments() {
        let desktop_entry = r#"
[Desktop Entry]
Exec=flatpak run org.example.Terminal --new-window
"#;

        let terminal = desktop_entry_terminal_launch_spec(desktop_entry).unwrap();
        assert_eq!(terminal.executable, "flatpak");
        assert_eq!(
            terminal.arguments,
            ["run", "org.example.Terminal", "--new-window"]
        );
    }

    #[test]
    fn decodes_desktop_entry_key_value_escapes() {
        let desktop_entry = r#"
[Desktop Entry]
Exec=kitty --title My\sTerminal --config C:\\work
"#;

        let terminal = desktop_entry_terminal_launch_spec(desktop_entry).unwrap();
        assert_eq!(terminal.executable, "kitty");
        assert_eq!(
            terminal.arguments,
            ["--title", "My Terminal", "--config", "C:\\work"]
        );

        let decoded_control = r#"
[Desktop Entry]
Exec=kitty "invalid\nargument"
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(decoded_control), None);

        let oversized_argument = format!("[Desktop Entry]\nExec=kitty {}\n", "a".repeat(4097));
        assert_eq!(
            desktop_entry_terminal_launch_spec(&oversized_argument),
            None
        );
    }

    #[test]
    fn handles_desktop_field_codes_without_orphaning_option_values() {
        let standalone_codes = r#"
[Desktop Entry]
Exec=kitty %f %F %u %U
"#;
        let terminal = desktop_entry_terminal_launch_spec(standalone_codes).unwrap();
        assert_eq!(terminal.executable, "kitty");
        assert!(terminal.arguments.is_empty());

        let trailing_code_after_flag = r#"
[Desktop Entry]
Exec=kitty --single-instance %U
"#;
        let terminal = desktop_entry_terminal_launch_spec(trailing_code_after_flag).unwrap();
        assert_eq!(terminal.arguments, ["--single-instance"]);

        let desktop_entry = r#"
[Desktop Entry]
Exec=kitty --working-directory %f
"#;

        assert_eq!(desktop_entry_terminal_launch_spec(desktop_entry), None);

        let embedded_code = r#"
[Desktop Entry]
Exec=kitty --working-directory=%F
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(embedded_code), None);

        let metadata_codes = r#"
[Desktop Entry]
Exec=kitty %i %c %k
"#;
        let terminal = desktop_entry_terminal_launch_spec(metadata_codes).unwrap();
        assert!(terminal.arguments.is_empty());

        let orphaned_title = r#"
[Desktop Entry]
Exec=kitty --title %c
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(orphaned_title), None);

        let wrapper_candidates = build_linux_terminal_candidates(None, Some("env -C /tmp kitty"));
        assert_eq!(wrapper_candidates[1].executable, "x-terminal-emulator");
    }

    #[test]
    fn decodes_embedded_desktop_percent_escapes() {
        let desktop_entry = r#"
[Desktop Entry]
Exec=kitty --title=100%%
"#;

        let terminal = desktop_entry_terminal_launch_spec(desktop_entry).unwrap();
        assert_eq!(terminal.arguments, ["--title=100%"]);

        let embedded_field_code = r#"
[Desktop Entry]
Exec=kitty%f --title=x%c
"#;
        assert_eq!(
            desktop_entry_terminal_launch_spec(embedded_field_code),
            None
        );
    }

    #[test]
    fn desktop_default_precedes_terminal_environment_and_alternatives() {
        let candidates = build_linux_terminal_candidates(
            Some("[General]\nTerminalApplication=kitty\n"),
            Some("x-terminal-emulator"),
        );
        let executables = candidates
            .iter()
            .map(|candidate| candidate.executable.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            &executables[..4],
            &[
                "xdg-terminal-exec",
                "kitty",
                "x-terminal-emulator",
                "gnome-terminal"
            ]
        );
    }

    #[test]
    fn detects_an_already_completed_terminal_startup_failure() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 1"])
            .spawn()
            .expect("the test shell is available");
        while child.try_wait().unwrap().is_none() {
            std::thread::yield_now();
        }

        let error = super::observe_terminal_startup(child, "terminal failed")
            .expect_err("a completed launcher failure must not be reported as success");
        assert_eq!(error.code(), "command_failed");
    }

    #[test]
    fn terminal_environment_supports_explicit_arguments() {
        let candidates = build_linux_terminal_candidates(
            None,
            Some("kitty --title 'My Terminal' --config C:\\\\work"),
        );

        assert_eq!(candidates[1].executable, "kitty");
        assert_eq!(
            candidates[1].arguments,
            ["--title", "My Terminal", "--config", "C:\\work"]
        );
    }

    #[test]
    fn configured_terminal_cannot_override_repository_working_directory() {
        let candidates =
            build_linux_terminal_candidates(None, Some("kitty --directory /tmp --single-instance"));

        assert_eq!(candidates[1].executable, "x-terminal-emulator");

        let desktop_entry = r#"
[Desktop Entry]
Exec=kitty --working-directory=/tmp
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(desktop_entry), None);

        let separated_env_chdir = r#"
[Desktop Entry]
Exec=env -C /tmp kitty
"#;
        assert_eq!(
            desktop_entry_terminal_launch_spec(separated_env_chdir),
            None
        );

        let joined_env_chdir = r#"
[Desktop Entry]
Exec=env -C/tmp kitty
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(joined_env_chdir), None);

        let nested_env_chdir = r#"
[Desktop Entry]
Exec=env FOO=bar -- /usr/bin/env -C /tmp kitty
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(nested_env_chdir), None);

        let deeply_nested_env = format!("[Desktop Entry]\nExec={}kitty\n", "env -- ".repeat(40));
        assert_eq!(desktop_entry_terminal_launch_spec(&deeply_nested_env), None);

        let split_string_chdir = r#"
[Desktop Entry]
Exec=env "-Senv -C /tmp kitty"
"#;
        assert_eq!(desktop_entry_terminal_launch_spec(split_string_chdir), None);

        for abbreviated_env_option in ["--c=/tmp", "--chdi=/tmp", "--s=env -C /tmp kitty"] {
            let desktop_entry =
                format!("[Desktop Entry]\nExec=env \"{abbreviated_env_option}\" kitty\n");
            assert_eq!(desktop_entry_terminal_launch_spec(&desktop_entry), None);
        }

        for abbreviated_value_option in ["--u NAME kitty -d /tmp", "--a NAME kitty -d /tmp"] {
            let desktop_entry = format!("[Desktop Entry]\nExec=env {abbreviated_value_option}\n");
            assert_eq!(desktop_entry_terminal_launch_spec(&desktop_entry), None);
        }

        for short_option_entry in [
            "[Desktop Entry]\nExec=kitty -d /tmp\n",
            "[Desktop Entry]\nExec=kitty -d/tmp\n",
            "[Desktop Entry]\nExec=foot -D /tmp\n",
            "[Desktop Entry]\nExec=footclient -D/tmp\n",
            "[Desktop Entry]\nExec=urxvt -cd /tmp\n",
            "[Desktop Entry]\nExec=env -- kitty -d /tmp\n",
            "[Desktop Entry]\nExec=flatpak run net.kovidgoyal.kitty -d /tmp\n",
            "[Desktop Entry]\nExec=flatpak --user run net.kovidgoyal.kitty -d /tmp\n",
            "[Desktop Entry]\nExec=flatpak --arch x86_64 run net.kovidgoyal.kitty -d /tmp\n",
            "[Desktop Entry]\nExec=flatpak run --command=env org.example.Terminal -C /tmp kitty\n",
            "[Desktop Entry]\nExec=flatpak run --env-fd 3 --command env org.example.Terminal -C /tmp kitty\n",
            "[Desktop Entry]\nExec=flatpak run --command env --log-session-bus org.example.Terminal -C /tmp kitty\n",
            "[Desktop Entry]\nExec=flatpak run --commit abc --command env org.example.Terminal -C /tmp kitty\n",
            "[Desktop Entry]\nExec=flatpak run --runtime-commit abc --command env org.example.Terminal -C /tmp kitty\n",
        ] {
            assert_eq!(desktop_entry_terminal_launch_spec(short_option_entry), None);
        }

        let terminal_argument = r#"
[Desktop Entry]
Exec=env -- kitty -C /tmp
"#;
        let terminal = desktop_entry_terminal_launch_spec(terminal_argument).unwrap();
        assert_eq!(terminal.executable, "env");
        assert_eq!(terminal.arguments, ["--", "kitty", "-C", "/tmp"]);

        for allowed_long_option in [
            "[Desktop Entry]\nExec=kitty --detach\n",
            "[Desktop Entry]\nExec=flatpak run -d org.example.Terminal\n",
            "[Desktop Entry]\nExec=flatpak run org.example.Terminal --device=dri\n",
            "[Desktop Entry]\nExec=flatpak run org.example.Terminal --compare=env -C /tmp\n",
            "[Desktop Entry]\nExec=flatpak run --commit=env org.example.Terminal -C /tmp\n",
        ] {
            assert!(desktop_entry_terminal_launch_spec(allowed_long_option).is_some());
        }
    }

    #[test]
    fn discovered_terminal_cannot_embed_a_child_command() {
        for desktop_entry in [
            "[Desktop Entry]\nExec=kitty -- some-tool\n",
            "[Desktop Entry]\nExec=xterm -e some-tool\n",
            "[Desktop Entry]\nExec=gnome-terminal --execute some-tool\n",
            "[Desktop Entry]\nExec=konsole --command some-tool\n",
            "[Desktop Entry]\nExec=env -- xterm -e some-tool\n",
            "[Desktop Entry]\nExec=flatpak run org.example.Terminal -- some-tool\n",
            "[Desktop Entry]\nExec=flatpak run --command=env org.example.Terminal xterm -e some-tool\n",
        ] {
            assert_eq!(desktop_entry_terminal_launch_spec(desktop_entry), None);
        }

        let candidates = build_linux_terminal_candidates(None, Some("xterm -e some-tool"));
        assert_eq!(candidates[1].executable, "x-terminal-emulator");
    }

    #[test]
    fn relative_path_entries_cannot_change_after_setting_repository_cwd() {
        let terminal = super::terminal_launch_spec("sh");

        assert_eq!(
            resolve_terminal_launch_spec_with_path(terminal, Some(OsStr::new(":relative"))),
            None
        );
    }

    #[test]
    fn resolved_terminal_identity_is_revalidated_after_canonicalization() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("bkm-system-terminal-link-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let kitty_target = root.join("kitty");
        fs::copy(std::env::current_exe().unwrap(), &kitty_target).unwrap();
        let selected_path = root.join("x-terminal-emulator");
        symlink(&kitty_target, &selected_path).unwrap();

        let terminal = super::TerminalLaunchSpec {
            executable: selected_path.to_string_lossy().into_owned(),
            arguments: vec!["-d".to_string(), "/tmp".to_string()],
        };
        assert_eq!(resolve_terminal_launch_spec_with_path(terminal, None), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn env_wrapped_terminal_command_is_resolved_without_relative_path_entries() {
        let root = std::env::temp_dir().join(format!("bkm-env-terminal-path-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let kitty = root.join("kitty");
        fs::copy(std::env::current_exe().unwrap(), &kitty).unwrap();
        let terminal = super::TerminalLaunchSpec {
            executable: "/usr/bin/env".to_string(),
            arguments: vec!["kitty".to_string()],
        };
        let path = std::env::join_paths([PathBuf::from("relative"), root.clone()]).unwrap();

        let resolved =
            resolve_terminal_launch_spec_with_path(terminal.clone(), Some(&path)).unwrap();
        assert_eq!(
            resolved.arguments[0],
            fs::canonicalize(&kitty).unwrap().to_string_lossy()
        );
        assert_eq!(
            resolve_terminal_launch_spec_with_path(terminal, Some(OsStr::new("relative"))),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_search_skips_an_earlier_non_executable_file() {
        let root = std::env::temp_dir().join(format!("bkm-terminal-path-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("terminal"), b"not executable").unwrap();
        let executable = second.join("terminal");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();

        let resolved = resolve_terminal_launch_spec_with_path(
            super::terminal_launch_spec("terminal"),
            Some(path.as_os_str()),
        )
        .unwrap();

        assert_eq!(
            fs::canonicalize(resolved.executable).unwrap(),
            fs::canonicalize(executable).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn separates_space_delimited_pango_weight_and_style() {
        let installed = BTreeSet::from([
            "arial black".to_string(),
            "cantarell".to_string(),
            "jetbrains mono".to_string(),
            "noto sans".to_string(),
        ]);
        assert_eq!(
            split_pango_family_and_style("Cantarell Bold Italic", &installed),
            ("Cantarell", "Bold Italic".to_string())
        );
        assert_eq!(
            split_pango_family_and_style("JetBrains Mono Extra Bold", &installed),
            ("JetBrains Mono", "Extra Bold".to_string())
        );
        assert_eq!(
            split_pango_family_and_style("Noto Sans", &installed),
            ("Noto Sans", String::new())
        );
        assert_eq!(
            split_pango_family_and_style("Arial Black", &installed),
            ("Arial Black", String::new())
        );
    }
}

#[cfg(test)]
mod terminal_config_tests {
    use super::{
        normalize_terminal_config, repository_placeholder_is_open_target,
        terminal_specific_arguments_override_working_directory,
        windows_path_requires_command_processor,
    };
    use crate::domain::TerminalConfig;

    #[test]
    fn preferred_terminal_requires_an_absolute_executable() {
        let error = normalize_terminal_config(TerminalConfig {
            executable: "kitty".to_string(),
            arguments: Vec::new(),
        })
        .expect_err("relative executable names must be rejected");

        assert_eq!(error.code(), "preferred_terminal_config_invalid");
    }

    #[test]
    fn preferred_terminal_arguments_are_preserved_without_a_shell() {
        #[cfg(target_os = "macos")]
        let executable = std::path::PathBuf::from("/usr/bin/open");
        #[cfg(not(target_os = "macos"))]
        let executable = std::env::current_exe().expect("test executable path is available");
        #[allow(unused_mut)]
        let mut arguments = vec!["  --single-instance  ".to_string(), "  ".to_string()];
        #[cfg(target_os = "macos")]
        arguments.push("{repository}".to_string());
        let expected_arguments = arguments.clone();
        let terminal = normalize_terminal_config(TerminalConfig {
            executable: executable.to_string_lossy().into_owned(),
            arguments,
        })
        .expect("the current test executable is launchable");

        assert!(std::path::Path::new(&terminal.executable).is_absolute());
        assert_eq!(terminal.arguments, expected_arguments);
    }

    #[test]
    fn windows_command_scripts_are_not_direct_executables() {
        assert!(windows_path_requires_command_processor(
            std::path::Path::new(r"C:\tools\terminal.CMD")
        ));
        assert!(windows_path_requires_command_processor(
            std::path::Path::new(r"C:\tools\terminal.bat")
        ));
        assert!(!windows_path_requires_command_processor(
            std::path::Path::new(r"C:\tools\terminal.exe")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_env_arguments_are_not_resolved_or_rewritten() {
        let arguments = vec![
            "PATH=/opt/custom-terminal/bin".to_string(),
            "my-terminal".to_string(),
            "--single-instance".to_string(),
        ];
        let terminal = normalize_terminal_config(TerminalConfig {
            executable: "/usr/bin/env".to_string(),
            arguments: arguments.clone(),
        })
        .expect("preferred env argv must remain a direct user configuration");

        assert_eq!(terminal.arguments, arguments);
    }

    #[test]
    fn macos_repository_placeholder_cannot_be_an_open_option_value() {
        for option in [
            "-a", "-b", "-s", "--arch", "--env", "--stdin", "--stdout", "--stderr",
        ] {
            let arguments = vec![option.to_string(), "{repository}".to_string()];
            assert!(!repository_placeholder_is_open_target(&arguments, 1));
        }
        let arguments = vec![
            "-a".to_string(),
            "Terminal".to_string(),
            "{repository}".to_string(),
        ];
        assert!(repository_placeholder_is_open_target(&arguments, 2));
    }

    #[test]
    fn windows_terminal_canonical_name_rejects_starting_directory_overrides() {
        let arguments = vec!["-d".to_string(), r"C:\other".to_string()];
        assert!(terminal_specific_arguments_override_working_directory(
            "windowsterminal.exe",
            &arguments,
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_terminal_preserves_legal_path_whitespace_until_canonicalization() {
        let root = std::env::temp_dir().join(format!(
            "bkm-preferred-terminal-whitespace-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let selected_path = root.join("terminal ");
        std::fs::copy(std::env::current_exe().unwrap(), &selected_path).unwrap();

        let terminal = normalize_terminal_config(TerminalConfig {
            executable: selected_path.to_string_lossy().into_owned(),
            arguments: Vec::new(),
        })
        .unwrap();

        assert_eq!(
            terminal.executable,
            std::fs::canonicalize(&selected_path)
                .unwrap()
                .to_string_lossy()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_terminal_binds_execution_to_the_validated_symlink_target() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "bkm-preferred-terminal-symlink-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let executable = std::env::current_exe().unwrap();
        let selected_path = root.join("terminal");
        symlink(executable, &selected_path).unwrap();

        let terminal = normalize_terminal_config(TerminalConfig {
            executable: selected_path.to_string_lossy().into_owned(),
            arguments: Vec::new(),
        })
        .unwrap();

        assert_eq!(
            terminal.executable,
            std::fs::canonicalize(&selected_path)
                .unwrap()
                .to_string_lossy()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_terminal_preserves_arguments_after_resolving_a_symlink_target() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "bkm-preferred-terminal-target-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let kitty_target = root.join("kitty");
        std::fs::copy(std::env::current_exe().unwrap(), &kitty_target).unwrap();
        let selected_path = root.join("terminal");
        symlink(&kitty_target, &selected_path).unwrap();

        let terminal = normalize_terminal_config(TerminalConfig {
            executable: selected_path.to_string_lossy().into_owned(),
            arguments: vec!["-d".to_string(), "/tmp".to_string()],
        })
        .expect("preferred argv is user-owned and must be preserved");

        assert_eq!(terminal.arguments, ["-d", "/tmp"]);
        assert_eq!(
            terminal.executable,
            std::fs::canonicalize(&kitty_target)
                .unwrap()
                .to_string_lossy()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_terminal_allows_direct_command_options_that_do_not_change_cwd() {
        let root = std::env::temp_dir().join(format!(
            "bkm-preferred-terminal-command-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let kitty = root.join("kitty");
        std::fs::copy(std::env::current_exe().unwrap(), &kitty).unwrap();

        let terminal = normalize_terminal_config(TerminalConfig {
            executable: kitty.to_string_lossy().into_owned(),
            arguments: vec![
                "--command".to_string(),
                "bash".to_string(),
                "--cwd".to_string(),
                "/tmp".to_string(),
            ],
        })
        .unwrap();

        assert_eq!(terminal.arguments, ["--command", "bash", "--cwd", "/tmp"]);

        let xterm = root.join("xterm");
        std::fs::copy(std::env::current_exe().unwrap(), &xterm).unwrap();
        let terminal = normalize_terminal_config(TerminalConfig {
            executable: xterm.to_string_lossy().into_owned(),
            arguments: vec![
                "-e".to_string(),
                "some-tool".to_string(),
                "--cwd".to_string(),
                "/tmp".to_string(),
            ],
        })
        .unwrap();
        assert_eq!(terminal.arguments, ["-e", "some-tool", "--cwd", "/tmp"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn env_wrapped_preferred_terminal_arguments_are_preserved() {
        let root = std::env::temp_dir().join(format!(
            "bkm-preferred-terminal-env-case-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let kitty = root.join("Kitty");
        std::fs::copy(std::env::current_exe().unwrap(), &kitty).unwrap();

        let terminal = normalize_terminal_config(TerminalConfig {
            executable: "/usr/bin/env".to_string(),
            arguments: vec![
                kitty.to_string_lossy().into_owned(),
                "-d".to_string(),
                "/tmp".to_string(),
            ],
        })
        .expect("preferred env argv is user-owned and must be preserved");

        assert_eq!(
            terminal.arguments,
            [
                kitty.to_string_lossy().into_owned(),
                "-d".to_string(),
                "/tmp".to_string()
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_terminal_preserves_cwd_named_arguments() {
        let executable = std::env::current_exe().expect("test executable path is available");
        let terminal = normalize_terminal_config(TerminalConfig {
            executable: executable.to_string_lossy().into_owned(),
            arguments: vec!["--working-directory=/tmp".to_string()],
        })
        .expect("arbitrary direct argv must be accepted");

        assert_eq!(terminal.arguments, ["--working-directory=/tmp"]);
    }
}

#[cfg(test)]
mod external_url_tests {
    use super::validated_external_url;

    #[test]
    fn external_urls_only_allow_credential_free_http_pages() {
        assert!(validated_external_url("https://github.com/example/project/pull/1").is_ok());
        assert!(validated_external_url("http://git.example.test/group/project").is_ok());
        assert!(validated_external_url("file:///tmp/project").is_err());
        assert!(validated_external_url("javascript:alert(1)").is_err());
        assert!(validated_external_url("https://token@example.test/project").is_err());
        assert!(
            validated_external_url("https://example.test/project?access_token=secret").is_err()
        );
        assert!(
            validated_external_url("https://example.test/project#access_token=secret").is_err()
        );
        assert!(validated_external_url("https://example.test/project?").is_err());
        assert!(validated_external_url("https://example.test/project#").is_err());
    }
}
