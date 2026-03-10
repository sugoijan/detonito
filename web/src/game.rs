use crate::no_guess_worker;
use crate::settings;
use crate::sprites::{Glyph, GlyphRun, GlyphSet, Icon, IconCrop, SpriteDefs};
use crate::utils::*;
use bitflags::bitflags;
use chrono::prelude::*;
use clap::Args;
use detonito_core as game;
use game::{NeighborIterExt, ToNdIndex};
use gloo::timers::callback::{Interval, Timeout};
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use yew::prelude::*;

fn utc_now() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(js_sys::Date::now() as i64).unwrap()
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum ViewCellState {
    Hidden,
    Revealed(u8),
    Flagged,
    QuestionMarked,
    TriggeredMine,
    Mine,
    Misflagged,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum ViewGameState {
    Ready,
    Active,
    Won,
    Lost,
    WonOnFirstMove,
    LostOnFirstMove,
}

impl ViewGameState {
    fn is_finished(self) -> bool {
        matches!(
            self,
            Self::Won | Self::Lost | Self::WonOnFirstMove | Self::LostOnFirstMove
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct GameSession {
    pub engine: game::PlayEngine,
    pub question_marks: Array2<bool>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub move_count: u32,
}

impl GameSession {
    fn new(engine: game::PlayEngine) -> Self {
        let question_marks = Array2::default(engine.size().to_nd_index());
        Self {
            engine,
            question_marks,
            started_at: None,
            ended_at: None,
            move_count: 0,
        }
    }

    fn elapsed_secs(&self, now: DateTime<Utc>) -> u32 {
        if let Some(started_at) = self.started_at {
            (self.ended_at.unwrap_or(now) - started_at)
                .num_seconds()
                .max(0) as u32
        } else {
            0
        }
    }

    fn mines_left_for_display(&self) -> i32 {
        if matches!(self.engine.state(), game::EngineState::Won) {
            0
        } else {
            self.engine.mines_left() as i32
        }
    }

    fn view_state(&self) -> ViewGameState {
        use game::EngineState::*;
        match self.engine.state() {
            Ready => ViewGameState::Ready,
            Active => ViewGameState::Active,
            Won if self.move_count <= 1 => ViewGameState::WonOnFirstMove,
            Won => ViewGameState::Won,
            Lost if self.move_count <= 1 => ViewGameState::LostOnFirstMove,
            Lost => ViewGameState::Lost,
        }
    }

    fn cell_state_at(&self, coords: game::Coord2) -> ViewCellState {
        if self.engine.state().is_finished() {
            return self.cell_state_finished(coords);
        }

        self.cell_state_active(coords)
    }

    fn cell_state_active(&self, coords: game::Coord2) -> ViewCellState {
        match self.engine.cell_at(coords) {
            game::EngineCell::Hidden if self.question_marks[coords.to_nd_index()] => {
                ViewCellState::QuestionMarked
            }
            game::EngineCell::Hidden => ViewCellState::Hidden,
            game::EngineCell::Revealed(count) => ViewCellState::Revealed(count),
            game::EngineCell::Flagged => ViewCellState::Flagged,
        }
    }

    fn cell_state_finished(&self, coords: game::Coord2) -> ViewCellState {
        use game::EngineState::*;
        let engine_cell = self.engine.cell_at(coords);
        let has_mine = self.engine.has_mine_at(coords);

        match self.engine.state() {
            Ready | Active => self.cell_state_active(coords),
            Won => {
                if has_mine {
                    ViewCellState::Flagged
                } else {
                    match engine_cell {
                        game::EngineCell::Hidden if self.question_marks[coords.to_nd_index()] => {
                            ViewCellState::QuestionMarked
                        }
                        game::EngineCell::Hidden => ViewCellState::Hidden,
                        game::EngineCell::Revealed(count) => ViewCellState::Revealed(count),
                        game::EngineCell::Flagged => ViewCellState::Misflagged,
                    }
                }
            }
            Lost => {
                if self.engine.triggered_mine() == Some(coords) {
                    return ViewCellState::TriggeredMine;
                }

                if has_mine {
                    match engine_cell {
                        game::EngineCell::Flagged => ViewCellState::Flagged,
                        _ => ViewCellState::Mine,
                    }
                } else {
                    match engine_cell {
                        game::EngineCell::Revealed(count) => ViewCellState::Revealed(count),
                        game::EngineCell::Flagged => ViewCellState::Misflagged,
                        game::EngineCell::Hidden if self.question_marks[coords.to_nd_index()] => {
                            ViewCellState::QuestionMarked
                        }
                        game::EngineCell::Hidden => ViewCellState::Hidden,
                    }
                }
            }
        }
    }

    fn can_chord_reveal_at(&self, coords: game::Coord2) -> bool {
        self.engine.can_chord_reveal_at(coords) && !self.has_question_mark_neighbor(coords)
    }

    fn can_interact_at(&self, coords: game::Coord2) -> bool {
        use ViewCellState::*;

        if self.engine.is_finished() {
            return false;
        }

        match self.cell_state_at(coords) {
            Hidden | Flagged | QuestionMarked => true,
            Revealed(count) if count == 0 => false,
            Revealed(count) => {
                let mut adjacent_flag_count = 0;
                for pos in self.question_marks.iter_neighbors(coords) {
                    match self.cell_state_at(pos) {
                        Flagged => adjacent_flag_count += 1,
                        Revealed(_) => continue,
                        _ => return true,
                    }
                }
                adjacent_flag_count != count
            }
            TriggeredMine | Mine | Misflagged => false,
        }
    }

    fn has_question_mark_neighbor(&self, coords: game::Coord2) -> bool {
        self.question_marks
            .iter_neighbors(coords)
            .any(|pos| self.question_marks[pos.to_nd_index()])
    }

    fn clear_question_mark(&mut self, coords: game::Coord2) {
        self.question_marks[coords.to_nd_index()] = false;
    }

    fn sync_question_marks_with_engine(&mut self) {
        let (x_end, y_end) = self.engine.size();
        for x in 0..x_end {
            for y in 0..y_end {
                let coords = (x, y);
                if !matches!(self.engine.cell_at(coords), game::EngineCell::Hidden) {
                    self.question_marks[coords.to_nd_index()] = false;
                }
            }
        }
    }

    fn on_successful_move(&mut self, now: DateTime<Utc>) {
        self.move_count = self.move_count.saturating_add(1);

        if self.started_at.is_none() {
            self.started_at = Some(now);
        }

        if self.engine.is_finished() && self.ended_at.is_none() {
            self.ended_at = Some(now);
        }
    }

    #[allow(dead_code)]
    fn to_observation(&self, mine_count: Option<game::CellCount>) -> game::Observation {
        game::Observation::from_engine_with_mine_count(&self.engine, mine_count)
    }
}

impl StorageKey for GameSession {
    const KEY: &'static str = "detonito:game:v2";
}

pub trait HasUpdate {
    fn has_update(self) -> bool;
}

impl<E> HasUpdate for Result<game::MarkOutcome, E> {
    fn has_update(self) -> bool {
        self.map_or(false, |outcome: game::MarkOutcome| outcome.has_update())
    }
}

impl<E> HasUpdate for Result<game::RevealOutcome, E> {
    fn has_update(self) -> bool {
        self.map_or(false, |outcome: game::RevealOutcome| outcome.has_update())
    }
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
pub(crate) struct CellPointerState {
    pos: (game::Coord, game::Coord),
    buttons: MouseButtons,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum CellMsg {
    Update(CellPointerState),
    Leave,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Msg {
    CellEvent(CellMsg),
    UpdateTime,
    NewGame,
    FaceButtonPress,
    FacePromptSelected(FacePromptAction),
    ToggleSettings,
    UpdateSettings(settings::Settings),
    NoGuessGenerated(no_guess_worker::NoGuessGenResponse),
    NoGuessGenerationTimeout(u64),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FacePromptAction {
    RestartGame,
    DismissPrompt,
    OpenSettings,
}

#[derive(Clone, Debug, PartialEq)]
struct FacePromptChoice {
    label: AttrValue,
    title: AttrValue,
    action: FacePromptAction,
}

#[derive(Clone, Debug, PartialEq)]
struct FacePrompt {
    message: AttrValue,
    choices: Vec<FacePromptChoice>,
}

impl FacePrompt {
    fn restart_confirmation() -> Self {
        Self {
            message: "Restart?".into(),
            choices: vec![
                FacePromptChoice {
                    label: "Yes".into(),
                    title: "Restart game".into(),
                    action: FacePromptAction::RestartGame,
                },
                FacePromptChoice {
                    label: "No".into(),
                    title: "Keep current game".into(),
                    action: FacePromptAction::DismissPrompt,
                },
                FacePromptChoice {
                    label: "Menu".into(),
                    title: "Open settings".into(),
                    action: FacePromptAction::OpenSettings,
                },
            ],
        }
    }
}

#[derive(Properties, Clone, PartialEq)]
struct CellProps {
    x: game::Coord,
    y: game::Coord,
    cell_state: ViewCellState,
    #[prop_or_default]
    pressed: bool,
    #[prop_or_default]
    loading: bool,
    #[prop_or_default]
    locked: bool,
    callback: Callback<CellMsg>,
}

#[function_component(CellView)]
fn cell_component(props: &CellProps) -> Html {
    use ViewCellState::*;

    let CellProps {
        x,
        y,
        cell_state,
        pressed,
        loading,
        locked,
        callback,
    } = props.clone();

    let mut class = classes!(
        "cell",
        match cell_state {
            Hidden => classes!(),
            Revealed(count) => classes!("open", format!("num-{}", count)),
            Flagged => classes!("flag"),
            QuestionMarked => classes!("question"),
            TriggeredMine => classes!("open", "mine", "oops"),
            Mine => classes!("open", "mine"),
            Misflagged => classes!("flag", "wrong"),
        }
    );
    if pressed {
        class.push("open");
    }
    if loading {
        class.push("loading");
    }
    if locked {
        class.push("locked");
    }

    let content = match cell_state {
        Hidden => Html::default(),
        Revealed(count) if count == 0 => Html::default(),
        Revealed(count) => html! {
            <Glyph
                set={GlyphSet::Cell}
                ch={char::from_digit(count.into(), 10).expect("Cell numbers fit in a single digit")}
                class={classes!("cell-glyph")}
            />
        },
        Flagged => html! { <Icon name="flag" class={classes!("cell-icon")}/> },
        QuestionMarked => html! { <Icon name="question" class={classes!("cell-icon")}/> },
        TriggeredMine => html! { <Icon name="mine-exploded" class={classes!("cell-icon")}/> },
        Mine => html! { <Icon name="mine" class={classes!("cell-icon")}/> },
        Misflagged => html! { <Icon name="flag" class={classes!("cell-icon")}/> },
    };

    let onmousedown = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            let pointer_state = CellPointerState {
                pos: (x, y),
                buttons,
            };
            callback.emit(CellMsg::Update(pointer_state));
            log::trace!("({}, {}) mouse down ({:?})", x, y, buttons);
        })
    };

    let onmouseup = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            let pointer_state = CellPointerState {
                pos: (x, y),
                buttons,
            };
            callback.emit(CellMsg::Update(pointer_state));
            log::trace!("({}, {}) mouse up ({:?})", x, y, buttons);
        })
    };

    let onmouseenter = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            let pointer_state = CellPointerState {
                pos: (x, y),
                buttons,
            };
            callback.emit(CellMsg::Update(pointer_state));
            log::trace!("({}, {}) mouse enter ({:?})", x, y, buttons);
        })
    };

    let onmouseleave = {
        let callback = callback.clone();
        Callback::from(move |e: MouseEvent| {
            let buttons = MouseButtons::from_bits_truncate(e.buttons());
            callback.emit(CellMsg::Leave);
            log::trace!("({}, {}) mouse leave ({:?})", x, y, buttons);
        })
    };

    html! {
        <td {class} {onmousedown} {onmouseup} {onmouseenter} {onmouseleave}>
            {content}
        </td>
    }
}

#[derive(Args, Properties, Debug, Clone, PartialEq)]
pub(crate) struct GameProps {
    /// Force a seed instead of random
    #[arg(short, long)]
    seed: Option<String>,
}

#[derive(Debug)]
pub(crate) struct GameView {
    settings: settings::Settings,
    game: Option<GameSession>,
    seed: u64,
    prev_time: u32,
    settings_open: bool,
    face_prompt: Option<FacePrompt>,
    current_cell_state: Option<CellPointerState>,
    no_guess_worker: Option<no_guess_worker::NoGuessWorkerBridge>,
    pending_first_action: Option<game::Coord2>,
    generation_id: u64,
    is_generating_layout: bool,
    generation_timeout: Option<Timeout>,
    _timer_interval: Interval,
    _init_settings: GameProps,
}

impl GameView {
    fn clear_face_prompt(&mut self) -> bool {
        self.face_prompt.take().is_some()
    }

    fn show_face_prompt(&mut self, prompt: FacePrompt) -> bool {
        if self.face_prompt.is_some() {
            false
        } else {
            self.face_prompt = Some(prompt);
            true
        }
    }

    fn open_restart_prompt(&mut self) -> bool {
        self.show_face_prompt(FacePrompt::restart_confirmation())
    }

    fn restart_game(&mut self) -> bool {
        let prompt_cleared = self.clear_face_prompt();
        let was_generating = self.cancel_pending_generation();
        let pointer_cleared = self.current_cell_state.take().is_some();
        self.seed = js_random_seed();
        self.game.take().is_some() || was_generating || pointer_cleared || prompt_cleared
    }

    fn open_settings(&mut self) -> bool {
        let prompt_cleared = self.clear_face_prompt();
        if self.settings_open {
            prompt_cleared
        } else {
            self.settings_open = true;
            prompt_cleared || true
        }
    }

    fn apply_face_prompt_action(&mut self, action: FacePromptAction) -> bool {
        match action {
            FacePromptAction::RestartGame => self.restart_game(),
            FacePromptAction::DismissPrompt => self.clear_face_prompt(),
            FacePromptAction::OpenSettings => self.open_settings(),
        }
    }

    fn get_or_create_game(&mut self, coords: game::Coord2) -> &mut GameSession {
        let Self {
            game,
            settings,
            seed,
            ..
        } = self;

        game.get_or_insert_with(|| {
            use game::{
                FirstMovePolicy, LayoutGenerator, NoGuessLayoutGenerator, RandomLayoutGenerator,
            };
            use settings::Generator::*;

            let mine_layout = match settings.generator {
                RandomGamble => RandomLayoutGenerator::new(*seed, coords, FirstMovePolicy::Random)
                    .generate(settings.game_config),
                RandomZeroStart => {
                    RandomLayoutGenerator::new(*seed, coords, FirstMovePolicy::FirstMoveZero)
                        .generate(settings.game_config)
                }
                NoGuess => {
                    NoGuessLayoutGenerator::new(*seed, coords).generate(settings.game_config)
                }
            };

            let engine = game::PlayEngine::new(mine_layout);
            GameSession::new(engine)
        })
    }

    fn ensure_no_guess_worker(&mut self, ctx: &Context<Self>) -> bool {
        if self.no_guess_worker.is_some() {
            return true;
        }

        let link = ctx.link().clone();
        self.no_guess_worker = no_guess_worker::spawn_bridge(move |response| {
            link.send_message(Msg::NoGuessGenerated(response));
        });

        self.no_guess_worker.is_some()
    }

    fn begin_no_guess_generation(&mut self, ctx: &Context<Self>, coords: game::Coord2) -> bool {
        if self.is_generating_layout {
            return false;
        }

        self.generation_id = self.generation_id.wrapping_add(1);
        self.pending_first_action = Some(coords);
        self.is_generating_layout = true;
        self.face_prompt = None;
        self.current_cell_state = None;

        let generation_id = self.generation_id;
        let req = no_guess_worker::NoGuessGenRequest {
            generation_id,
            seed: self.seed,
            first_move: coords,
            config: self.settings.game_config,
        };

        self.ensure_no_guess_worker(ctx);
        let send_ok = self.no_guess_worker.as_ref().map(|w| w.send(req));
        match send_ok {
            Some(true) => {
                self.arm_generation_timeout(ctx, generation_id);
                true
            }
            Some(false) => self.fail_generation(
                "No-guess worker rejected generation request; cancelled generation",
            ),
            None => self.fail_generation("No-guess worker unavailable; cancelled generation"),
        }
    }

    fn complete_no_guess_generation(
        &mut self,
        response: no_guess_worker::NoGuessGenResponse,
    ) -> bool {
        if response.generation_id != self.generation_id {
            return false;
        }

        self.is_generating_layout = false;
        self.generation_timeout = None;

        let summary = response.summary;
        log::debug!(
            "No-guess generation done: attempts={} backtracks={} depth={} elapsed={}us succeeded={}",
            summary.attempts,
            summary.backtracks,
            summary.max_depth_reached,
            summary.elapsed_micros,
            summary.succeeded
        );

        self.game = Some(GameSession::new(game::PlayEngine::new(response.layout)));

        if let Some(coords) = self.pending_first_action.take() {
            self.reveal_cell(coords);
        }

        true
    }

    fn cancel_pending_generation(&mut self) -> bool {
        if !self.is_generating_layout {
            return false;
        }

        self.generation_id = self.generation_id.wrapping_add(1);
        self.pending_first_action = None;
        self.is_generating_layout = false;
        self.generation_timeout = None;
        true
    }

    fn arm_generation_timeout(&mut self, ctx: &Context<Self>, generation_id: u64) {
        let timeout_ms = self.generation_timeout_ms();
        let link = ctx.link().clone();
        self.generation_timeout = Some(Timeout::new(timeout_ms, move || {
            link.send_message(Msg::NoGuessGenerationTimeout(generation_id));
        }));
    }

    fn generation_timeout_ms(&self) -> u32 {
        let total_cells = self.settings.game_config.total_cells() as u32;
        20_000u32
            .saturating_add(total_cells.saturating_mul(150))
            .clamp(20_000, 180_000)
    }

    fn fail_generation(&mut self, reason: &str) -> bool {
        console_error(reason);
        self.generation_timeout = None;
        self.pending_first_action = None;
        self.is_generating_layout = false;
        self.current_cell_state = None;
        self.generation_id = self.generation_id.wrapping_add(1);
        if let Some(worker) = self.no_guess_worker.take() {
            worker.terminate();
        }
        true
    }

    fn get_size(&self) -> game::Coord2 {
        self.game
            .as_ref()
            .map(|game| game.engine.size())
            .unwrap_or_else(|| self.settings.game_config.size)
    }

    fn get_total_mines(&self) -> game::CellCount {
        self.game
            .as_ref()
            .map(|game| game.engine.total_mines())
            .unwrap_or_else(|| self.settings.game_config.mines)
    }

    fn get_time(&self) -> u32 {
        self.game
            .as_ref()
            .map(|g| g.elapsed_secs(utc_now()))
            .unwrap_or(0)
    }

    fn get_mines_left(&self) -> i32 {
        self.game
            .as_ref()
            .map(|g| g.mines_left_for_display())
            .unwrap_or(self.get_total_mines() as i32)
    }

    fn get_game_state(&self) -> ViewGameState {
        self.game
            .as_ref()
            .map_or(ViewGameState::Ready, |game| game.view_state())
    }

    fn is_mid_open(&self) -> bool {
        matches!(
            self.current_cell_state,
            Some(CellPointerState {
                buttons: MouseButtons::LEFT,
                ..
            })
        )
    }

    fn get_game_state_icon_name(&self) -> &'static str {
        if self.is_generating_layout {
            return "mid-open";
        }

        let mid_open = self.is_mid_open();
        let game_state = self.get_game_state();

        match game_state {
            ViewGameState::Ready | ViewGameState::Active if mid_open => "mid-open",
            ViewGameState::Ready => "not-started",
            ViewGameState::Active => "in-progress",
            ViewGameState::Won => "win",
            ViewGameState::Lost => "lose",
            ViewGameState::WonOnFirstMove => "instant-win",
            ViewGameState::LostOnFirstMove => "instant-loss",
        }
    }

    fn is_playable(&self) -> bool {
        if self.is_generating_layout {
            return false;
        }

        matches!(
            self.get_game_state(),
            ViewGameState::Ready | ViewGameState::Active
        )
    }

    fn reveal_cell(&mut self, coords: game::Coord2) -> bool {
        if self.is_generating_layout {
            return false;
        }

        use ViewCellState::*;

        let now = utc_now();
        let game = self.get_or_create_game(coords);

        let updated = match game.cell_state_at(coords) {
            Hidden => game.engine.reveal(coords).has_update(),
            Revealed(_) if game.can_chord_reveal_at(coords) => {
                game.engine.chord_reveal(coords).has_update()
            }
            _ => false,
        };

        if updated {
            game.sync_question_marks_with_engine();
            game.on_successful_move(now);
        }

        updated
    }

    fn mark_cell(&mut self, coords: game::Coord2) -> bool {
        if self.is_generating_layout {
            return false;
        }

        use ViewCellState::*;

        let enable_question_mark = self.settings.enable_question_mark;
        let enable_flag_chord = self.settings.enable_flag_chord;
        let now = utc_now();
        let game = self.get_or_create_game(coords);

        let updated = match game.cell_state_at(coords) {
            Flagged if enable_question_mark => {
                let updated = game.engine.toggle_flag(coords).has_update();
                if updated {
                    game.question_marks[coords.to_nd_index()] = true;
                }
                updated
            }
            Hidden | Flagged | QuestionMarked
                if matches!(game.engine.state(), game::EngineState::Active) =>
            {
                match game.cell_state_at(coords) {
                    QuestionMarked => {
                        game.clear_question_mark(coords);
                        true
                    }
                    _ => {
                        let updated = game.engine.toggle_flag(coords).has_update();
                        if updated {
                            game.clear_question_mark(coords);
                        }
                        updated
                    }
                }
            }
            Revealed(_) if enable_flag_chord => game.engine.chord_flag(coords).has_update(),
            _ => false,
        };

        if updated {
            game.sync_question_marks_with_engine();
            game.on_successful_move(now);
        }

        updated
    }

    fn create_timer(ctx: &Context<Self>) -> Interval {
        let link = ctx.link().clone();
        Interval::new(500, move || link.send_message(Msg::UpdateTime))
    }

    fn is_pressed(&self, coords: game::Coord2, cell_state: ViewCellState) -> bool {
        use ViewCellState::*;

        if self.get_game_state().is_finished() {
            return false;
        }

        const fn is_neighbor(a: game::Coord2, b: game::Coord2) -> bool {
            (a.0.abs_diff(b.0) <= 1) && (a.1.abs_diff(b.1) <= 1)
        }

        match (self.current_cell_state, cell_state) {
            (None, _) => false,
            (_, Flagged | QuestionMarked | TriggeredMine | Mine | Misflagged) => false,
            (
                Some(CellPointerState {
                    pos,
                    buttons: MouseButtons::LEFT,
                }),
                Hidden,
            ) if pos == coords => true,
            (
                Some(CellPointerState {
                    pos,
                    buttons: MouseButtons::LEFT,
                }),
                Hidden,
            ) if is_neighbor(pos, coords) => self
                .game
                .as_ref()
                .map_or(false, |game| game.can_chord_reveal_at(pos)),
            _ => false,
        }
    }

    fn face_button_title(&self) -> &'static str {
        if self.is_generating_layout {
            "Cancel generation"
        } else if self.face_prompt.is_some() {
            "Restart prompt open"
        } else {
            "New game"
        }
    }

    fn view_face_prompt(&self, ctx: &Context<Self>) -> Html {
        let Some(prompt) = self.face_prompt.as_ref() else {
            return Html::default();
        };

        html! {
            <div class="face-prompt-rail" aria-live="polite">
                <div class="face-prompt-bubble">{prompt.message.clone()}</div>
                <div class="face-prompt-choices">
                    {
                        for prompt.choices.iter().map(|choice| {
                            let label = choice.label.clone();
                            let title = choice.title.clone();
                            let action = choice.action;
                            let onclick = ctx.link().callback(move |e: MouseEvent| {
                                e.stop_propagation();
                                Msg::FacePromptSelected(action)
                            });
                            html! {
                                <button class="face-prompt-choice" {title} {onclick}>{label}</button>
                            }
                        })
                    }
                </div>
            </div>
        }
    }
}

impl Component for GameView {
    type Message = Msg;
    type Properties = GameProps;

    fn create(ctx: &Context<Self>) -> Self {
        Self {
            settings: LocalOrDefault::local_or_default(),
            game: LocalOrDefault::local_or_default(),
            seed: js_random_seed(),
            prev_time: 0,
            settings_open: false,
            face_prompt: None,
            current_cell_state: None,
            no_guess_worker: None,
            pending_first_action: None,
            generation_id: 0,
            is_generating_layout: false,
            generation_timeout: None,
            _timer_interval: GameView::create_timer(ctx),
            _init_settings: ctx.props().clone(),
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        use CellMsg::*;
        use Msg::*;

        let updated = match msg {
            CellEvent(Leave) => {
                if self.is_generating_layout {
                    false
                } else {
                    log::trace!("cell leave");
                    self.current_cell_state.take().is_some()
                }
            }
            CellEvent(Update(cell_state)) => {
                if self.is_generating_layout {
                    self.current_cell_state = None;
                    false
                } else {
                    log::trace!("cell update: {:?}", cell_state);
                    if cell_state.buttons.is_empty() {
                        match self.current_cell_state.take() {
                            None => false,
                            Some(CellPointerState { pos, buttons }) => match buttons {
                                MouseButtons::LEFT => {
                                    log::debug!("reveal cell: {:?}", pos);
                                    if self.game.is_none()
                                        && matches!(
                                            self.settings.generator,
                                            settings::Generator::NoGuess
                                        )
                                    {
                                        self.begin_no_guess_generation(ctx, pos)
                                    } else {
                                        self.reveal_cell(pos)
                                    }
                                }
                                MouseButtons::RIGHT => {
                                    log::debug!("mark cell: {:?}", pos);
                                    if self.game.is_none()
                                        && matches!(
                                            self.settings.generator,
                                            settings::Generator::NoGuess
                                        )
                                    {
                                        false
                                    } else {
                                        self.mark_cell(pos)
                                    }
                                }
                                _ => true,
                            },
                        }
                    } else {
                        match self.current_cell_state.replace(cell_state) {
                            None => true,
                            Some(CellPointerState { pos, buttons }) => {
                                (pos != cell_state.pos)
                                    && ((buttons & MouseButtons::LEFT)
                                        != (cell_state.buttons & MouseButtons::LEFT))
                            }
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
            NewGame => self.restart_game(),
            FaceButtonPress => {
                if self.is_generating_layout {
                    self.restart_game()
                } else if self.face_prompt.is_some() {
                    false
                } else {
                    self.open_restart_prompt()
                }
            }
            FacePromptSelected(action) => self.apply_face_prompt_action(action),
            ToggleSettings => {
                self.clear_face_prompt();
                self.settings_open = !self.settings_open;
                if !self.settings_open {
                    self.settings = LocalOrDefault::local_or_default();
                }
                true
            }
            UpdateSettings(settings) => {
                if self.settings != settings {
                    self.settings = settings;
                    self.clear_face_prompt();
                    self.cancel_pending_generation();
                    true
                } else {
                    false
                }
            }
            NoGuessGenerated(response) => self.complete_no_guess_generation(response),
            NoGuessGenerationTimeout(generation_id) => {
                if self.is_generating_layout && self.generation_id == generation_id {
                    let reason = format!(
                        "No-guess worker timed out after {} ms; cancelled generation (no inline fallback)",
                        self.generation_timeout_ms()
                    );
                    self.fail_generation(&reason)
                } else {
                    false
                }
            }
        };

        self.game.local_save();
        updated
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        use Msg::*;
        use settings::SettingsView;

        let (cols, rows) = self.get_size();
        let game_state_icon = self.get_game_state_icon_name();
        let game_state_class = classes!("face-button", game_state_icon);
        let is_playable = self.is_playable();
        let is_generating_layout = self.is_generating_layout;
        let new_game_button_title = self.face_button_title();
        let mines_left = format_for_counter(self.get_mines_left());
        let elapsed_time = format_for_counter(self.get_time() as i32);

        let cb_face_button = ctx.link().callback(|e: MouseEvent| {
            e.stop_propagation();
            FaceButtonPress
        });
        let cb_toggle_settings = ctx.link().callback(|_| ToggleSettings);

        html! {
            <div
                class={classes!("detonito", self.settings_open.then_some("settings-open"))}
                oncontextmenu={Callback::from(move |e: MouseEvent| e.prevent_default())}
            >
                <SpriteDefs/>
                <small onclick={cb_toggle_settings.clone()}>{"···"}</small>
                {
                    if self.settings_open {
                        html! {
                            <SettingsView open={true} on_apply={cb_toggle_settings.clone()}/>
                        }
                    } else {
                        html! {
                            <>
                                <nav>
                                    <aside>
                                        <GlyphRun set={GlyphSet::Counter} text={mines_left} class={classes!("counter-glyphs")}/>
                                    </aside>
                                    <span class={classes!("face-slot", self.face_prompt.is_some().then_some("prompt-open"))}>
                                        {self.view_face_prompt(ctx)}
                                        <button class={game_state_class} title={new_game_button_title} onclick={cb_face_button}>
                                            <Icon
                                                name={game_state_icon}
                                                crop={IconCrop::CenteredSquare64}
                                                class={classes!("state-icon")}
                                            />
                                        </button>
                                    </span>
                                    <aside>
                                        <GlyphRun set={GlyphSet::Counter} text={elapsed_time} class={classes!("counter-glyphs")}/>
                                    </aside>
                                </nav>
                                <table class={classes!(is_playable.then_some("playable"), is_generating_layout.then_some("loading"))}>
                                    {
                                        for (0..rows).map(|y| html! {
                                            <tr>
                                                {
                                                    for (0..cols).map(|x| {
                                                        let pos = (x, y);
                                                        let cell_state = self
                                                            .game
                                                            .as_ref()
                                                            .map_or(ViewCellState::Hidden, |game| game.cell_state_at(pos));
                                                        let loading_cell = self.is_generating_layout
                                                            && self.pending_first_action == Some(pos);
                                                        let locked = self
                                                            .game
                                                            .as_ref()
                                                            .map_or(false, |game| !game.can_interact_at(pos));
                                                        let pressed = loading_cell || self.is_pressed(pos, cell_state);
                                                        let callback = ctx.link().callback(Msg::CellEvent);
                                                        html! {
                                                            <CellView {x} {y} {cell_state} {callback} {pressed} loading={loading_cell} {locked}/>
                                                        }
                                                    })
                                                }
                                            </tr>
                                        })
                                    }
                                </table>
                            </>
                        }
                    }
                }
            </div>
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(0).unwrap()
    }

    #[test]
    fn endgame_render_adapter_maps_triggered_mine_mine_and_misflagged() {
        let layout = game::MineLayout::from_mine_coords((2, 2), &[(0, 0), (0, 1)]).unwrap();
        let mut session = GameSession::new(game::PlayEngine::new(layout));

        assert_eq!(
            session.engine.reveal((1, 1)).unwrap(),
            game::RevealOutcome::Revealed
        );
        session.on_successful_move(t0());

        assert_eq!(
            session.engine.toggle_flag((1, 0)).unwrap(),
            game::MarkOutcome::Changed
        );
        session.on_successful_move(t0());

        assert_eq!(
            session.engine.reveal((0, 0)).unwrap(),
            game::RevealOutcome::HitMine
        );
        session.on_successful_move(t0());

        assert_eq!(session.cell_state_at((0, 0)), ViewCellState::TriggeredMine);
        assert_eq!(session.cell_state_at((0, 1)), ViewCellState::Mine);
        assert_eq!(session.cell_state_at((1, 0)), ViewCellState::Misflagged);
    }

    #[test]
    fn first_move_finish_is_derived_in_session_state() {
        let layout = game::MineLayout::from_mine_coords((2, 1), &[(0, 0)]).unwrap();
        let mut session = GameSession::new(game::PlayEngine::new(layout));

        assert_eq!(
            session.engine.reveal((1, 0)).unwrap(),
            game::RevealOutcome::Won
        );
        session.on_successful_move(t0());

        assert_eq!(session.view_state(), ViewGameState::WonOnFirstMove);
    }

    #[test]
    fn mines_left_for_display_is_zero_when_won() {
        let layout = game::MineLayout::from_mine_coords((2, 1), &[(0, 0)]).unwrap();
        let mut session = GameSession::new(game::PlayEngine::new(layout));

        assert_eq!(
            session.engine.reveal((1, 0)).unwrap(),
            game::RevealOutcome::Won
        );
        session.on_successful_move(t0());

        assert_eq!(session.mines_left_for_display(), 0);
    }

    #[test]
    fn mines_left_for_display_matches_engine_while_active() {
        let layout = game::MineLayout::from_mine_coords((2, 2), &[(0, 0), (1, 1)]).unwrap();
        let mut session = GameSession::new(game::PlayEngine::new(layout));

        assert_eq!(
            session.engine.reveal((1, 0)).unwrap(),
            game::RevealOutcome::Revealed
        );
        session.on_successful_move(t0());

        assert_eq!(
            session.engine.toggle_flag((0, 0)).unwrap(),
            game::MarkOutcome::Changed
        );
        session.on_successful_move(t0());

        assert_eq!(
            session.mines_left_for_display(),
            session.engine.mines_left() as i32
        );
    }

    #[test]
    fn storage_key_uses_new_versioned_namespace() {
        assert_eq!(<GameSession as StorageKey>::KEY, "detonito:game:v2");
    }

    #[test]
    fn restart_confirmation_prompt_has_expected_message_and_choices() {
        let prompt = FacePrompt::restart_confirmation();
        let labels_and_actions = prompt
            .choices
            .iter()
            .map(|choice| (choice.label.as_ref(), choice.action))
            .collect::<Vec<_>>();

        assert_eq!(prompt.message.as_ref(), "Restart?");
        assert_eq!(
            labels_and_actions,
            vec![
                ("Yes", FacePromptAction::RestartGame),
                ("No", FacePromptAction::DismissPrompt),
                ("Menu", FacePromptAction::OpenSettings),
            ]
        );
    }
}
