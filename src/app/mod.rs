use bitflags::bitflags;
use gloo::storage::{LocalStorage, Storage};
use gloo::timers::callback::Interval;
use gloo::utils::document;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use yew::prelude::*;

use crate::game;

mod settings;
mod utils;

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct MouseButtons: u16 {
        const LEFT    = 1;
        const RIGHT   = 1 << 1;
        const MIDDLE  = 1 << 2;
        const BACK    = 1 << 3;
        const FORWARD = 1 << 4;
    }
}

/*
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    const fn scheme(self) -> Option<&'static str> {
        use Theme::*;
        match self {
            Auto => None,
            Light => Some("light"),
            Dark => Some("dark"),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::Auto
    }
}
*/

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct TileState {
    pos: (usize, usize),
    buttons: MouseButtons,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
enum TileMsg {
    Update(TileState),
    Leave,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    TileEvent(TileMsg),
    UpdateTime,
    NewGame,
    ToggleSettings,
}

#[derive(Properties, Clone, PartialEq)]
struct TileProps {
    x: usize,
    y: usize,
    tile: game::Tile,
    #[prop_or_default]
    pressed: bool,
    callback: Callback<TileMsg>,
}

#[function_component(Tile)]
fn tile_component(props: &TileProps) -> Html {
    use game::Tile::*;

    let TileProps {
        x,
        y,
        tile,
        pressed,
        callback,
    } = props.clone();
    let mut class = classes!(
        "cell",
        match tile {
            Closed => classes!(),
            Open(count) => classes!("open", format!("num-{}", count)),
            Flag => classes!("flag"),
            Question => classes!("question"),
            Exploded => classes!("mine", "oops"),
            Mine => classes!("mine"),
            AutoFlag => classes!("flag"),
            IncorrectFlag => classes!("flag", "wrong"),
        }
    );
    if pressed {
        class.push("open");
    }

    let onmousedown = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            let tile_state = TileState {
                pos: (x, y),
                buttons,
            };
            callback.emit(TileMsg::Update(tile_state));
            log::trace!("({}, {}) mouse down ({:?})", x, y, buttons);
        })
    };

    let onmouseup = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            let tile_state = TileState {
                pos: (x, y),
                buttons,
            };
            callback.emit(TileMsg::Update(tile_state));
            log::trace!("({}, {}) mouse up ({:?})", x, y, buttons);
        })
    };

    let onmouseenter = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            let tile_state = TileState {
                pos: (x, y),
                buttons,
            };
            callback.emit(TileMsg::Update(tile_state));
            log::trace!("({}, {}) mouse enter ({:?})", x, y, buttons);
        })
    };

    let onmouseleave = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            callback.emit(TileMsg::Leave);
            log::trace!("({}, {}) mouse leave ({:?})", x, y, buttons);
        })
    };

    html! {
        <td {class} {onmousedown} {onmouseup} {onmouseenter} {onmouseleave}/>
    }
}

fn create_game(seed: u64, difficulty: &game::Difficulty, start: (usize, usize)) -> game::Game {
    use game::MinefieldGenerator;
    let minefield = game::RandomMinefieldGenerator::new(seed, start, game::StartTile::AlwaysZero)
        .generate(difficulty);
    game::Game::new(minefield)
}

fn format_for_counter(num: i32) -> String {
    match num {
        ..-99 => "-99".to_string(),
        // Some places do 0-1 for -1, I've also seen -01, which I'm leaning more to
        //-99..-9 => format!("-{:02}", -num),
        //-9..0 => format!("0-{:01}", -num),
        -99..0 => format!("-{:02}", -num),
        0..1000 => format!("{:03}", num),
        1000.. => "999".to_string(),
    }
}

struct Game {
    difficulty: game::Difficulty,
    game: Option<game::Game>,
    seed: u64,
    prev_time: u32,
    settings_open: bool,
    cur_tile_state: Option<TileState>,
    _timer_interval: Interval,
}

impl Game {
    const GAME_KEY: &'static str = "detonito:game";
    //const THEME_KEY: &'static str = "detonito:theme";

    const DEFAULT_DIFFICULTIES: &'static [(&'static str, game::Difficulty)] = &[
        ("Beginner", game::Difficulty::beginner()),
        ("Intermediate", game::Difficulty::intermediate()),
        ("Expert", game::Difficulty::expert()),
        (
            "Min",
            game::Difficulty {
                size: (1, 1),
                mines: 1,
            },
        ),
    ];

    fn default_difficulty() -> game::Difficulty {
        Game::DEFAULT_DIFFICULTIES[0].1.clone()
    }

    fn get_or_create_game(&mut self, coords: (usize, usize)) -> &mut game::Game {
        let seed = self.seed;
        let difficulty = self.difficulty.clone();
        self.game
            .get_or_insert_with(|| create_game(seed, &difficulty, coords))
    }

    fn get_size(&self) -> (usize, usize) {
        self.game
            .as_ref()
            .map(|game| game.size())
            .unwrap_or_else(|| self.difficulty.size)
    }

    fn get_total_mines(&self) -> usize {
        self.game
            .as_ref()
            .map(|game| game.total_mines())
            .unwrap_or_else(|| self.difficulty.mines)
    }

    fn get_time(&self) -> u32 {
        self.game.as_ref().map(|g| g.elapsed_secs()).unwrap_or(0)
    }

    fn get_mines_left(&self) -> i32 {
        self.game
            .as_ref()
            .map(|g| g.mines_left() as i32)
            .unwrap_or(self.get_total_mines() as i32)
    }

    fn get_game_state_class(&self) -> Classes {
        use crate::game::GameState::*;
        classes!(match self
            .game
            .as_ref()
            .map_or(NotStarted, |game| game.cur_state())
        {
            NotStarted => "not-started",
            InProgress => "in-progress",
            Won => "won",
            Lost => "lost",
        })
    }

    fn open_tile(&mut self, coords: (usize, usize)) -> bool {
        self.get_or_create_game(coords)
            .open_with_chords(coords)
            .map_or(false, |r| r.has_update())
    }

    fn flag_question(&mut self, coords: (usize, usize)) -> bool {
        self.get_or_create_game(coords)
            .flag_question(coords)
            .map_or(false, |r| r.has_update())
    }

    fn create_timer(ctx: &Context<Self>) -> Interval {
        let link = ctx.link().clone();
        Interval::new(500, move || link.send_message(Msg::UpdateTime))
    }

    fn is_pressed(&self, coords: (usize, usize), tile: game::Tile) -> bool {
        use game::Tile::*;
        const fn is_neighbor(a: (usize, usize), b: (usize, usize)) -> bool {
            (a.0.abs_diff(b.0) <= 1) && (a.1.abs_diff(b.1) <= 1)
        }
        match (self.cur_tile_state, tile) {
            (None, _) => false,
            (_, Flag | Question | Exploded | Mine | AutoFlag | IncorrectFlag) => false,
            (
                Some(TileState {
                    pos,
                    buttons: MouseButtons::LEFT,
                }),
                Closed,
            ) if pos == coords => true,
            (
                Some(TileState {
                    pos,
                    buttons: MouseButtons::LEFT,
                }),
                Closed,
            ) if is_neighbor(pos, coords) => self
                .game
                .as_ref()
                .map_or(false, |game| game.is_chordable(pos)),
            _ => false,
        }
    }
}

impl Component for Game {
    type Message = Msg;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        let game = LocalStorage::get(Game::GAME_KEY).ok();
        Self {
            difficulty: Game::default_difficulty(),
            game,
            seed: utils::js_random_seed(),
            prev_time: 0,
            settings_open: false,
            cur_tile_state: None,
            _timer_interval: Game::create_timer(ctx),
        }
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Self::Message) -> bool {
        use Msg::*;
        use TileMsg::*;

        let updated = match msg {
            TileEvent(Leave) => {
                log::debug!("tile leave");
                self.cur_tile_state.take().is_some()
            }
            TileEvent(Update(tile_state)) => {
                //log::debug!("tile update: {:?}", tile_state);
                if tile_state.buttons.is_empty() {
                    // all mouse buttons were released while in tile_state.pos
                    // we have to figure out which mouse buttons were released
                    match self.cur_tile_state.take() {
                        // nothing to do, mouse is just moving unpressed
                        None => false,
                        Some(TileState { pos, buttons }) => match buttons {
                            // only the left button was released, this means we open the tile
                            MouseButtons::LEFT => {
                                log::debug!("open tile: {:?}", pos);
                                self.open_tile(pos);
                                true
                            }
                            // only the right button was released, this means we flag the tile
                            MouseButtons::RIGHT => {
                                log::debug!("flag tile: {:?}", pos);
                                self.flag_question(pos);
                                true
                            }
                            // otherwise some combination of multiple buttons was released, we treat this as a cancel
                            // we should update because we might have to visually "unpress" some tiles
                            _ => {
                                log::debug!("redraw1");
                                true
                            }
                        },
                    }
                } else {
                    // there's some non-empty button state, we have to update the cur_tile_state, but whether there is
                    // a need for a re-render will depend on whether either the position or the LEFT button state
                    // changed
                    match self.cur_tile_state.replace(tile_state.clone()) {
                        None => {
                            log::debug!("redraw2");
                            true
                        }
                        Some(TileState { pos, buttons }) => {
                            log::debug!("redraw?");
                            (pos != tile_state.pos)
                                && ((buttons & MouseButtons::LEFT)
                                    != (tile_state.buttons & MouseButtons::LEFT))
                        }
                    }
                }
            }
            UpdateTime => {
                let time = self.get_time();
                if self.prev_time != time {
                    self.prev_time = time;
                    true
                } else {
                    false
                }
            }
            NewGame => {
                self.seed = utils::js_random_seed();
                self.game.take().map_or(false, |_| true)
            }
            ToggleSettings => {
                self.settings_open = !self.settings_open;
                true
            }
        };
        if updated {
            if let Err(err) = LocalStorage::set(Game::GAME_KEY, self.game.clone()) {
                log::error!("Could not save game to local storage: {:?}", err);
            }
        }
        updated
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        use settings::Settings;
        use Msg::*;

        let (cols, rows) = self.get_size();
        let game_state_class = classes!(self.get_game_state_class());
        let mines_left = format_for_counter(self.get_mines_left());
        let elapsed_time = format_for_counter(self.get_time() as i32);
        let cb_new_game = ctx.link().callback(|e: MouseEvent| {
            e.stop_propagation();
            NewGame
        });
        let cb_show_settings = ctx.link().callback(|_| ToggleSettings);

        html! {
            <div class={"detonito"} oncontextmenu={Callback::from(move |e: MouseEvent| e.prevent_default())}>
                <small onclick={cb_show_settings}>{"···"}</small>
                <nav>
                    <aside>{mines_left}</aside>
                    <span><button class={game_state_class} onclick={cb_new_game}/></span>
                    <aside>{elapsed_time}</aside>
                </nav>
                <table>
                    {
                        for (0..rows).map(|y| html! {
                            <tr>
                                {
                                    for (0..cols).map(|x| {
                                        let tile = self.game.as_ref().map_or(game::Tile::Closed, |game| game.tile_at((x, y)));
                                        let pressed = self.is_pressed((x, y), tile);
                                        let callback = ctx.link().callback(Msg::TileEvent);
                                        html! {
                                            <Tile {x} {y} {tile} {callback} {pressed}/>
                                        }
                                    })
                                }
                            </tr>
                        })
                    }
                </table>
                <Settings open={self.settings_open}/>
            </div>
        }
    }
}

#[wasm_bindgen(start)]
pub fn run_app() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).expect("Error initializing logger");
    let root = document()
        .get_element_by_id("game")
        .expect("Could not find id=\"game\" element");
    log::info!("Application started"); // Log an info message
    yew::Renderer::<Game>::with_root(root).render();
}
