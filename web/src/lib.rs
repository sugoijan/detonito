use clap::Parser;
use wasm_bindgen::prelude::*;

mod game;
mod no_guess_worker;
mod settings;
mod sprites;
mod theme;
mod utils;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// What log level to use
    #[command(flatten)]
    verbose: clap_verbosity_flag::Verbosity,

    #[command(flatten)]
    init_settings: game::GameProps,
}

#[wasm_bindgen(start)]
pub fn run_app() {
    #[cfg(feature = "console_error_panic_hook")]
    {
        console_error_panic_hook::set_once();
    }

    if is_worker_context() {
        let _ = console_log::init_with_level(log::Level::Info);
        no_guess_worker::register_worker();
        return;
    }

    use gloo::utils::{document, window};

    let location_hash = window()
        .location()
        .hash()
        .unwrap_or_else(|_| "".to_string());

    let args = Args::try_parse_from(location_hash.split(['#', '&'])).expect("Could not parse args");
    let log_level = args.verbose.log_level().unwrap_or(log::Level::Info);
    let _ = console_log::init_with_level(log_level);

    theme::Theme::init();

    let root = document()
        .get_element_by_id("game")
        .expect("Could not find id=\"game\" element");

    log::debug!("App started");
    yew::Renderer::<game::GameView>::with_root_and_props(root, args.init_settings).render();
}

fn is_worker_context() -> bool {
    web_sys::window().is_none()
}
