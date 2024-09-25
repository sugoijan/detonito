use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub(in crate::app) struct ModalProps {
    #[prop_or_default]
    pub children: Html,
}

/// Helper component to attatch the contents into the document.body instead of in the place where it's used.
#[function_component]
pub(in crate::app) fn Modal(props: &ModalProps) -> Html {
    let modal_host = gloo::utils::body();
    create_portal(props.children.clone(), modal_host.into())
}

/// Helper function to use JavaScript's Math.random
pub(in crate::app) fn js_random_seed() -> u64 {
    use js_sys::Math::random;
    u64::from_be_bytes([
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
        (256. * random()) as u8,
    ])
}
