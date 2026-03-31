use crate::hazard_variant::HazardVariant;
use crate::utils::*;
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

const AUTO_THEME_QUERY: &str = "(prefers-color-scheme: dark)";
const FAVICON_SELECTOR: &str = r#"link[data-dtn-favicon]"#;
const OPENMOJI_SYMBOLS: &str = include_str!("../generated/openmoji-symbols.svg");
const FAVICON_ICON_BOX_SIZE: f64 = 40.0;

thread_local! {
    static AUTO_THEME_LISTENER: RefCell<Option<AutoThemeListener>> = const { RefCell::new(None) };
}

struct AutoThemeListener {
    _media_query: web_sys::MediaQueryList,
    _onchange: Closure<dyn FnMut(JsValue)>,
}

#[derive(Clone, Copy)]
struct FaviconPalette {
    highlight: &'static str,
    primary: &'static str,
    shadow: &'static str,
    sprite_gray_light: &'static str,
    sprite_gray: &'static str,
    sprite_gray_dark: &'static str,
    sprite_red: &'static str,
    sprite_red_shade: &'static str,
    sprite_green: &'static str,
    sprite_green_shade: &'static str,
    ink: &'static str,
}

struct FaviconIcon {
    view_box: SvgViewBox,
    markup: String,
}

#[derive(Clone, Copy)]
struct SvgViewBox {
    min_x: f64,
    min_y: f64,
    width: f64,
    height: f64,
}

impl SvgViewBox {
    fn parse(value: &str) -> Option<Self> {
        let mut numbers = value
            .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
            .filter(|part| !part.is_empty())
            .map(str::parse::<f64>);
        let min_x = numbers.next()?.ok()?;
        let min_y = numbers.next()?.ok()?;
        let width = numbers.next()?.ok()?;
        let height = numbers.next()?.ok()?;
        if numbers.next().is_some() || width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some(Self {
            min_x,
            min_y,
            width,
            height,
        })
    }
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
                sprite_gray_light: "#e1e5e7",
                sprite_gray: "#b0b6ba",
                sprite_gray_dark: "#383838",
                sprite_red: "#ff554d",
                sprite_red_shade: "#e51308",
                sprite_green: "#47ab3d",
                sprite_green_shade: "#188f0d",
                ink: "#040404",
            },
            Self::Dark => FaviconPalette {
                highlight: "#5f5c5c",
                primary: "#404142",
                shadow: "#202121",
                sprite_gray_light: "#aaabab",
                sprite_gray: "#5f5c5c",
                sprite_gray_dark: "#202121",
                sprite_red: "#c35560",
                sprite_red_shade: "#c72727",
                sprite_green: "#47ab3d",
                sprite_green_shade: "#188f0d",
                ink: "#040404",
            },
        }
    }

    fn favicon_svg(self, hazard_variant: HazardVariant) -> String {
        let palette = self.favicon_palette();
        let Some(icon_group) =
            favicon_icon_group(hazard_variant.hidden_hazard_icon_name(), palette)
        else {
            log::error!(
                "failed to build favicon icon for hazard variant {:?}",
                hazard_variant
            );
            return self.legacy_favicon_svg();
        };

        format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">"#,
                r#"<rect width="64" height="64" fill="{shadow}"/>"#,
                r#"<rect x="6" y="6" width="52" height="52" fill="{primary}"/>"#,
                r#"<path fill="{highlight}" d="M0 0h64l-6 6H6v52L0 64V0z"/>"#,
                r#"<path fill="{shadow}" d="M64 0v64l-6-6V6zM0 64h64l-6-6H6L0 64z"/>"#,
                r#"{icon_group}"#,
                r#"</svg>"#
            ),
            highlight = palette.highlight,
            primary = palette.primary,
            shadow = palette.shadow,
            icon_group = icon_group,
        )
    }

    fn legacy_favicon_svg(self) -> String {
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
            mine = palette.sprite_gray_dark,
            mine_glint = palette.sprite_gray,
            ink = palette.ink,
        )
    }

    fn favicon_data_url(self, hazard_variant: HazardVariant) -> String {
        format!(
            "data:image/svg+xml,{}",
            js_sys::encode_uri_component(&self.favicon_svg(hazard_variant))
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
        let hazard_variant = HazardVariant::local_or_default();
        if let Err(err) = link.set_attribute("href", &resolved.favicon_data_url(hazard_variant)) {
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
                    Theme::refresh_favicon();
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

    pub(crate) fn refresh_favicon() {
        let theme: Option<Self> = LocalOrDefault::local_or_default();
        Self::update_favicon(theme);
    }
}

impl StorageKey for Theme {
    const KEY: &'static str = "detonito:theme";
}

fn favicon_icon_group(icon_name: &str, palette: FaviconPalette) -> Option<String> {
    let icon = favicon_icon(icon_name)?;
    let scale = FAVICON_ICON_BOX_SIZE / icon.view_box.width.max(icon.view_box.height);
    let x = 32.0 - ((icon.view_box.width * scale) / 2.0) - (icon.view_box.min_x * scale);
    let y = 32.0 - ((icon.view_box.height * scale) / 2.0) - (icon.view_box.min_y * scale);

    Some(format!(
        r#"<g transform="translate({x} {y}) scale({scale})">{markup}</g>"#,
        x = format_svg_number(x),
        y = format_svg_number(y),
        scale = format_svg_number(scale),
        markup = recolor_favicon_icon(icon.markup, palette),
    ))
}

fn favicon_icon(icon_name: &str) -> Option<FaviconIcon> {
    let document = Document::parse(OPENMOJI_SYMBOLS).ok()?;
    let symbol_id = format!("dtn-icon-{icon_name}");
    let symbol = document.descendants().find(|node| {
        node.has_tag_name("symbol") && node.attribute("id") == Some(symbol_id.as_str())
    })?;
    let view_box = SvgViewBox::parse(symbol.attribute("viewBox")?)?;

    let mut markup = String::new();
    for child in symbol.children().filter(|child| child.is_element()) {
        markup.push_str(&OPENMOJI_SYMBOLS[child.range()]);
    }

    Some(FaviconIcon { view_box, markup })
}

fn recolor_favicon_icon(mut markup: String, palette: FaviconPalette) -> String {
    for (token, value) in [
        ("var(--dtn-sprite-gray-light)", palette.sprite_gray_light),
        ("var(--dtn-sprite-gray)", palette.sprite_gray),
        ("var(--dtn-sprite-gray-dark)", palette.sprite_gray_dark),
        ("var(--dtn-sprite-red)", palette.sprite_red),
        ("var(--dtn-sprite-red-shade)", palette.sprite_red_shade),
        ("var(--dtn-sprite-green)", palette.sprite_green),
        ("var(--dtn-sprite-green-shade)", palette.sprite_green_shade),
        ("var(--dtn-sprite-ink)", palette.ink),
    ] {
        markup = markup.replace(token, value);
    }
    markup
}

fn format_svg_number(value: f64) -> String {
    let mut formatted = format!("{value:.4}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    if formatted == "-0" {
        "0".to_string()
    } else {
        formatted
    }
}

#[cfg(test)]
mod tests {
    use super::Theme;
    use crate::hazard_variant::HazardVariant;

    #[test]
    fn light_favicon_uses_closed_cell_palette() {
        let svg = Theme::Light.favicon_svg(HazardVariant::Mines);
        assert!(svg.contains("#e1e5e7"));
        assert!(svg.contains("#b0b6ba"));
        assert!(svg.contains("#73787b"));
        assert!(svg.contains("viewBox=\"0 0 64 64\""));
    }

    #[test]
    fn dark_favicon_uses_closed_cell_palette() {
        let svg = Theme::Dark.favicon_svg(HazardVariant::Mines);
        assert!(svg.contains("#5f5c5c"));
        assert!(svg.contains("#404142"));
        assert!(svg.contains("#202121"));
    }

    #[test]
    fn flower_favicon_uses_processed_rose_icon() {
        let svg = Theme::Light.favicon_svg(HazardVariant::Flowers);
        assert!(svg.contains("translate("));
        assert!(svg.contains("#ff554d"));
        assert!(svg.contains("#47ab3d"));
        assert!(!svg.contains("var(--dtn-sprite-red)"));
    }
}
