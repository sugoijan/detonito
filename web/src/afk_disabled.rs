use yew::prelude::*;

use crate::menu::{menu_header_row, menu_nav_back_button, menu_section_gap, menu_wide_detail_row};
use crate::sprites::SpriteDefs;

#[derive(Properties, PartialEq)]
pub(crate) struct AfkViewProps {
    pub on_menu: Callback<()>,
    pub on_open_settings: Callback<()>,
    #[prop_or_default]
    pub auth_error: Option<String>,
    #[prop_or_default]
    pub start_after_connect: bool,
    pub on_consume_start_after_connect: Callback<()>,
}

#[function_component]
pub(crate) fn AfkView(props: &AfkViewProps) -> Html {
    let on_back = {
        let on_menu = props.on_menu.clone();
        Callback::from(move |_: MouseEvent| on_menu.emit(()))
    };

    html! {
        <div class="detonito settings-open start-menu-shell">
            <SpriteDefs/>
            <dialog open=true>
                <table class="menu-grid start-menu">
                    <tbody>
                        {menu_section_gap()}
                        {menu_header_row("AFK mode", on_back.clone())}
                        {menu_section_gap()}
                        {menu_wide_detail_row(
                            "Unavailable",
                            "Disabled in this build",
                            menu_nav_back_button("Go back", false, on_back),
                        )}
                        {menu_section_gap()}
                    </tbody>
                </table>
            </dialog>
        </div>
    }
}
