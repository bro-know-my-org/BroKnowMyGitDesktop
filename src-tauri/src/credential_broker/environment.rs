impl CredentialEnvironment {
    /// Build the environment plan for a resolved strategy.
    pub fn from_strategy(strategy: &CredentialStrategy) -> Result<Self, AppError> {
        Self::from_strategy_with_account(strategy)
    }

    /// Build the environment plan using the selected account's login for the
    /// Git username prompt.  This is preferred by callers that still have the
    /// resolution and account together.
    pub fn from_resolution(
        resolution: &CredentialResolution,
        account: &AccountProfile,
    ) -> Result<Self, AppError> {
        if resolution.account_profile_id != account.id
            || strategy_account_profile_id(&resolution.strategy) != resolution.account_profile_id
        {
            return Err(AppError::new(
                "credential_resolution_mismatch",
                "认证策略与所选账号不匹配。",
            ));
        }
        Self::from_strategy_with_account(&resolution.strategy)
    }

    fn from_strategy_with_account(strategy: &CredentialStrategy) -> Result<Self, AppError> {
        let askpass = match strategy {
            CredentialStrategy::KeychainPatAskPass {
                account_profile_id,
                keychain_ref,
                authority,
                username,
                ..
            } => Some(AskPassConfig::for_keychain(
                account_profile_id.clone(),
                keychain_ref.clone(),
                authority.clone(),
                username.clone(),
            )?),
        };

        Ok(Self {
            variables: Vec::new(),
            askpass,
        })
    }

    /// Environment pairs that do not depend on the askpass executable path.
    /// These are safe to hand to another backend module; none is a secret.
    pub fn neutral_environment_pairs(&self) -> &[(OsString, OsString)] {
        &self.variables
    }

    /// Returns askpass metadata, not a PAT.  The metadata is only useful to
    /// construct a child-process environment with a trusted helper path.
    pub fn askpass_config(&self) -> Option<&AskPassConfig> {
        self.askpass.as_ref()
    }

    /// Return all environment pairs needed by a Git child process.
    ///
    /// `askpass_program` must be the application's trusted askpass entry
    /// point, which calls [`run_keychain_askpass_from_args`].  The resulting
    /// pairs contain keychain references and a username, never the PAT.
    pub fn environment_pairs(
        &self,
        askpass_program: Option<&Path>,
    ) -> Result<Vec<(OsString, OsString)>, AppError> {
        let mut pairs = self.variables.clone();
        if let Some(askpass) = &self.askpass {
            let program = askpass_program.ok_or_else(|| {
                AppError::new(
                    "askpass_program_required",
                    "该认证方式需要应用的安全 askpass helper。",
                )
            })?;
            pairs.extend(askpass.environment_pairs(program)?);
        }
        Ok(pairs)
    }

    /// Apply this plan directly to a `std::process::Command`.  This uses
    /// process environment APIs only; it does not build a shell command.
    pub fn apply_to_command(
        &self,
        command: &mut Command,
        askpass_program: Option<&Path>,
    ) -> Result<(), AppError> {
        remove_inherited_git_credential_environment(command);
        for (key, value) in self.environment_pairs(askpass_program)? {
            command.env(key, value);
        }
        Ok(())
    }
}

fn remove_inherited_git_credential_environment(command: &mut Command) {
    for name in [
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_TERMINAL_PROMPT",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_GRAFT_FILE",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
    ] {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("GIT_CONFIG_") || normalized.starts_with("BKM_GIT_ASKPASS_") {
            command.env_remove(name);
        }
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
}

fn strategy_account_profile_id(strategy: &CredentialStrategy) -> &str {
    match strategy {
        CredentialStrategy::KeychainPatAskPass {
            account_profile_id, ..
        } => account_profile_id,
    }
}

/// Configuration for the private askpass helper. It is stored in a
/// short-lived, owner-only capability record rather than trusted directly
/// from a child process environment.
#[derive(Clone)]
pub struct AskPassConfig {
    source: AskPassSecretSource,
    username: String,
}

