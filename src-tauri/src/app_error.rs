use serde::Serialize;
use std::fmt::{Display, Formatter};

/// A safe, structured error sent over Tauri IPC.
///
/// `details` must never contain a PAT, authorization header, or an unredacted
/// remote URL with embedded credentials.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    code: String,
    message: String,
    details: Option<String>,
}

impl AppError {
    /// Construct an IPC error from application-defined, compile-time-vetted
    /// text. Runtime diagnostics must never be passed here: they may contain
    /// a path, a remote URL, or credentials. Use [`Self::io`] for OS errors
    /// and [`Self::with_safe_details`] only for fixed remediation text.
    pub fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Attach a compile-time-vetted diagnostic.  Do not use this for process,
    /// I/O, HTTP, or parser errors: such messages can contain credentials.
    pub fn with_safe_details(
        code: &'static str,
        message: &'static str,
        details: &'static str,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Some(details.into()),
        }
    }

    /// I/O errors are deliberately not forwarded over IPC. OS diagnostics can
    /// include a credential-bearing path or command-line value.
    pub fn io(_action: &str, _error: impl Display) -> Self {
        Self::new("io_error", "本地操作未能完成。")
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    #[cfg(test)]
    pub fn details(&self) -> Option<&str> {
        self.details.as_deref()
    }
}

impl Display for AppError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn io_errors_do_not_forward_untrusted_diagnostics() {
        let error = AppError::io(
            "读取文件",
            "https://token-value@example.test/private repository",
        );
        assert_eq!(error.details(), None);
        assert!(!error.message().contains("token-value"));
        assert_eq!(error.message(), "本地操作未能完成。");
    }

    #[test]
    fn constructor_only_accepts_compile_time_vetted_text() {
        let error = AppError::new("example", "已知安全的说明。");
        assert_eq!(error.code(), "example");
        assert_eq!(error.message(), "已知安全的说明。");
    }
}
