//! The interface's language, English or French, which the settings pick.
//! Each text is written in both, side by side where it is used:
//! `tr("Save", "Enregistrer")` for a plain text, and
//! `tr!("Saved {}", "Enregistré : {}", path)` for one with values.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    French,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::French];

    /// As `settings.txt` writes it.
    pub fn id(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::French => "fr",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Language::ALL.into_iter().find(|l| l.id() == id.trim())
    }

    /// In its own language, as the settings list it.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
        }
    }
}

/// Whether the interface is in French.
static FRENCH: AtomicBool = AtomicBool::new(false);

pub fn set_language(language: Language) {
    FRENCH.store(language == Language::French, Ordering::Relaxed);
}

pub fn french() -> bool {
    FRENCH.load(Ordering::Relaxed)
}

/// The text in the interface's language.
pub fn tr(english: &'static str, french: &'static str) -> &'static str {
    if self::french() { french } else { english }
}

/// A text with values in the interface's language, as `format!` writes
/// it: `tr!("{} wires", "{} fils", count)`.
#[macro_export]
macro_rules! tr {
    ($english:literal, $french:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::french() { format!($french $(, $arg)*) } else { format!($english $(, $arg)*) }
    };
}
