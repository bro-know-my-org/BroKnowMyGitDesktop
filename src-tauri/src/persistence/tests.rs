#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AccountSelection, AuthenticationMethod, AuthenticationStatus};

    fn repository(path: &str, name: &str) -> RepositoryRecord {
        RepositoryRecord {
            path: path.into(),
            name: name.into(),
            alias: None,
            group: None,
            host: None,
            namespace: None,
            remote_name: None,
            remote_repository: None,
            remote_host: None,
            remote_namespace: None,
            last_opened_at: None,
        }
    }

    #[test]
    fn serialized_profile_metadata_has_no_pat_field() {
        let store = ProfileStore {
            schema_version: 1,
            accounts: vec![AccountProfile {
                id: "account-1".into(),
                endpoint_id: "gitee.com".into(),
                login: "alice".into(),
                display_name: None,
                auth_method: AuthenticationMethod::PersonalAccessToken,
                secret_ref: None,
                status: AuthenticationStatus::Available,
                is_default_candidate: false,
            }],
            ..ProfileStore::default()
        };
        let json = serde_json::to_string(&store).expect("metadata serializes");
        assert!(!json.contains("token"));
        assert!(!json.contains("password"));
    }

    #[test]
    fn older_repository_records_load_without_grouping_fields() {
        let record: RepositoryRecord =
            serde_json::from_str(r#"{"path":"/work/project","name":"project"}"#)
                .expect("older persisted records remain compatible");

        assert_eq!(record.group, None);
        assert_eq!(record.alias, None);
        assert_eq!(record.host, None);
        assert_eq!(record.namespace, None);
        assert_eq!(record.remote_name, None);
        assert_eq!(record.remote_repository, None);
        assert_eq!(record.remote_host, None);
        assert_eq!(record.remote_namespace, None);
        assert_eq!(record.last_opened_at, None);
    }

    #[test]
    fn older_profile_stores_load_without_repository_editors() {
        let store: ProfileStore = serde_json::from_str(r#"{"schemaVersion":7}"#)
            .expect("the new repository editor list has a backward-compatible default");

        assert!(store.repository_editors.is_empty());
    }

    #[test]
    fn invalid_profile_store_is_backed_up_and_recovers_to_empty_state() {
        let directory = std::env::temp_dir().join(format!("bkm-profile-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(STORE_FILE_NAME), b"{ invalid json").unwrap();

        let state = AppState::load(&directory).expect("invalid metadata must not prevent launch");
        assert!(state.snapshot().unwrap().accounts.is_empty());
        assert!(!directory.join(STORE_FILE_NAME).exists());
        let backups = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("profiles.invalid.")
            })
            .count();
        assert_eq!(backups, 1);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clone_lease_prevents_two_operations_from_sharing_one_destination_intent() {
        let directory = std::env::temp_dir().join(format!("bkm-clone-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_clone_lease("/work/clone-target")
            .expect("first clone owns its destination");

        let error = match state.acquire_clone_lease("/work/clone-target") {
            Ok(_) => panic!("a concurrent clone must not overwrite the pending intent"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "clone_in_progress");

        drop(lease);
        assert!(state.acquire_clone_lease("/work/clone-target").is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn repository_operation_lease_serializes_conflicting_git_operations() {
        let directory =
            std::env::temp_dir().join(format!("bkm-repository-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_repository_operation_leases(&["/work/project", "/work/project-linked"])
            .expect("first operation owns both linked worktrees");

        let error = match state.acquire_repository_operation_lease("/work/project") {
            Ok(_) => panic!("conflicting Git operations must not overlap"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "repository_operation_in_progress");
        let linked_error = match state.acquire_repository_operation_lease("/work/project-linked") {
            Ok(_) => panic!("a linked worktree must be covered by the same lease"),
            Err(error) => error,
        };
        assert_eq!(linked_error.code(), "repository_operation_in_progress");

        drop(lease);
        assert!(state
            .acquire_repository_operation_leases(&["/work/project", "/work/project-linked"])
            .is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn account_operation_lease_blocks_concurrent_account_removal() {
        let directory = std::env::temp_dir().join(format!("bkm-account-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_account_operation_lease("account-1")
            .expect("a publish operation owns its selected account");

        let error = match state.acquire_account_operation_lease("account-1") {
            Ok(_) => panic!("account removal must not overlap a publish operation"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_operation_in_progress");

        drop(lease);
        assert!(state.acquire_account_operation_lease("account-1").is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn account_operation_leases_acquire_all_profiles_or_none() {
        let directory = std::env::temp_dir().join(format!("bkm-account-leases-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let existing = state
            .acquire_account_operation_lease("account-2")
            .expect("another operation owns the second profile");

        let error = match state.acquire_account_operation_leases(&["account-1", "account-2"]) {
            Ok(_) => panic!("a partially conflicting account group must not be acquired"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_operation_in_progress");
        // The failed group acquisition must not leave the first profile
        // reserved, otherwise a later import could spuriously block removal.
        assert!(state.acquire_account_operation_lease("account-1").is_ok());

        drop(existing);
        let group = state
            .acquire_account_operation_leases(&["account-1", "account-2"])
            .expect("both profiles become available together");
        let error = match state.acquire_account_operation_lease("account-2") {
            Ok(_) => panic!("a group lease protects every account"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_operation_in_progress");

        drop(group);
        assert!(state
            .acquire_account_operation_leases(&["account-1", "account-2"])
            .is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn cli_account_import_lease_serializes_matching_endpoint_profiles() {
        let directory =
            std::env::temp_dir().join(format!("bkm-cli-import-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GhCli)
            .expect("CLI import lease is acquired")
            .expect("GitHub CLI has an import lease");

        let error = match state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GhCli)
        {
            Ok(_) => panic!("matching CLI import cannot overlap"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_import_in_progress");
        assert!(state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GlabCli)
            .is_ok());
        assert!(state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::SshAgent)
            .expect("SSH does not use a CLI import lease")
            .is_none());

        drop(lease);
        assert!(state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GhCli)
            .is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn stale_account_status_refresh_is_not_committed() {
        let directory =
            std::env::temp_dir().join(format!("bkm-account-refresh-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let stale_generation = state
            .begin_account_status_refresh()
            .expect("first refresh generation is issued");
        state
            .mutate_and_invalidate_account_status_refreshes(|_| Ok(()))
            .expect("an account or Endpoint update invalidates the old refresh atomically");

        let committed = state
            .commit_account_status_refresh(stale_generation, |_| -> Result<(), AppError> {
                panic!("a stale refresh must not mutate the profile store")
            })
            .expect("stale generation is ignored safely");
        assert!(committed.is_none());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clean_untracked_confirmation_is_repository_scoped_and_one_shot() {
        let directory =
            std::env::temp_dir().join(format!("bkm-clean-confirmation-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        GitCore::new()
            .init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::create_dir_all(repository.join("generated"))
            .expect("untracked directory is created");
        std::fs::write(repository.join("generated/output.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let fingerprints = GitCore::new()
            .capture_clean_untracked_path_fingerprints(
                &repository,
                &["generated/output.txt".into()],
            )
            .expect("preview paths are captured");
        let token = state
            .issue_clean_untracked_confirmation(&repository_path, fingerprints)
            .expect("preview issues a token");

        assert_eq!(
            state
                .take_clean_untracked_confirmation(&repository_path, &token)
                .expect("matching token consumes its preview")
                .into_iter()
                .map(|fingerprint| fingerprint.path)
                .collect::<Vec<_>>(),
            vec!["generated/output.txt"]
        );
        assert_eq!(
            state
                .take_clean_untracked_confirmation(&repository_path, &token)
                .expect_err("a consumed preview cannot be replayed")
                .code(),
            "clean_untracked_confirmation_required"
        );

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn discard_confirmation_is_repository_scoped_and_one_shot() {
        let directory =
            std::env::temp_dir().join(format!("bkm-discard-confirmation-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let other_repository = directory.join("other-repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        git.init_repository(&other_repository, None)
            .expect("other repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let other_repository_path = other_repository.to_string_lossy().into_owned();
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");
        let token = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint)
            .expect("discard preview issues a token");

        assert_eq!(
            state
                .take_discard_changes_confirmation(&other_repository_path, &token)
                .expect_err("a token cannot authorize another repository")
                .code(),
            "discard_confirmation_required"
        );
        let fingerprint = state
            .take_discard_changes_confirmation(&repository_path, &token)
            .expect("the matching repository consumes its confirmation");
        assert_eq!(fingerprint.path_count(), 1);
        assert_eq!(
            state
                .take_discard_changes_confirmation(&repository_path, &token)
                .expect_err("a consumed confirmation cannot be replayed")
                .code(),
            "discard_confirmation_required"
        );

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn discard_confirmation_expires_and_is_removed() {
        let directory = std::env::temp_dir().join(format!(
            "bkm-discard-confirmation-expiry-{}",
            Uuid::new_v4()
        ));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");
        let token = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint)
            .expect("discard preview issues a token");
        {
            let mut confirmations = state
                .discard_changes_confirmations
                .lock()
                .expect("confirmation state is available");
            confirmations
                .get_mut(&repository_path)
                .expect("discard confirmation exists")
                .issued_at =
                Instant::now() - DISCARD_CHANGES_CONFIRMATION_TTL - Duration::from_secs(1);
        }

        assert_eq!(
            state
                .take_discard_changes_confirmation(&repository_path, &token)
                .expect_err("an expired confirmation cannot be consumed")
                .code(),
            "discard_confirmation_expired"
        );
        assert!(!state
            .discard_changes_confirmations
            .lock()
            .expect("confirmation state is available")
            .contains_key(&repository_path));

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn newer_discard_preview_supersedes_the_previous_token() {
        let directory = std::env::temp_dir().join(format!(
            "bkm-discard-confirmation-replace-{}",
            Uuid::new_v4()
        ));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");
        let first = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint.clone())
            .expect("first preview issues a token");
        let second = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint)
            .expect("second preview replaces the token");

        assert_ne!(first, second);
        assert_eq!(
            state
                .take_discard_changes_confirmation(&repository_path, &first)
                .expect_err("the older token is invalidated")
                .code(),
            "discard_confirmation_required"
        );
        state
            .take_discard_changes_confirmation(&repository_path, &second)
            .expect("the newest token remains valid");

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn discard_confirmation_store_evicts_the_oldest_repository() {
        let directory =
            std::env::temp_dir().join(format!("bkm-discard-confirmation-bound-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");

        for index in 0..MAX_DISCARD_CHANGES_CONFIRMATIONS {
            state
                .issue_discard_changes_confirmation(
                    &format!("repository-{index}"),
                    fingerprint.clone(),
                )
                .expect("bounded confirmation is issued");
        }
        state
            .discard_changes_confirmations
            .lock()
            .expect("confirmation state is available")
            .get_mut("repository-0")
            .expect("oldest confirmation exists")
            .issued_at = Instant::now() - Duration::from_secs(1);
        state
            .issue_discard_changes_confirmation(
                &format!("repository-{}", MAX_DISCARD_CHANGES_CONFIRMATIONS),
                fingerprint,
            )
            .expect("a newer repository evicts the oldest confirmation");
        let confirmations = state
            .discard_changes_confirmations
            .lock()
            .expect("confirmation state is available");
        assert_eq!(confirmations.len(), MAX_DISCARD_CHANGES_CONFIRMATIONS);
        assert!(!confirmations.contains_key("repository-0"));
        assert!(confirmations
            .contains_key(&format!("repository-{}", MAX_DISCARD_CHANGES_CONFIRMATIONS)));

        drop(confirmations);
        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn issuing_clean_confirmation_prunes_expired_previews_for_other_repositories() {
        let directory =
            std::env::temp_dir().join(format!("bkm-clean-confirmation-prune-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let stale_repository = directory.join("stale");
        let current_repository = directory.join("current");
        let git = GitCore::new();
        git.init_repository(&stale_repository, None)
            .expect("stale test repository initializes");
        git.init_repository(&current_repository, None)
            .expect("current test repository initializes");
        std::fs::write(stale_repository.join("old.txt"), "old\n")
            .expect("stale untracked file is created");
        std::fs::write(current_repository.join("current.txt"), "current\n")
            .expect("current untracked file is created");
        let stale_path = stale_repository.to_string_lossy().into_owned();
        let current_path = current_repository.to_string_lossy().into_owned();
        let stale_fingerprints = git
            .capture_clean_untracked_path_fingerprints(&stale_repository, &["old.txt".into()])
            .expect("stale preview paths are captured");
        state
            .issue_clean_untracked_confirmation(&stale_path, stale_fingerprints)
            .expect("stale preview is issued");
        {
            let mut confirmations = state
                .clean_untracked_confirmations
                .lock()
                .expect("confirmation state is available");
            confirmations
                .get_mut(&stale_path)
                .expect("stale preview exists")
                .issued_at =
                Instant::now() - CLEAN_UNTRACKED_CONFIRMATION_TTL - Duration::from_secs(1);
        }

        let current_fingerprints = git
            .capture_clean_untracked_path_fingerprints(&current_repository, &["current.txt".into()])
            .expect("current preview paths are captured");
        state
            .issue_clean_untracked_confirmation(&current_path, current_fingerprints)
            .expect("current preview is issued");
        let confirmations = state
            .clean_untracked_confirmations
            .lock()
            .expect("confirmation state is available");
        assert!(!confirmations.contains_key(&stale_path));
        assert!(confirmations.contains_key(&current_path));

        drop(confirmations);
        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn newer_profile_store_version_is_preserved_and_blocks_startup() {
        let directory =
            std::env::temp_dir().join(format!("bkm-profile-version-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(STORE_FILE_NAME),
            format!(
                r#"{{"schemaVersion":{},"futureField":{{"preserve":true}}}}"#,
                default_schema_version() + 1
            ),
        )
        .unwrap();

        let error = match AppState::load(&directory) {
            Ok(_) => panic!("newer metadata must not be overwritten"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "profile_store_version_unsupported");
        assert!(directory.join(STORE_FILE_NAME).exists());
        assert!(std::fs::read_to_string(directory.join(STORE_FILE_NAME))
            .unwrap()
            .contains("futureField"));
        let backups = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("profiles.invalid.")
            })
            .count();
        assert_eq!(backups, 0);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn saved_profile_store_is_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!("bkm-profile-mode-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        {
            let state = AppState::load(&directory).expect("profile store loads");
            state
                .mutate(|store| {
                    store.upsert_repository(repository("/work/private", "private"));
                    Ok(())
                })
                .expect("profile store saves");
        }

        let permissions = std::fs::metadata(directory.join(STORE_FILE_NAME))
            .expect("saved store exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(permissions, 0o600);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn repository_upsert_retains_local_display_metadata() {
        let mut store = ProfileStore::default();
        let mut first = repository("/work/project", "project");
        first.alias = Some("项目主仓".into());
        first.group = Some("工作".into());
        first.host = Some("github.com".into());
        first.namespace = Some("example-org".into());
        first.remote_name = Some("origin".into());
        first.remote_repository = Some("remote-project".into());
        first.last_opened_at = Some(123);
        store.upsert_repository(first);

        let updated = store.upsert_repository(repository("/work/project", "renamed-project"));

        assert_eq!(updated.name, "renamed-project");
        assert_eq!(updated.alias.as_deref(), Some("项目主仓"));
        assert_eq!(updated.group.as_deref(), Some("工作"));
        assert_eq!(updated.host.as_deref(), Some("github.com"));
        assert_eq!(updated.namespace.as_deref(), Some("example-org"));
        assert_eq!(updated.remote_name.as_deref(), Some("origin"));
        assert_eq!(updated.remote_repository.as_deref(), Some("remote-project"));
        let restored: ProfileStore = serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(restored.repository_with_metadata(repository("/work/project", "project")).remote_repository.as_deref(), Some("remote-project"));
        assert_eq!(updated.last_opened_at, Some(123));
    }

    #[test]
    fn repository_batch_add_preserves_existing_entries_at_capacity() {
        let existing = (0..MAX_REPOSITORIES)
            .map(|index| repository(&format!("/work/existing-{index}"), &format!("existing-{index}")))
            .collect::<Vec<_>>();
        let original_paths = existing
            .iter()
            .map(|repository| repository.path.clone())
            .collect::<std::collections::HashSet<_>>();
        let mut store = ProfileStore {
            recent_repositories: existing,
            ..ProfileStore::default()
        };

        let accepted = store.add_repositories(vec![
            repository("/work/new", "new"),
            repository("/work/existing-0", "refreshed"),
        ]);

        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].name, "refreshed");
        assert_eq!(store.recent_repositories.len(), MAX_REPOSITORIES);
        assert_eq!(
            store
                .recent_repositories
                .iter()
                .map(|repository| repository.path.clone())
                .collect::<std::collections::HashSet<_>>(),
            original_paths
        );
    }

    #[test]
    fn repository_batch_add_uses_only_remaining_capacity() {
        let mut store = ProfileStore {
            recent_repositories: (0..MAX_REPOSITORIES - 1)
                .map(|index| repository(&format!("/work/existing-{index}"), &format!("existing-{index}")))
                .collect(),
            ..ProfileStore::default()
        };

        let accepted = store.add_repositories(vec![
            repository("/work/first-new", "first-new"),
            repository("/work/second-new", "second-new"),
        ]);

        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].path, "/work/first-new");
        assert_eq!(store.recent_repositories.len(), MAX_REPOSITORIES);
        assert!(store
            .recent_repositories
            .iter()
            .all(|repository| repository.path != "/work/second-new"));
    }

    #[test]
    fn repository_batch_add_returns_one_last_wins_record_per_path() {
        let mut store = ProfileStore::default();

        let accepted = store.add_repositories(vec![
            repository("/work/project", "first"),
            repository("/work/project", "second"),
        ]);

        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].name, "second");
        assert_eq!(store.recent_repositories.len(), 1);
        assert_eq!(store.recent_repositories[0].name, "second");
    }

    #[test]
    fn repository_batch_add_repairs_duplicate_and_oversized_catalog_state() {
        let mut recent_repositories = vec![
            repository("/work/duplicate", "first"),
            repository("/work/duplicate", "second"),
        ];
        recent_repositories.extend((0..MAX_REPOSITORIES + 10).map(|index| {
            repository(&format!("/work/project-{index}"), &format!("project-{index}"))
        }));
        let mut store = ProfileStore {
            recent_repositories,
            ..ProfileStore::default()
        };

        let accepted = store.add_repositories(Vec::new());

        assert!(accepted.is_empty());
        assert_eq!(store.recent_repositories.len(), MAX_REPOSITORIES);
        assert_eq!(store.recent_repositories[0].name, "first");
        assert_eq!(
            store
                .recent_repositories
                .iter()
                .filter(|repository| repository.path == "/work/duplicate")
                .count(),
            1
        );
    }

    #[test]
    fn touching_repository_updates_usage_without_losing_manual_group() {
        let mut store = ProfileStore::default();
        let mut record = repository("/work/project", "project");
        record.alias = Some("客户端主仓".into());
        record.group = Some("客户端".into());
        store.upsert_repository(record);

        let mut fresh = repository("/work/project", "project");
        fresh.remote_name = Some("origin".into());
        fresh.remote_repository = Some("renamed-remote".into());
        let touched = store.touch_repository(
            fresh,
            Some("gitlab.com".into()),
            Some("team/platform".into()),
            456,
        );

        assert_eq!(touched.alias.as_deref(), Some("客户端主仓"));
        assert_eq!(touched.group.as_deref(), Some("客户端"));
        assert_eq!(touched.host.as_deref(), Some("gitlab.com"));
        assert_eq!(touched.namespace.as_deref(), Some("team/platform"));
        assert_eq!(touched.remote_repository.as_deref(), Some("renamed-remote"));
        assert_eq!(touched.last_opened_at, Some(456));
        let cleared = store.touch_repository(repository("/work/project", "project"), None, None, 789);
        assert_eq!(cleared.remote_name, None);
        assert_eq!(cleared.remote_repository, None);
    }

    #[test]
    fn touching_repository_promotes_it_before_later_discoveries() {
        let mut store = ProfileStore::default();
        store.upsert_repository(repository("/work/older", "older"));
        store.upsert_repository(repository("/work/newer", "newer"));

        store.touch_repository(repository("/work/older", "older"), None, None, 456);

        assert_eq!(store.recent_repositories[0].path, "/work/older");
        assert_eq!(store.recent_repositories[1].path, "/work/newer");
    }

    #[test]
    fn forgetting_a_worktree_path_clears_all_path_scoped_metadata() {
        let path = "/work/linked-worktree";
        let mut store = ProfileStore {
            recent_repositories: vec![repository(path, "linked-worktree")],
            remote_bindings: vec![RemoteBinding {
                repository_path: path.into(),
                remote_name: "origin".into(),
                endpoint_id: "github.com".into(),
                account_selection: AccountSelection::Auto,
            }],
            pending_publishes: vec![PendingPublish {
                repository_path: path.into(),
                endpoint_id: "github.com".into(),
                account_profile_id: "account-1".into(),
                remote_name: "origin".into(),
                expected_head: "0123456789012345678901234567890123456789".into(),
                expected_branch: "main".into(),
                expected_repository_slug: "octo/project".into(),
                remote_repository_id: Some("R_kgDOExample".into()),
            }],
            repository_network_trusts: vec![RepositoryNetworkTrust {
                repository_path: path.into(),
                fingerprint: "abc123".into(),
                approved_keys: vec!["merge.translation.driver".into()],
            }],
            ..ProfileStore::default()
        };

        store.forget_repository_path(path);

        assert!(store.recent_repositories.is_empty());
        assert!(store.remote_bindings.is_empty());
        assert!(store.pending_publishes.is_empty());
        assert!(store.repository_network_trusts.is_empty());
    }

    #[test]
    fn removing_an_account_clears_its_pending_clone_binding() {
        let mut store = ProfileStore {
            accounts: vec![AccountProfile {
                id: "account-1".into(),
                endpoint_id: "github.com".into(),
                login: "octo".into(),
                display_name: None,
                auth_method: AuthenticationMethod::GhCli,
                secret_ref: None,
                status: AuthenticationStatus::Valid,
                is_default_candidate: false,
            }],
            pending_clones: vec![PendingClone {
                destination_path: "/work/pending-clone".into(),
                binding: Some(RemoteBinding {
                    repository_path: "/work/pending-clone".into(),
                    remote_name: "origin".into(),
                    endpoint_id: "github.com".into(),
                    account_selection: AccountSelection::Profile("account-1".into()),
                }),
                expected_remote_authority: "github.com".into(),
                expected_repository_slug: "octo/pending-clone".into(),
            }],
            ..ProfileStore::default()
        };

        assert!(store.remove_account("account-1").is_some());
        assert!(store.pending_clones.is_empty());
    }
}
