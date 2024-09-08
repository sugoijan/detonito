use wasm_bindgen::prelude::*;
use yew::prelude::*;
use gloo::timers::callback::Interval;
use gloo::storage::{Storage, LocalStorage};
use gloo::utils::document;

use crate::game;
use utils::Modal;

pub(crate) mod utils {
    use yew::prelude::*;

    #[derive(Properties, PartialEq)]
    pub(crate) struct ModalProps {
        #[prop_or_default]
        pub(crate) children: Html,
    }

    #[function_component]
    pub(crate) fn Modal(props: &ModalProps) -> Html {
        let modal_host = gloo::utils::body();
        create_portal(
            props.children.clone(),
            modal_host.into(),
        )
    }
}

enum Msg {
    Open((usize, usize)),
    Flag((usize, usize)),
    UpdateTime,
    NewGame,
    ShowSettings,
    HideSettings,
}

#[derive(Properties, Clone, PartialEq)]
struct CellProps {
    x: usize,
    y: usize,
    cell: game::Cell,
    cb_left_click: Callback<(usize, usize)>,
    cb_right_click: Callback<(usize, usize)>,
}

#[function_component(CellComponent)]
fn cell_component(props: &CellProps) -> Html {
    use game::Cell::*;

    let CellProps { x, y, cell, cb_left_click, cb_right_click } = props.clone();
    let class = classes!("cell", match cell {
        Closed => classes!("closed"),
        Open(count) => classes!("open", format!("num-{}", count)),
        Flag => classes!("closed", "flag"),
        Question => classes!("closed", "question"),
        Exploded => classes!("open", "mine", "exploded"),
        Mine => classes!("open", "mine"),
        AutoFlag => classes!("closed", "flag"),
        IncorrectFlag => classes!("closed", "flag", "incorrect"),
    });

    html! {
        <td
            class={class}
            onmousedown={Callback::from(move |_| {
                log::debug!("({}, {}) mouse down", x, y);
            })}
            onmouseup={Callback::from(move |_| {
                log::debug!("({}, {}) mouse up", x, y);
            })}
            onmouseenter={Callback::from(move |e: MouseEvent| {
                log::debug!("({}, {}) mouse enter ({}, {})", x, y, e.button(), e.buttons());
            })}
            onmouseleave={Callback::from(move |_| {
                log::debug!("({}, {}) mouse leave", x, y);
            })}
            onmouseover={Callback::from(move |_| {
                log::debug!("({}, {}) mouse over", x, y);
            })}
            onmouseout={Callback::from(move |_| {
                log::debug!("({}, {}) mouse out", x, y);
            })}
            onclick={Callback::from(move |_| cb_left_click.emit((x, y)))}
            onauxclick={Callback::from(move |_| cb_right_click.emit((x, y)))}
        />
    }
}

fn js_random_seed() -> u64 {
    use js_sys::Math::random;
    u64::from_be_bytes([
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
    ])
}


fn create_game(seed: u64, difficulty: &game::Difficulty, start: (usize, usize)) -> game::Game {
    use game::MinefieldGenerator;
    let minefield = game::RandomMinefieldGenerator::new(seed, start, game::StartCell::AlwaysZero).generate(difficulty);
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
    _timer_interval: Interval,
}

impl Game {
    const GAME_KEY: &'static str = "detonito:game";

    fn default_difficulty() -> game::Difficulty {
        //game::Difficulty::intermediate()
        //game::Difficulty::expert()
        game::Difficulty::beginner()
    }

    fn get_or_create_game(&mut self, coords: (usize, usize)) -> &mut game::Game {
        let seed = self.seed;
        let difficulty = self.difficulty.clone();
        self.game.get_or_insert_with(|| create_game(seed, &difficulty, coords))
    }

    fn get_size(&self) -> (usize, usize) {
        self.game.as_ref().map(|game| game.size()).unwrap_or_else(|| self.difficulty.size)
    }

    fn get_total_mines(&self) -> usize {
        self.game.as_ref().map(|game| game.total_mines()).unwrap_or_else(|| self.difficulty.mines)
    }

    fn get_time(&self) -> u32 {
        self.game.as_ref().map(|g| g.elapsed_secs()).unwrap_or(0)
    }

    fn get_mines_left(&self) -> i32 {
        self.game.as_ref().map(|g| g.mines_left() as i32).unwrap_or(self.get_total_mines() as i32)
    }

    fn open_cell(&mut self, coords: (usize, usize)) -> bool {
        self.get_or_create_game(coords).open_clear(coords).map_or(false, |r| r.has_update())
    }

    fn flag_question(&mut self, coords: (usize, usize)) -> bool {
        self.get_or_create_game(coords).flag_question(coords).map_or(false, |r| r.has_update())
    }
}

impl Component for Game {
    type Message = Msg;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        Self {
            difficulty: Game::default_difficulty(),
            game: LocalStorage::get(Game::GAME_KEY).ok(),
            seed: js_random_seed(),
            prev_time: 0,
            settings_open: false,
            _timer_interval: {
                let link = ctx.link().clone();
                Interval::new(500, move || link.send_message(Msg::UpdateTime))
            },
        }
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Self::Message) -> bool {
        let updated = match msg {
            Msg::Open(coords) => self.open_cell(coords),
            Msg::Flag(coords) => self.flag_question(coords),
            Msg::UpdateTime => {
                let time = self.get_time();
                if self.prev_time != time {
                    self.prev_time = time;
                    true
                } else {
                    false
                }
            }
            Msg::NewGame => {
                self.seed = js_random_seed();
                self.game.take().map_or(false, |_| true)
            }
            Msg::ShowSettings => {
                self.settings_open = true;
                true
            }
            Msg::HideSettings => {
                self.settings_open = false;
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
        use crate::game::GameState::*;
        let (x_size, y_size) = self.get_size();
        let game_state_class = classes!(
            "state",
            match self.game.as_ref().map_or(NotStarted, |game| game.cur_state()) {
                NotStarted => "not-started",
                InProgress => "in-progress",
                Won => "won",
                Lost => "lost",
            },
        );

        let mines_left = format_for_counter(self.get_mines_left());
        let elapsed_time = format_for_counter(self.get_time() as i32);
        let cb_new_game = ctx.link().callback(|e: MouseEvent| { e.stop_propagation(); Msg::NewGame });
        let cb_show_settings = ctx.link().callback(|_| Msg::ShowSettings);
        let cb_hide_settings = ctx.link().callback(|_| Msg::HideSettings);
        let is_small = x_size < 8;

        html! {
            <div class={"detonito"} oncontextmenu={Callback::from(move |e: MouseEvent| e.prevent_default())}>
                <div class={classes!("dotdotdot")} onclick={cb_show_settings}>{"···"}</div>
                <div class={classes!("top", is_small.then(|| Some("small")))}>
                    {
                        if !is_small {
                            html! {
                                <>
                                    <div class={"left"}>{mines_left}</div>
                                    <div class={"center"}><div class={game_state_class} onclick={cb_new_game}/></div>
                                    <div class={"right"}>{elapsed_time}</div>
                                </>
                            }
                        } else {
                            html! {
                                <div class={"center"} onclick={cb_new_game}><div class={game_state_class}/></div>
                            }
                        }
                    }
                </div>
                <table class={"grid"}>
                    {
                        for (0..y_size).map(|y| html! {
                            <tr>
                                {
                                    for (0..x_size).map(|x| {
                                        let cell = self.game.as_ref().map_or(game::Cell::Closed, |game| game.cell_at((x, y)));
                                        let cb_left_click = ctx.link().callback(move |_| Msg::Open((x, y)));
                                        let cb_right_click = ctx.link().callback(move |_| Msg::Flag((x, y)));
                                        html! {
                                            <CellComponent x={x} y={y} cell={cell} {cb_right_click} {cb_left_click}/>
                                        }
                                    })
                                }
                            </tr>
                        })
                    }
                </table>
                <Modal><Settings open={self.settings_open} {cb_hide_settings}/></Modal>
            </div>
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct SettingsProps {
    #[prop_or_default]
    pub open: bool,
    cb_hide_settings: Callback<()>,
}

#[function_component]
fn Settings(props: &SettingsProps) -> Html {
    let cb_hide_settings = props.cb_hide_settings.clone();
    html! {
        <dialog id="settings" open={props.open}>
            <article>
                <h2>{"Settings"}</h2>
                <ul>
                    <li><a href="#" data-theme-switcher="auto">{"Auto"}</a></li>
                    <li><a href="#" data-theme-switcher="light">{"Light"}</a></li>
                    <li><a href="#" data-theme-switcher="dark">{"Dark"}</a></li>
                </ul>
                <footer>
                    <button onclick={move |_| cb_hide_settings.emit(())} type="reset">{"Cancel"}</button>
                    <button>{"Apply"}</button>
                </footer>
            </article>
        </dialog>
    }
}


#[function_component]
fn App() -> Html {
    html! {
        <>
            <header>
              <nav>
                <ul>
                  // TODO: title?
                  // <li><strong>Detonito</strong></li>
                </ul>
              </nav>
            </header>

            <main>
                <section id="game">
                    <Game/>
                </section>
            </main>

            <footer>
                // TODO
            </footer>

        </>
    }
}

#[wasm_bindgen(start)]
pub fn run_app() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).expect("Error initializing logger");
    let root = document().get_element_by_id("game").expect("Could not find id=\"game\" element");
    log::info!("Application started"); // Log an info message
    yew::Renderer::<Game>::with_root(root).render();
}
