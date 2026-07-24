// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Git invokes this same trusted executable as a fixed no-op editor while
    // continuing a rebase, cherry-pick, or revert. Handle that mode before
    // constructing Tauri so no GUI process is opened for Git's message file.
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(
            broknowmygitdesktop_lib::git_core::GIT_NOOP_EDITOR_ARGUMENT,
        ))
    {
        std::process::exit(0);
    }
    // Git starts the application binary as GIT_ASKPASS for one credential
    // prompt. Handle that private mode before the Tauri runtime is created.
    if broknowmygitdesktop_lib::credential_broker::askpass_invocation_requested() {
        let exit_code =
            match broknowmygitdesktop_lib::credential_broker::run_keychain_askpass_from_args() {
                Ok(()) => 0,
                Err(_) => 1,
            };
        std::process::exit(exit_code);
    }

    broknowmygitdesktop_lib::run()
}
