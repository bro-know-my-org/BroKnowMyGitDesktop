use super::editor::*;
use super::environment::*;
use super::error::*;
use super::history::{parse_exact_history_ref, trim_incomplete_utf8_suffix};
use super::models::*;
use super::parser::*;
use super::runtime::read_git_preview_pipe;
use super::validation::*;

use super::*;
use crate::domain::CommitFileStatus;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

fn as_strings(arguments: &[OsString]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

fn test_git_template_dir() -> &'static Path {
    static TEMPLATE_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    TEMPLATE_DIRECTORY
        .get_or_init(|| {
            // A fixed template path can retain files from an earlier failed
            // test run (or be pre-created), which would make every `git init`
            // fixture inherit untrusted config. Keep one fresh, randomized
            // empty directory alive for this process instead.
            let directory = std::env::temp_dir().join(format!(
                "broknowmygitdesktop-empty-git-template-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&directory).expect("the test Git template directory is available");
            directory
        })
        .as_path()
}

/// Builds a Git command for integration-test setup. Tests must not inherit a
/// developer's repository selection, configuration, hooks, or template
/// directory: those can make an otherwise local fixture operate on the wrong
/// repository or execute a user-installed hook.
fn test_git_command() -> Command {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", DISABLED_GIT_HOOKS_PATH)
        .env("GIT_TEMPLATE_DIR", test_git_template_dir())
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("-c")
        .arg(format!("core.hooksPath={DISABLED_GIT_HOOKS_PATH}"));
    command
}

struct TemporaryTestDirectory(PathBuf);

impl TemporaryTestDirectory {
    fn create(path: PathBuf) -> Self {
        let base_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("test");
        let path = path.with_file_name(format!("{base_name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).expect("temporary test directory is exclusively available");
        Self(path)
    }
}

impl std::ops::Deref for TemporaryTestDirectory {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TemporaryTestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Keep shared fixtures in one test module while grouping scenarios by domain.
include!("tests/repository.rs");
include!("tests/changes.rs");
include!("tests/history_network.rs");
include!("tests/safety.rs");
