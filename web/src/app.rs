use yew::prelude::*;

use crate::afk::AfkView;
use crate::game::{GameInitArgs, GameView, has_saved_game};
use crate::menu::{menu_blank_row, menu_entry_row, menu_header_row, menu_icon_button};
use crate::runtime::{AppRoute, current_route_state, frontend_runtime_config, replace_route};
use crate::settings::{AboutView, SettingsView};
use crate::sprites::SpriteDefs;

#[derive(Properties, Clone, PartialEq)]
pub(crate) struct AppShellProps {
    pub init: GameInitArgs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResumeTarget {
    Classic,
    Afk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellScreen {
    Menu,
    Classic,
    Afk,
    Settings,
    About,
}

fn route_to_screen(route: AppRoute) -> ShellScreen {
    match route {
        AppRoute::Menu => ShellScreen::Menu,
        AppRoute::Classic => ShellScreen::Classic,
        AppRoute::Afk => ShellScreen::Afk,
        AppRoute::Settings => ShellScreen::Settings,
        AppRoute::About => ShellScreen::About,
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

#[function_component]
pub(crate) fn AppShell(props: &AppShellProps) -> Html {
    let runtime = frontend_runtime_config();
    let initial_route = current_route_state();
    let screen = use_state_eq(move || route_to_screen(initial_route.route));
    let resume_target = use_state_eq(|| has_saved_game().then_some(ResumeTarget::Classic));
    let afk_auth_error = use_state_eq(move || initial_route.afk_auth_error);

    let open_menu_for = {
        let screen = screen.clone();
        let resume_target = resume_target.clone();
        let afk_auth_error = afk_auth_error.clone();
        move |target: ResumeTarget| {
            resume_target.set(Some(target));
            afk_auth_error.set(None);
            replace_route(AppRoute::Menu);
            screen.set(ShellScreen::Menu);
        }
    };

    let on_classic_menu = {
        let open_menu_for = open_menu_for.clone();
        Callback::from(move |_| open_menu_for(ResumeTarget::Classic))
    };

    let on_afk_menu = {
        let open_menu_for = open_menu_for.clone();
        Callback::from(move |_| open_menu_for(ResumeTarget::Afk))
    };

    let open_normal_mode = {
        let screen = screen.clone();
        let resume_target = resume_target.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_| {
            resume_target.set(Some(ResumeTarget::Classic));
            afk_auth_error.set(None);
            replace_route(AppRoute::Classic);
            screen.set(ShellScreen::Classic);
        })
    };

    let open_settings = {
        let screen = screen.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_| {
            afk_auth_error.set(None);
            replace_route(AppRoute::Settings);
            screen.set(ShellScreen::Settings);
        })
    };

    let open_about = {
        let screen = screen.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_| {
            afk_auth_error.set(None);
            replace_route(AppRoute::About);
            screen.set(ShellScreen::About);
        })
    };

    let open_afk = {
        let screen = screen.clone();
        let resume_target = resume_target.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_| {
            if runtime.afk_enabled {
                resume_target.set(Some(ResumeTarget::Afk));
                afk_auth_error.set(None);
                replace_route(AppRoute::Afk);
                screen.set(ShellScreen::Afk);
            }
        })
    };

    let back_to_menu = {
        let screen = screen.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_: MouseEvent| {
            afk_auth_error.set(None);
            replace_route(AppRoute::Menu);
            screen.set(ShellScreen::Menu);
        })
    };

    let close_menu = {
        let screen = screen.clone();
        let resume_target = resume_target.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_: MouseEvent| {
            afk_auth_error.set(None);
            if let Some(target) = *resume_target {
                match target {
                    ResumeTarget::Classic => {
                        replace_route(AppRoute::Classic);
                        screen.set(ShellScreen::Classic);
                    }
                    ResumeTarget::Afk => {
                        replace_route(AppRoute::Afk);
                        screen.set(ShellScreen::Afk);
                    }
                }
            }
        })
    };

    match *screen {
        ShellScreen::Classic => html! {
            <GameView on_menu={on_classic_menu} init={props.init.clone()}/>
        },
        ShellScreen::Afk => html! {
            <AfkView on_menu={on_afk_menu} auth_error={(*afk_auth_error).clone()}/>
        },
        ShellScreen::Settings => html! {
            <div class="detonito settings-open">
                <SpriteDefs/>
                <SettingsView
                    open={true}
                    on_apply={back_to_menu.clone()}
                    on_back={Some(back_to_menu.clone())}
                />
            </div>
        },
        ShellScreen::About => html! {
            <div class="detonito settings-open">
                <SpriteDefs/>
                <AboutView open={true} on_back={back_to_menu.clone()}/>
            </div>
        },
        ShellScreen::Menu => html! {
            <div class="detonito settings-open start-menu-shell">
                <SpriteDefs/>
                <dialog open=true>
                    <table class="menu-grid start-menu">
                        <tbody>
                            {menu_blank_row()}
                            {
                                if resume_target.is_some() {
                                    menu_header_row("Menu", close_menu.clone())
                                } else {
                                    menu_title_row("Detonito")
                                }
                            }
                            {menu_blank_row()}
                            {menu_entry_row(
                                "Normal mode",
                                "Single-player",
                                menu_icon_button("plus", "Open normal mode", false, open_normal_mode.clone()),
                            )}
                            {
                                if runtime.afk_enabled {
                                    menu_entry_row(
                                        "AFK mode",
                                        "Twitch plays",
                                        menu_icon_button("plus", "Open AFK mode", false, open_afk.clone()),
                                    )
                                } else {
                                    Html::default()
                                }
                            }
                            {menu_entry_row(
                                "Settings",
                                "Options",
                                menu_icon_button("plus", "Open settings", false, open_settings),
                            )}
                            {menu_entry_row(
                                "About",
                                "Credits",
                                menu_icon_button("plus", "Open about", false, open_about),
                            )}
                            {menu_blank_row()}
                        </tbody>
                    </table>
                </dialog>
            </div>
        },
    }
}
