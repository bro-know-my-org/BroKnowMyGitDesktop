use regex::Regex;
use std::sync::OnceLock;

/// Redact before truncation so a size boundary cannot expose half a credential.
/// This is a diagnostic filter, not a parser for arbitrary secret formats.
pub(super) fn sanitize(raw: &str) -> Option<String> {
    static RULES: OnceLock<Vec<Regex>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            // Strip terminal escapes, including OSC hyperlinks.
            r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)",
            r"\x1b\[[0-?]*[ -/]*[@-~]",
            // Redact multiline keys before removing their opening line.
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
            // Omit entire credential-bearing lines, including header values.
            r#"(?im)^.*(?:authorization\s*:|proxy-authorization\s*:|cookie\s*:|\b[\w-]*(?:password|passwd|token|credential|secret|api[_-]?key|access[_-]?key|private[_-]?key)[\w-]*["']?\s*[=:])[ \t]*(?:\n[ \t]*)?[^\n]*(?:\n[ \t]+[^\n]*)*$"#,
            r"(?im)^.*(?:--(?:password|passwd|token|credential|secret|api[_-]?key|access[_-]?token)\s+|(?:^|[ \t])machine[ \t]+\S+.*[ \t]password\s+|^password\s+)\S.*$",
            // URLs can contain userinfo, query credentials, or secret path segments.
            r#"(?i)[a-z][a-z0-9+.-]*://[^\s<>"']+"#,
            r#"[^\s<>"'@]+@[^\s<>"']+"#,
            r"(?i)\b(?:gh[pousr]_|github_pat_|glpat[-_]|gldt-|glrt-|glcbt-|glptt-|gloas-)[a-z0-9_-]+",
            r"(?i)\b(?:bearer|basic)\s+[a-z0-9+/=_~.-]+",
            // Common JWT credentials.
            r"\beyJ[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+",
        ]
        .iter()
        .map(|pattern| Regex::new(pattern).expect("valid diagnostic redaction pattern"))
        .collect()
    });
    let mut text = raw.to_string();
    for (index, rule) in rules.iter().enumerate() {
        if index == 2 {
            // Normalize controls before matching secrets; removing them after
            // redaction could reconstruct a previously unmatched credential.
            text.retain(|c| !c.is_control() || c == '\n' || c == '\t');
        }
        if let std::borrow::Cow::Owned(redacted) =
            rule.replace_all(&text, if index < 2 { "" } else { "[REDACTED]" })
        {
            text = redacted;
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    const LIMIT: usize = 12_000;
    Some(if text.chars().take(LIMIT + 1).count() > LIMIT {
        // Git prints actionable failures after its progress output.
        let tail: Vec<char> = text.chars().rev().take(LIMIT / 2).collect();
        format!(
            "{}\n[…]\n{}",
            text.chars().take(LIMIT / 2).collect::<String>(),
            tail.iter().rev().collect::<String>()
        )
    } else {
        text.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn normalizes_controls_before_matching_credentials() {
        for raw in ["gh\x08p_controlsecret", "pass\0word=controlsecret"] {
            let safe = sanitize(raw).unwrap();
            assert!(!safe.contains("controlsecret"));
        }
    }

    #[test]
    fn redacts_provider_and_helper_secret_assignments() {
        for key in [
            "AWS_SECRET_ACCESS_KEY",
            "AWS_ACCESS_KEY_ID",
            "api-key",
            "GITLAB_TOKEN",
            "clientSecret",
        ] {
            let safe = sanitize(&format!("remote: {key}=provider-secret")).unwrap();
            assert!(!safe.contains("provider-secret"));
        }
    }

    #[test]
    fn redacts_cookie_headers_and_netrc_passwords() {
        for raw in [
            "Cookie: sessionid=context-secret",
            "Cookie:\n session=context-secret",
            "Cookie: first=value;\n session=context-secret",
            "password=\ncontext-secret",
            "helper --token context-secret",
            "password=-----BEGIN PRIVATE KEY-----\ncontext-secret\n-----END PRIVATE KEY-----",
            "Set-Cookie: sessionid=context-secret; HttpOnly",
            "machine example.test login alice password context-secret",
            "password\ncontext-secret",
        ] {
            let safe = sanitize(raw).unwrap();
            assert!(!safe.contains("context-secret"));
        }
    }

    #[test]
    fn retains_actionable_errors_and_redacts_credentials() {
        let raw = "remote: GH013: Repository rule violations\nremote: Changes must be made through a pull request.\nfatal: rejected https://user:secret@example.test/repo?token=hidden\nAuthorization: Bearer header-secret\npassword=local-secret\nremote: ghp_testsecret github_pat_othersecret glpat_gitlabsecret\nerror: custom server rejection";
        let safe = sanitize(raw).unwrap();
        for secret in [
            "user:secret",
            "hidden",
            "header-secret",
            "local-secret",
            "testsecret",
            "othersecret",
            "gitlabsecret",
        ] {
            assert!(!safe.contains(secret), "leaked {secret}");
        }
        assert!(safe.contains("Changes must be made through a pull request."));
        assert!(safe.contains("custom server rejection"));
        let prose = "remote: token ABC is required for release";
        assert_eq!(sanitize(prose).as_deref(), Some(prose));
    }

    #[test]
    fn strips_terminal_controls_and_keeps_unicode_and_tail() {
        let raw = format!(
            "\x1b[31m错误\x1b[0m\n{}\nremote: rejected by custom hook",
            "进度".repeat(10_000)
        );
        let safe = sanitize(&raw).unwrap();
        assert!(safe.starts_with("错误"));
        assert!(!safe.contains('\x1b'));
        assert!(safe.ends_with("remote: rejected by custom hook"));
        assert!(safe.chars().count() < 12_100);
        assert_eq!(sanitize(" \n\t"), None);
    }
}
