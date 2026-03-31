use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use web_sys::MouseEvent;
use yew::prelude::Callback;

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub(crate) struct MouseButtons: u16 {
        const LEFT    = 1;
        const RIGHT   = 1 << 1;
        const MIDDLE  = 1 << 2;
        const BACK    = 1 << 3;
        const FORWARD = 1 << 4;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CellPointerState<Pos> {
    pub(crate) pos: Pos,
    pub(crate) buttons: MouseButtons,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CellMsg<Pos> {
    Update(CellPointerState<Pos>),
    Leave,
}

pub(crate) struct CellPointerCallbacks {
    pub(crate) onmousedown: Callback<MouseEvent>,
    pub(crate) onmouseup: Callback<MouseEvent>,
    pub(crate) onmouseenter: Callback<MouseEvent>,
    pub(crate) onmouseleave: Callback<MouseEvent>,
}

pub(crate) fn cell_pointer_callbacks<Pos>(
    pos: Pos,
    callback: Callback<CellMsg<Pos>>,
) -> CellPointerCallbacks
where
    Pos: Copy + 'static,
{
    let onmousedown = {
        let callback = callback.clone();
        Callback::from(move |event: MouseEvent| {
            callback.emit(CellMsg::Update(CellPointerState {
                pos,
                buttons: MouseButtons::from_bits_truncate(event.buttons()),
            }));
        })
    };

    let onmouseup = {
        let callback = callback.clone();
        Callback::from(move |event: MouseEvent| {
            callback.emit(CellMsg::Update(CellPointerState {
                pos,
                buttons: MouseButtons::from_bits_truncate(event.buttons()),
            }));
        })
    };

    let onmouseenter = {
        let callback = callback.clone();
        Callback::from(move |event: MouseEvent| {
            callback.emit(CellMsg::Update(CellPointerState {
                pos,
                buttons: MouseButtons::from_bits_truncate(event.buttons()),
            }));
        })
    };

    let onmouseleave = Callback::from(move |_event: MouseEvent| callback.emit(CellMsg::Leave));

    CellPointerCallbacks {
        onmousedown,
        onmouseup,
        onmouseenter,
        onmouseleave,
    }
}

pub(crate) fn update_cell_pointer_state<Pos, F>(
    current_state: &mut Option<CellPointerState<Pos>>,
    msg: CellMsg<Pos>,
    mut on_release: F,
) -> bool
where
    Pos: Copy + PartialEq,
    F: FnMut(CellPointerState<Pos>) -> bool,
{
    match msg {
        CellMsg::Leave => current_state.take().is_some(),
        CellMsg::Update(cell_state) if cell_state.buttons.is_empty() => current_state
            .take()
            .is_some_and(|previous_state| on_release(previous_state)),
        CellMsg::Update(cell_state) => match current_state.replace(cell_state) {
            None => true,
            Some(previous_state) => {
                (previous_state.pos != cell_state.pos)
                    && ((previous_state.buttons & MouseButtons::LEFT)
                        != (cell_state.buttons & MouseButtons::LEFT))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_cell_pointer_state_defers_action_until_release() {
        let mut current_state = None;
        let press = CellMsg::Update(CellPointerState {
            pos: (1_u8, 2_u8),
            buttons: MouseButtons::LEFT,
        });

        assert!(update_cell_pointer_state(&mut current_state, press, |_| {
            false
        }));
        assert_eq!(
            current_state,
            Some(CellPointerState {
                pos: (1, 2),
                buttons: MouseButtons::LEFT,
            })
        );

        let mut released_state = None;
        let released = update_cell_pointer_state(
            &mut current_state,
            CellMsg::Update(CellPointerState {
                pos: (1_u8, 2_u8),
                buttons: MouseButtons::empty(),
            }),
            |state| {
                released_state = Some(state);
                true
            },
        );

        assert!(released);
        assert_eq!(
            released_state,
            Some(CellPointerState {
                pos: (1, 2),
                buttons: MouseButtons::LEFT,
            })
        );
        assert_eq!(current_state, None);
    }

    #[test]
    fn leave_clears_existing_pointer_state() {
        let mut current_state = Some(CellPointerState {
            pos: (3_u8, 4_u8),
            buttons: MouseButtons::RIGHT,
        });

        assert!(update_cell_pointer_state(
            &mut current_state,
            CellMsg::Leave,
            |_| false
        ));
        assert_eq!(current_state, None);
    }
}
