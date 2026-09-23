mod diagnostic;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
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
            diagnostic: None,
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
            diagnostic: None,
        }
    }

    /// Runtime output may only cross IPC through this redaction boundary.
    pub(crate) fn with_diagnostic(mut self, diagnostic: &str) -> Self {
        self.diagnostic = diagnostic::sanitize(diagnostic);
        self
    }

    pub(crate) fn with_cleanup_failure(mut self) -> Self {
        self.details = Some("原操作失败，且未能恢复分支上游配置；请检查仓库配置。".into());
        self
    }

    pub fn io(_action: &str, error: impl Display) -> Self {
        Self::new("io_error", "本地操作未能完成。").with_diagnostic(&error.to_string())
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
        let json = serde_json::to_value(&error).unwrap();
        assert!(!json.to_string().contains("token-value"));
        assert!(json["diagnostic"].as_str().unwrap().contains("[REDACTED]"));
        assert_eq!(error.message(), "本地操作未能完成。");
    }

    #[test]
    fn io_errors_retain_actionable_os_diagnostics() {
        let error = AppError::io(
            "读取文件",
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        assert!(serde_json::to_value(error).unwrap()["diagnostic"]
            .as_str()
            .unwrap()
            .contains("permission denied"));
    }

    #[test]
    fn constructor_only_accepts_compile_time_vetted_text() {
        let error = AppError::new("example", "已知安全的说明。");
        assert_eq!(error.code(), "example");
        assert_eq!(error.message(), "已知安全的说明。");
    }
}
