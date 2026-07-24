use super::*;

/// Environment variables supplied only by backend code for one Git process.
/// It intentionally has no `Serialize` or `Debug` implementation so values
/// such as askpass configuration are not accidentally sent to the frontend or
/// emitted by standard debug logging.
#[derive(Default)]
pub struct GitEnvironment {
    variables: Vec<(OsString, OsString)>,
}

impl GitEnvironment {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one environment variable for a Git child process.
    pub fn set(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        let name = name.as_ref().to_os_string();
        let value = value.as_ref().to_os_string();
        if let Some((_, existing_value)) = self
            .variables
            .iter_mut()
            .find(|(existing_name, _)| *existing_name == name)
        {
            *existing_value = value;
        } else {
            self.variables.push((name, value));
        }
    }

    pub fn with_variable(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.set(name, value);
        self
    }

    /// Disable credential/configuration values inherited from an opened
    /// repository for an operation that uses the app-selected askpass source.
    /// Command-scope config has higher precedence than local/global/system
    /// config; an empty helper resets the configured helper list before Git
    /// could execute a repository-provided `credential.helper = !...` value.
    pub fn isolate_network_credentials(&mut self) {
        const CONFIGURATION: [(&str, &str); 6] = [
            ("credential.helper", ""),
            ("http.extraheader", ""),
            ("http.proxy", ""),
            ("http.sslverify", "true"),
            ("http.cookiefile", ""),
            ("http.savecookies", "false"),
        ];
        self.disable_ambient_network_configuration();
        self.set_network_credential_configuration(&CONFIGURATION);
    }

    /// Isolate an app-managed HTTPS credential and prevent repository or
    /// global Git hooks from inheriting its one-shot askpass capability. A
    /// hook runs inside Git's process tree and is not a trusted recipient of
    /// a PAT authorization token.
    pub fn isolate_network_credentials_and_disable_hooks(&mut self) {
        const CONFIGURATION: [(&str, &str); 7] = [
            ("credential.helper", ""),
            ("http.extraheader", ""),
            ("http.proxy", ""),
            ("http.sslverify", "true"),
            ("http.cookiefile", ""),
            ("http.savecookies", "false"),
            ("core.hooksPath", DISABLED_GIT_HOOKS_PATH),
        ];
        self.disable_ambient_network_configuration();
        self.set_network_credential_configuration(&CONFIGURATION);
    }

    /// A selected account is an explicit authorization decision. Global and
    /// system Git configuration can rewrite a validated URL or install a
    /// custom CA, neither of which is a trusted input for that decision.
    /// Repository-local network settings are checked separately before every
    /// network operation.
    fn disable_ambient_network_configuration(&mut self) {
        self.set("GIT_CONFIG_NOSYSTEM", "1");
        self.set("GIT_CONFIG_GLOBAL", DISABLED_GIT_HOOKS_PATH);
    }

    pub(super) fn set_network_credential_configuration(&mut self, configuration: &[(&str, &str)]) {
        self.variables.retain(|(name, _)| {
            let name = name.to_string_lossy();
            name != "GIT_CONFIG_COUNT"
                && !name.starts_with("GIT_CONFIG_KEY_")
                && !name.starts_with("GIT_CONFIG_VALUE_")
        });
        self.set("GIT_CONFIG_COUNT", configuration.len().to_string());
        for (index, (key, value)) in configuration.iter().enumerate() {
            self.set(format!("GIT_CONFIG_KEY_{index}"), key);
            self.set(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }
}

/// Every Git child starts from a credential-neutral environment. A parent
/// shell, IDE, or another app process must not silently choose an askpass
/// program or inject an alternate credential helper into a resolved operation.
pub(super) fn apply_git_environment(command: &mut Command, environment: &GitEnvironment) {
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
    // Git commands launched from the GUI cannot safely service an interactive
    // terminal prompt. CredentialEnvironment may replace this with the app's
    // trusted askpass helper for the selected account.
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        // Askpass prompt parsing is intentionally narrow. Force Git's stable
        // C locale so localized Username/Password prompts cannot make a
        // selected HTTPS credential fail unpredictably.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    for (name, value) in &environment.variables {
        command.env(name, value);
    }
}
