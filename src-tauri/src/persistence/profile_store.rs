impl ProfileStore {
    pub fn all_endpoints(&self) -> Vec<CodeHostEndpoint> {
        let mut endpoints = CodeHostEndpoint::builtin();
        endpoints.extend(self.custom_endpoints.clone());
        endpoints
    }

    pub fn endpoint(&self, endpoint_id: &str) -> Option<CodeHostEndpoint> {
        self.all_endpoints()
            .into_iter()
            .find(|endpoint| endpoint.id == endpoint_id)
    }

    pub fn upsert_binding(&mut self, binding: RemoteBinding) {
        if let Some(existing) = self.remote_bindings.iter_mut().find(|existing| {
            existing.repository_path == binding.repository_path
                && existing.remote_name == binding.remote_name
        }) {
            *existing = binding;
        } else {
            self.remote_bindings.push(binding);
        }
    }

    pub fn binding_for(&self, repository_path: &str, remote_name: &str) -> Option<RemoteBinding> {
        self.remote_bindings
            .iter()
            .find(|binding| {
                binding.repository_path == repository_path && binding.remote_name == remote_name
            })
            .cloned()
    }

    pub fn repository_network_trust_for(
        &self,
        repository_path: &str,
    ) -> Option<RepositoryNetworkTrust> {
        self.repository_network_trusts
            .iter()
            .find(|trust| trust.repository_path == repository_path)
            .cloned()
    }

    pub fn upsert_repository_network_trust(&mut self, trust: RepositoryNetworkTrust) {
        self.repository_network_trusts
            .retain(|existing| existing.repository_path != trust.repository_path);
        self.repository_network_trusts.push(trust);
    }

    pub fn revoke_repository_network_trust(&mut self, repository_path: &str) {
        self.repository_network_trusts
            .retain(|trust| trust.repository_path != repository_path);
    }

    pub fn remove_account(&mut self, account_id: &str) -> Option<AccountProfile> {
        let index = self
            .accounts
            .iter()
            .position(|account| account.id == account_id)?;
        let removed = self.accounts.remove(index);
        self.remote_bindings.retain(|binding| {
            !matches!(
                &binding.account_selection,
                crate::domain::AccountSelection::Profile(profile_id) if profile_id == account_id
            )
        });
        self.pending_publishes
            .retain(|pending| pending.account_profile_id != account_id);
        self.pending_clones.retain(|pending| {
            !pending.binding.as_ref().is_some_and(|binding| {
                matches!(
                    &binding.account_selection,
                    crate::domain::AccountSelection::Profile(profile_id) if profile_id == account_id
                )
            })
        });
        Some(removed)
    }

    pub fn queue_secret_deletion(&mut self, keychain_ref: KeychainReference) {
        if !self
            .pending_secret_deletions
            .iter()
            .any(|existing| existing == &keychain_ref)
        {
            self.pending_secret_deletions.push(keychain_ref);
        }
    }

    pub fn clear_pending_secret_deletion(&mut self, keychain_ref: &KeychainReference) {
        self.pending_secret_deletions
            .retain(|existing| existing != keychain_ref);
    }

    pub fn queue_pending_publish(&mut self, pending: PendingPublish) {
        self.pending_publishes
            .retain(|existing| existing.repository_path != pending.repository_path);
        self.pending_publishes.push(pending);
    }

    pub fn pending_publish_for(&self, repository_path: &str) -> Option<PendingPublish> {
        self.pending_publishes
            .iter()
            .find(|pending| pending.repository_path == repository_path)
            .cloned()
    }

    pub fn clear_pending_publish(&mut self, repository_path: &str) {
        self.pending_publishes
            .retain(|pending| pending.repository_path != repository_path);
    }

    pub fn queue_pending_clone(&mut self, pending: PendingClone) {
        self.pending_clones
            .retain(|existing| existing.destination_path != pending.destination_path);
        self.pending_clones.push(pending);
    }

    pub fn clear_pending_clone(&mut self, destination_path: &str) {
        self.pending_clones
            .retain(|pending| pending.destination_path != destination_path);
    }

    /// Remove every durable record scoped to a repository path. This is used
    /// after a linked worktree is deleted so a later, unrelated repository
    /// created at the same path cannot inherit its endpoint/account binding.
    pub fn forget_repository_path(&mut self, repository_path: &str) {
        self.recent_repositories
            .retain(|repository| repository.path != repository_path);
        self.remote_bindings
            .retain(|binding| binding.repository_path != repository_path);
        self.repository_network_trusts
            .retain(|trust| trust.repository_path != repository_path);
        self.pending_publishes
            .retain(|pending| pending.repository_path != repository_path);
        self.pending_clones
            .retain(|pending| pending.destination_path != repository_path);
    }

    /// Register a repository discovered by Git while retaining local-only
    /// metadata already attached to it. Discovery is deliberately not treated
    /// as usage: only an explicit open updates `last_opened_at`.
    pub fn upsert_repository(&mut self, mut repository: RepositoryRecord) -> RepositoryRecord {
        if let Some(existing) = self
            .recent_repositories
            .iter_mut()
            .find(|existing| existing.path == repository.path)
        {
            repository.alias = existing.alias.clone();
            repository.group = existing.group.clone();
            repository.host = existing.host.clone();
            repository.namespace = existing.namespace.clone();
            repository.remote_name = existing.remote_name.clone();
            repository.remote_repository = existing.remote_repository.clone();
            repository.remote_host = existing.remote_host.clone();
            repository.remote_namespace = existing.remote_namespace.clone();
            repository.last_opened_at = existing.last_opened_at;
            *existing = repository.clone();
            return repository;
        }

        self.recent_repositories.insert(0, repository.clone());
        self.recent_repositories.truncate(MAX_REPOSITORIES);
        repository
    }

    /// Add a reviewed repository batch without evicting catalog entries that
    /// were already present. Existing records may still be refreshed, while
    /// new records are accepted only up to the remaining durable capacity.
    pub fn add_repositories(&mut self, repositories: Vec<RepositoryRecord>) -> Vec<RepositoryRecord> {
        let mut normalized_paths = std::collections::HashSet::new();
        self.recent_repositories
            .retain(|repository| normalized_paths.insert(repository.path.clone()));
        self.recent_repositories.truncate(MAX_REPOSITORIES);
        let mut existing_paths = self
            .recent_repositories
            .iter()
            .map(|repository| repository.path.clone())
            .collect::<std::collections::HashSet<_>>();
        let remaining_capacity = MAX_REPOSITORIES.saturating_sub(self.recent_repositories.len());
        let mut new_repository_count = 0;
        let mut accepted = Vec::new();
        let mut accepted_positions = std::collections::HashMap::new();
        for repository in repositories {
            if let Some(index) = accepted_positions.get(&repository.path).copied() {
                accepted[index] = self.upsert_repository(repository);
                continue;
            }
            let already_present = existing_paths.contains(&repository.path);
            if !already_present && new_repository_count >= remaining_capacity {
                continue;
            }
            let updated = self.upsert_repository(repository);
            if !already_present {
                existing_paths.insert(updated.path.clone());
                new_repository_count += 1;
            }
            accepted_positions.insert(updated.path.clone(), accepted.len());
            accepted.push(updated);
        }
        accepted
    }

    /// Merge the persisted local-only fields into a freshly read Git record.
    pub fn repository_with_metadata(&self, mut repository: RepositoryRecord) -> RepositoryRecord {
        if let Some(existing) = self
            .recent_repositories
            .iter()
            .find(|existing| existing.path == repository.path)
        {
            repository.alias = existing.alias.clone();
            repository.group = existing.group.clone();
            repository.host = existing.host.clone();
            repository.namespace = existing.namespace.clone();
            repository.remote_name = existing.remote_name.clone();
            repository.remote_repository = existing.remote_repository.clone();
            repository.remote_host = existing.remote_host.clone();
            repository.remote_namespace = existing.remote_namespace.clone();
            repository.last_opened_at = existing.last_opened_at;
        }
        repository
    }

    /// Mark an explicitly opened repository as recently used and refresh its
    /// safe remote owner metadata. A missing recognizable remote intentionally
    /// clears a stale owner group rather than showing incorrect old data.
    pub fn touch_repository(
        &mut self,
        repository: RepositoryRecord,
        host: Option<String>,
        namespace: Option<String>,
        opened_at: u64,
    ) -> RepositoryRecord {
        let remote_name = repository.remote_name.clone();
        let remote_repository = repository.remote_repository.clone();
        let remote_host = repository.remote_host.clone();
        let remote_namespace = repository.remote_namespace.clone();
        let repository = self.upsert_repository(repository);
        let index = self
            .recent_repositories
            .iter()
            .position(|existing| existing.path == repository.path)
            .expect("repository was just inserted or updated");
        let mut updated = self.recent_repositories.remove(index);
        updated.host = host;
        updated.namespace = namespace;
        updated.remote_name = remote_name;
        updated.remote_repository = remote_repository;
        updated.remote_host = remote_host;
        updated.remote_namespace = remote_namespace;
        updated.last_opened_at = Some(opened_at);
        // The collection is bounded from its tail, so a repository that was
        // explicitly opened must be promoted before subsequent discoveries
        // can evict it.
        self.recent_repositories.insert(0, updated.clone());
        updated
    }

    pub fn set_repository_group(
        &mut self,
        repository: RepositoryRecord,
        group: Option<String>,
    ) -> RepositoryRecord {
        let repository = self.upsert_repository(repository);
        let existing = self
            .recent_repositories
            .iter_mut()
            .find(|existing| existing.path == repository.path)
            .expect("repository was just inserted or updated");
        existing.group = group;
        existing.clone()
    }

    pub fn set_repository_alias(
        &mut self,
        repository: RepositoryRecord,
        alias: Option<String>,
    ) -> RepositoryRecord {
        let repository = self.upsert_repository(repository);
        let existing = self
            .recent_repositories
            .iter_mut()
            .find(|existing| existing.path == repository.path)
            .expect("repository was just inserted or updated");
        existing.alias = alias;
        existing.clone()
    }
}
