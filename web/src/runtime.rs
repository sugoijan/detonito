use serde::{Deserialize, Serialize};

use crate::utils::{LocalDelete, LocalOrDefault, LocalSave, StorageKey, browser_now_ms};

const ROUTE_STORAGE_TTL_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AppRoute {
    Menu,
    NormalMenu,
    Classic,
    Afk,
    Settings,
    SettingsNormal,
    SettingsAfk,
    About,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteState {
    pub route: AppRoute,
    pub afk_auth_error: Option<String>,
    pub afk_start_after_connect: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrontendRuntimeConfig {
    pub afk_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedRoute {
    route: AppRoute,
    updated_at_ms: i64,
}

impl PersistedRoute {
    fn fresh_route_at(&self, now_ms: i64) -> Option<AppRoute> {
        (now_ms.abs_diff(self.updated_at_ms) <= ROUTE_STORAGE_TTL_MS as u64)
            .then_some(self.route)
            .and_then(supported_route)
    }
}

impl StorageKey for PersistedRoute {
    const KEY: &'static str = "detonito:route";
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

pub(crate) fn initialize_route_state() -> RouteState {
    let now_ms = browser_now_ms();
    let view = query_param("view");
    let afk_auth_error = query_param("afk_auth_error");
    let afk_start_after_connect = query_param("afk_start").is_some();
    let route = resolve_startup_route(
        route_from_view(view.as_deref()),
        load_persisted_route(now_ms),
    );
    persist_route(route, now_ms);
    if view.is_some() || afk_auth_error.is_some() || afk_start_after_connect {
        replace_visible_url();
    }
    RouteState {
        route,
        afk_auth_error,
        afk_start_after_connect,
    }
}

#[cfg(feature = "afk-runtime")]
pub(crate) fn auth_return_to(route: AppRoute) -> String {
    match route_view(route) {
        Some(view) => format!("/?view={view}"),
        None => "/".to_string(),
    }
}

pub(crate) fn replace_route(route: AppRoute) {
    persist_route(route, browser_now_ms());
    replace_visible_url();
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

fn route_from_view(view: Option<&str>) -> Option<AppRoute> {
    match view {
        Some("menu") => Some(AppRoute::Menu),
        Some("normal") => Some(AppRoute::NormalMenu),
        Some("classic") => Some(AppRoute::Classic),
        #[cfg(feature = "afk-runtime")]
        Some("afk") => Some(AppRoute::Afk),
        Some("settings") => Some(AppRoute::Settings),
        Some("settings-normal") => Some(AppRoute::SettingsNormal),
        #[cfg(feature = "afk-runtime")]
        Some("settings-afk") => Some(AppRoute::SettingsAfk),
        Some("about") => Some(AppRoute::About),
        _ => None,
    }
}

#[cfg(feature = "afk-runtime")]
fn route_view(route: AppRoute) -> Option<&'static str> {
    match route {
        AppRoute::Menu => None,
        AppRoute::NormalMenu => Some("normal"),
        AppRoute::Classic => Some("classic"),
        AppRoute::Afk => Some("afk"),
        AppRoute::Settings => Some("settings"),
        AppRoute::SettingsNormal => Some("settings-normal"),
        AppRoute::SettingsAfk => Some("settings-afk"),
        AppRoute::About => Some("about"),
    }
}

fn supported_route(route: AppRoute) -> Option<AppRoute> {
    match route {
        AppRoute::Afk | AppRoute::SettingsAfk if !cfg!(feature = "afk-runtime") => None,
        _ => Some(route),
    }
}

fn resolve_startup_route(url_route: Option<AppRoute>, stored_route: Option<AppRoute>) -> AppRoute {
    url_route.or(stored_route).unwrap_or(AppRoute::Menu)
}

fn load_persisted_route(now_ms: i64) -> Option<AppRoute> {
    let stored = Option::<PersistedRoute>::local_or_default()?;
    let route = stored.fresh_route_at(now_ms);
    if route.is_none() {
        clear_persisted_route();
    }
    route
}

fn persist_route(route: AppRoute, now_ms: i64) {
    PersistedRoute {
        route,
        updated_at_ms: now_ms,
    }
    .local_save();
}

fn clear_persisted_route() {
    PersistedRoute::local_delete();
}

fn replace_visible_url() {
    let history = match gloo::utils::window().history() {
        Ok(history) => history,
        Err(_) => return,
    };
    let _ = history.replace_state_with_url(
        &wasm_bindgen::JsValue::NULL,
        "",
        Some(&clean_visible_url()),
    );
}

fn clean_visible_url() -> String {
    let root = app_root_path();
    let hash = gloo::utils::window()
        .location()
        .hash()
        .unwrap_or_else(|_| "".to_string());
    format!("{root}{hash}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_route_is_fresh_up_to_ttl() {
        let stored = PersistedRoute {
            route: AppRoute::Classic,
            updated_at_ms: 1_000,
        };

        assert_eq!(
            stored.fresh_route_at(1_000 + ROUTE_STORAGE_TTL_MS),
            Some(AppRoute::Classic)
        );
    }

    #[test]
    fn persisted_route_expires_after_ttl() {
        let stored = PersistedRoute {
            route: AppRoute::Classic,
            updated_at_ms: 1_000,
        };

        assert_eq!(stored.fresh_route_at(1_001 + ROUTE_STORAGE_TTL_MS), None);
    }

    #[test]
    fn startup_route_prefers_url_route() {
        assert_eq!(
            resolve_startup_route(Some(AppRoute::About), Some(AppRoute::Classic)),
            AppRoute::About
        );
    }

    #[test]
    fn startup_route_falls_back_to_persisted_route() {
        assert_eq!(
            resolve_startup_route(None, Some(AppRoute::SettingsNormal)),
            AppRoute::SettingsNormal
        );
    }

    #[test]
    fn startup_route_defaults_to_menu() {
        assert_eq!(resolve_startup_route(None, None), AppRoute::Menu);
    }

    #[test]
    fn menu_query_route_is_supported() {
        assert_eq!(route_from_view(Some("menu")), Some(AppRoute::Menu));
    }

    #[test]
    fn normal_query_route_is_supported() {
        assert_eq!(route_from_view(Some("normal")), Some(AppRoute::NormalMenu));
    }

    #[test]
    fn normal_settings_query_route_is_supported() {
        assert_eq!(
            route_from_view(Some("settings-normal")),
            Some(AppRoute::SettingsNormal)
        );
    }

    #[cfg(feature = "afk-runtime")]
    #[test]
    fn afk_route_is_supported_when_enabled() {
        let stored = PersistedRoute {
            route: AppRoute::Afk,
            updated_at_ms: 1_000,
        };

        assert_eq!(stored.fresh_route_at(1_000), Some(AppRoute::Afk));
    }

    #[cfg(feature = "afk-runtime")]
    #[test]
    fn afk_settings_route_is_supported_when_enabled() {
        let stored = PersistedRoute {
            route: AppRoute::SettingsAfk,
            updated_at_ms: 1_000,
        };

        assert_eq!(stored.fresh_route_at(1_000), Some(AppRoute::SettingsAfk));
    }

    #[cfg(not(feature = "afk-runtime"))]
    #[test]
    fn afk_route_is_rejected_when_disabled() {
        let stored = PersistedRoute {
            route: AppRoute::Afk,
            updated_at_ms: 1_000,
        };

        assert_eq!(stored.fresh_route_at(1_000), None);
    }

    #[cfg(not(feature = "afk-runtime"))]
    #[test]
    fn afk_settings_route_is_rejected_when_disabled() {
        let stored = PersistedRoute {
            route: AppRoute::SettingsAfk,
            updated_at_ms: 1_000,
        };

        assert_eq!(stored.fresh_route_at(1_000), None);
    }
}
