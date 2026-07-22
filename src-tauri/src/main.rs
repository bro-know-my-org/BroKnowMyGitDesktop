// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
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
