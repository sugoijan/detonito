use yew::prelude::*;

use crate::sprites::{Icon, IconCrop};

#[cfg(feature = "afk-runtime")]
pub(crate) fn menu_icon_button(
    icon: &'static str,
    title: impl Into<AttrValue>,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    menu_icon_button_with_class(icon, "button-icon", title, disabled, onclick)
}

fn menu_icon_button_with_class(
    icon: &'static str,
    icon_class: &'static str,
    title: impl Into<AttrValue>,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    html! {
        <button {disabled} {onclick} title={title.into()}>
            <Icon name={icon} crop={IconCrop::CenteredSquare64} class={classes!(icon_class)}/>
        </button>
    }
}

pub(crate) fn menu_nav_enter_button(
    title: impl Into<AttrValue>,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    menu_icon_button_with_class("menu-enter", "nav-icon", title, disabled, onclick)
}

pub(crate) fn menu_nav_back_button(
    title: impl Into<AttrValue>,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    menu_icon_button_with_class("menu-back", "nav-icon", title, disabled, onclick)
}

pub(crate) fn menu_blank_row() -> Html {
    html! {
        <tr>
            <td class="menu-pad"/><td class="menu-pad"/><td class="menu-pad"/><td class="menu-pad"/>
            <td class="menu-pad"/><td class="menu-pad"/><td class="menu-pad"/><td class="menu-pad"/>
            <td class="menu-pad"/><td class="menu-pad"/><td class="menu-pad"/><td class="menu-pad"/>
            <td class="menu-pad"/><td class="menu-pad"/>
        </tr>
    }
}

pub(crate) fn menu_header_row(title: impl Into<AttrValue>, on_back: Callback<MouseEvent>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">
                {menu_nav_back_button("Go back", false, on_back)}
            </td>
            <td class="menu-heading" colspan="11">{title.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

pub(crate) fn menu_entry_row(
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
