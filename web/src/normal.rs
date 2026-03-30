use crate::menu::{
    menu_copy_row, menu_header_row, menu_icon_button, menu_nav_enter_button,
    menu_number_stepper_row, menu_primary_row, menu_section_gap, menu_wide_detail_row,
};
use crate::settings::{
    BEGINNER, EVIL, EXPERT, Generator, INTERMEDIATE, Settings, SettingsAction,
    SettingsChangeImpact, difficulty_choice, difficulty_label, game_config_summary,
    generator_label,
};
use crate::utils::LocalOrDefault;
use std::rc::Rc;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub(crate) struct NormalMenuProps {
    #[prop_or_default]
    pub open: bool,
    pub can_resume: bool,
    pub on_back: Callback<MouseEvent>,
    pub on_resume: Callback<MouseEvent>,
    pub on_start_new: Callback<MouseEvent>,
    pub on_open_settings: Callback<MouseEvent>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum NormalMenuPage {
    Root,
    Difficulty,
    Classic,
    ModernNg,
    Custom,
    Generation,
    ConfirmRestart,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingSettingsChange {
    next_settings: Settings,
    return_page: NormalMenuPage,
}

#[derive(Clone, Debug, PartialEq)]
enum NormalSettingsChangePlan {
    NoChange,
    Commit(Settings),
    ConfirmRestart(Settings),
}

/// Restart-gated settings stay local until the user confirms discarding the
/// current board. This keeps the menu honest about which changes are live now.
fn plan_normal_settings_change(
    current: &Settings,
    next: Settings,
    can_resume: bool,
) -> NormalSettingsChangePlan {
    match current.change_impact_from(&next) {
        SettingsChangeImpact::None => NormalSettingsChangePlan::NoChange,
        SettingsChangeImpact::RestartRequired if can_resume => {
            NormalSettingsChangePlan::ConfirmRestart(next)
        }
        SettingsChangeImpact::RestartRequired | SettingsChangeImpact::DisplayOnly => {
            NormalSettingsChangePlan::Commit(next)
        }
    }
}

#[function_component]
pub(crate) fn NormalMenuView(props: &NormalMenuProps) -> Html {
    let settings = use_state_eq(Settings::local_or_default);
    let page = use_state_eq(|| NormalMenuPage::Root);
    let pending_change = use_state_eq(|| None::<PendingSettingsChange>);
    let can_resume = props.can_resume;

    let commit_settings = {
        let settings = settings.clone();
        Callback::from(move |next: Settings| {
            next.commit();
            settings.set(next);
        })
    };

    let apply_settings_change =
        {
            let page = page.clone();
            let settings = settings.clone();
            let pending_change = pending_change.clone();
            let commit_settings = commit_settings.clone();
            Rc::new(move |next: Settings, return_page: NormalMenuPage| {
                match plan_normal_settings_change(&*settings, next, can_resume) {
                    NormalSettingsChangePlan::NoChange => {}
                    NormalSettingsChangePlan::Commit(next) => {
                        commit_settings.emit(next);
                    }
                    NormalSettingsChangePlan::ConfirmRestart(next) => {
                        pending_change.set(Some(PendingSettingsChange {
                            next_settings: next,
                            return_page,
                        }));
                        page.set(NormalMenuPage::ConfirmRestart);
                    }
                }
            })
        };

    let open_difficulty = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Difficulty))
    };

    let open_classic = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Classic))
    };

    let open_modern_ng = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::ModernNg))
    };

    let open_generation = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Generation))
    };

    let open_custom = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Custom))
    };

    let back_to_root = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Root))
    };

    let back_to_difficulty = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Difficulty))
    };

    let back_to_custom = {
        let page = page.clone();
        Callback::from(move |_| page.set(NormalMenuPage::Custom))
    };

    let back_to_pending_source = {
        let page = page.clone();
        let pending_change = pending_change.clone();
        Callback::from(move |_| {
            if let Some(pending) = &*pending_change {
                page.set(pending.return_page);
            }
            pending_change.set(None);
        })
    };

    let set_classic_beginner = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next = (*settings).with_generator_and_config(Generator::RandomZeroStart, BEGINNER);
            apply_settings_change(next, NormalMenuPage::Classic);
        })
    };

    let set_classic_intermediate = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next =
                (*settings).with_generator_and_config(Generator::RandomZeroStart, INTERMEDIATE);
            apply_settings_change(next, NormalMenuPage::Classic);
        })
    };

    let set_classic_expert = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next = (*settings).with_generator_and_config(Generator::RandomZeroStart, EXPERT);
            apply_settings_change(next, NormalMenuPage::Classic);
        })
    };

    let set_modern_easy = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next = (*settings).with_generator_and_config(Generator::NoGuess, BEGINNER);
            apply_settings_change(next, NormalMenuPage::ModernNg);
        })
    };

    let set_modern_medium = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next = (*settings).with_generator_and_config(Generator::NoGuess, INTERMEDIATE);
            apply_settings_change(next, NormalMenuPage::ModernNg);
        })
    };

    let set_modern_hard = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next = (*settings).with_generator_and_config(Generator::NoGuess, EXPERT);
            apply_settings_change(next, NormalMenuPage::ModernNg);
        })
    };

    let set_modern_evil = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |_| {
            let next = (*settings).with_generator_and_config(Generator::NoGuess, EVIL);
            apply_settings_change(next, NormalMenuPage::ModernNg);
        })
    };

    let set_generator_gamble = {
        let apply_settings_change = apply_settings_change.clone();
        let settings = settings.clone();
        Callback::from(move |_| {
            let next = (*settings).applying(SettingsAction::SetGenerator(Generator::RandomGamble));
            apply_settings_change(next, NormalMenuPage::Generation);
        })
    };

    let set_generator_random = {
        let apply_settings_change = apply_settings_change.clone();
        let settings = settings.clone();
        Callback::from(move |_| {
            let next =
                (*settings).applying(SettingsAction::SetGenerator(Generator::RandomZeroStart));
            apply_settings_change(next, NormalMenuPage::Generation);
        })
    };

    let set_generator_puzzle = {
        let apply_settings_change = apply_settings_change.clone();
        let settings = settings.clone();
        Callback::from(move |_| {
            let next = (*settings).applying(SettingsAction::SetGenerator(Generator::NoGuess));
            apply_settings_change(next, NormalMenuPage::Generation);
        })
    };

    let inc_mines = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |()| {
            let next = (*settings).applying(SettingsAction::IncreaseMines);
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let dec_mines = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |()| {
            let next = (*settings).applying(SettingsAction::DecreaseMines);
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let set_mines = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |value: u16| {
            let next = (*settings).applying(SettingsAction::SetMines(value));
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let inc_size_x = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |()| {
            let next = (*settings).applying(SettingsAction::IncreaseSizeX);
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let dec_size_x = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |()| {
            let next = (*settings).applying(SettingsAction::DecreaseSizeX);
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let set_size_x = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |value: u16| {
            let next = (*settings).applying(SettingsAction::SetSizeX(value));
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let inc_size_y = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |()| {
            let next = (*settings).applying(SettingsAction::IncreaseSizeY);
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let dec_size_y = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |()| {
            let next = (*settings).applying(SettingsAction::DecreaseSizeY);
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let set_size_y = {
        let settings = settings.clone();
        let apply_settings_change = apply_settings_change.clone();
        Callback::from(move |value: u16| {
            let next = (*settings).applying(SettingsAction::SetSizeY(value));
            apply_settings_change(next, NormalMenuPage::Custom);
        })
    };

    let confirm_restart = {
        let pending_change = pending_change.clone();
        let commit_settings = commit_settings.clone();
        let on_start_new = props.on_start_new.clone();
        Callback::from(move |event: MouseEvent| {
            if let Some(pending) = &*pending_change {
                commit_settings.emit(pending.next_settings.clone());
            }
            pending_change.set(None);
            on_start_new.emit(event);
        })
    };

    let current_choice = difficulty_choice(&settings);
    let current_difficulty_label = difficulty_label(current_choice);
    let current_generator_label = generator_label(settings.generator);
    let custom_summary = game_config_summary(&settings.game_config);
    let classic_detail = match current_choice {
        crate::settings::DifficultyChoice::ClassicBeginner => "Beginner",
        crate::settings::DifficultyChoice::ClassicIntermediate => "Intermediate",
        crate::settings::DifficultyChoice::ClassicExpert => "Expert",
        _ => "",
    };
    let modern_detail = match current_choice {
        crate::settings::DifficultyChoice::ModernEasy => "Easy",
        crate::settings::DifficultyChoice::ModernMedium => "Medium",
        crate::settings::DifficultyChoice::ModernHard => "Hard",
        crate::settings::DifficultyChoice::ModernEvil => "Evil",
        _ => "",
    };
    let custom_detail = if matches!(current_choice, crate::settings::DifficultyChoice::Custom) {
        custom_summary.clone()
    } else {
        String::new()
    };

    let confirm_summary = (*pending_change)
        .as_ref()
        .map(|pending| game_config_summary(&pending.next_settings.game_config))
        .unwrap_or_else(|| custom_summary.clone());

    let menu_body = match *page {
        NormalMenuPage::Root => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Normal mode", props.on_back.clone())}
                {menu_section_gap()}
                {
                    if props.can_resume {
                        html! {
                            <>
                                {menu_primary_row(
                                    "Resume",
                                    menu_nav_enter_button("Resume normal mode", false, props.on_resume.clone()),
                                )}
                                {menu_primary_row(
                                    "Start New",
                                    menu_nav_enter_button("Start a new normal game", false, props.on_start_new.clone()),
                                )}
                            </>
                        }
                    } else {
                        html! {
                            {menu_primary_row(
                                "Start",
                                menu_nav_enter_button("Start normal mode", false, props.on_start_new.clone()),
                            )}
                        }
                    }
                }
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Difficulty",
                    current_difficulty_label,
                    menu_nav_enter_button("Open difficulty menu", false, open_difficulty),
                )}
                {menu_wide_detail_row(
                    "Settings",
                    "",
                    menu_nav_enter_button("Open settings", false, props.on_open_settings.clone()),
                )}
                {menu_section_gap()}
            </>
        },
        NormalMenuPage::Difficulty => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Difficulty", back_to_root)}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Modern NG",
                    modern_detail,
                    menu_nav_enter_button("Open modern difficulty menu", false, open_modern_ng),
                )}
                {menu_wide_detail_row(
                    "Classic",
                    classic_detail,
                    menu_nav_enter_button("Open classic difficulty menu", false, open_classic),
                )}
                {menu_wide_detail_row(
                    "Custom",
                    custom_detail,
                    menu_nav_enter_button("Open custom board menu", false, open_custom),
                )}
                {menu_section_gap()}
            </>
        },
        NormalMenuPage::Classic => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Classic", back_to_difficulty)}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Beginner",
                    game_config_summary(&BEGINNER),
                    menu_icon_button(
                        "diff-beginner",
                        "Use classic beginner preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ClassicBeginner),
                        false,
                        set_classic_beginner,
                    ),
                )}
                {menu_wide_detail_row(
                    "Intermediate",
                    game_config_summary(&INTERMEDIATE),
                    menu_icon_button(
                        "diff-intermediate",
                        "Use classic intermediate preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ClassicIntermediate),
                        false,
                        set_classic_intermediate,
                    ),
                )}
                {menu_wide_detail_row(
                    "Expert",
                    game_config_summary(&EXPERT),
                    menu_icon_button(
                        "diff-expert",
                        "Use classic expert preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ClassicExpert),
                        false,
                        set_classic_expert,
                    ),
                )}
                {menu_section_gap()}
            </>
        },
        NormalMenuPage::ModernNg => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Modern NG", back_to_difficulty)}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Easy",
                    game_config_summary(&BEGINNER),
                    menu_icon_button(
                        "diff-beginner",
                        "Use modern easy preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ModernEasy),
                        false,
                        set_modern_easy,
                    ),
                )}
                {menu_wide_detail_row(
                    "Medium",
                    game_config_summary(&INTERMEDIATE),
                    menu_icon_button(
                        "diff-intermediate",
                        "Use modern medium preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ModernMedium),
                        false,
                        set_modern_medium,
                    ),
                )}
                {menu_wide_detail_row(
                    "Hard",
                    game_config_summary(&EXPERT),
                    menu_icon_button(
                        "diff-expert",
                        "Use modern hard preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ModernHard),
                        false,
                        set_modern_hard,
                    ),
                )}
                {menu_wide_detail_row(
                    "Evil",
                    game_config_summary(&EVIL),
                    menu_icon_button(
                        "diff-evil",
                        "Use modern evil preset",
                        matches!(current_choice, crate::settings::DifficultyChoice::ModernEvil),
                        false,
                        set_modern_evil,
                    ),
                )}
                {menu_section_gap()}
            </>
        },
        NormalMenuPage::Custom => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Custom", back_to_difficulty)}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Generation",
                    current_generator_label,
                    menu_nav_enter_button("Open generation menu", false, open_generation),
                )}
                {menu_section_gap()}
                {menu_number_stepper_row(
                    "Width",
                    settings.game_config.size.0.into(),
                    1,
                    Settings::MAX_SIZE.into(),
                    dec_size_x,
                    set_size_x,
                    inc_size_x,
                )}
                {menu_number_stepper_row(
                    "Height",
                    settings.game_config.size.1.into(),
                    1,
                    Settings::MAX_SIZE.into(),
                    dec_size_y,
                    set_size_y,
                    inc_size_y,
                )}
                {menu_number_stepper_row(
                    "Mines",
                    settings.game_config.mines,
                    1,
                    settings.game_config.total_cells(),
                    dec_mines,
                    set_mines,
                    inc_mines,
                )}
                {menu_section_gap()}
            </>
        },
        NormalMenuPage::Generation => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Generation", back_to_custom)}
                {menu_section_gap()}
                {menu_wide_detail_row(
                    "Gamble",
                    "",
                    menu_icon_button(
                        "gamble",
                        "Use gamble generator",
                        settings.generator == Generator::RandomGamble,
                        false,
                        set_generator_gamble,
                    ),
                )}
                {menu_wide_detail_row(
                    "Zero-start",
                    "",
                    menu_icon_button(
                        "random",
                        "Use zero-start generator",
                        settings.generator == Generator::RandomZeroStart,
                        false,
                        set_generator_random,
                    ),
                )}
                {menu_wide_detail_row(
                    "No-guess",
                    "",
                    menu_icon_button(
                        "puzzle",
                        "Use no-guess generator",
                        settings.generator == Generator::NoGuess,
                        false,
                        set_generator_puzzle,
                    ),
                )}
                {menu_section_gap()}
            </>
        },
        NormalMenuPage::ConfirmRestart => html! {
            <>
                {menu_section_gap()}
                {menu_header_row("Start New Game", back_to_pending_source.clone())}
                {menu_section_gap()}
                {menu_copy_row("This change only applies by starting a new normal game now.")}
                {menu_copy_row(format!("New board: {confirm_summary}"))}
                {menu_section_gap()}
                {menu_primary_row(
                    "Start New Game",
                    menu_icon_button("ok", "Apply changes and start a new game", false, false, confirm_restart),
                )}
                {menu_primary_row(
                    "Keep Current Game",
                    menu_icon_button("cancel", "Discard the pending change", false, false, back_to_pending_source),
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

#[cfg(test)]
mod tests {
    use super::{NormalSettingsChangePlan, plan_normal_settings_change};
    use crate::settings::{Settings, SettingsAction};

    #[test]
    fn resumable_board_changes_require_confirmation() {
        let current = Settings::default();
        let next = current.applying(SettingsAction::IncreaseSizeX);

        assert!(matches!(
            plan_normal_settings_change(&current, next, true),
            NormalSettingsChangePlan::ConfirmRestart(_)
        ));
    }

    #[test]
    fn confirmed_change_is_a_restart_path() {
        let current = Settings::default();
        let next = current.applying(SettingsAction::IncreaseMines);

        match plan_normal_settings_change(&current, next.clone(), true) {
            NormalSettingsChangePlan::ConfirmRestart(pending) => assert_eq!(pending, next),
            other => panic!("expected confirmation path, got {other:?}"),
        }
    }

    #[test]
    fn no_resumable_board_commits_immediately() {
        let current = Settings::default();
        let next = current.applying(SettingsAction::IncreaseSizeY);

        assert!(matches!(
            plan_normal_settings_change(&current, next, false),
            NormalSettingsChangePlan::Commit(_)
        ));
    }
}
