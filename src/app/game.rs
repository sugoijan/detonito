use crate::app::settings;
use crate::app::utils::*;
use crate::game;
use bitflags::bitflags;
use gloo::timers::callback::Interval;
use serde::{Deserialize, Serialize};
use yew::prelude::*;

impl StorageKey for game::Game {
    const KEY: &'static str = "detonito:game";
}

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

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) struct TileState {
    pos: (game::Ix, game::Ix),
    buttons: MouseButtons,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) enum TileMsg {
    Update(TileState),
    Leave,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) enum Msg {
    TileEvent(TileMsg),
    UpdateTime,
    NewGame,
    ToggleSettings,
    UpdateSettings(settings::Settings),
}

#[derive(Properties, Clone, PartialEq)]
struct TileProps {
    x: game::Ix,
    y: game::Ix,
    tile: game::AnyTile,
    #[prop_or_default]
    pressed: bool,
    callback: Callback<TileMsg>,
}

#[function_component(Tile)]
fn tile_component(props: &TileProps) -> Html {
    use game::AnyTile::*;

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
            Exploded => classes!("open", "mine", "oops"),
            Mine => classes!("open", "mine"),
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

pub(in crate::app) struct GameView {
    settings: settings::Settings,
    game: Option<game::Game>,
    seed: u64,
    prev_time: u32,
    settings_open: bool,
    cur_tile_state: Option<TileState>,
    _timer_interval: Interval,
}

impl GameView {
    fn get_or_create_game(&mut self, coords: game::Ix2) -> &mut game::Game {
        let Self {
            game,
            settings,
            seed,
            ..
        } = self;
        game.get_or_insert_with(|| {
            use game::{MinefieldGenerator, RandomMinefieldGenerator, StartTile};
            use settings::Generator::*;
            let minefield = match settings.generator {
                Random => RandomMinefieldGenerator::new(*seed, coords, StartTile::Random)
                    .generate(settings.difficulty),
                NoRandom => RandomMinefieldGenerator::new(*seed, coords, StartTile::AlwaysZero)
                    .generate(settings.difficulty),
            };
            game::Game::new(minefield)
        })
    }

    fn get_size(&self) -> game::Ix2 {
        self.game
            .as_ref()
            .map(|game| game.size())
            .unwrap_or_else(|| self.settings.difficulty.size)
    }

    fn get_total_mines(&self) -> game::Ax {
        self.game
            .as_ref()
            .map(|game| game.total_mines())
            .unwrap_or_else(|| self.settings.difficulty.mines)
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

    fn get_game_state(&self) -> game::GameState {
        self.game
            .as_ref()
            .map_or(game::GameState::NotStarted, |game| game.cur_state())
    }

    fn is_mid_open(&self) -> bool {
        matches!(
            self.cur_tile_state,
            Some(TileState {
                buttons: MouseButtons::LEFT,
                ..
            })
        )
    }

    fn get_game_state_class(&self) -> Classes {
        use game::GameState::*;
        let mid_open = self.is_mid_open();
        let game_state = self.get_game_state();
        classes!(match game_state {
            NotStarted | InProgress if mid_open => "mid-open",
            NotStarted => "not-started",
            InProgress => "in-progress",
            Win => "win",
            Lose => "lose",
            InstantWin => "instant-win",
            InstantLoss => "instant-loss",
        })
    }

    fn is_playable(&self) -> bool {
        use game::GameState::*;
        match self.get_game_state() {
            NotStarted => true,
            InProgress => true,
            Win => false,
            Lose => false,
            InstantWin => false,
            InstantLoss => false,
        }
    }

    fn open_tile(&mut self, coords: game::Ix2) -> bool {
        use game::AnyTile::*;
        let game = self.get_or_create_game(coords);
        match game.tile_at(coords) {
            Closed => game.open(coords).map_or(false, |r| r.has_update()),
            Open(_) => game.chord_open(coords).map_or(false, |r| r.has_update()),
            _ => false,
        }
    }

    fn flag_question(&mut self, coords: game::Ix2) -> bool {
        use game::AnyTile::*;
        let mark_question = self.settings.mark_question;
        let game = self.get_or_create_game(coords);
        match game.tile_at(coords) {
            Flag if mark_question => game.flag_question(coords).map_or(false, |r| r.has_update()),
            Closed | Flag | Question => game.flag(coords).map_or(false, |r| r.has_update()),
            Open(_) => game.chord_flag(coords).map_or(false, |r| r.has_update()),
            _ => false,
        }
    }

    fn create_timer(ctx: &Context<Self>) -> Interval {
        let link = ctx.link().clone();
        Interval::new(500, move || link.send_message(Msg::UpdateTime))
    }

    fn is_pressed(&self, coords: game::Ix2, tile: game::AnyTile) -> bool {
        use game::AnyTile::*;
        if self.get_game_state().is_final() {
            return false;
        }
        const fn is_neighbor(a: game::Ix2, b: game::Ix2) -> bool {
            (a.0.abs_diff(b.0) <= 1) && (a.1.abs_diff(b.1) <= 1)
        }
        match (self.cur_tile_state, tile) {
            (None, _) => false,
            (_, Flag | Question | Exploded | Mine | IncorrectFlag) => false,
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

impl Component for GameView {
    type Message = Msg;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        Self {
            settings: LocalOrDefault::local_or_default(),
            game: LocalOrDefault::local_or_default(),
            seed: js_random_seed(),
            prev_time: 0,
            settings_open: false,
            cur_tile_state: None,
            _timer_interval: GameView::create_timer(ctx),
        }
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Self::Message) -> bool {
        use Msg::*;
        use TileMsg::*;

        let updated = match msg {
            TileEvent(Leave) => {
                log::trace!("tile leave");
                self.cur_tile_state.take().is_some()
            }
            TileEvent(Update(tile_state)) => {
                log::trace!("tile update: {:?}", tile_state);
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
                                log::trace!("redraw: multiple buttons changed, maybe redraw");
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
                            log::trace!("redraw: tile state removed");
                            true
                        }
                        Some(TileState { pos, buttons }) => {
                            log::trace!("redraw: maybe new tile states causes changes");
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
                self.seed = js_random_seed();
                self.game.take().map_or(false, |_| true)
            }
            ToggleSettings => {
                self.settings_open = !self.settings_open;
                if !self.settings_open {
                    self.settings = LocalOrDefault::local_or_default();
                }
                true
            }
            UpdateSettings(settings) => {
                if self.settings != settings {
                    self.settings = settings;
                    true
                } else {
                    false
                }
            }
        };
        self.game.local_save();
        updated
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        use settings::SettingsView;
        use Msg::*;

        let (cols, rows) = self.get_size();
        let game_state_class = classes!(self.get_game_state_class());
        let is_playable = self.is_playable();
        let mines_left = format_for_counter(self.get_mines_left());
        let elapsed_time = format_for_counter(self.get_time() as i32);
        let cb_new_game = ctx.link().callback(|e: MouseEvent| {
            e.stop_propagation();
            NewGame
        });
        let cb_show_settings = ctx.link().callback(|_| ToggleSettings);

        html! {
            <div class="detonito" oncontextmenu={Callback::from(move |e: MouseEvent| e.prevent_default())}>
                <small onclick={cb_show_settings}>{"···"}</small>
                <nav>
                    <aside>{mines_left}</aside>
                    <span><button class={game_state_class} onclick={cb_new_game}/></span>
                    <aside>{elapsed_time}</aside>
                </nav>
                <table class={is_playable.then_some("playable")}>
                    {
                        for (0..rows).map(|y| html! {
                            <tr>
                                {
                                    for (0..cols).map(|x| {
                                        let tile = self.game.as_ref().map_or(game::AnyTile::Closed, |game| game.tile_at((x, y)));
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
                <SettingsView open={self.settings_open}/>
            </div>
        }
    }
}
