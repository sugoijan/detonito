use clap::Parser;
use wasm_bindgen::prelude::*;

mod game;
mod settings;
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
    use gloo::utils::{document, window};

    #[cfg(feature = "console_error_panic_hook")]
    {
        console_error_panic_hook::set_once();
    }

    let location_hash = window()
        .location()
        .hash()
        .unwrap_or_else(|_| "".to_string());

    let args = Args::try_parse_from(location_hash.split(['#', '&'])).expect("Could not parse args");
    if let Some(log_level) = args.verbose.log_level() {
        console_log::init_with_level(log_level).expect("Error initializing logger");
    }
    log::debug!("seed: {:?}", args.seed);

    theme::Theme::init();

    let root = document()
        .get_element_by_id("game")
        .expect("Could not find id=\"game\" element");

    log::debug!("App started");
    yew::Renderer::<game::GameView>::with_root(root).render();
}
