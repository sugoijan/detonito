#[cfg(feature = "afk-runtime")]
use crate::runtime::app_path;
use crate::utils::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "afk-runtime")]
use wasm_bindgen::{JsCast, JsValue};
#[cfg(feature = "afk-runtime")]
use wasm_bindgen_futures::spawn_local;
#[cfg(feature = "afk-runtime")]
use web_sys::{Request, RequestCredentials, RequestInit};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum HazardVariant {
    #[default]
    Mines,
    Flowers,
}

impl HazardVariant {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Mines => "Mines",
            Self::Flowers => "Flowers",
        }
    }

    pub(crate) const fn hidden_hazard_icon_name(self) -> &'static str {
        match self {
            Self::Mines => "mine",
            Self::Flowers => "rose",
        }
    }

    pub(crate) const fn triggered_hazard_icon_name(self) -> &'static str {
        match self {
            Self::Mines => "mine-exploded",
            Self::Flowers => "wilted-flower",
        }
    }

    pub(crate) const fn cell_class(self) -> &'static str {
        match self {
            Self::Mines => "hazard-mines",
            Self::Flowers => "hazard-flowers",
        }
    }

    #[cfg(feature = "afk-runtime")]
    pub(crate) const fn to_afk_protocol(self) -> detonito_protocol::AfkHazardVariant {
        match self {
            Self::Mines => detonito_protocol::AfkHazardVariant::Mines,
            Self::Flowers => detonito_protocol::AfkHazardVariant::Flowers,
        }
    }

    #[cfg(feature = "afk-runtime")]
    pub(crate) const fn from_afk_protocol(value: detonito_protocol::AfkHazardVariant) -> Self {
        match value {
            detonito_protocol::AfkHazardVariant::Mines => Self::Mines,
            detonito_protocol::AfkHazardVariant::Flowers => Self::Flowers,
        }
    }

    #[cfg_attr(not(feature = "afk-runtime"), allow(dead_code))]
    pub(crate) fn mine_hit_message(self, actor_label: &str, coord_label: &str) -> String {
        match self {
            Self::Mines => format!("{actor_label} hit a mine at {coord_label}"),
            Self::Flowers => format!("{actor_label} stepped on a flower at {coord_label}"),
        }
    }

    pub(crate) fn apply(self) {
        self.local_save();
        crate::theme::Theme::refresh_favicon();
        #[cfg(feature = "afk-runtime")]
        self.sync_afk_session();
    }

    #[cfg(feature = "afk-runtime")]
    pub(crate) fn sync_afk_session(self) {
        spawn_local(async move {
            let _ = post_afk_variant(self).await;
        });
    }
}

impl StorageKey for HazardVariant {
    const KEY: &'static str = "detonito:hazard-variant";
}

#[cfg(feature = "afk-runtime")]
async fn post_afk_variant(hazard_variant: HazardVariant) -> Result<(), String> {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::Include);
    init.set_body(&JsValue::from_str(
        &serde_json::json!({
            "hazard_variant": hazard_variant.to_afk_protocol(),
        })
        .to_string(),
    ));
    let request =
        Request::new_with_str_and_init(&app_path("/api/afk/variant"), &init).map_err(js_error)?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(js_error)?;
    let response =
        wasm_bindgen_futures::JsFuture::from(gloo::utils::window().fetch_with_request(&request))
            .await
            .map_err(js_error)?;
    let response: web_sys::Response = response.dyn_into().map_err(js_error)?;
    if response.ok() {
        Ok(())
    } else {
        Err(format!("request failed with {}", response.status()))
    }
}

#[cfg(feature = "afk-runtime")]
fn js_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::HazardVariant;

    #[test]
    fn mines_variant_maps_to_mine_icons() {
        assert_eq!(HazardVariant::Mines.hidden_hazard_icon_name(), "mine");
        assert_eq!(
            HazardVariant::Mines.triggered_hazard_icon_name(),
            "mine-exploded"
        );
    }

    #[test]
    fn flowers_variant_maps_to_flower_icons() {
        assert_eq!(HazardVariant::Flowers.hidden_hazard_icon_name(), "rose");
        assert_eq!(
            HazardVariant::Flowers.triggered_hazard_icon_name(),
            "wilted-flower"
        );
        assert_eq!(HazardVariant::Flowers.cell_class(), "hazard-flowers");
    }

    #[test]
    fn flowers_variant_uses_flower_wording() {
        assert_eq!(
            HazardVariant::Flowers.mine_hit_message("Jan", "1A"),
            "Jan stepped on a flower at 1A"
        );
    }

    #[cfg(feature = "afk-runtime")]
    #[test]
    fn afk_protocol_variant_round_trips() {
        assert_eq!(
            HazardVariant::from_afk_protocol(HazardVariant::Flowers.to_afk_protocol()),
            HazardVariant::Flowers
        );
    }
}
