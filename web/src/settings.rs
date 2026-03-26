use crate::sprites::{Icon, IconCrop};
use crate::theme::Theme;
use crate::utils::*;
use detonito_core as game;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::sync::LazyLock;
use yew::prelude::*;

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
}

impl Settings {
    const MAX_SIZE: game::Coord = 99;
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game_config: BEGINNER,
            generator: Generator::NoGuess,
            enable_question_mark: false,
            enable_flag_chord: true,
            enable_auto_trivial: true,
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
    IncreaseSizeX,
    DecreaseSizeX,
    IncreaseSizeY,
    DecreaseSizeY,
    IncreaseMines,
    DecreaseMines,
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
            IncreaseSizeX => {
                settings.game_config.size.0 =
                    (settings.game_config.size.0 + 1).clamp(1, Settings::MAX_SIZE);
            }
            DecreaseSizeX => {
                settings.game_config.size.0 =
                    (settings.game_config.size.0 - 1).clamp(1, Settings::MAX_SIZE);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            IncreaseSizeY => {
                settings.game_config.size.1 =
                    (settings.game_config.size.1 + 1).clamp(1, Settings::MAX_SIZE);
            }
            DecreaseSizeY => {
                settings.game_config.size.1 =
                    (settings.game_config.size.1 - 1).clamp(1, Settings::MAX_SIZE);
                settings.game_config.mines = settings
                    .game_config
                    .mines
                    .clamp(1, settings.game_config.total_cells());
            }
            IncreaseMines => {
                settings.game_config.mines =
                    (settings.game_config.mines + 1).clamp(1, settings.game_config.total_cells());
            }
            DecreaseMines => {
                settings.game_config.mines =
                    (settings.game_config.mines - 1).clamp(1, settings.game_config.total_cells());
            }
        }
        settings.local_save();
        settings.into()
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct SettingsProps {
    #[prop_or_default]
    pub open: bool,
    pub on_apply: Callback<MouseEvent>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SettingsMenuPage {
    Root,
    Difficulty,
    Classic,
    ModernNg,
    Custom,
    Generation,
    Appearance,
    About,
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

fn menu_entry_row(
    label: impl Into<AttrValue>,
    detail: impl Into<AttrValue>,
    button: Html,
) -> Html {
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

fn credit_index_row(
    entry: &CreditEntry,
    on_open: Callback<MouseEvent>,
) -> Html {
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

fn menu_adjust_row(
    label: &'static str,
    value: impl Into<AttrValue>,
    on_decrease: Callback<MouseEvent>,
    on_increase: Callback<MouseEvent>,
) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-text" colspan="5">{label}</td>
            <td class="menu-button-slot">
                {menu_icon_button("minus", format!("Decrease {}", label), false, on_decrease)}
            </td>
            <td class="menu-detail" colspan="5">{value.into()}</td>
            <td class="menu-button-slot">
                {menu_icon_button("plus", format!("Increase {}", label), false, on_increase)}
            </td>
            <td class="menu-pad"/>
        </tr>
    }
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
    let page = use_state_eq(|| SettingsMenuPage::Root);
    let selected_credit = use_state_eq(|| None::<usize>);

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
        move |_| settings.dispatch(SettingsAction::IncreaseMines)
    };

    let dec_mines = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::DecreaseMines)
    };

    let inc_size_x = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::IncreaseSizeX)
    };

    let dec_size_x = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::DecreaseSizeX)
    };

    let inc_size_y = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::IncreaseSizeY)
    };

    let dec_size_y = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::DecreaseSizeY)
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

    let open_appearance = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Appearance))
    };

    let open_about = {
        let page = page.clone();
        let selected_credit = selected_credit.clone();
        Callback::from(move |_| {
            selected_credit.set(None);
            page.set(SettingsMenuPage::About);
        })
    };

    let back_to_root = {
        let page = page.clone();
        Callback::from(move |_| page.set(SettingsMenuPage::Root))
    };

    let back_to_about = {
        let selected_credit = selected_credit.clone();
        Callback::from(move |_| selected_credit.set(None))
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
            settings_snapshot.local_save();
            theme.set(theme_snapshot);
            Theme::apply(theme_snapshot);
            on_apply.emit(event);
        })
    };

    let current_choice = difficulty_choice(&settings);
    let current_difficulty_label = difficulty_label(current_choice);
    let current_theme_label = theme_label(*theme);
    let current_generator_label = generator_label(settings.generator);
    let custom_summary = game_config_summary(&settings.game_config);
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
                {menu_title_row("Settings")}
                {menu_blank_row()}
                {menu_entry_row(
                    "Difficulty",
                    current_difficulty_label,
                    menu_icon_button("plus", "Open difficulty menu", false, open_difficulty),
                )}
                {menu_blank_row()}
                {menu_entry_row(
                    "Appearance",
                    current_theme_label,
                    menu_icon_button("plus", "Open appearance menu", false, open_appearance),
                )}
                {menu_entry_row(
                    "About",
                    "Credits",
                    menu_icon_button("plus", "Open about page", false, open_about),
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
                    settings.game_config.size.0.to_string(),
                    Callback::from(dec_size_x),
                    Callback::from(inc_size_x),
                )}
                {menu_adjust_row(
                    "Height",
                    settings.game_config.size.1.to_string(),
                    Callback::from(dec_size_y),
                    Callback::from(inc_size_y),
                )}
                {menu_adjust_row(
                    "Mines",
                    settings.game_config.mines.to_string(),
                    Callback::from(dec_mines),
                    Callback::from(inc_mines),
                )}
                {menu_blank_row()}
            </>
        },
        SettingsMenuPage::Appearance => html! {
            <>
                {menu_blank_row()}
                {menu_header_row("Appearance", back_to_root)}
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
            </>
        },
        SettingsMenuPage::About => {
            if let Some(index) = *selected_credit {
                let entry = &CREDITS_MANIFEST.entries[index];
                let children: Vec<&CreditEntry> = CREDITS_MANIFEST
                    .entries
                    .iter()
                    .filter(|candidate| candidate.parent.as_deref() == Some(entry.id.as_str()))
                    .collect();
                html! {
                    <>
                        {menu_blank_row()}
                        {menu_header_row(entry.name.clone(), back_to_about)}
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
                        {menu_header_row("About", back_to_root)}
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
            }
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
