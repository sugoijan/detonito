use clap::Parser;
use wasm_bindgen::prelude::*;

#[cfg(feature = "afk-runtime")]
mod afk;
#[cfg(not(feature = "afk-runtime"))]
#[path = "afk_disabled.rs"]
mod afk;
mod app;
mod board_input;
mod game;
mod hazard_variant;
mod menu;
mod no_guess_worker;
mod normal;
mod runtime;
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

    /// Force a seed instead of random
    #[arg(short, long)]
    seed: Option<String>,
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
    settings::Settings::init();

    let root = document()
        .get_element_by_id("game")
        .expect("Could not find id=\"game\" element");

    log::debug!("App started");
    yew::Renderer::<app::AppShell>::with_root_and_props(
        root,
        app::AppShellProps {
            init: game::GameInitArgs { seed: args.seed },
        },
    )
    .render();
}

fn is_worker_context() -> bool {
    web_sys::window().is_none()
}
