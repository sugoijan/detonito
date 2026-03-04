#![cfg(target_arch = "wasm32")]

use detonito_core::{GameConfig, LayoutGenerator, NoGuessLayoutGenerator};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn no_guess_generator_runs_on_wasm() {
    let cfg = GameConfig::new((6, 6), 8);
    let first_move = (2, 2);
    let layout = NoGuessLayoutGenerator::new(0xC0FFEE, first_move).generate(cfg);
    assert_eq!(layout.size(), cfg.size);
    assert!(layout.mine_count() <= cfg.mines);
}
