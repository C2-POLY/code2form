//! The keyboard shortcuts, which the settings can change: each action's
//! keys, the press they match, how `settings.txt` writes them, and which
//! actions share keys.

use std::cmp::Reverse;

use eframe::egui::{Context, Event, InputState, Key, KeyboardShortcut, Modifiers};

use crate::i18n::tr;

/// What a shortcut does, in the order of the Shortcuts page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    CloseTab,
    NextTab,
    PreviousTab,
    Render,
    Export,
    Find,
    FindReplace,
    FindNext,
    FindPrevious,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Agent,
    UndoAgent,
    Settings,
}

const fn keys(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

const SHIFT_COMMAND: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
const ALT_COMMAND: Modifiers = Modifiers { alt: true, ..Modifiers::COMMAND };

impl Action {
    pub const ALL: [Action; 19] = [
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::CloseTab,
        Action::NextTab,
        Action::PreviousTab,
        Action::Render,
        Action::Export,
        Action::Find,
        Action::FindReplace,
        Action::FindNext,
        Action::FindPrevious,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
        Action::Agent,
        Action::UndoAgent,
        Action::Settings,
    ];

    /// As `settings.txt` writes it, after `shortcut-`.
    pub fn id(self) -> &'static str {
        match self {
            Action::New => "new",
            Action::Open => "open",
            Action::Save => "save",
            Action::SaveAs => "save-as",
            Action::CloseTab => "close-tab",
            Action::NextTab => "next-tab",
            Action::PreviousTab => "previous-tab",
            Action::Render => "render",
            Action::Export => "export",
            Action::Find => "find",
            Action::FindReplace => "find-replace",
            Action::FindNext => "find-next",
            Action::FindPrevious => "find-previous",
            Action::ZoomIn => "zoom-in",
            Action::ZoomOut => "zoom-out",
            Action::ZoomReset => "zoom-reset",
            Action::Agent => "agent",
            Action::UndoAgent => "undo-agent",
            Action::Settings => "settings",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::New => tr("New File", "Nouveau fichier"),
            Action::Open => tr("Open…", "Ouvrir…"),
            Action::Save => tr("Save", "Enregistrer"),
            Action::SaveAs => tr("Save As…", "Enregistrer sous…"),
            Action::CloseTab => tr("Close Tab", "Fermer l'onglet"),
            Action::NextTab => tr("Next Tab", "Onglet suivant"),
            Action::PreviousTab => tr("Previous Tab", "Onglet précédent"),
            Action::Render => tr("Render", "Rendu"),
            Action::Export => tr("Export STL…", "Exporter en STL…"),
            Action::Find => tr("Find", "Rechercher"),
            Action::FindReplace => tr("Find and Replace", "Rechercher et remplacer"),
            Action::FindNext => tr("Find Next", "Occurrence suivante"),
            Action::FindPrevious => tr("Find Previous", "Occurrence précédente"),
            Action::ZoomIn => tr("Larger Interface", "Agrandir l'interface"),
            Action::ZoomOut => tr("Smaller Interface", "Réduire l'interface"),
            Action::ZoomReset => tr("Interface at 100%", "Interface à 100 %"),
            Action::Agent => tr("Chat with the Agent", "Discussion avec l'agent"),
            Action::UndoAgent => tr("Undo the Agent's Change", "Annuler la modification de l'agent"),
            Action::Settings => tr("Settings…", "Réglages…"),
        }
    }

    pub fn default_keys(self) -> KeyboardShortcut {
        match self {
            Action::New => keys(Modifiers::COMMAND, Key::N),
            Action::Open => keys(Modifiers::COMMAND, Key::O),
            Action::Save => keys(Modifiers::COMMAND, Key::S),
            Action::SaveAs => keys(SHIFT_COMMAND, Key::S),
            Action::CloseTab => keys(Modifiers::COMMAND, Key::W),
            Action::NextTab => keys(SHIFT_COMMAND, Key::CloseBracket),
            Action::PreviousTab => keys(SHIFT_COMMAND, Key::OpenBracket),
            Action::Render => keys(Modifiers::NONE, Key::F5),
            Action::Export => keys(Modifiers::NONE, Key::F7),
            Action::Find => keys(Modifiers::COMMAND, Key::F),
            Action::FindReplace => keys(ALT_COMMAND, Key::F),
            Action::FindNext => keys(Modifiers::COMMAND, Key::G),
            Action::FindPrevious => keys(SHIFT_COMMAND, Key::G),
            // ⌘= too: `Plus` matches `Equals`, the key it is typed with.
            Action::ZoomIn => keys(Modifiers::COMMAND, Key::Plus),
            Action::ZoomOut => keys(Modifiers::COMMAND, Key::Minus),
            Action::ZoomReset => keys(Modifiers::COMMAND, Key::Num0),
            Action::Agent => keys(Modifiers::COMMAND, Key::L),
            Action::UndoAgent => keys(ALT_COMMAND, Key::Z),
            Action::Settings => keys(Modifiers::COMMAND, Key::Comma),
        }
    }
}

/// Each action's keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Shortcuts {
    keys: [KeyboardShortcut; Action::ALL.len()],
}

impl Default for Shortcuts {
    fn default() -> Self {
        Shortcuts { keys: Action::ALL.map(Action::default_keys) }
    }
}

impl Shortcuts {
    pub fn get(&self, action: Action) -> KeyboardShortcut {
        self.keys[action as usize]
    }

    pub fn set(&mut self, action: Action, keys: KeyboardShortcut) {
        self.keys[action as usize] = normalize(keys);
    }

    pub fn is_default(&self, action: Action) -> bool {
        self.get(action) == action.default_keys()
    }

    /// Whether any action has keys of its own.
    pub fn customized(&self) -> bool {
        Action::ALL.iter().any(|&a| !self.is_default(a))
    }

    pub fn reset(&mut self, action: Action) {
        self.set(action, action.default_keys());
    }

    /// The other actions with the keys of `action`.
    pub fn sharing(&self, action: Action) -> Vec<Action> {
        let keys = self.get(action);
        Action::ALL.into_iter().filter(|&a| a != action && self.get(a) == keys).collect()
    }

    /// The actions that share keys, in groups, in the page's order: a
    /// press of their keys does the group's first.
    pub fn conflicts(&self) -> Vec<Vec<Action>> {
        let mut groups: Vec<Vec<Action>> = Vec::new();
        for action in Action::ALL {
            if groups.iter().flatten().any(|&a| a == action) {
                continue;
            }
            let sharing = self.sharing(action);
            if !sharing.is_empty() {
                groups.push(std::iter::once(action).chain(sharing).collect());
            }
        }
        groups
    }

    /// Takes the key presses of this frame that are shortcuts: the actions
    /// they ask for, in order.
    pub fn take(&self, input: &mut InputState) -> Vec<Action> {
        let mut actions = Vec::new();
        input.events.retain(|event| {
            let Event::Key { key, physical_key, pressed: true, modifiers, .. } = event else { return true };
            match self.action_of(*key, *physical_key, *modifiers) {
                Some(action) => {
                    actions.push(action);
                    false
                }
                None => true,
            }
        });
        actions
    }

    /// The action a press of `key` asks for. Of the actions whose keys it
    /// presses, with an extra Shift or Alt, as some keys need to be typed,
    /// the one with the most modifiers, so that ⇧⌘S is Save As, not Save.
    /// The key typed comes before the key it is typed with (`=` for `+`)
    /// and, for a sign, before the key at its place on the keyboard, as
    /// for a bracket that a layout types with Alt.
    fn action_of(&self, key: Key, physical: Option<Key>, modifiers: Modifiers) -> Option<Action> {
        let typed = canonical(key);
        let physical = physical.map(canonical);
        Action::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(index, action)| {
                let keys = self.get(action);
                if !modifiers.matches_logically(keys.modifiers) {
                    return None;
                }
                let rank = if keys.logical_key == typed {
                    2
                } else if typed_with(typed) == Some(keys.logical_key)
                    || (is_sign(keys.logical_key) && physical == Some(keys.logical_key))
                {
                    1
                } else {
                    return None;
                };
                Some(((rank, modifier_count(keys.modifiers), Reverse(index)), action))
            })
            .max_by_key(|&(order, _)| order)
            .map(|(_, action)| action)
    }

    /// Reads `shortcut-<id> = <keys>`; whether `id` is an action's.
    pub fn read(&mut self, id: &str, value: &str) -> bool {
        let Some(action) = Action::ALL.into_iter().find(|a| a.id() == id) else { return false };
        if let Some(keys) = from_text(value).filter(|&k| check(k).is_ok()) {
            self.set(action, keys);
        }
        true
    }

    /// The `shortcut-<id> = <keys>` lines of the actions with keys of their
    /// own.
    pub fn write(&self, text: &mut String) {
        for action in Action::ALL.into_iter().filter(|&a| !self.is_default(a)) {
            text.push_str(&format!("shortcut-{} = {}\n", action.id(), to_text(self.get(action))));
        }
    }
}

/// The keys a press with `modifiers` gives, as the Shortcuts page records
/// them; why not, for keys that type text or that the editor needs.
pub fn record(modifiers: Modifiers, key: Key) -> Result<KeyboardShortcut, String> {
    let keys = normalize(KeyboardShortcut::new(modifiers, canonical(key)));
    check(keys).map(|()| keys)
}

/// Whether the keys can be a shortcut; why not.
fn check(keys: KeyboardShortcut) -> Result<(), String> {
    let KeyboardShortcut { modifiers: m, logical_key: key } = keys;
    let editing = [
        Key::ArrowUp,
        Key::ArrowDown,
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Backspace,
        Key::Delete,
        Key::Enter,
        Key::Tab,
        Key::Escape,
    ];
    if editing.contains(&key) {
        return Err(tr!(
            "{} moves or edits in the editor: pick other keys",
            "{} déplace ou modifie dans l'éditeur : choisissez d'autres touches",
            key.name()
        ));
    }
    let function = key.name().strip_prefix('F').is_some_and(|n| n.parse::<u8>().is_ok());
    if !(m.command || m.ctrl || m.alt || function) {
        let with = if cfg!(target_os = "macos") { tr("⌘, ⌃ or ⌥", "⌘, ⌃ ou ⌥") } else { tr("Ctrl or Alt", "Ctrl ou Alt") };
        return Err(tr!(
            "Alone, these keys type text: add {with}, or use a function key",
            "Seules, ces touches tapent du texte : ajoutez {with}, ou utilisez une touche de fonction"
        ));
    }
    let command = Modifiers::COMMAND;
    let taken = [
        (KeyboardShortcut::new(command, Key::Z), tr("Undo", "Annuler")),
        (KeyboardShortcut::new(SHIFT_COMMAND, Key::Z), tr("Redo", "Rétablir")),
        (KeyboardShortcut::new(command, Key::X), tr("Cut", "Couper")),
        (KeyboardShortcut::new(command, Key::C), tr("Copy", "Copier")),
        (KeyboardShortcut::new(command, Key::V), tr("Paste", "Coller")),
        (KeyboardShortcut::new(command, Key::A), tr("Select All", "Tout sélectionner")),
        (KeyboardShortcut::new(command, Key::Q), tr("Quit", "Quitter")),
    ];
    match taken.iter().find(|(k, _)| *k == keys) {
        Some((_, name)) => Err(tr!("These keys are {name}'s: pick others", "Ces touches sont celles de « {name} » : choisissez-en d'autres")),
        None => Ok(()),
    }
}

/// The same keys as a press reports them on every system: `command` for
/// ⌘ on macOS and Ctrl elsewhere, `ctrl` for macOS's Control key only.
fn normalize(keys: KeyboardShortcut) -> KeyboardShortcut {
    let m = keys.modifiers;
    let mac = cfg!(target_os = "macos");
    let modifiers = Modifiers {
        alt: m.alt,
        shift: m.shift,
        ctrl: mac && m.ctrl,
        mac_cmd: false,
        command: m.command || m.mac_cmd || (!mac && m.ctrl),
    };
    KeyboardShortcut::new(modifiers, keys.logical_key)
}

/// The keys as the interface shows them: egui's names, with a sign's
/// sign (`]` rather than `CloseBracket`) except for `+` and `−`, which
/// read better as names after a `+`.
pub fn describe(ctx: &Context, keys: &KeyboardShortcut) -> String {
    let text = ctx.format_shortcut(keys);
    let key = keys.logical_key;
    if !is_sign(key) || matches!(key, Key::Plus | Key::Minus) {
        return text;
    }
    match text.strip_suffix(key.name()) {
        Some(modifiers) => format!("{modifiers}{}", key.symbol_or_name()),
        None => text,
    }
}

/// One name for the keys typed in two ways: `{` is `[` with Shift.
fn canonical(key: Key) -> Key {
    match key {
        Key::OpenCurlyBracket => Key::OpenBracket,
        Key::CloseCurlyBracket => Key::CloseBracket,
        other => other,
    }
}

/// The sign typed with the same key, where shortcuts take one for the
/// other: `=` for `+`.
fn typed_with(key: Key) -> Option<Key> {
    (key == Key::Equals).then_some(Key::Plus)
}

/// A punctuation sign, whose key moves from one layout to another.
fn is_sign(key: Key) -> bool {
    let mut chars = key.symbol_or_name().chars();
    chars.next().is_some_and(|c| !c.is_alphanumeric()) && chars.next().is_none()
}

fn modifier_count(m: Modifiers) -> u8 {
    u8::from(m.alt) + u8::from(m.shift) + u8::from(m.ctrl) + u8::from(m.command)
}

/// As `settings.txt` writes keys: `Ctrl+Alt+Shift+Cmd+S`, where `Cmd` is
/// ⌘ on macOS and Ctrl elsewhere, and `Ctrl` macOS's Control key.
pub fn to_text(keys: KeyboardShortcut) -> String {
    let m = normalize(keys).modifiers;
    let mut parts: Vec<&str> = [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.command, "Cmd")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    parts.push(keys.logical_key.name());
    parts.join("+")
}

pub fn from_text(text: &str) -> Option<KeyboardShortcut> {
    let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let key = Key::from_name(parts.pop()?)?;
    let mut modifiers = Modifiers::NONE;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" => modifiers.command = true,
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            _ => return None,
        }
    }
    Some(normalize(KeyboardShortcut::new(modifiers, canonical(key))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press of `key`, as winit reports it on this system.
    fn press(key: Key, physical: Option<Key>, modifiers: Modifiers) -> Event {
        let mac = cfg!(target_os = "macos");
        let modifiers = Modifiers {
            mac_cmd: mac && modifiers.command,
            ctrl: modifiers.ctrl || (!mac && modifiers.command),
            ..modifiers
        };
        Event::Key { key, physical_key: physical.or(Some(key)), pressed: true, repeat: false, modifiers }
    }

    fn take(shortcuts: &Shortcuts, events: Vec<Event>) -> (Vec<Action>, usize) {
        let ctx = eframe::egui::Context::default();
        // As the app: ⌘+ and ⌘− are its shortcuts, not egui's zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut actions = Vec::new();
        let mut left = 0;
        let input = eframe::egui::RawInput { events, ..Default::default() };
        let _ = ctx.run(input, |ctx| {
            actions = ctx.input_mut(|i| shortcuts.take(i));
            left = ctx.input(|i| i.events.len());
        });
        (actions, left)
    }

    #[test]
    fn actions_are_in_order_and_named_once() {
        for (index, action) in Action::ALL.into_iter().enumerate() {
            assert_eq!(action as usize, index);
            assert_eq!(Action::ALL.iter().filter(|a| a.id() == action.id()).count(), 1);
            assert!(check(action.default_keys()).is_ok(), "{action:?}");
        }
        assert!(Shortcuts::default().conflicts().is_empty());
        assert!(!Shortcuts::default().customized());
    }

    #[test]
    fn keys_survive_their_text() {
        for action in Action::ALL {
            let keys = action.default_keys();
            assert_eq!(from_text(&to_text(keys)), Some(keys), "{action:?}");
        }
        assert_eq!(to_text(Action::SaveAs.default_keys()), "Shift+Cmd+S");
        assert_eq!(to_text(Action::ZoomIn.default_keys()), "Cmd+Plus");
        assert_eq!(to_text(Action::Render.default_keys()), "F5");
        assert_eq!(from_text("shift + cmd + S"), Some(Action::SaveAs.default_keys()));
        assert_eq!(from_text("Alt+Option+F9"), Some(KeyboardShortcut::new(Modifiers::ALT, Key::F9)));
        assert_eq!(from_text("Shift+Cmd+}"), Some(Action::NextTab.default_keys()), "a curly bracket is a bracket");
        for broken in ["", "Cmd+", "Hyper+S", "Cmd+Nope", "Cmd+S+"] {
            assert_eq!(from_text(broken), None, "{broken:?}");
        }
        let control = from_text("Ctrl+K").unwrap().modifiers;
        if cfg!(target_os = "macos") {
            assert!(control.ctrl && !control.command, "macOS's Control key");
        } else {
            assert!(control.command && !control.ctrl, "Ctrl, which is Cmd elsewhere");
        }
    }

    #[test]
    fn settings_keep_only_changed_keys() {
        let mut shortcuts = Shortcuts::default();
        let mut text = String::new();
        shortcuts.write(&mut text);
        assert_eq!(text, "");
        shortcuts.set(Action::Render, KeyboardShortcut::new(Modifiers::COMMAND, Key::R));
        shortcuts.set(Action::NextTab, KeyboardShortcut::new(ALT_COMMAND, Key::ArrowRight));
        shortcuts.write(&mut text);
        assert_eq!(text, "shortcut-next-tab = Alt+Cmd+Right\nshortcut-render = Cmd+R\n", "in the page's order");

        let mut read = Shortcuts::default();
        assert!(read.read("render", "Cmd+R"));
        assert!(read.read("next-tab", "Alt+Cmd+ArrowRight"), "known, though refused");
        assert!(!read.read("teleport", "Cmd+T"));
        assert!(read.read("save", "S"), "known, though it would type text");
        assert!(read.read("open", "Cmd+C"), "known, though it is Copy");
        assert_eq!(read.get(Action::Render), KeyboardShortcut::new(Modifiers::COMMAND, Key::R));
        assert!(read.is_default(Action::NextTab) && read.is_default(Action::Save) && read.is_default(Action::Open));
        read.reset(Action::Render);
        assert_eq!(read, Shortcuts::default());
    }

    #[test]
    fn recording_refuses_typing_and_editing_keys() {
        assert_eq!(record(Modifiers::COMMAND, Key::R), Ok(KeyboardShortcut::new(Modifiers::COMMAND, Key::R)));
        assert_eq!(record(Modifiers::NONE, Key::F9), Ok(KeyboardShortcut::new(Modifiers::NONE, Key::F9)));
        assert_eq!(record(SHIFT_COMMAND, Key::CloseCurlyBracket), Ok(Action::NextTab.default_keys()));
        let mac_press = Modifiers { mac_cmd: true, ..Modifiers::COMMAND };
        assert_eq!(record(mac_press, Key::K), Ok(KeyboardShortcut::new(Modifiers::COMMAND, Key::K)));
        assert!(record(Modifiers::NONE, Key::K).unwrap_err().contains("type text"));
        assert!(record(Modifiers::SHIFT, Key::K).is_err());
        assert!(record(Modifiers::COMMAND, Key::ArrowLeft).unwrap_err().contains("editor"));
        assert!(record(Modifiers::COMMAND, Key::C).unwrap_err().contains("Copy"));
        assert!(record(SHIFT_COMMAND, Key::Z).unwrap_err().contains("Redo"));
    }

    #[test]
    fn shared_keys_are_conflicts() {
        let mut shortcuts = Shortcuts::default();
        let f5 = Action::Render.default_keys();
        shortcuts.set(Action::Export, f5);
        shortcuts.set(Action::Settings, f5);
        shortcuts.set(Action::Find, Action::Open.default_keys());
        assert_eq!(
            shortcuts.conflicts(),
            [vec![Action::Open, Action::Find], vec![Action::Render, Action::Export, Action::Settings]]
        );
        assert_eq!(shortcuts.sharing(Action::Export), [Action::Render, Action::Settings]);
        assert!(shortcuts.sharing(Action::Save).is_empty());
        // The first of a group runs.
        let (actions, _) = take(&shortcuts, vec![press(Key::F5, None, Modifiers::NONE)]);
        assert_eq!(actions, [Action::Render]);
    }

    #[test]
    fn a_press_does_the_most_specific_action() {
        let shortcuts = Shortcuts::default();
        let presses = vec![
            press(Key::S, None, SHIFT_COMMAND),
            press(Key::S, None, Modifiers::COMMAND),
            press(Key::F, None, ALT_COMMAND),
            press(Key::G, None, SHIFT_COMMAND),
            // ⌘= and ⇧⌘= (which types `+`) both make the interface larger.
            press(Key::Equals, None, Modifiers::COMMAND),
            press(Key::Plus, Some(Key::Equals), SHIFT_COMMAND),
            press(Key::CloseCurlyBracket, Some(Key::CloseBracket), SHIFT_COMMAND),
            // A layout that types `[` with Alt, on the key of `5`.
            press(Key::OpenBracket, Some(Key::Num5), Modifiers { alt: true, ..SHIFT_COMMAND }),
            press(Key::F7, None, Modifiers::NONE),
        ];
        let (actions, left) = take(&shortcuts, presses);
        use Action::*;
        assert_eq!(actions, [SaveAs, Save, FindReplace, FindPrevious, ZoomIn, ZoomIn, NextTab, PreviousTab, Export]);
        assert_eq!(left, 0, "all taken");

        let typing = vec![press(Key::S, None, Modifiers::NONE), press(Key::K, None, Modifiers::COMMAND)];
        let (actions, left) = take(&shortcuts, typing);
        assert!(actions.is_empty());
        assert_eq!(left, 2, "left to the editor");
    }

    #[test]
    fn signs_show_as_signs() {
        let ctx = eframe::egui::Context::default();
        // The fonts tell whether ⌘ can show: they come with a frame.
        let _ = ctx.run(Default::default(), |ctx| {
            let next = describe(ctx, &Action::NextTab.default_keys());
            assert!(next.ends_with(']') && !next.contains("Bracket"), "{next}");
            assert!(describe(ctx, &Action::ZoomIn.default_keys()).ends_with("Plus"));
            assert!(describe(ctx, &Action::Render.default_keys()).ends_with("F5"));
        });
    }

    #[test]
    fn a_sign_is_found_at_its_place() {
        let mut shortcuts = Shortcuts::default();
        shortcuts.set(Action::Render, KeyboardShortcut::new(Modifiers::COMMAND, Key::Semicolon));
        shortcuts.set(Action::Export, KeyboardShortcut::new(Modifiers::COMMAND, Key::E));
        // `;` typed on another key, and `E` at the place of `;`: a letter
        // is the one typed, a sign also the one at its place.
        let (actions, _) = take(&shortcuts, vec![press(Key::M, Some(Key::Semicolon), Modifiers::COMMAND)]);
        assert_eq!(actions, [Action::Render]);
        let (actions, _) = take(&shortcuts, vec![press(Key::Q, Some(Key::E), Modifiers::COMMAND)]);
        assert!(actions.is_empty());
        // A key bound for itself wins over a sign at its place.
        shortcuts.set(Action::Find, KeyboardShortcut::new(Modifiers::COMMAND, Key::M));
        let (actions, _) = take(&shortcuts, vec![press(Key::M, Some(Key::Semicolon), Modifiers::COMMAND)]);
        assert_eq!(actions, [Action::Find]);
    }
}
