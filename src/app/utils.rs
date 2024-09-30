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

pub(in crate::app) fn format_for_counter(num: i32) -> String {
    match num {
        ..-99 => "-99".to_string(),
        // Some places do 0-1 for -1, I've also seen -01, which I'm leaning more to
        //-99..-9 => format!("-{:02}", -num),
        //-9..0 => format!("0-{:01}", -num),
        -99..0 => format!("-{:02}", -num),
        0..1000 => format!("{:03}", num),
        1000.. => "999".to_string(),
    }
}

pub(in crate::app) trait StorageKey {
    const KEY: &'static str;
}

impl<T> StorageKey for Option<T>
where
    T: StorageKey,
{
    const KEY: &'static str = T::KEY;
}

/// Easily load values from local storage
pub(in crate::app) trait LocalOrDefault: Default + StorageKey {
    fn local_or_default() -> Self;
}

impl<T> LocalOrDefault for T
where
    T: for<'a> serde::Deserialize<'a> + Default + StorageKey,
{
    fn local_or_default() -> Self {
        use gloo::storage::{LocalStorage, Storage};
        LocalStorage::get(Self::KEY).unwrap_or_default()
    }
}

/// Easily save values to local storage
pub(in crate::app) trait LocalSave: Clone + StorageKey {
    fn local_save(&self);
}

impl<T> LocalSave for T
where
    T: serde::Serialize + Clone + StorageKey,
{
    fn local_save(&self) {
        use gloo::storage::{LocalStorage, Storage};
        if let Err(err) = LocalStorage::set(Self::KEY, self.clone()) {
            log::error!(
                "Could not save to local storage key {}: {:?}",
                Self::KEY,
                err
            );
        }
    }
}
