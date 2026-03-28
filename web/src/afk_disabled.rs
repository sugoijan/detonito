use yew::prelude::*;

use crate::menu::{menu_blank_row, menu_entry_row, menu_header_row, menu_icon_button};
use crate::sprites::SpriteDefs;

#[derive(Properties, PartialEq)]
pub(crate) struct AfkViewProps {
    pub on_menu: Callback<()>,
    #[prop_or_default]
    pub auth_error: Option<String>,
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
                        {menu_blank_row()}
                        {menu_header_row("AFK mode", on_back.clone())}
                        {menu_blank_row()}
                        {menu_entry_row(
                            "Unavailable",
                            "Disabled in this build",
                            menu_icon_button("minus", "Go back", false, on_back),
                        )}
                        {menu_blank_row()}
                    </tbody>
                </table>
            </dialog>
        </div>
    }
}
