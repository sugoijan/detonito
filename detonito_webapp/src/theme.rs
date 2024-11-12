use crate::utils::*;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Theme {
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

    pub(crate) fn init() {
        Self::update_html(LocalOrDefault::local_or_default());
    }

    pub(crate) fn apply(theme: Option<Self>) {
        theme.local_save();
        Self::update_html(theme);
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::Light
    }
}

impl StorageKey for Theme {
    const KEY: &'static str = "detonito:theme";
}
