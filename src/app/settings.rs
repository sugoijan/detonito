use crate::app::utils::*;
use crate::game;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use yew::prelude::*;

pub const BEGINNER: game::Difficulty = game::Difficulty::new_unchecked((9, 9), 10);
pub const INTERMEDIATE: game::Difficulty = game::Difficulty::new_unchecked((16, 16), 40);
pub const EXPERT: game::Difficulty = game::Difficulty::new_unchecked((30, 16), 99);
pub const EVIL: game::Difficulty = game::Difficulty::new_unchecked((30, 20), 130);

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) enum Generator {
    /// Purely random, even the first tile can have a bomb, that's unlucky
    Random,
    /// First tile is always zero (when possible), in the future this will guaranteed a solvable game
    NoRandom,
    // TODO: NoGuess where guesses are guaranteed losses
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) struct Settings {
    pub mark_question: bool,
    pub difficulty: game::Difficulty,
    pub generator: Generator,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mark_question: false,
            difficulty: BEGINNER,
            generator: Generator::NoRandom,
        }
    }
}

impl StorageKey for Settings {
    const KEY: &'static str = "detonito:settings";
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::app) enum SettingsAction {
    ToggleMarkQuestion,
    SetDifficulty(game::Difficulty),
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
        use game::Difficulty;
        use SettingsAction::*;
        let mut settings = Rc::unwrap_or_clone(self);
        match action {
            ToggleMarkQuestion => {
                settings.mark_question = !settings.mark_question;
            }
            SetDifficulty(difficulty) => {
                settings.difficulty = difficulty;
            }
            IncreaseSizeX => {
                settings.difficulty.size.0 = (settings.difficulty.size.0 + 1).clamp(1, Difficulty::MAX_SIZE);
            }
            DecreaseSizeX => {
                settings.difficulty.size.0 = (settings.difficulty.size.0 - 1).clamp(1, Difficulty::MAX_SIZE);
            }
            IncreaseSizeY => {
                settings.difficulty.size.1 = (settings.difficulty.size.1 + 1).clamp(1, Difficulty::MAX_SIZE);
            }
            DecreaseSizeY => {
                settings.difficulty.size.1 = (settings.difficulty.size.1 - 1).clamp(1, Difficulty::MAX_SIZE);
            }
            IncreaseMines => {
                settings.difficulty.mines = (settings.difficulty.mines + 1).clamp(1, settings.difficulty.total_tiles());
            }
            DecreaseMines => {
                settings.difficulty.mines = (settings.difficulty.mines - 1).clamp(1, settings.difficulty.total_tiles());
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
    use crate::app::theme::Theme;

    let settings: UseReducerHandle<Settings> = use_reducer_eq(LocalOrDefault::local_or_default);
    let theme: UseStateHandle<Option<Theme>> = use_state_eq(LocalOrDefault::local_or_default);

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

    let toggle_question = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::ToggleMarkQuestion)
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

    let set_diff_beginner = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetDifficulty(BEGINNER))
    };

    let set_diff_intermediate = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetDifficulty(INTERMEDIATE))
    };

    let set_diff_expert = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetDifficulty(EXPERT))
    };

    let set_diff_evil = {
        let settings = settings.clone();
        move |_| settings.dispatch(SettingsAction::SetDifficulty(EVIL))
    };

    html! {
        <dialog open={props.open}>
            <button class={classes!("theme-light", matches!(*theme, Some(Theme::Light)).then_some("pressed"))} onclick={set_theme_light}/>
            {" "}
            <button class={classes!("theme-dark", matches!(*theme, Some(Theme::Dark)).then_some("pressed"))} onclick={set_theme_dark}/>
            {" "}
            <button class={classes!("theme-auto", matches!(*theme, None).then_some("pressed"))} onclick={set_theme_auto}/>
            <hr/>
            <table>
                <tr><td/><td/><td/></tr>
                <tr><td/><td/><td/></tr>
                <tr><td/><td/><td/></tr>
            </table>
            <button class={classes!("diff-beginner", (settings.difficulty == BEGINNER).then_some("pressed"))} onclick={set_diff_beginner}/>
            {" "}
            <button class={classes!("diff-intermediate", (settings.difficulty == INTERMEDIATE).then_some("pressed"))} onclick={set_diff_intermediate}/>
            {" "}
            <button class={classes!("diff-expert", (settings.difficulty == EXPERT).then_some("pressed"))} onclick={set_diff_expert}/>
            {" "}
            <button class={classes!("diff-evil", (settings.difficulty == EVIL).then_some("pressed"))} onclick={set_diff_evil}/>
            <br/>
            <small>
                <button class={classes!("minus")} onclick={dec_size_x}/>
                <button class={classes!("plus")} onclick={inc_size_x}/>
            </small>
            {format!(" {} × {} ", settings.difficulty.size.0, settings.difficulty.size.1)}
            <small>
                <button class={classes!("minus")} onclick={dec_size_y}/>
                <button class={classes!("plus")} onclick={inc_size_y}/>
            </small>
            <br/>
            <button class={classes!("mine", "pressed", "locked")}/>
            {format!(" × {} ", settings.difficulty.mines)}
            <small>
                <button class={classes!("minus")} onclick={dec_mines}/>
                <button class={classes!("plus")} onclick={inc_mines}/>
            </small>
            <br/>
            <hr/>
            <button class="locked"/>
            {" "}
            <button class={classes!("flag", "locked")}/>
            {" "}
            <button class={classes!("question", (!settings.mark_question).then_some("pressed"))} onclick={toggle_question}/>
            <hr/>
        </dialog>
    }
}
