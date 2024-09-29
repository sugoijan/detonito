use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub(in crate::app) struct SettingsProps {
    #[prop_or_default]
    pub open: bool,
}

#[function_component]
pub(in crate::app) fn SettingsView(props: &SettingsProps) -> Html {
    html! {
        <dialog id="settings" open={props.open}>
            <table>
                <tr><td/><td/><td/></tr>
                <tr><td/><td/><td/></tr>
                <tr><td/><td/><td/></tr>
            </table>
            <article>
                <h2>{"Settings"}</h2>
                <ul>
                    <li><a href="#" data-theme-switcher="auto">{"Auto"}</a></li>
                    <li><a href="#" data-theme-switcher="light">{"Light"}</a></li>
                    <li><a href="#" data-theme-switcher="dark">{"Dark"}</a></li>
                </ul>
                <footer>
                    <button type="reset">{"Cancel"}</button>
                    <button>{"Apply"}</button>
                </footer>
            </article>
        </dialog>
    }
}
