use wasm_bindgen::prelude::*;

mod game;
mod settings;
mod theme;
mod utils;

#[wasm_bindgen(start)]
pub fn run_app() {
    use gloo::utils::{document, window};
    use log::Level;
    use std::str::FromStr;

    const DEFAULT_LOG_LEVEL: Level = Level::Info;

    console_error_panic_hook::set_once();

    let location_hash = window()
        .location()
        .hash()
        .unwrap_or_else(|_| "".to_string());
    let log_level_str = location_hash.trim_start_matches("#");
    let log_level = Level::from_str(&log_level_str).unwrap_or(DEFAULT_LOG_LEVEL);
    console_log::init_with_level(log_level).expect("Error initializing logger");

    theme::Theme::init();

    let root = document()
        .get_element_by_id("game")
        .expect("Could not find id=\"game\" element");

    log::debug!("App started");
    yew::Renderer::<game::GameView>::with_root(root).render();
}
