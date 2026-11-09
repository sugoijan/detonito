use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCredentials, RequestInit, Response};

use crate::runtime::app_path;

pub(crate) async fn fetch_request(
    method: &str,
    path: &str,
    json_body: Option<String>,
) -> Result<Response, String> {
    let init = RequestInit::new();
    init.set_method(method);
    init.set_credentials(RequestCredentials::Include);
    let has_body = json_body.is_some();
    if let Some(body) = json_body {
        init.set_body(&JsValue::from_str(&body));
    }
    let request = Request::new_with_str_and_init(&app_path(path), &init).map_err(js_error)?;
    if has_body {
        request
            .headers()
            .set("Content-Type", "application/json")
            .map_err(js_error)?;
    }
    let response = JsFuture::from(gloo::utils::window().fetch_with_request(&request))
        .await
        .map_err(js_error)?;
    response.dyn_into().map_err(js_error)
}

pub(crate) async fn post_request(
    path: &str,
    json_body: Option<String>,
) -> Result<Response, String> {
    let response = fetch_request("POST", path, json_body).await?;
    if response.ok() {
        Ok(response)
    } else {
        Err(format!("request failed with {}", response.status()))
    }
}

pub(crate) fn js_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}
