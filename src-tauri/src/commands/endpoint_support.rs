use super::*;

pub(super) fn validate_custom_endpoint(endpoint: &CodeHostEndpoint) -> Result<(), AppError> {
    if endpoint.id.trim().is_empty()
        || endpoint.display_name.trim().is_empty()
        || normalize_git_authority(&endpoint.git_host).is_none()
    {
        return Err(AppError::new(
            "invalid_endpoint",
            "Endpoint 标识或 Git host 无效。",
        ));
    }

    if matches!(endpoint.kind, CodeHostKind::Unknown) {
        return Err(AppError::new(
            "invalid_endpoint",
            "请为自定义 Git host 选择具体平台类型或“通用 Git 服务”。",
        ));
    }

    let requires_platform_urls = !matches!(endpoint.kind, CodeHostKind::GenericGit);
    for value in [&endpoint.web_url, &endpoint.api_url] {
        if value.is_empty() {
            if requires_platform_urls {
                return Err(AppError::new(
                    "invalid_endpoint",
                    "该平台 Endpoint 需要网页地址和 API 地址。",
                ));
            }
            continue;
        }
        let url = url::Url::parse(value)
            .map_err(|_| AppError::new("invalid_endpoint", "Endpoint URL 无效。"))?;
        if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
            return Err(AppError::new(
                "invalid_endpoint",
                "Endpoint URL 必须是 HTTP(S) 地址。",
            ));
        }
    }
    if endpoint.kind == CodeHostKind::GitHub
        && !git_authority_has_explicit_port(&endpoint.git_host)
        && !github_endpoint_has_gh_compatible_api(endpoint)
    {
        return Err(AppError::new(
            "invalid_endpoint",
            "GitHub Endpoint 的 API 地址必须与 gh 的 host 路由一致。",
        ));
    }
    Ok(())
}

/// Account auth methods and remote bindings are scoped to an Endpoint's
/// platform type and Git host. Editing cosmetic metadata is safe, but changing
/// that routing identity while it is referenced would make existing bindings
/// silently point at a different service.
pub(super) fn endpoint_routing_change_is_referenced(
    store: &ProfileStore,
    endpoint: &CodeHostEndpoint,
) -> bool {
    let Some(existing) = store
        .custom_endpoints
        .iter()
        .find(|existing| existing.id == endpoint.id)
    else {
        return false;
    };
    let routing_changed =
        existing.kind != endpoint.kind || !endpoint_matches_authority(existing, &endpoint.git_host);
    routing_changed
        && (store
            .accounts
            .iter()
            .any(|account| account.endpoint_id == endpoint.id)
            || store
                .remote_bindings
                .iter()
                .any(|binding| binding.endpoint_id == endpoint.id))
}

/// Automatic remote routing must have exactly one Endpoint for an authority.
/// Built-ins are included because custom metadata may not shadow a public
/// host such as github.com and silently make its account routing ambiguous.
pub(super) fn endpoint_git_host_conflicts(
    store: &ProfileStore,
    endpoint: &CodeHostEndpoint,
) -> bool {
    store.all_endpoints().iter().any(|existing| {
        existing.id != endpoint.id && endpoint_matches_authority(existing, &endpoint.git_host)
    })
}
