use crate::app::utils::*;
use crate::game;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use yew::prelude::*;

pub const GAME_KEY: &'static str = "detonito:game";
//pub const THEME_KEY: &'static str = "detonito:theme";
pub const SETTINGS_KEY: &'static str = "detonito:settings";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) struct Settings {
    pub mark_question: bool,
    pub difficulty: game::Difficulty,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mark_question: false,
            difficulty: Settings::DEFAULT_DIFFICULTIES[0].1,
        }
    }
}

impl StorageKey for Settings {
    const KEY: &'static str = SETTINGS_KEY;
}

impl Settings {
    pub const DEFAULT_DIFFICULTIES: &'static [(&'static str, game::Difficulty)] = &[
        ("Beginner", game::Difficulty::beginner()),
        ("Intermediate", game::Difficulty::intermediate()),
        ("Expert", game::Difficulty::expert()),
        (
            "Min",
            game::Difficulty {
                size: (3, 3),
                mines: 9,
            },
        ),
        (
            "Max",
            game::Difficulty {
                size: (3, 3),
                mines: 8,
            },
        ),
    ];
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) enum SettingsAction {
    ToggleMarkQuestion,
}

impl Reducible for Settings {
    type Action = SettingsAction;

    fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
        use SettingsAction::*;

        //let mut settings = self.unwrap_or_clone();
        let mut settings = Rc::unwrap_or_clone(self);

        match action {
            ToggleMarkQuestion => {
                settings.mark_question = !settings.mark_question;
            }
        }

        settings.local_save();
        settings.into()
    }
}

#[derive(Properties, PartialEq)]
pub(in crate::app) struct SettingsProps {
    #[prop_or_default]
    pub open: bool,
}

#[function_component]
pub(in crate::app) fn SettingsView(props: &SettingsProps) -> Html {
    //let settings: UseStateHandle<Settings> = use_state_eq(LocalOrDefault::local_or_default);
    let settings: UseReducerHandle<Settings> = use_reducer_eq(LocalOrDefault::local_or_default);

    let toggle_question = {
        let settings = settings.clone();
        move |_| {
            //let mark_question = !settings.mark_question;
            //settings.set(Settings { mark_question, ..*settings });
            settings.dispatch(SettingsAction::ToggleMarkQuestion)
        }
    };

    html! {
        <dialog open={props.open}>
            <table>
                <tr><td/><td/><td/></tr>
                <tr><td/><td/><td/></tr>
                <tr><td/><td/><td/></tr>
            </table>
            <button class={classes!("question", settings.mark_question.then_some("pressed"))} onclick={toggle_question}/>
            /*
            <article>
                <h2>{"Settings"}</h2>
                <ul>
                    <li><a href="#" data-theme-switcher="auto">{"Auto"}</a></li>
                    <li><a href="#" data-theme-switcher="light">{"Light"}</a></li>
                    <li><a href="#" data-theme-switcher="dark">{"Dark"}</a></li>
                </ul>
                <footer>
                    <button type="reset">{"Cancel"}</button>
                    <button>{"Apply"}</button>
                </footer>
            </article>
            */
        </dialog>
    }
}
