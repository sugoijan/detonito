use crate::sprites::{Icon, IconCrop};
use crate::theme::Theme;
use crate::utils::*;
use detonito_core as game;
use gloo::timers::callback::{Interval, Timeout};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::sync::LazyLock;
use wasm_bindgen::JsCast;
use web_sys::{FocusEvent, HtmlElement, HtmlInputElement, InputEvent, KeyboardEvent, PointerEvent};
use yew::{TargetCast, prelude::*};

pub const BEGINNER: game::GameConfig = game::GameConfig::new_unchecked((9, 9), 10);
pub const INTERMEDIATE: game::GameConfig = game::GameConfig::new_unchecked((16, 16), 40);
pub const EXPERT: game::GameConfig = game::GameConfig::new_unchecked((30, 16), 99);
pub const EVIL: game::GameConfig = game::GameConfig::new_unchecked((30, 20), 130);

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Generator {
    /// Purely random, even the first tile can have a bomb, that's unlucky
    RandomGamble,
    /// First move is forced to a zero-cell when possible.
    RandomZeroStart,
    /// Guaranteed no guess needed to win
    NoGuess,
    // TODO: NoGuess where guesses are guaranteed losses
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub game_config: game::GameConfig,
    pub generator: Generator,
    pub enable_question_mark: bool,
    pub enable_flag_chord: bool,
    pub enable_auto_trivial: bool,
    #[serde(default = "Settings::default_zoom_percent")]
    pub zoom_percent: u16,
}

impl Settings {
    const MAX_SIZE: game::Coord = 99;
    const DEFAULT_ZOOM_PERCENT: u16 = 175;
    const MIN_ZOOM_PERCENT: u16 = 50;
    const MAX_ZOOM_PERCENT: u16 = 500;
    const ZOOM_STEP_PERCENT: u16 = 5;
    const ZOOM_CSS_VAR_NAME: &'static str = "--detonito-zoom";

    const fn default_zoom_percent() -> u16 {
        Self::DEFAULT_ZOOM_PERCENT
    }

    pub(crate) fn normalize_zoom_percent(value: u16) -> u16 {
        value.clamp(Self::MIN_ZOOM_PERCENT, Self::MAX_ZOOM_PERCENT)
    }

    pub(crate) fn zoom_percent(&self) -> u16 {
        Self::normalize_zoom_percent(self.zoom_percent)
    }

    fn zoom_css_value(&self) -> String {
        format!("{}%", self.zoom_percent())
    }

    fn update_html_zoom(zoom: &str) {
        use gloo::utils::document;

        let Some(html) = document().document_element() else {
            return;
        };

        let Ok(html) = html.dyn_into::<HtmlElement>() else {
            return;
        };

        if let Err(err) = html.style().set_property(Self::ZOOM_CSS_VAR_NAME, zoom) {
            log::error!("failed to set zoom css variable: {:?}", err);
        }
    }

    pub(crate) fn init() {
        let settings = Self::local_or_default();
        settings.apply_display();
    }

    pub(crate) fn apply_display(&self) {
        Self::update_html_zoom(&self.zoom_css_value());
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game_config: BEGINNER,
            generator: Generator::NoGuess,
            enable_question_mark: false,
            enable_flag_chord: true,
            enable_auto_trivial: true,
            zoom_percent: Self::DEFAULT_ZOOM_PERCENT,
        }
    }
}

impl StorageKey for Settings {
    const KEY: &'static str = "detonito:settings";
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum SettingsAction {
    SetGameConfig(game::GameConfig),
    SetGenerator(Generator),
    SetSizeX(u16),
    SetSizeY(u16),
    SetMines(u16),
    SetZoom(u16),
    IncreaseSizeX,
    DecreaseSizeX,
    IncreaseSizeY,
    DecreaseSizeY,
    IncreaseMines,
    DecreaseMines,
    IncreaseZoom,
    DecreaseZoom,
}

impl Reducible for Settings {
    type Action = SettingsAction;

    fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
        use SettingsAction::*;
        let mut settings = Rc::unwrap_or_clone(self);
        match action {
            SetGameConfig(game_config) => {
                settings.game_config = game_config;
            }
            SetGenerator(generator) => {
                settings.generator = generator;
            }
            SetSizeX(value) => {
                settings.game_config.size.0 =
                    value.clamp(1, Settings::MAX_SIZE.into()) as game::Coord;
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            SetSizeY(value) => {
                settings.game_config.size.1 =
                    value.clamp(1, Settings::MAX_SIZE.into()) as game::Coord;
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            SetMines(value) => {
                settings.game_config.mines = value.clamp(1, settings.game_config.total_cells());
            }
            SetZoom(value) => {
                settings.zoom_percent = Settings::normalize_zoom_percent(value);
            }
            IncreaseSizeX => {
                settings.game_config.size.0 = settings.game_config.size.0.saturating_add(1);
                settings.game_config.size.0 =
                    settings.game_config.size.0.clamp(1, Settings::MAX_SIZE);
            }
            DecreaseSizeX => {
                settings.game_config.size.0 = settings.game_config.size.0.saturating_sub(1);
                settings.game_config.size.0 =
                    settings.game_config.size.0.clamp(1, Settings::MAX_SIZE);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            IncreaseSizeY => {
                settings.game_config.size.1 = settings.game_config.size.1.saturating_add(1);
                settings.game_config.size.1 =
                    settings.game_config.size.1.clamp(1, Settings::MAX_SIZE);
            }
            DecreaseSizeY => {
                settings.game_config.size.1 = settings.game_config.size.1.saturating_sub(1);
                settings.game_config.size.1 =
                    settings.game_config.size.1.clamp(1, Settings::MAX_SIZE);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            IncreaseMines => {
                settings.game_config.mines = settings.game_config.mines.saturating_add(1);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            DecreaseMines => {
                settings.game_config.mines = settings.game_config.mines.saturating_sub(1);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            IncreaseZoom => {
                settings.zoom_percent = settings
                    .zoom_percent()
                    .saturating_add(Settings::ZOOM_STEP_PERCENT);
                settings.zoom_percent = Settings::normalize_zoom_percent(settings.zoom_percent);
            }
            DecreaseZoom => {
                settings.zoom_percent = settings
                    .zoom_percent()
                    .saturating_sub(Settings::ZOOM_STEP_PERCENT);
                settings.zoom_percent = Settings::normalize_zoom_percent(settings.zoom_percent);
            }
        }
        settings.apply_display();
        settings.local_save();
        settings.into()
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct SettingsProps {
    #[prop_or_default]
    pub open: bool,
    pub on_apply: Callback<MouseEvent>,
    #[prop_or_default]
    pub on_back: Option<Callback<MouseEvent>>,
    #[prop_or(SettingsEntryPoint::Root)]
    pub initial_page: SettingsEntryPoint,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum SettingsEntryPoint {
    #[default]
    Root,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SettingsMenuPage {
    Root,
    Difficulty,
    Classic,
    ModernNg,
    Custom,
    Generation,
    Theme,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DifficultyChoice {
    ClassicBeginner,
    ClassicIntermediate,
    ClassicExpert,
    ModernEasy,
    ModernMedium,
    ModernHard,
    ModernEvil,
    Custom,
}

const MENU_COLUMNS: usize = 14;
const MENU_CHAR_PADDING: usize = 2;
const MENU_CHARS_PER_FIVE_COLS: usize = 12;
const MENU_MARQUEE_EXTRA_SHIFT: usize = 2;
const ABOUT_INDEX_LABEL_COLSPAN: usize = 5;
const DETAIL_LINK_LABEL_COLSPAN: usize = 4;
const HOLD_REPEAT_DELAY_MS: u32 = 350;
const HOLD_REPEAT_INTERVAL_MS: u32 = 75;
const HOLD_CLICK_SUPPRESSION_CLEAR_MS: u32 = 250;

#[derive(Properties, PartialEq)]
pub(crate) struct AboutProps {
    #[prop_or_default]
    pub open: bool,
    pub on_back: Callback<MouseEvent>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct CreditsManifest {
    entries: Vec<CreditEntry>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct CreditEntry {
    id: String,
    #[serde(default)]
    parent: Option<String>,
    name: String,
    kind: CreditEntryKind,
    relation: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    text: String,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    links: Vec<CreditLink>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CreditEntryKind {
    License,
    Note,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct CreditLink {
    label: String,
    url: String,
}

static CREDITS_MANIFEST: LazyLock<CreditsManifest> = LazyLock::new(|| {
    toml::from_str(include_str!("../assets/licenses/third_party.toml"))
        .expect("credits manifest should parse")
});

fn difficulty_choice(settings: &Settings) -> DifficultyChoice {
    match (settings.generator, settings.game_config) {
        (Generator::RandomZeroStart, config) if config == BEGINNER => {
            DifficultyChoice::ClassicBeginner
        }
        (Generator::RandomZeroStart, config) if config == INTERMEDIATE => {
            DifficultyChoice::ClassicIntermediate
        }
        (Generator::RandomZeroStart, config) if config == EXPERT => DifficultyChoice::ClassicExpert,
        (Generator::NoGuess, config) if config == BEGINNER => DifficultyChoice::ModernEasy,
        (Generator::NoGuess, config) if config == INTERMEDIATE => DifficultyChoice::ModernMedium,
        (Generator::NoGuess, config) if config == EXPERT => DifficultyChoice::ModernHard,
        (Generator::NoGuess, config) if config == EVIL => DifficultyChoice::ModernEvil,
        _ => DifficultyChoice::Custom,
    }
}

fn difficulty_label(choice: DifficultyChoice) -> &'static str {
    match choice {
        DifficultyChoice::ClassicBeginner => "Beginner",
        DifficultyChoice::ClassicIntermediate => "Intermediate",
        DifficultyChoice::ClassicExpert => "Expert",
        DifficultyChoice::ModernEasy => "Easy",
        DifficultyChoice::ModernMedium => "Medium",
        DifficultyChoice::ModernHard => "Hard",
        DifficultyChoice::ModernEvil => "Evil",
        DifficultyChoice::Custom => "Custom",
    }
}

fn game_config_summary(config: &game::GameConfig) -> String {
    format!("{}x{} / {}", config.size.0, config.size.1, config.mines)
}

fn generator_label(generator: Generator) -> &'static str {
    match generator {
        Generator::RandomGamble => "Gamble",
        Generator::RandomZeroStart => "Zero-start",
        Generator::NoGuess => "No-guess",
    }
}

fn theme_label(theme: Option<Theme>) -> &'static str {
    match theme {
        Some(Theme::Light) => "Light",
        Some(Theme::Dark) => "Dark",
        None => "System",
    }
}

fn zoom_label(zoom_percent: u16) -> String {
    format!("{}%", zoom_percent)
}

fn menu_blank_cells(count: usize) -> Html {
    Html::from_iter((0..count).map(|_| html! { <td class="menu-pad"/> }))
}

fn menu_blank_row() -> Html {
    html! {
        <tr>{menu_blank_cells(MENU_COLUMNS)}</tr>
    }
}

fn menu_icon_button(
    icon: &'static str,
    title: impl Into<AttrValue>,
    pressed: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    html! {
        <button class={classes!(pressed.then_some("pressed"))} title={title.into()} {onclick}>
            <Icon name={icon} crop={IconCrop::CenteredSquare64} class={classes!("button-icon")}/>
        </button>
    }
}

fn menu_title_row(title: impl Into<AttrValue>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-heading" colspan="12">{title.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn approximate_menu_char_capacity(colspan: usize) -> usize {
    (((MENU_CHARS_PER_FIVE_COLS + MENU_CHAR_PADDING) * colspan) + 2)
        .div_euclid(5)
        .saturating_sub(MENU_CHAR_PADDING)
}

fn maybe_marquee_label(label: String, colspan: usize) -> Html {
    let overflow = label
        .chars()
        .count()
        .saturating_sub(approximate_menu_char_capacity(colspan));
    if overflow == 0 {
        return html! { <>{label}</> };
    }

    let shift = overflow + MENU_MARQUEE_EXTRA_SHIFT;
    let duration = 3.0 + shift as f32 * 0.35;
    html! {
        <span class="menu-marquee-window">
            <span
                class="menu-marquee-text"
                style={format!(
                    "--menu-marquee-shift: {shift}ch; --menu-marquee-duration: {duration:.2}s;"
                )}
            >
                {label}
            </span>
        </span>
    }
}

fn menu_entry_row(label: impl Into<AttrValue>, detail: impl Into<AttrValue>, button: Html) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">{button}</td>
            <td class="menu-text" colspan="5">{label.into()}</td>
            <td class="menu-detail" colspan="6">{detail.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_about_index_row(label: String, detail: impl Into<AttrValue>, button: Html) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">{button}</td>
            <td class="menu-text" colspan="5">{maybe_marquee_label(label, ABOUT_INDEX_LABEL_COLSPAN)}</td>
            <td class="menu-detail" colspan="6">{detail.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_header_row(title: impl Into<AttrValue>, on_back: Callback<MouseEvent>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">
                {menu_icon_button("minus", "Go back", false, on_back)}
            </td>
            <td class="menu-heading" colspan="11">{title.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_info_row(label: impl Into<AttrValue>, detail: impl Into<AttrValue>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-text" colspan="5">{label.into()}</td>
            <td class="menu-detail" colspan="7">{detail.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_link_row(label: String, detail: impl Into<AttrValue>, button: Html) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">{button}</td>
            <td class="menu-text" colspan="4">{maybe_marquee_label(label, DETAIL_LINK_LABEL_COLSPAN)}</td>
            <td class={classes!("menu-detail", "menu-link-detail")} colspan="7">{detail.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_copy_row(text: impl Into<AttrValue>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-about-copy" colspan="12">{text.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

fn menu_adjust_row(
    label: &'static str,
    value: u16,
    min: u16,
    max: u16,
    on_decrease: Callback<()>,
    on_set: Callback<u16>,
    on_increase: Callback<()>,
) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-text" colspan="5">{label}</td>
            <td class="menu-button-slot">
                <RepeatIconButton
                    icon="minus"
                    title={format!("Decrease {}", label)}
                    on_activate={on_decrease}
                />
            </td>
            <td class={classes!("menu-detail", "menu-number-detail")} colspan="5">
                <MenuNumberField
                    label={label}
                    value={value}
                    min={min}
                    max={max}
                    on_set={on_set}
                />
            </td>
            <td class="menu-button-slot">
                <RepeatIconButton
                    icon="plus"
                    title={format!("Increase {}", label)}
                    on_activate={on_increase}
                />
            </td>
            <td class="menu-pad"/>
        </tr>
    }
}

#[derive(Properties, PartialEq)]
struct RepeatIconButtonProps {
    icon: &'static str,
    title: AttrValue,
    on_activate: Callback<()>,
}

#[function_component]
fn RepeatIconButton(props: &RepeatIconButtonProps) -> Html {
    let is_pressed = use_state_eq(|| false);
    let repeat_timeout = use_mut_ref(|| None::<Timeout>);
    let repeat_interval = use_mut_ref(|| None::<Interval>);
    let suppress_click = use_mut_ref(|| false);
    let suppress_clear_timeout = use_mut_ref(|| None::<Timeout>);

    let stop_repeat = {
        let is_pressed = is_pressed.clone();
        let repeat_timeout = repeat_timeout.clone();
        let repeat_interval = repeat_interval.clone();
        let suppress_click = suppress_click.clone();
        let suppress_clear_timeout = suppress_clear_timeout.clone();
        Rc::new(move || {
            is_pressed.set(false);
            repeat_timeout.borrow_mut().take();
            repeat_interval.borrow_mut().take();
            suppress_clear_timeout.borrow_mut().take();
            if *suppress_click.borrow() {
                let suppress_click = suppress_click.clone();
                *suppress_clear_timeout.borrow_mut() =
                    Some(Timeout::new(HOLD_CLICK_SUPPRESSION_CLEAR_MS, move || {
                        *suppress_click.borrow_mut() = false;
                    }));
            }
        })
    };

    let onpointerdown = {
        let is_pressed = is_pressed.clone();
        let repeat_timeout = repeat_timeout.clone();
        let repeat_interval = repeat_interval.clone();
        let suppress_click = suppress_click.clone();
        let suppress_clear_timeout = suppress_clear_timeout.clone();
        let on_activate = props.on_activate.clone();
        Callback::from(move |event: PointerEvent| {
            if !event.is_primary() || event.button() != 0 {
                return;
            }

            is_pressed.set(true);
            *suppress_click.borrow_mut() = true;
            suppress_clear_timeout.borrow_mut().take();
            repeat_timeout.borrow_mut().take();
            repeat_interval.borrow_mut().take();

            on_activate.emit(());

            let on_activate = on_activate.clone();
            let repeat_interval = repeat_interval.clone();
            *repeat_timeout.borrow_mut() = Some(Timeout::new(HOLD_REPEAT_DELAY_MS, move || {
                on_activate.emit(());

                let on_activate = on_activate.clone();
                *repeat_interval.borrow_mut() =
                    Some(Interval::new(HOLD_REPEAT_INTERVAL_MS, move || {
                        on_activate.emit(());
                    }));
            }));
        })
    };

    let onclick = {
        let suppress_click = suppress_click.clone();
        let suppress_clear_timeout = suppress_clear_timeout.clone();
        let on_activate = props.on_activate.clone();
        Callback::from(move |_event: MouseEvent| {
            let was_suppressed = *suppress_click.borrow();
            if was_suppressed {
                *suppress_click.borrow_mut() = false;
                suppress_clear_timeout.borrow_mut().take();
                return;
            }
            on_activate.emit(());
        })
    };

    let onpointerup = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: PointerEvent| stop_repeat())
    };

    let onpointerleave = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: PointerEvent| stop_repeat())
    };

    let onpointercancel = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: PointerEvent| stop_repeat())
    };

    let onblur = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: FocusEvent| stop_repeat())
    };

    html! {
        <button
            class={classes!((*is_pressed).then_some("pressed"))}
            type="button"
            title={props.title.clone()}
            {onclick}
            {onblur}
            {onpointerdown}
            {onpointerup}
            {onpointerleave}
            {onpointercancel}
        >
            <Icon name={props.icon} crop={IconCrop::CenteredSquare64} class={classes!("button-icon")}/>
        </button>
    }
}

#[derive(Properties, PartialEq)]
struct MenuNumberFieldProps {
    label: &'static str,
    value: u16,
    min: u16,
    max: u16,
    on_set: Callback<u16>,
}

#[function_component]
fn MenuNumberField(props: &MenuNumberFieldProps) -> Html {
    let input_ref = use_node_ref();
    let is_editing = use_state_eq(|| false);
    let draft = use_state_eq(|| props.value.to_string());

    let commit = {
        let draft = draft.clone();
        let is_editing = is_editing.clone();
        let on_set = props.on_set.clone();
        Rc::new(move || {
            if let Ok(parsed) = draft.trim().parse::<u16>() {
                on_set.emit(parsed);
            }
            is_editing.set(false);
        })
    };

    let cancel = {
        let is_editing = is_editing.clone();
        Rc::new(move || is_editing.set(false))
    };

    let onfocus = {
        let draft = draft.clone();
        let input_ref = input_ref.clone();
        let is_editing = is_editing.clone();
        let value = props.value;
        Callback::from(move |_event: FocusEvent| {
            is_editing.set(true);
            draft.set(value.to_string());
            if let Some(input) = input_ref.cast::<HtmlInputElement>() {
                let _ = input.select();
            }
        })
    };

    let oninput = {
        let draft = draft.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            draft.set(input.value());
        })
    };

    let onblur = {
        let commit = commit.clone();
        Callback::from(move |_event: FocusEvent| commit())
    };

    let onkeydown = {
        let commit = commit.clone();
        let cancel = cancel.clone();
        let input_ref = input_ref.clone();
        Callback::from(move |event: KeyboardEvent| match event.key().as_str() {
            "Enter" => {
                event.prevent_default();
                commit();
                if let Some(input) = input_ref.cast::<HtmlInputElement>() {
                    let _ = input.blur();
                }
            }
            "Escape" => {
                event.prevent_default();
                cancel();
                if let Some(input) = input_ref.cast::<HtmlInputElement>() {
                    let _ = input.blur();
                }
            }
            _ => {}
        })
    };

    let value = if *is_editing {
        (*draft).clone()
    } else {
        props.value.to_string()
    };

    html! {
        <input
            ref={input_ref}
            class="menu-number-input"
            type="number"
            inputmode="numeric"
            title={format!("Set {}", props.label)}
            aria-label={format!("Set {}", props.label)}
            min={props.min.to_string()}
            max={props.max.to_string()}
            step="1"
            value={value}
            {onfocus}
            {oninput}
            {onblur}
            {onkeydown}
        />
    }
}

fn link_summary(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

fn open_external_link(url: String) -> Callback<MouseEvent> {
    Callback::from(move |_| {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Err(err) = window.open_with_url_and_target(&url, "_blank") {
            log::error!("failed to open external link {url}: {:?}", err);
        }
    })
}

fn credit_link_row(link: &CreditLink) -> Html {
    let icon = if link.label == "Project" {
        "home"
    } else {
        "details"
    };
    menu_link_row(
        link.label.clone(),
        link_summary(&link.url),
        menu_icon_button(
            icon,
            format!("Open {}", link.label),
            false,
            open_external_link(link.url.clone()),
        ),
    )
}

fn credit_index_row(entry: &CreditEntry, on_open: Callback<MouseEvent>) -> Html {
    menu_about_index_row(
        entry.name.clone(),
        entry.relation.clone(),
        menu_icon_button("plus", format!("Open {}", entry.name), false, on_open),
    )
}

fn credit_detail_rows(entry: &CreditEntry) -> Vec<Html> {
    let mut rows = Vec::new();
    if let Some(license) = entry.license.as_deref() {
        rows.push(menu_info_row("License", license.to_string()));
    }
    rows.push(menu_copy_row(entry.text.clone()));
    if let Some(details) = entry.details.as_deref() {
        rows.push(menu_copy_row(details.to_string()));
    }
    if !entry.links.is_empty() {
        rows.push(menu_blank_row());
        rows.extend(entry.links.iter().map(credit_link_row));
    }
    rows
}

fn child_credit_rows(entry: &CreditEntry) -> Vec<Html> {
    let mut rows = Vec::new();
    rows.push(menu_blank_row());
    rows.push(menu_title_row(entry.name.clone()));
    if let Some(license) = entry.license.as_deref() {
        rows.push(menu_info_row("License", license.to_string()));
    }
    rows.push(menu_copy_row(entry.text.clone()));
    if let Some(details) = entry.details.as_deref() {
        rows.push(menu_copy_row(details.to_string()));
    }
    rows.extend(entry.links.iter().map(credit_link_row));
    rows
}

fn menu_dual_action_row(
    left_button: Html,
    left_label: &'static str,
    right_button: Html,
    right_label: &'static str,
) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">{left_button}</td>
            <td class="menu-text" colspan="4">{left_label}</td>
            <td class="menu-pad" colspan="2"/>
            <td class="menu-button-slot">{right_button}</td>
            <td class="menu-text" colspan="4">{right_label}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

#[function_component]
pub(crate) fn SettingsView(props: &SettingsProps) -> Html {
    let settings: UseReducerHandle<Settings> = use_reducer_eq(LocalOrDefault::local_or_default);
    let theme: UseStateHandle<Option<Theme>> = use_state_eq(LocalOrDefault::local_or_default);
    let original_settings = use_state_eq(|| (*settings).clone());
    let original_theme = use_state_eq(|| *theme);
    let page = {
        let initial_page = props.initial_page;
        use_state_eq(move || match initial_page {
            SettingsEntryPoint::Root => SettingsMenuPage::Root,
        })
    };

    let set_theme_light = {
        let theme = theme.clone();
        move |_| {
            let new_theme = Theme::Light.into();
            theme.set(new_theme);
            Theme::apply(new_theme)
        }
    };

    let set_theme_dark = {
        let theme = theme.clone();
        move |_| {
            let new_theme = Theme::Dark.into();
            theme.set(new_theme);
            Theme::apply(new_theme)
        }
    };

    let set_theme_auto = {
        let theme = theme.clone();
        move |_| {
            let new_theme = None;
            theme.set(new_theme);
            Theme::apply(new_theme)
        }
    };

    let set_generator_gamble = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetGenerator(Generator::RandomGamble))
    };

    let set_generator_random = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetGenerator(Generator::RandomZeroStart))
    };

    let set_generator_puzzle = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetGenerator(Generator::NoGuess))
    };

    let inc_mines = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::IncreaseMines))
    };

    let dec_mines = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::DecreaseMines))
    };

    let set_mines = {
        let settings = settings.clone();
        Callback::from(move |value: u16| settings.dispatch(SettingsAction::SetMines(value)))
    };

    let inc_zoom = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::IncreaseZoom))
    };

    let dec_zoom = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::DecreaseZoom))
    };

    let set_zoom = {
        let settings = settings.clone();
        Callback::from(move |value: u16| settings.dispatch(SettingsAction::SetZoom(value)))
    };

    let inc_size_x = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::IncreaseSizeX))
    };

    let dec_size_x = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::DecreaseSizeX))
    };

    let set_size_x = {
        let settings = settings.clone();
        Callback::from(move |value: u16| settings.dispatch(SettingsAction::SetSizeX(value)))
    };

    let inc_size_y = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::IncreaseSizeY))
    };

    let dec_size_y = {
        let settings = settings.clone();
        Callback::from(move |()| settings.dispatch(SettingsAction::DecreaseSizeY))
    };

    let set_size_y = {
        let settings = settings.clone();
        Callback::from(move |value: u16| settings.dispatch(SettingsAction::SetSizeY(value)))
    };

    let set_classic_beginner = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::RandomZeroStart));
            settings.dispatch(SettingsAction::SetGameConfig(BEGINNER));
        }
    };

    let set_classic_intermediate = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::RandomZeroStart));
            settings.dispatch(SettingsAction::SetGameConfig(INTERMEDIATE));
        }
    };

    let set_classic_expert = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::RandomZeroStart));
            settings.dispatch(SettingsAction::SetGameConfig(EXPERT));
        }
    };

    let set_modern_easy = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::NoGuess));
            settings.dispatch(SettingsAction::SetGameConfig(BEGINNER));
        }
    };

    let set_modern_medium = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::NoGuess));
            settings.dispatch(SettingsAction::SetGameConfig(INTERMEDIATE));
        }
    };

    let set_modern_hard = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::NoGuess));
            settings.dispatch(SettingsAction::SetGameConfig(EXPERT));
        }
    };

    let set_modern_evil = {
        let settings = settings.clone();
        move |_| {
            settings.dispatch(SettingsAction::SetGenerator(Generator::NoGuess));
            settings.dispatch(SettingsAction::SetGameConfig(EVIL));
        }
    };

    let open_difficulty = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Difficulty))
    };

    let open_classic = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Classic))
    };

    let open_modern_ng = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::ModernNg))
    };

    let open_generation = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Generation))
    };

    let open_custom = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Custom))
    };

    let open_theme = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Theme))
    };

    let back_to_root = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Root))
    };

    let back_to_difficulty = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Difficulty))
    };

    let back_to_custom = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Custom))
    };

    let cancel_changes = {
        let original_settings = original_settings.clone();
        let original_theme = original_theme.clone();
        let theme = theme.clone();
        let on_apply = props.on_apply.clone();
        Callback::from(move |event: MouseEvent| {
            let settings_snapshot = (*original_settings).clone();
            let theme_snapshot = *original_theme;
            settings_snapshot.apply_display();
            settings_snapshot.local_save();
            theme.set(theme_snapshot);
            Theme::apply(theme_snapshot);
            on_apply.emit(event);
        })
    };

    let current_choice = difficulty_choice(&settings);
    let current_difficulty_label = difficulty_label(current_choice);
    let current_theme_label = theme_label(*theme);
    let current_zoom_label = zoom_label(settings.zoom_percent());
    let current_generator_label = generator_label(settings.generator);
    let custom_summary = game_config_summary(&settings.game_config);
    let theme_detail = format!("{current_theme_label} / {current_zoom_label}");
    let classic_detail = match current_choice {
        DifficultyChoice::ClassicBeginner => "Beginner",
        DifficultyChoice::ClassicIntermediate => "Intermediate",
        DifficultyChoice::ClassicExpert => "Expert",
        _ => "",
    };
    let modern_detail = match current_choice {
        DifficultyChoice::ModernEasy => "Easy",
        DifficultyChoice::ModernMedium => "Medium",
        DifficultyChoice::ModernHard => "Hard",
        DifficultyChoice::ModernEvil => "Evil",
        _ => "",
    };
    let custom_detail = if current_choice == DifficultyChoice::Custom {
        custom_summary.clone()
    } else {
        String::new()
    };

    let menu_body = match *page {
        SettingsMenuPage::Root => html! {
            <>
                {menu_blank_row()}
                {
                    if let Some(on_back) = props.on_back.clone() {
                        menu_header_row("Settings", on_back)
                    } else {
                        menu_title_row("Settings")
                    }
                }
                {menu_blank_row()}
                {menu_entry_row(
                    "Difficulty",
                    current_difficulty_label,
                    menu_icon_button("plus", "Open difficulty menu", false, open_difficulty),
                )}
                {menu_blank_row()}
                {menu_entry_row(
                    "Theme",
                    theme_detail,
                    menu_icon_button("plus", "Open theme menu", false, open_theme),
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::Difficulty => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Difficulty", back_to_root)}
                {menu_blank_row()}
                {menu_entry_row(
                    "Modern NG",
                    modern_detail,
                    menu_icon_button("plus", "Open modern difficulty menu", false, open_modern_ng),
                )}
                {menu_entry_row(
                    "Classic",
                    classic_detail,
                    menu_icon_button("plus", "Open classic difficulty menu", false, open_classic),
                )}
                {menu_entry_row(
                    "Custom",
                    custom_detail.clone(),
                    menu_icon_button("plus", "Open custom board menu", false, open_custom),
                )}
                {menu_blank_row()}
                {menu_dual_action_row(
                    menu_icon_button("ok", "Apply settings", false, props.on_apply.clone()),
                    "Apply",
                    menu_icon_button("cancel", "Cancel changes", false, cancel_changes),
                    "Cancel",
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::Classic => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Classic", back_to_difficulty)}
                {menu_blank_row()}
                {menu_entry_row(
                    "Beginner",
                    game_config_summary(&BEGINNER),
                    menu_icon_button(
                        "diff-beginner",
                        "Use classic beginner preset",
                        current_choice == DifficultyChoice::ClassicBeginner,
                        Callback::from(set_classic_beginner),
                    ),
                )}
                {menu_entry_row(
                    "Intermediate",
                    game_config_summary(&INTERMEDIATE),
                    menu_icon_button(
                        "diff-intermediate",
                        "Use classic intermediate preset",
                        current_choice == DifficultyChoice::ClassicIntermediate,
                        Callback::from(set_classic_intermediate),
                    ),
                )}
                {menu_entry_row(
                    "Expert",
                    game_config_summary(&EXPERT),
                    menu_icon_button(
                        "diff-expert",
                        "Use classic expert preset",
                        current_choice == DifficultyChoice::ClassicExpert,
                        Callback::from(set_classic_expert),
                    ),
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::ModernNg => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Modern NG", back_to_difficulty)}
                {menu_blank_row()}
                {menu_entry_row(
                    "Easy",
                    game_config_summary(&BEGINNER),
                    menu_icon_button(
                        "diff-beginner",
                        "Use modern easy preset",
                        current_choice == DifficultyChoice::ModernEasy,
                        Callback::from(set_modern_easy),
                    ),
                )}
                {menu_entry_row(
                    "Medium",
                    game_config_summary(&INTERMEDIATE),
                    menu_icon_button(
                        "diff-intermediate",
                        "Use modern medium preset",
                        current_choice == DifficultyChoice::ModernMedium,
                        Callback::from(set_modern_medium),
                    ),
                )}
                {menu_entry_row(
                    "Hard",
                    game_config_summary(&EXPERT),
                    menu_icon_button(
                        "diff-expert",
                        "Use modern hard preset",
                        current_choice == DifficultyChoice::ModernHard,
                        Callback::from(set_modern_hard),
                    ),
                )}
                {menu_entry_row(
                    "Evil",
                    game_config_summary(&EVIL),
                    menu_icon_button(
                        "diff-evil",
                        "Use modern evil preset",
                        current_choice == DifficultyChoice::ModernEvil,
                        Callback::from(set_modern_evil),
                    ),
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::Generation => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Generation", back_to_custom)}
                {menu_blank_row()}
                {menu_entry_row(
                    "Gamble",
                    "",
                    menu_icon_button(
                        "gamble",
                        "Use gamble generator",
                        settings.generator == Generator::RandomGamble,
                        Callback::from(set_generator_gamble),
                    ),
                )}
                {menu_entry_row(
                    "Zero-start",
                    "",
                    menu_icon_button(
                        "random",
                        "Use zero-start generator",
                        settings.generator == Generator::RandomZeroStart,
                        Callback::from(set_generator_random),
                    ),
                )}
                {menu_entry_row(
                    "No-guess",
                    "",
                    menu_icon_button(
                        "puzzle",
                        "Use no-guess generator",
                        settings.generator == Generator::NoGuess,
                        Callback::from(set_generator_puzzle),
                    ),
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::Custom => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Custom", back_to_difficulty)}
                {menu_blank_row()}
                {menu_entry_row(
                    "Generation",
                    current_generator_label,
                    menu_icon_button("plus", "Open generation menu", false, open_generation),
                )}
                {menu_blank_row()}
                {menu_adjust_row(
                    "Width",
                    settings.game_config.size.0.into(),
                    1,
                    Settings::MAX_SIZE.into(),
                    dec_size_x,
                    set_size_x,
                    inc_size_x,
                )}
                {menu_adjust_row(
                    "Height",
                    settings.game_config.size.1.into(),
                    1,
                    Settings::MAX_SIZE.into(),
                    dec_size_y,
                    set_size_y,
                    inc_size_y,
                )}
                {menu_adjust_row(
                    "Mines",
                    settings.game_config.mines,
                    1,
                    settings.game_config.total_cells(),
                    dec_mines,
                    set_mines,
                    inc_mines,
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::Theme => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Theme", back_to_root)}
                {menu_blank_row()}
                {menu_entry_row(
                    "Dark",
                    "Window theme",
                    menu_icon_button(
                        "theme-dark",
                        "Use dark theme",
                        matches!(*theme, Some(Theme::Dark)),
                        Callback::from(set_theme_dark),
                    ),
                )}
                {menu_entry_row(
                    "Light",
                    "Paper theme",
                    menu_icon_button(
                        "theme-light",
                        "Use light theme",
                        matches!(*theme, Some(Theme::Light)),
                        Callback::from(set_theme_light),
                    ),
                )}
                {menu_entry_row(
                    "System",
                    "Follow device",
                    menu_icon_button(
                        "theme-auto",
                        "Use system theme",
                        matches!(*theme, None),
                        Callback::from(set_theme_auto),
                    ),
                )}
                {menu_blank_row()}
                {menu_adjust_row(
                    "Zoom",
                    settings.zoom_percent(),
                    Settings::MIN_ZOOM_PERCENT,
                    Settings::MAX_ZOOM_PERCENT,
                    dec_zoom,
                    set_zoom,
                    inc_zoom,
                )}
                {menu_blank_row()}
            </>
        },
    };

    html! {
        <dialog open={props.open}>
            <table class="menu-grid">
                <tbody>
                    {menu_body}
                </tbody>
            </table>
        </dialog>
    }
}

#[function_component]
pub(crate) fn AboutView(props: &AboutProps) -> Html {
    let selected_credit = use_state_eq(|| None::<usize>);

    let back_to_index = {
        let selected_credit = selected_credit.clone();
        Callback::from(move |_| selected_credit.set(None))
    };

    let menu_body = if let Some(index) = *selected_credit {
        let entry = &CREDITS_MANIFEST.entries[index];
        let children: Vec<&CreditEntry> = CREDITS_MANIFEST
            .entries
            .iter()
            .filter(|candidate| candidate.parent.as_deref() == Some(entry.id.as_str()))
            .collect();
        html! {
            <>
                {menu_blank_row()}
                {menu_header_row(entry.name.clone(), back_to_index)}
                {menu_blank_row()}
                {for credit_detail_rows(entry)}
                {for children.into_iter().flat_map(child_credit_rows)}
                {menu_blank_row()}
            </>
        }
    } else {
        html! {
            <>
                {menu_blank_row()}
                {menu_header_row("About", props.on_back.clone())}
                {menu_blank_row()}
                {for CREDITS_MANIFEST.entries.iter().enumerate().filter_map(|(index, entry)| {
                    if entry.parent.is_some() {
                        return None;
                    }
                    let selected_credit = selected_credit.clone();
                    let on_open = Callback::from(move |_| selected_credit.set(Some(index)));
                    Some(credit_index_row(entry, on_open))
                })}
                {menu_blank_row()}
            </>
        }
    };

    html! {
        <dialog open={props.open}>
            <table class="menu-grid">
                <tbody>
                    {menu_body}
                </tbody>
            </table>
        </dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::{BEGINNER, Settings};
    use serde_json::json;

    #[test]
    fn settings_deserialization_defaults_zoom_percent() {
        let settings: Settings = serde_json::from_value(json!({
            "game_config": {
                "size": [BEGINNER.size.0, BEGINNER.size.1],
                "mines": BEGINNER.mines
            },
            "generator": "NoGuess",
            "enable_question_mark": false,
            "enable_flag_chord": true,
            "enable_auto_trivial": true
        }))
        .expect("settings should deserialize");

        assert_eq!(settings.zoom_percent(), Settings::DEFAULT_ZOOM_PERCENT);
    }

    #[test]
    fn zoom_percent_is_clamped_to_supported_range() {
        let mut settings = Settings::default();

        settings.zoom_percent = 0;
        assert_eq!(settings.zoom_percent(), Settings::MIN_ZOOM_PERCENT);

        settings.zoom_percent = 999;
        assert_eq!(settings.zoom_percent(), Settings::MAX_ZOOM_PERCENT);
    }
}
