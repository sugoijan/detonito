use yew::prelude::*;

const SPRITE_SHEET: &str = include_str!("../generated/sprite.svg");
const OPENMOJI_VIEW_BOX: &str = "0 0 72 72";
const OPENMOJI_CENTERED_64_VIEW_BOX: &str = "4 4 64 64";

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum IconCrop {
    #[default]
    Full,
    CenteredSquare64,
}

impl IconCrop {
    fn view_box(self) -> &'static str {
        match self {
            Self::Full => OPENMOJI_VIEW_BOX,
            Self::CenteredSquare64 => OPENMOJI_CENTERED_64_VIEW_BOX,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum GlyphSet {
    Counter,
    Cell,
}

impl GlyphSet {
    fn symbol_prefix(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Cell => "cell",
        }
    }

    fn view_box(self) -> &'static str {
        match self {
            Self::Counter => "0 0 5 9",
            Self::Cell => "0 0 4 5",
        }
    }

    fn class_name(self) -> &'static str {
        match self {
            Self::Counter => "dtn-glyph-counter",
            Self::Cell => "dtn-glyph-cell",
        }
    }
}

fn glyph_symbol_name(ch: char) -> Option<String> {
    match ch {
        '0'..='9' => Some(ch.to_string()),
        '-' => Some("minus".to_string()),
        '×' => Some("times".to_string()),
        _ => None,
    }
}

#[function_component(SpriteDefs)]
pub(crate) fn sprite_defs() -> Html {
    Html::from_html_unchecked(AttrValue::from(SPRITE_SHEET))
}

#[derive(Properties, PartialEq)]
pub(crate) struct IconProps {
    pub name: AttrValue,
    #[prop_or_default]
    pub crop: IconCrop,
    #[prop_or_default]
    pub class: Classes,
}

#[function_component(Icon)]
pub(crate) fn icon(props: &IconProps) -> Html {
    let href = AttrValue::from(format!("#dtn-icon-{}", props.name.as_str()));
    html! {
        <svg
            class={classes!("dtn-icon", props.class.clone())}
            viewBox={props.crop.view_box()}
            aria-hidden="true"
            focusable="false"
        >
            <use href={href}/>
        </svg>
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct GlyphProps {
    pub set: GlyphSet,
    pub ch: char,
    #[prop_or_default]
    pub class: Classes,
}

#[function_component(Glyph)]
pub(crate) fn glyph(props: &GlyphProps) -> Html {
    let Some(name) = glyph_symbol_name(props.ch) else {
        return Html::default();
    };
    let href = AttrValue::from(format!("#dtn-glyph-{}-{}", props.set.symbol_prefix(), name));
    html! {
        <svg
            class={classes!("dtn-glyph", props.set.class_name(), props.class.clone())}
            viewBox={props.set.view_box()}
            aria-hidden="true"
            focusable="false"
        >
            <use href={href}/>
        </svg>
    }
}

#[derive(Properties, PartialEq)]
pub(crate) struct GlyphRunProps {
    pub set: GlyphSet,
    pub text: AttrValue,
    #[prop_or_default]
    pub class: Classes,
}

#[function_component(GlyphRun)]
pub(crate) fn glyph_run(props: &GlyphRunProps) -> Html {
    html! {
        <span
            class={classes!("dtn-glyph-run", props.set.class_name(), props.class.clone())}
            aria-label={props.text.clone()}
        >
            {
                for props.text.as_str().chars().map(|ch| {
                    if ch == ' ' {
                        html! { <span class="dtn-glyph-space" aria-hidden="true"/> }
                    } else {
                        html! { <Glyph set={props.set} ch={ch}/> }
                    }
                })
            }
        </span>
    }
}
