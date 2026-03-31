use std::rc::Rc;

use yew::prelude::*;

use crate::afk::AfkView;
use crate::game::{GameInitArgs, GameView, clear_saved_game, has_saved_game};
use crate::menu::{
    menu_header_row, menu_nav_enter_button, menu_section_gap, menu_title_row, menu_wide_detail_row,
};
use crate::normal::NormalMenuView;
use crate::runtime::{AppRoute, frontend_runtime_config, initialize_route_state, replace_route};
use crate::settings::{AboutView, SettingsEntryPoint, SettingsView};
use crate::sprites::SpriteDefs;

#[derive(Properties, Clone, PartialEq)]
pub(crate) struct AppShellProps {
    pub init: GameInitArgs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellScreen {
    Menu,
    NormalMenu,
    Classic,
    Afk,
    Settings(SettingsEntryPoint),
    About,
}

fn route_to_screen(route: AppRoute) -> ShellScreen {
    match route {
        AppRoute::Menu => ShellScreen::Menu,
        AppRoute::NormalMenu => ShellScreen::NormalMenu,
        AppRoute::Classic => ShellScreen::Classic,
        AppRoute::Afk => ShellScreen::Afk,
        AppRoute::Settings => ShellScreen::Settings(SettingsEntryPoint::Main),
        AppRoute::SettingsNormal => ShellScreen::Settings(SettingsEntryPoint::Normal),
        AppRoute::SettingsAfk => ShellScreen::Settings(SettingsEntryPoint::Afk),
        AppRoute::About => ShellScreen::About,
    }
}

fn screen_to_route(screen: ShellScreen) -> AppRoute {
    match screen {
        ShellScreen::Menu => AppRoute::Menu,
        ShellScreen::NormalMenu => AppRoute::NormalMenu,
        ShellScreen::Classic => AppRoute::Classic,
        ShellScreen::Afk => AppRoute::Afk,
        ShellScreen::Settings(SettingsEntryPoint::Main) => AppRoute::Settings,
        ShellScreen::Settings(SettingsEntryPoint::Normal) => AppRoute::SettingsNormal,
        ShellScreen::Settings(SettingsEntryPoint::Afk) => AppRoute::SettingsAfk,
        ShellScreen::About => AppRoute::About,
    }
}

#[function_component]
pub(crate) fn AppShell(props: &AppShellProps) -> Html {
    let runtime = frontend_runtime_config();
    let initial_route = use_memo((), |_| initialize_route_state());
    let initial_screen = route_to_screen(initial_route.route);
    let restore_afk_view_state = matches!(initial_screen, ShellScreen::Afk);
    let initial_afk_err = initial_route.afk_auth_error.clone();
    let initial_afk_start_after_connect = initial_route.afk_start_after_connect;
    let screen = use_state_eq(move || initial_screen);
    let menu_return_target = use_state_eq(|| None::<ShellScreen>);
    let normal_can_resume = use_state_eq(|| {
        matches!(
            initial_screen,
            ShellScreen::Classic | ShellScreen::NormalMenu
        ) || has_saved_game()
    });
    let afk_auth_error = use_state_eq(move || initial_afk_err);
    let afk_start_after_connect = use_state_eq(move || initial_afk_start_after_connect);

    let navigate_to = {
        let screen = screen.clone();
        Rc::new(move |next: ShellScreen| {
            replace_route(screen_to_route(next));
            screen.set(next);
        })
    };

    let open_main_menu = {
        let navigate_to = navigate_to.clone();
        let menu_return_target = menu_return_target.clone();
        let afk_auth_error = afk_auth_error.clone();
        Rc::new(move |return_target: Option<ShellScreen>| {
            afk_auth_error.set(None);
            menu_return_target.set(return_target);
            navigate_to(ShellScreen::Menu);
        })
    };

    let on_classic_menu = {
        let navigate_to = navigate_to.clone();
        let normal_can_resume = normal_can_resume.clone();
        Callback::from(move |_| {
            normal_can_resume.set(true);
            navigate_to(ShellScreen::NormalMenu);
        })
    };

    let on_afk_menu = {
        let open_main_menu = open_main_menu.clone();
        Callback::from(move |_| open_main_menu(Some(ShellScreen::Afk)))
    };

    let open_normal_mode = {
        let navigate_to = navigate_to.clone();
        let normal_can_resume = normal_can_resume.clone();
        Callback::from(move |_| {
            normal_can_resume.set(*normal_can_resume || has_saved_game());
            navigate_to(ShellScreen::NormalMenu);
        })
    };

    let resume_normal_mode = {
        let navigate_to = navigate_to.clone();
        let normal_can_resume = normal_can_resume.clone();
        Callback::from(move |_: MouseEvent| {
            normal_can_resume.set(true);
            navigate_to(ShellScreen::Classic);
        })
    };

    let start_new_normal_game = {
        let navigate_to = navigate_to.clone();
        let normal_can_resume = normal_can_resume.clone();
        Callback::from(move |_: MouseEvent| {
            clear_saved_game();
            normal_can_resume.set(true);
            navigate_to(ShellScreen::Classic);
        })
    };

    let back_to_main_menu = {
        let open_main_menu = open_main_menu.clone();
        Callback::from(move |_: MouseEvent| open_main_menu(None))
    };

    let open_main_settings = {
        let navigate_to = navigate_to.clone();
        let afk_auth_error = afk_auth_error.clone();
        let menu_return_target = menu_return_target.clone();
        Callback::from(move |_| {
            afk_auth_error.set(None);
            menu_return_target.set(None);
            navigate_to(ShellScreen::Settings(SettingsEntryPoint::Main));
        })
    };

    let open_normal_settings = {
        let navigate_to = navigate_to.clone();
        Callback::from(move |_: MouseEvent| {
            navigate_to(ShellScreen::Settings(SettingsEntryPoint::Normal));
        })
    };

    let open_afk_settings = {
        let navigate_to = navigate_to.clone();
        Callback::from(move |_| {
            navigate_to(ShellScreen::Settings(SettingsEntryPoint::Afk));
        })
    };

    let open_about = {
        let navigate_to = navigate_to.clone();
        let afk_auth_error = afk_auth_error.clone();
        let menu_return_target = menu_return_target.clone();
        Callback::from(move |_| {
            afk_auth_error.set(None);
            menu_return_target.set(None);
            navigate_to(ShellScreen::About);
        })
    };

    let open_afk = {
        let navigate_to = navigate_to.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_| {
            if runtime.afk_enabled {
                afk_auth_error.set(None);
                navigate_to(ShellScreen::Afk);
            }
        })
    };

    let consume_afk_start_after_connect = {
        let afk_start_after_connect = afk_start_after_connect.clone();
        Callback::from(move |_| afk_start_after_connect.set(false))
    };

    let close_menu = {
        let navigate_to = navigate_to.clone();
        let menu_return_target = menu_return_target.clone();
        let afk_auth_error = afk_auth_error.clone();
        Callback::from(move |_: MouseEvent| {
            afk_auth_error.set(None);
            if let Some(target) = *menu_return_target {
                navigate_to(target);
            }
        })
    };

    match *screen {
        ShellScreen::Classic => html! {
            <GameView on_menu={on_classic_menu} init={props.init.clone()}/>
        },
        ShellScreen::NormalMenu => html! {
            <div class="detonito settings-open start-menu-shell">
                <SpriteDefs/>
                <NormalMenuView
                    open={true}
                    can_resume={*normal_can_resume}
                    on_back={back_to_main_menu}
                    on_resume={resume_normal_mode}
                    on_start_new={start_new_normal_game}
                    on_open_settings={open_normal_settings}
                />
            </div>
        },
        ShellScreen::Afk => html! {
            <AfkView
                on_menu={on_afk_menu}
                on_open_settings={open_afk_settings}
                auth_error={(*afk_auth_error).clone()}
                start_after_connect={*afk_start_after_connect}
                on_consume_start_after_connect={consume_afk_start_after_connect}
                restore_view_state={restore_afk_view_state}
            />
        },
        ShellScreen::Settings(entry_point) => {
            let navigate_to = navigate_to.clone();
            let on_back = Callback::from(move |_: MouseEvent| {
                navigate_to(route_to_screen(entry_point.back_route()));
            });
            html! {
                <div class="detonito settings-open">
                    <SpriteDefs/>
                    <SettingsView
                        open={true}
                        on_back={on_back}
                        entry_point={entry_point}
                    />
                </div>
            }
        }
        ShellScreen::About => html! {
            <div class="detonito settings-open">
                <SpriteDefs/>
                <AboutView open={true} on_back={back_to_main_menu}/>
            </div>
        },
        ShellScreen::Menu => html! {
            <div class="detonito settings-open start-menu-shell">
                <SpriteDefs/>
                <dialog open=true>
                    <table class="menu-grid start-menu">
                        <tbody>
                            {menu_section_gap()}
                            {
                                if menu_return_target.is_some() {
                                    menu_header_row("Menu", close_menu.clone())
                                } else {
                                    menu_title_row("Detonito")
                                }
                            }
                            {menu_section_gap()}
                            {menu_wide_detail_row(
                                "Normal mode",
                                "Single-player",
                                menu_nav_enter_button("Open normal mode", false, open_normal_mode.clone()),
                            )}
                            {
                                if runtime.afk_enabled {
                                    menu_wide_detail_row(
                                        "AFK mode",
                                        "Twitch plays",
                                        menu_nav_enter_button("Open AFK mode", false, open_afk.clone()),
                                    )
                                } else {
                                    Html::default()
                                }
                            }
                            {menu_section_gap()}
                            {menu_wide_detail_row(
                                "Settings",
                                "",
                                menu_nav_enter_button("Open settings", false, open_main_settings),
                            )}
                            {menu_wide_detail_row(
                                "About",
                                "Credits",
                                menu_nav_enter_button("Open about", false, open_about),
                            )}
                            {menu_section_gap()}
                        </tbody>
                    </table>
                </dialog>
            </div>
        },
    }
}
