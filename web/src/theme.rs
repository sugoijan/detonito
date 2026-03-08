use crate::utils::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

const AUTO_THEME_QUERY: &str = "(prefers-color-scheme: dark)";
const FAVICON_SELECTOR: &str = r#"link[data-dtn-favicon]"#;

thread_local! {
    static AUTO_THEME_LISTENER: RefCell<Option<AutoThemeListener>> = const { RefCell::new(None) };
}

struct AutoThemeListener {
    _media_query: web_sys::MediaQueryList,
    _onchange: Closure<dyn FnMut(JsValue)>,
}

struct FaviconPalette {
    highlight: &'static str,
    primary: &'static str,
    shadow: &'static str,
    mine: &'static str,
    mine_glint: &'static str,
    ink: &'static str,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub(crate) enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    pub const ATTR_NAME: &'static str = "data-theme";

    pub(crate) const fn scheme(self) -> &'static str {
        use Theme::*;
        match self {
            Light => "light",
            Dark => "dark",
        }
    }

    fn preferred() -> Self {
        use gloo::utils::window;

        let prefers_dark = window()
            .match_media(AUTO_THEME_QUERY)
            .ok()
            .flatten()
            .is_some_and(|media_query| media_query.matches());

        if prefers_dark {
            Self::Dark
        } else {
            Self::Light
        }
    }

    fn resolved(theme: Option<Self>) -> Self {
        theme.unwrap_or_else(Self::preferred)
    }

    fn update_html(theme: Option<Self>) {
        use gloo::utils::document;
        let html = document()
            .query_selector("html")
            .expect("query must be correct")
            .expect("must have html element");
        if let Some(theme) = theme {
            let scheme = theme.scheme();
            log::debug!("theme-scheme: {}", scheme);
            if let Err(err) = html.set_attribute(Self::ATTR_NAME, scheme) {
                log::error!("failed to set theme: {:?}", err);
            }
        } else {
            log::debug!("no theme preference");
            if let Err(err) = html.remove_attribute(Self::ATTR_NAME) {
                log::error!("failed to set theme: {:?}", err);
            }
        }
    }

    fn favicon_palette(self) -> FaviconPalette {
        match self {
            Self::Light => FaviconPalette {
                highlight: "#e1e5e7",
                primary: "#b0b6ba",
                shadow: "#73787b",
                mine: "#383838",
                mine_glint: "#b0b6ba",
                ink: "#040404",
            },
            Self::Dark => FaviconPalette {
                highlight: "#5f5c5c",
                primary: "#404142",
                shadow: "#202121",
                mine: "#202121",
                mine_glint: "#5f5c5c",
                ink: "#040404",
            },
        }
    }

    fn favicon_svg(self) -> String {
        let palette = self.favicon_palette();

        format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">"#,
                r#"<rect width="64" height="64" fill="{shadow}"/>"#,
                r#"<rect x="6" y="6" width="52" height="52" fill="{primary}"/>"#,
                r#"<path fill="{highlight}" d="M0 0h64l-6 6H6v52L0 64V0z"/>"#,
                r#"<path fill="{shadow}" d="M64 0v64l-6-6V6zM0 64h64l-6-6H6L0 64z"/>"#,
                r#"<g stroke="{ink}" stroke-width="4.5" stroke-linecap="round">"#,
                r#"<line x1="32" y1="12" x2="32" y2="20"/>"#,
                r#"<line x1="32" y1="44" x2="32" y2="52"/>"#,
                r#"<line x1="12" y1="32" x2="20" y2="32"/>"#,
                r#"<line x1="44" y1="32" x2="52" y2="32"/>"#,
                r#"<line x1="18" y1="18" x2="23" y2="23"/>"#,
                r#"<line x1="41" y1="41" x2="46" y2="46"/>"#,
                r#"<line x1="18" y1="46" x2="23" y2="41"/>"#,
                r#"<line x1="41" y1="23" x2="46" y2="18"/>"#,
                r#"</g>"#,
                r#"<circle cx="32" cy="32" r="13" fill="{mine}"/>"#,
                r#"<ellipse cx="26.5" cy="26" rx="7" ry="3.5" transform="rotate(-35 26.5 26)" fill="{mine_glint}"/>"#,
                r#"<circle cx="32" cy="32" r="13" fill="none" stroke="{ink}" stroke-width="3"/>"#,
                r#"</svg>"#
            ),
            highlight = palette.highlight,
            primary = palette.primary,
            shadow = palette.shadow,
            mine = palette.mine,
            mine_glint = palette.mine_glint,
            ink = palette.ink,
        )
    }

    fn favicon_data_url(self) -> String {
        format!(
            "data:image/svg+xml,{}",
            js_sys::encode_uri_component(&self.favicon_svg())
        )
    }

    fn favicon_link() -> Option<web_sys::Element> {
        use gloo::utils::document;

        let document = document();
        if let Ok(Some(link)) = document.query_selector(FAVICON_SELECTOR) {
            return Some(link);
        }

        let head = document
            .query_selector("head")
            .expect("query must be correct")?;
        let link = match document.create_element("link") {
            Ok(link) => link,
            Err(err) => {
                log::error!("failed to create favicon link element: {:?}", err);
                return None;
            }
        };

        for (name, value) in [
            ("data-dtn-favicon", "true"),
            ("rel", "icon"),
            ("type", "image/svg+xml"),
            ("sizes", "any"),
        ] {
            if let Err(err) = link.set_attribute(name, value) {
                log::error!("failed to set favicon attribute {name}: {:?}", err);
                return None;
            }
        }

        if let Err(err) = head.append_child(&link) {
            log::error!("failed to append favicon link element: {:?}", err);
            return None;
        }

        Some(link)
    }

    fn update_favicon(theme: Option<Self>) {
        let Some(link) = Self::favicon_link() else {
            return;
        };

        let resolved = Self::resolved(theme);
        if let Err(err) = link.set_attribute("href", &resolved.favicon_data_url()) {
            log::error!("failed to set favicon href: {:?}", err);
        }
    }

    fn ensure_auto_theme_listener() {
        use gloo::utils::window;

        AUTO_THEME_LISTENER.with(|slot| {
            if slot.borrow().is_some() {
                return;
            }

            let Ok(Some(media_query)) = window().match_media(AUTO_THEME_QUERY) else {
                return;
            };

            let onchange = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
                let theme: Option<Theme> = LocalOrDefault::local_or_default();
                if theme.is_none() {
                    Theme::update_favicon(None);
                }
            });

            media_query.set_onchange(Some(onchange.as_ref().unchecked_ref()));
            slot.replace(Some(AutoThemeListener {
                _media_query: media_query,
                _onchange: onchange,
            }));
        });
    }

    pub(crate) fn init() {
        let theme: Option<Self> = LocalOrDefault::local_or_default();
        Self::ensure_auto_theme_listener();
        Self::update_html(theme);
        Self::update_favicon(theme);
    }

    pub(crate) fn apply(theme: Option<Self>) {
        theme.local_save();
        Self::update_html(theme);
        Self::update_favicon(theme);
    }
}

impl StorageKey for Theme {
    const KEY: &'static str = "detonito:theme";
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn light_favicon_uses_closed_cell_palette() {
        let svg = Theme::Light.favicon_svg();
        assert!(svg.contains("#e1e5e7"));
        assert!(svg.contains("#b0b6ba"));
        assert!(svg.contains("#73787b"));
        assert!(svg.contains("viewBox=\"0 0 64 64\""));
    }

    #[test]
    fn dark_favicon_uses_closed_cell_palette() {
        let svg = Theme::Dark.favicon_svg();
        assert!(svg.contains("#5f5c5c"));
        assert!(svg.contains("#404142"));
        assert!(svg.contains("#202121"));
    }
}
