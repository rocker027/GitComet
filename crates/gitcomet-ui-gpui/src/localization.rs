use crate::preference_writer::PreferenceWriter;
use gitcomet_state::session::{self, UiSettings};
use gpui::{App, Global};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Language {
    #[default]
    English,
    TraditionalChinese,
}

impl Global for Language {}

impl Language {
    pub(crate) const ALL: [Self; 2] = [Self::English, Self::TraditionalChinese];

    pub(crate) fn current(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    pub(crate) fn initialize(stored: Option<&str>, cx: &mut App) {
        if cx.try_global::<Self>().is_none() {
            let language = match stored {
                Some("zh-TW") => Self::TraditionalChinese,
                _ => Self::English,
            };
            cx.set_global(language);
        }
    }

    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::TraditionalChinese => "zh-TW",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::TraditionalChinese => "繁體中文（台灣）",
        }
    }

    pub(crate) const fn text<'a>(self, english: &'a str, traditional_chinese: &'a str) -> &'a str {
        match self {
            Self::English => english,
            Self::TraditionalChinese => traditional_chinese,
        }
    }

    pub(crate) fn select(self, cx: &mut App) {
        static WRITER: OnceLock<PreferenceWriter> = OnceLock::new();
        let writer = WRITER.get_or_init(PreferenceWriter::default).clone();
        let sequence = writer.next();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = writer.persist(sequence, || {
                    session::persist_ui_settings(UiSettings {
                        ui_language: Some(self.key().to_owned()),
                        ..UiSettings::default()
                    })
                }) {
                    eprintln!("Could not save the interface language: {error}");
                }
            })
            .detach();

        cx.set_global(self);
        cx.refresh_windows();
    }
}

pub(crate) fn text<'a>(cx: &App, english: &'a str, traditional_chinese: &'a str) -> &'a str {
    Language::current(cx).text(english, traditional_chinese)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn ui_language_restores_traditional_chinese_and_keeps_the_live_choice(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            Language::initialize(Some("zh-TW"), cx);
            assert_eq!(text(cx, "Appearance", "外觀"), "外觀");
            Language::English.select(cx);
            Language::initialize(Some("zh-TW"), cx);
            assert_eq!(text(cx, "Appearance", "外觀"), "Appearance");
        });
    }

    #[gpui::test]
    fn ui_language_defaults_to_english_for_unknown_saved_values(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(Language::current(cx), Language::English);
            Language::initialize(Some("unknown"), cx);
            assert_eq!(Language::current(cx), Language::English);
        });
    }
}
