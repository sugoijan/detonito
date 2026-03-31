use crate::menu::{
    menu_copy_row, menu_header_row, menu_icon_button, menu_nav_enter_button,
    menu_number_stepper_row_with_suffix, menu_section_gap, menu_title_row, menu_wide_detail_row,
};
use crate::runtime::AppRoute;
use crate::theme::Theme;
use crate::utils::*;
use detonito_core as game;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::sync::LazyLock;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;
use yew::prelude::*;

pub const BEGINNER: game::GameConfig = game::GameConfig::new_unchecked((9, 9), 10);
pub const INTERMEDIATE: game::GameConfig = game::GameConfig::new_unchecked((16, 16), 40);
pub const EXPERT: game::GameConfig = game::GameConfig::new_unchecked((30, 16), 99);
pub const EVIL: game::GameConfig = game::GameConfig::new_unchecked((30, 20), 130);

const MENU_CHAR_PADDING: usize = 2;
const MENU_CHARS_PER_FIVE_COLS: usize = 12;
const MENU_MARQUEE_EXTRA_SHIFT: usize = 2;
const ABOUT_INDEX_LABEL_COLSPAN: usize = 5;
const DETAIL_LINK_LABEL_COLSPAN: usize = 4;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Generator {
    /// Purely random, even the first tile can have a bomb, that's unlucky.
    RandomGamble,
    /// First move is forced to a zero-cell when possible.
    RandomZeroStart,
    /// Guaranteed no guess needed to win.
    NoGuess,
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsChangeImpact {
    None,
    DisplayOnly,
    RestartRequired,
}

impl Settings {
    pub(crate) const MAX_SIZE: game::Coord = 99;
    pub(crate) const DEFAULT_ZOOM_PERCENT: u16 = 175;
    pub(crate) const MIN_ZOOM_PERCENT: u16 = 50;
    pub(crate) const MAX_ZOOM_PERCENT: u16 = 500;
    const ZOOM_LEVELS: [u16; 14] = [
        50, 60, 70, 80, 90, 100, 125, 150, 175, 200, 250, 300, 400, 500,
    ];
    const ZOOM_CSS_VAR_NAME: &'static str = "--detonito-zoom";

    const fn default_zoom_percent() -> u16 {
        Self::DEFAULT_ZOOM_PERCENT
    }

    pub(crate) fn normalize_zoom_percent(value: u16) -> u16 {
        let next_index = Self::ZOOM_LEVELS.partition_point(|&level| level < value);

        if next_index == 0 {
            return Self::MIN_ZOOM_PERCENT;
        }

        if next_index == Self::ZOOM_LEVELS.len() {
            return Self::MAX_ZOOM_PERCENT;
        }

        let lower = Self::ZOOM_LEVELS[next_index - 1];
        let upper = Self::ZOOM_LEVELS[next_index];
        if value - lower <= upper - value {
            lower
        } else {
            upper
        }
    }

    fn increase_zoom_percent(value: u16) -> u16 {
        let value = Self::normalize_zoom_percent(value);
        let index = Self::ZOOM_LEVELS
            .binary_search(&value)
            .expect("normalized zoom percent should match a supported preset");

        Self::ZOOM_LEVELS.get(index + 1).copied().unwrap_or(value)
    }

    fn decrease_zoom_percent(value: u16) -> u16 {
        let value = Self::normalize_zoom_percent(value);
        let index = Self::ZOOM_LEVELS
            .binary_search(&value)
            .expect("normalized zoom percent should match a supported preset");

        index
            .checked_sub(1)
            .map(|prev| Self::ZOOM_LEVELS[prev])
            .unwrap_or(value)
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

    pub(crate) fn persist(&self) {
        self.local_save();
    }

    pub(crate) fn commit(&self) {
        self.apply_display();
        self.persist();
    }

    /// Applies a settings action without side effects so menu flows can decide
    /// whether the result should commit immediately or be confirmation-gated.
    pub(crate) fn applying(&self, action: SettingsAction) -> Self {
        use SettingsAction::*;

        let mut settings = self.clone();
        match action {
            SetGameConfig(game_config) => {
                settings.game_config = game_config;
            }
            SetGenerator(generator) => {
                settings.generator = generator;
            }
            SetSizeX(value) => {
                settings.game_config.size.0 = value.clamp(1, Self::MAX_SIZE.into()) as game::Coord;
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            SetSizeY(value) => {
                settings.game_config.size.1 = value.clamp(1, Self::MAX_SIZE.into()) as game::Coord;
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            SetMines(value) => {
                settings.game_config.mines = value.clamp(1, settings.game_config.total_cells());
            }
            SetZoom(value) => {
                settings.zoom_percent = Self::normalize_zoom_percent(value);
            }
            IncreaseSizeX => {
                settings.game_config.size.0 = settings.game_config.size.0.saturating_add(1);
                settings.game_config.size.0 = settings.game_config.size.0.clamp(1, Self::MAX_SIZE);
            }
            DecreaseSizeX => {
                settings.game_config.size.0 = settings.game_config.size.0.saturating_sub(1);
                settings.game_config.size.0 = settings.game_config.size.0.clamp(1, Self::MAX_SIZE);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            IncreaseSizeY => {
                settings.game_config.size.1 = settings.game_config.size.1.saturating_add(1);
                settings.game_config.size.1 = settings.game_config.size.1.clamp(1, Self::MAX_SIZE);
            }
            DecreaseSizeY => {
                settings.game_config.size.1 = settings.game_config.size.1.saturating_sub(1);
                settings.game_config.size.1 = settings.game_config.size.1.clamp(1, Self::MAX_SIZE);
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
                settings.zoom_percent = Self::increase_zoom_percent(settings.zoom_percent());
            }
            DecreaseZoom => {
                settings.zoom_percent = Self::decrease_zoom_percent(settings.zoom_percent());
            }
        }
        settings
    }

    pub(crate) fn with_generator_and_config(
        &self,
        generator: Generator,
        game_config: game::GameConfig,
    ) -> Self {
        self.applying(SettingsAction::SetGenerator(generator))
            .applying(SettingsAction::SetGameConfig(game_config))
    }

    /// Display-only settings can commit in place, while generator or board
    /// shape changes need a fresh run to apply coherently.
    pub(crate) fn change_impact_from(&self, next: &Self) -> SettingsChangeImpact {
        if self == next {
            SettingsChangeImpact::None
        } else if self.game_config != next.game_config || self.generator != next.generator {
            SettingsChangeImpact::RestartRequired
        } else {
            SettingsChangeImpact::DisplayOnly
        }
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum DifficultyChoice {
    ClassicBeginner,
    ClassicIntermediate,
    ClassicExpert,
    ModernEasy,
    ModernMedium,
    ModernHard,
    ModernEvil,
    Custom,
}

pub(crate) fn difficulty_choice(settings: &Settings) -> DifficultyChoice {
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

pub(crate) fn difficulty_label(choice: DifficultyChoice) -> &'static str {
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

pub(crate) fn game_config_summary(config: &game::GameConfig) -> String {
    format!("{}x{} / {}", config.size.0, config.size.1, config.mines)
}

pub(crate) fn generator_label(generator: Generator) -> &'static str {
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
        None => "Automatic",
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct SettingsProps {
    #[prop_or_default]
    pub open: bool,
    pub on_back: Callback<MouseEvent>,
    #[prop_or_default]
    pub entry_point: SettingsEntryPoint,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum SettingsEntryPoint {
    #[default]
    Main,
    Normal,
    Afk,
}

impl SettingsEntryPoint {
    /// The entry point owns the back destination so the shared settings screen
    /// can stay single-instance while still behaving like a contextual submenu.
    pub(crate) const fn back_route(self) -> AppRoute {
        match self {
            Self::Main => AppRoute::Menu,
            Self::Normal => AppRoute::NormalMenu,
            Self::Afk => AppRoute::Afk,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SettingsMenuPage {
    Root,
    Theme,
}

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
            false,
            open_external_link(link.url.clone()),
        ),
    )
}

fn credit_index_row(entry: &CreditEntry, on_open: Callback<MouseEvent>) -> Html {
    menu_about_index_row(
        entry.name.clone(),
        entry.relation.clone(),
        menu_nav_enter_button(format!("Open {}", entry.name), false, on_open),
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
        rows.push(menu_section_gap());
        rows.extend(entry.links.iter().map(credit_link_row));
    }
    rows
}

fn child_credit_rows(entry: &CreditEntry) -> Vec<Html> {
    let mut rows = Vec::new();
    rows.push(menu_section_gap());
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

#[function_component]
pub(crate) fn SettingsView(props: &SettingsProps) -> Html {
    let settings = use_state_eq(Settings::local_or_default);
    let theme = use_state_eq(|| Option::<Theme>::local_or_default());
    let page = use_state_eq(|| SettingsMenuPage::Root);

    let commit_settings = {
        let settings = settings.clone();
        Callback::from(move |next: Settings| {
            next.commit();
            settings.set(next);
        })
    };

    let commit_theme = {
        let theme = theme.clone();
        Callback::from(move |next_theme: Option<Theme>| {
            theme.set(next_theme);
            Theme::apply(next_theme);
        })
    };

    let adjust_zoom = {
        let settings = settings.clone();
        let commit_settings = commit_settings.clone();
        Rc::new(move |action: SettingsAction| {
            let next = (*settings).applying(action);
            commit_settings.emit(next);
        })
    };

    let inc_zoom = {
        let adjust_zoom = adjust_zoom.clone();
        Callback::from(move |()| adjust_zoom(SettingsAction::IncreaseZoom))
    };

    let dec_zoom = {
        let adjust_zoom = adjust_zoom.clone();
        Callback::from(move |()| adjust_zoom(SettingsAction::DecreaseZoom))
    };

    let set_zoom = {
        let adjust_zoom = adjust_zoom.clone();
        Callback::from(move |value: u16| adjust_zoom(SettingsAction::SetZoom(value)))
    };

    let reset_zoom = {
        let settings = settings.clone();
        let commit_settings = commit_settings.clone();
        Callback::from(move |_| {
            let next =
                (*settings).applying(SettingsAction::SetZoom(Settings::DEFAULT_ZOOM_PERCENT));
            commit_settings.emit(next);
        })
    };

    let set_theme_light = {
        let commit_theme = commit_theme.clone();
        Callback::from(move |_| commit_theme.emit(Some(Theme::Light)))
    };

    let set_theme_dark = {
        let commit_theme = commit_theme.clone();
        Callback::from(move |_| commit_theme.emit(Some(Theme::Dark)))
    };

    let set_theme_auto = {
        let commit_theme = commit_theme.clone();
        Callback::from(move |_| commit_theme.emit(None))
    };

    let open_theme = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Theme))
    };

    let back_to_root = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Root))
    };

    let current_theme_label = theme_label(*theme);

    let menu_body = match *page {
        SettingsMenuPage::Root => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Settings", props.on_back.clone())}
                {menu_section_gap()}
                {menu_number_stepper_row_with_suffix(
                    "Zoom",
                    settings.zoom_percent(),
                    Settings::MIN_ZOOM_PERCENT,
                    Settings::MAX_ZOOM_PERCENT,
                    Some("%".into()),
                    Some(menu_icon_button(
                        "zoom-reset",
                        "Reset zoom",
                        false,
                        false,
                        reset_zoom,
                    )),
                    dec_zoom,
                    set_zoom,
                    inc_zoom,
                )}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Theme",
                    current_theme_label,
                    menu_nav_enter_button("Open theme menu", false, open_theme),
                )}
                {menu_section_gap()}
            </>
        },
        SettingsMenuPage::Theme => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Theme", back_to_root)}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Dark",
                    "Dark theme",
                    menu_icon_button(
                        "theme-dark",
                        "Use dark theme",
                        matches!(*theme, Some(Theme::Dark)),
                        false,
                        set_theme_dark,
                    ),
                )}
                {menu_wide_detail_row(
                    "Light",
                    "Light theme",
                    menu_icon_button(
                        "theme-light",
                        "Use light theme",
                        matches!(*theme, Some(Theme::Light)),
                        false,
                        set_theme_light,
                    ),
                )}
                {menu_wide_detail_row(
                    "Automatic",
                    "Automatic theme",
                    menu_icon_button(
                        "theme-auto",
                        "Use automatic theme",
                        matches!(*theme, None),
                        false,
                        set_theme_auto,
                    ),
                )}
                {menu_section_gap()}
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
                {menu_section_gap()}
                {menu_header_row(entry.name.clone(), back_to_index)}
                {menu_section_gap()}
                {for credit_detail_rows(entry)}
                {for children.into_iter().flat_map(child_credit_rows)}
                {menu_section_gap()}
            </>
        }
    } else {
        html! {
            <>
                {menu_section_gap()}
                {menu_header_row("About", props.on_back.clone())}
                {menu_section_gap()}
                {for CREDITS_MANIFEST.entries.iter().enumerate().filter_map(|(index, entry)| {
                    if entry.parent.is_some() {
                        return None;
                    }
                    let selected_credit = selected_credit.clone();
                    let on_open = Callback::from(move |_| selected_credit.set(Some(index)));
                    Some(credit_index_row(entry, on_open))
                })}
                {menu_section_gap()}
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
    use super::{BEGINNER, Settings, SettingsAction, SettingsChangeImpact, SettingsEntryPoint};
    use crate::runtime::AppRoute;
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
    fn zoom_percent_is_snapped_to_supported_levels() {
        let mut settings = Settings::default();

        settings.zoom_percent = 0;
        assert_eq!(settings.zoom_percent(), Settings::MIN_ZOOM_PERCENT);

        settings.zoom_percent = 999;
        assert_eq!(settings.zoom_percent(), Settings::MAX_ZOOM_PERCENT);

        settings.zoom_percent = 119;
        assert_eq!(settings.zoom_percent(), 125);
    }

    #[test]
    fn applying_settings_action_is_pure() {
        let settings = Settings::default();
        let next = settings.applying(SettingsAction::IncreaseZoom);

        assert_eq!(settings.zoom_percent(), Settings::DEFAULT_ZOOM_PERCENT);
        assert_eq!(next.zoom_percent(), 200);
    }

    #[test]
    fn zoom_actions_follow_supported_progression() {
        let settings = Settings::default();

        assert_eq!(
            settings
                .applying(SettingsAction::DecreaseZoom)
                .zoom_percent(),
            150
        );
        assert_eq!(
            settings
                .applying(SettingsAction::SetZoom(119))
                .zoom_percent(),
            125
        );
    }

    #[test]
    fn settings_change_impact_distinguishes_restart_changes() {
        let settings = Settings::default();
        let zoom_only = settings.applying(SettingsAction::IncreaseZoom);
        let board_change = settings.applying(SettingsAction::IncreaseSizeX);

        assert_eq!(
            settings.change_impact_from(&zoom_only),
            SettingsChangeImpact::DisplayOnly
        );
        assert_eq!(
            settings.change_impact_from(&board_change),
            SettingsChangeImpact::RestartRequired
        );
        assert_eq!(
            settings.change_impact_from(&settings),
            SettingsChangeImpact::None
        );
    }

    #[test]
    fn settings_entry_points_map_to_expected_back_routes() {
        assert_eq!(SettingsEntryPoint::Main.back_route(), AppRoute::Menu);
        assert_eq!(
            SettingsEntryPoint::Normal.back_route(),
            AppRoute::NormalMenu
        );
        assert_eq!(SettingsEntryPoint::Afk.back_route(), AppRoute::Afk);
    }
}
