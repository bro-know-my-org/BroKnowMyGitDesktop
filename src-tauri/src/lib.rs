pub mod app_error;
pub mod codehost;
pub mod command_models;
pub mod commands;
pub mod credential_broker;
pub mod domain;
pub mod git_core;
pub mod github_adapter;
pub mod persistence;
pub mod process_tree;

use crate::persistence::AppState;
use tauri::webview::PageLoadEvent;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            app.manage(AppState::load(&app_data_dir)?);
            Ok(())
        })
        .on_page_load(|webview, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let window = webview.window();
            if window.show().is_err() {
                eprintln!("BroKnowMyGitDesktop could not reveal its main window.");
                return;
            }
            let _ = window.set_focus();
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_repositories,
            commands::discover_repositories,
            commands::init_repository,
            commands::clone_repository,
            commands::get_repository_snapshot,
            commands::refresh_repository_snapshot,
            commands::watch_repository_changes,
            commands::stop_repository_watcher,
            commands::set_repository_group,
            commands::set_repository_alias,
            commands::get_file_diff,
            commands::get_file_blame,
            commands::list_reflog,
            commands::preview_clean_untracked,
            commands::clean_untracked,
            commands::get_external_editor,
            commands::save_external_editor,
            commands::open_conflicted_files_in_external_editor,
            commands::list_commits,
            commands::get_commit_diff,
            commands::list_commit_changed_files,
            commands::get_commit_file_diff,
            commands::stage_paths,
            commands::unstage_paths,
            commands::apply_patch,
            commands::preview_discard_changes,
            commands::discard_changes,
            commands::commit_changes,
            commands::create_branch,
            commands::checkout_branch,
            commands::checkout_remote_branch,
            commands::rename_branch,
            commands::delete_branch,
            commands::merge_branch,
            commands::list_worktrees,
            commands::add_worktree,
            commands::remove_worktree,
            commands::list_submodules,
            commands::update_submodules,
            commands::get_lfs_status,
            commands::install_lfs,
            commands::track_lfs_pattern,
            commands::start_rebase,
            commands::cherry_pick_commits,
            commands::continue_git_operation,
            commands::abort_git_operation,
            commands::list_stashes,
            commands::stash_changes,
            commands::pop_stash,
            commands::drop_stash,
            commands::reset_branch,
            commands::squash_recent_commits,
            commands::revert_commit,
            commands::fetch_remote,
            commands::pull_remote,
            commands::push_remote,
            commands::get_repository_network_security,
            commands::approve_repository_network_configuration,
            commands::revoke_repository_network_trust,
            commands::list_accounts,
            commands::refresh_account_statuses,
            commands::save_pat_account,
            commands::save_account_profile,
            commands::remove_account,
            commands::import_cli_accounts,
            commands::set_default_account,
            commands::get_remote_binding,
            commands::save_remote_binding,
            commands::get_codehost_capabilities,
            commands::list_endpoints,
            commands::save_custom_endpoint,
            commands::create_tag,
            commands::delete_tag,
            commands::add_remote,
            commands::get_repository_remote_web_url,
            commands::set_remote_url,
            commands::rename_remote,
            commands::remove_remote,
            commands::get_codehost_repository_info,
            commands::list_pull_requests,
            commands::create_pull_request,
            commands::fork_github_repository,
            commands::list_github_publish_owners,
            commands::publish_github_repository,
            commands::list_system_fonts,
            commands::get_system_font_preferences,
            commands::get_system_terminal,
            commands::get_preferred_terminal,
            commands::save_preferred_terminal,
            commands::open_external_url,
            commands::open_repository_in_file_manager,
            commands::open_repository_in_terminal,
            commands::open_repository_in_preferred_terminal,
        ])
        .run(tauri::generate_context!());
    if result.is_err() {
        // At this point Tauri may not have created a window yet, so keep the
        // startup failure controlled and avoid panicking with an untyped
        // runtime error. Do not print the error object: platform diagnostics
        // can include user-controlled file paths.
        eprintln!("BroKnowMyGitDesktop failed to start.");
        std::process::exit(1);
    }
}
