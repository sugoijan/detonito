use std::rc::Rc;

use crate::sprites::{Icon, IconCrop};
use gloo::timers::callback::{Interval, Timeout};
use web_sys::{FocusEvent, HtmlInputElement, InputEvent, KeyboardEvent, PointerEvent};
use yew::{TargetCast, prelude::*};

/// The menu grid is a fixed 14-column layout:
/// pad + content columns + pad. Shared row builders keep the colspans aligned
/// so new menu pages do not drift visually.
const MENU_COLUMNS: usize = 14;
const MENU_STEPPER_LABEL_COLSPAN: usize = 6;
const MENU_STEPPER_DETAIL_COLSPAN: usize = 4;
const MENU_STEPPER_LABEL_COLSPAN_WITH_PREFIX: usize = MENU_STEPPER_LABEL_COLSPAN - 1;
const HOLD_REPEAT_DELAY_MS: u32 = 350;
const HOLD_REPEAT_INTERVAL_MS: u32 = 75;
const HOLD_CLICK_SUPPRESSION_CLEAR_MS: u32 = 250;

fn menu_blank_cells(count: usize) -> Html {
    Html::from_iter((0..count).map(|_| html! { <td class="menu-pad"/> }))
}

fn menu_icon_button_with_class(
    icon: &'static str,
    icon_class: &'static str,
    title: impl Into<AttrValue>,
    pressed: bool,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    html! {
        <button class={classes!(pressed.then_some("pressed"))} {disabled} {onclick} title={title.into()}>
            <Icon name={icon} crop={IconCrop::CenteredSquare64} class={classes!(icon_class)}/>
        </button>
    }
}

pub(crate) fn menu_icon_button(
    icon: &'static str,
    title: impl Into<AttrValue>,
    pressed: bool,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    menu_icon_button_with_class(icon, "button-icon", title, pressed, disabled, onclick)
}

pub(crate) fn menu_nav_enter_button(
    title: impl Into<AttrValue>,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    menu_icon_button_with_class("menu-enter", "nav-icon", title, false, disabled, onclick)
}

pub(crate) fn menu_nav_back_button(
    title: impl Into<AttrValue>,
    disabled: bool,
    onclick: Callback<MouseEvent>,
) -> Html {
    menu_icon_button_with_class("menu-back", "nav-icon", title, false, disabled, onclick)
}

pub(crate) fn menu_section_gap() -> Html {
    html! {
        <tr>{menu_blank_cells(MENU_COLUMNS)}</tr>
    }
}

pub(crate) fn menu_title_row(title: impl Into<AttrValue>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-heading" colspan="12">{title.into()}</td>
            <td class="menu-pad"/>
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

/// Wide-detail rows are the default navigation row: icon slot, left-aligned
/// label, then a right-side detail slot.
pub(crate) fn menu_wide_detail_row(
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

pub(crate) fn menu_primary_row(label: impl Into<AttrValue>, button: Html) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-button-slot">{button}</td>
            <td class="menu-text" colspan="11">{label.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

#[cfg(feature = "afk-runtime")]
pub(crate) fn menu_toggle_row(
    label: impl Into<AttrValue>,
    left_button: Html,
    right_button: Html,
) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-text" colspan="9">{label.into()}</td>
            <td class="menu-button-slot">{left_button}</td>
            <td class="menu-button-slot">{right_button}</td>
            <td class="menu-pad"/>
            <td class="menu-pad"/>
        </tr>
    }
}

/// Stepper rows own the +/- geometry for every numeric menu control. New
/// steppers should go through this helper so alignment stays consistent.
pub(crate) fn menu_stepper_row(
    prefix_button: Option<Html>,
    label: impl Into<AttrValue>,
    detail: Html,
    decrease_button: Html,
    increase_button: Html,
) -> Html {
    let label_colspan = if prefix_button.is_some() {
        MENU_STEPPER_LABEL_COLSPAN_WITH_PREFIX
    } else {
        MENU_STEPPER_LABEL_COLSPAN
    };

    html! {
        <tr>
            <td class="menu-pad"/>
            {
                prefix_button.map(|button| html! {
                    <td class="menu-button-slot">{button}</td>
                }).unwrap_or_default()
            }
            <td class="menu-text" colspan={label_colspan.to_string()}>{label.into()}</td>
            <td class="menu-button-slot">{decrease_button}</td>
            <td
                class={classes!("menu-detail", "menu-number-detail")}
                colspan={MENU_STEPPER_DETAIL_COLSPAN.to_string()}
            >
                {detail}
            </td>
            <td class="menu-button-slot">{increase_button}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

pub(crate) fn menu_copy_row(text: impl Into<AttrValue>) -> Html {
    html! {
        <tr>
            <td class="menu-pad"/>
            <td class="menu-about-copy" colspan="12">{text.into()}</td>
            <td class="menu-pad"/>
        </tr>
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct RepeatIconButtonProps {
    pub icon: &'static str,
    pub title: AttrValue,
    #[prop_or_default]
    pub disabled: bool,
    pub on_activate: Callback<()>,
}

#[function_component]
pub(crate) fn RepeatIconButton(props: &RepeatIconButtonProps) -> Html {
    let is_pressed = use_state_eq(|| false);
    let repeat_timeout = use_mut_ref(|| None::<Timeout>);
    let repeat_interval = use_mut_ref(|| None::<Interval>);
    let suppress_click = use_mut_ref(|| false);
    let suppress_clear_timeout = use_mut_ref(|| None::<Timeout>);
    let disabled = props.disabled;

    let stop_repeat = {
        let is_pressed = is_pressed.clone();
        let repeat_timeout = repeat_timeout.clone();
        let repeat_interval = repeat_interval.clone();
        let suppress_click = suppress_click.clone();
        let suppress_clear_timeout = suppress_clear_timeout.clone();
        Rc::new(move || {
            is_pressed.set(false);
            repeat_timeout.borrow_mut().take();
            repeat_interval.borrow_mut().take();
            suppress_clear_timeout.borrow_mut().take();
            if *suppress_click.borrow() {
                let suppress_click = suppress_click.clone();
                *suppress_clear_timeout.borrow_mut() =
                    Some(Timeout::new(HOLD_CLICK_SUPPRESSION_CLEAR_MS, move || {
                        *suppress_click.borrow_mut() = false;
                    }));
            }
        })
    };

    let onpointerdown = {
        let is_pressed = is_pressed.clone();
        let repeat_timeout = repeat_timeout.clone();
        let repeat_interval = repeat_interval.clone();
        let suppress_click = suppress_click.clone();
        let suppress_clear_timeout = suppress_clear_timeout.clone();
        let on_activate = props.on_activate.clone();
        Callback::from(move |event: PointerEvent| {
            if disabled || !event.is_primary() || event.button() != 0 {
                return;
            }

            is_pressed.set(true);
            *suppress_click.borrow_mut() = true;
            suppress_clear_timeout.borrow_mut().take();
            repeat_timeout.borrow_mut().take();
            repeat_interval.borrow_mut().take();

            on_activate.emit(());

            let on_activate = on_activate.clone();
            let repeat_interval = repeat_interval.clone();
            *repeat_timeout.borrow_mut() = Some(Timeout::new(HOLD_REPEAT_DELAY_MS, move || {
                on_activate.emit(());

                let on_activate = on_activate.clone();
                *repeat_interval.borrow_mut() =
                    Some(Interval::new(HOLD_REPEAT_INTERVAL_MS, move || {
                        on_activate.emit(());
                    }));
            }));
        })
    };

    let onclick = {
        let suppress_click = suppress_click.clone();
        let suppress_clear_timeout = suppress_clear_timeout.clone();
        let on_activate = props.on_activate.clone();
        Callback::from(move |_event: MouseEvent| {
            if disabled {
                return;
            }

            let was_suppressed = *suppress_click.borrow();
            if was_suppressed {
                *suppress_click.borrow_mut() = false;
                suppress_clear_timeout.borrow_mut().take();
                return;
            }
            on_activate.emit(());
        })
    };

    let onpointerup = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: PointerEvent| stop_repeat())
    };

    let onpointerleave = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: PointerEvent| stop_repeat())
    };

    let onpointercancel = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: PointerEvent| stop_repeat())
    };

    let onblur = {
        let stop_repeat = stop_repeat.clone();
        Callback::from(move |_event: FocusEvent| stop_repeat())
    };

    html! {
        <button
            class={classes!((*is_pressed).then_some("pressed"))}
            type="button"
            title={props.title.clone()}
            disabled={props.disabled}
            {onclick}
            {onblur}
            {onpointerdown}
            {onpointerup}
            {onpointerleave}
            {onpointercancel}
        >
            <Icon name={props.icon} crop={IconCrop::CenteredSquare64} class={classes!("button-icon")}/>
        </button>
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct MenuNumberFieldProps {
    pub label: AttrValue,
    pub value: u16,
    pub min: u16,
    pub max: u16,
    #[prop_or_default]
    pub suffix: Option<AttrValue>,
    pub on_set: Callback<u16>,
}

#[function_component]
pub(crate) fn MenuNumberField(props: &MenuNumberFieldProps) -> Html {
    let input_ref = use_node_ref();
    let is_editing = use_state_eq(|| false);
    let draft = use_state_eq(|| props.value.to_string());

    let commit = {
        let draft = draft.clone();
        let is_editing = is_editing.clone();
        let on_set = props.on_set.clone();
        Rc::new(move || {
            if let Ok(parsed) = draft.trim().parse::<u16>() {
                on_set.emit(parsed);
            }
            is_editing.set(false);
        })
    };

    let cancel = {
        let is_editing = is_editing.clone();
        Rc::new(move || is_editing.set(false))
    };

    let onfocus = {
        let draft = draft.clone();
        let input_ref = input_ref.clone();
        let is_editing = is_editing.clone();
        let value = props.value;
        Callback::from(move |_event: FocusEvent| {
            is_editing.set(true);
            draft.set(value.to_string());
            if let Some(input) = input_ref.cast::<HtmlInputElement>() {
                let _ = input.select();
            }
        })
    };

    let oninput = {
        let draft = draft.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            draft.set(input.value());
        })
    };

    let onblur = {
        let commit = commit.clone();
        Callback::from(move |_event: FocusEvent| commit())
    };

    let onkeydown = {
        let commit = commit.clone();
        let cancel = cancel.clone();
        let input_ref = input_ref.clone();
        Callback::from(move |event: KeyboardEvent| match event.key().as_str() {
            "Enter" => {
                event.prevent_default();
                commit();
                if let Some(input) = input_ref.cast::<HtmlInputElement>() {
                    let _ = input.blur();
                }
            }
            "Escape" => {
                event.prevent_default();
                cancel();
                if let Some(input) = input_ref.cast::<HtmlInputElement>() {
                    let _ = input.blur();
                }
            }
            _ => {}
        })
    };

    let value = if *is_editing {
        (*draft).clone()
    } else {
        props.value.to_string()
    };

    let input = html! {
        <input
            ref={input_ref}
            class={classes!("menu-number-input", props.suffix.as_ref().map(|_| "with-suffix"))}
            type="number"
            inputmode="numeric"
            title={format!("Set {}", props.label)}
            aria-label={format!("Set {}", props.label)}
            min={props.min.to_string()}
            max={props.max.to_string()}
            step="1"
            value={value}
            {onfocus}
            {oninput}
            {onblur}
            {onkeydown}
        />
    };

    if let Some(suffix) = props.suffix.clone() {
        html! {
            <span class="menu-number-field">
                {input}
                <span class="menu-number-suffix" aria-hidden="true">{suffix}</span>
            </span>
        }
    } else {
        input
    }
}

pub(crate) fn menu_number_stepper_row_with_suffix(
    label: &'static str,
    value: u16,
    min: u16,
    max: u16,
    suffix: Option<AttrValue>,
    prefix_button: Option<Html>,
    on_decrease: Callback<()>,
    on_set: Callback<u16>,
    on_increase: Callback<()>,
) -> Html {
    menu_stepper_row(
        prefix_button,
        label,
        html! {
            <MenuNumberField
                label={label}
                value={value}
                min={min}
                max={max}
                suffix={suffix}
                on_set={on_set}
            />
        },
        html! {
            <RepeatIconButton
                icon="minus"
                title={format!("Decrease {}", label)}
                on_activate={on_decrease}
            />
        },
        html! {
            <RepeatIconButton
                icon="plus"
                title={format!("Increase {}", label)}
                on_activate={on_increase}
            />
        },
    )
}

pub(crate) fn menu_number_stepper_row(
    label: &'static str,
    value: u16,
    min: u16,
    max: u16,
    on_decrease: Callback<()>,
    on_set: Callback<u16>,
    on_increase: Callback<()>,
) -> Html {
    menu_number_stepper_row_with_suffix(
        label,
        value,
        min,
        max,
        None,
        None,
        on_decrease,
        on_set,
        on_increase,
    )
}
