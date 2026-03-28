#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppRoute {
    Menu,
    Classic,
    Afk,
    Settings,
    About,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteState {
    pub route: AppRoute,
    pub afk_auth_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrontendRuntimeConfig {
    pub afk_enabled: bool,
}

pub(crate) fn frontend_runtime_config() -> FrontendRuntimeConfig {
    FrontendRuntimeConfig {
        afk_enabled: cfg!(feature = "afk-runtime"),
    }
}

pub(crate) fn app_base_path() -> String {
    let pathname = gloo::utils::window()
        .location()
        .pathname()
        .unwrap_or_else(|_| "/".to_string());
    let trimmed = pathname.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(feature = "afk-runtime")]
pub(crate) fn app_path(path: &str) -> String {
    let base_path = app_base_path();
    if base_path == "/" {
        path.to_string()
    } else {
        format!("{base_path}{path}")
    }
}

pub(crate) fn app_root_path() -> String {
    let base_path = app_base_path();
    if base_path == "/" {
        "/".to_string()
    } else {
        format!("{base_path}/")
    }
}

pub(crate) fn current_route_state() -> RouteState {
    let view = query_param("view");
    let route = match view.as_deref() {
        Some("classic") => AppRoute::Classic,
        #[cfg(feature = "afk-runtime")]
        Some("afk") => AppRoute::Afk,
        Some("settings") => AppRoute::Settings,
        Some("about") => AppRoute::About,
        _ => AppRoute::Menu,
    };
    RouteState {
        route,
        afk_auth_error: query_param("afk_auth_error"),
    }
}

pub(crate) fn route_return_to(route: AppRoute) -> String {
    let root = app_root_path();
    match route {
        AppRoute::Menu => root,
        AppRoute::Classic => format!("{root}?view=classic"),
        AppRoute::Afk => format!("{root}?view=afk"),
        AppRoute::Settings => format!("{root}?view=settings"),
        AppRoute::About => format!("{root}?view=about"),
    }
}

#[cfg(feature = "afk-runtime")]
pub(crate) fn auth_return_to(route: AppRoute) -> String {
    match route {
        AppRoute::Menu => "/".to_string(),
        AppRoute::Classic => "/?view=classic".to_string(),
        AppRoute::Afk => "/?view=afk".to_string(),
        AppRoute::Settings => "/?view=settings".to_string(),
        AppRoute::About => "/?view=about".to_string(),
    }
}

pub(crate) fn replace_route(route: AppRoute) {
    let history = match gloo::utils::window().history() {
        Ok(history) => history,
        Err(_) => return,
    };
    let _ = history.replace_state_with_url(
        &wasm_bindgen::JsValue::NULL,
        "",
        Some(&route_return_to(route)),
    );
}

#[cfg(feature = "afk-runtime")]
pub(crate) fn websocket_path(path: &str) -> String {
    if path.starts_with("ws://") || path.starts_with("wss://") {
        return path.to_string();
    }

    let origin = gloo::utils::window()
        .location()
        .origin()
        .unwrap_or_else(|_| "".to_string());
    let scheme = if origin.starts_with("https://") {
        "wss"
    } else {
        "ws"
    };
    let authority = origin
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let base_path = app_base_path();
    let resolved_path = if base_path != "/" {
        let prefixed = format!("{base_path}/");
        if path == base_path || path.starts_with(&prefixed) {
            path.to_string()
        } else {
            app_path(path)
        }
    } else {
        path.to_string()
    };
    format!("{scheme}://{authority}{resolved_path}")
}

fn query_param(name: &str) -> Option<String> {
    let search = gloo::utils::window()
        .location()
        .search()
        .unwrap_or_else(|_| "".to_string());
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|segment| !segment.is_empty())
        .find_map(|segment| {
            let (key, value) = segment.split_once('=').unwrap_or((segment, ""));
            if key == name && !value.is_empty() {
                Some(value.to_string())
            } else {
                None
            }
        })
}
