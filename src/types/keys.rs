//! Keyboard shortcuts as text ("Ctrl+Shift+F8", "Alt+M"): parsing, normalising, and a platform-neutral description
//! that each backend maps to its own key codes.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A key, independent of any platform's key codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    /// A letter, upper case.
    Letter(char),
    /// A digit of the main row.
    Digit(u8),
    /// `F1` to `F24`.
    Function(u8),
    /// A digit of the numeric keypad.
    Numpad(u8),
    /// Another key: the operators of the keypad, the punctuation keys, the locks.
    Extra(Extra),
    Space,
    Enter,
    Tab,
    Escape,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

/// The keys that are neither a letter, a digit, a function key nor one of the named ones: the operators of the numeric keypad, the keys of
/// punctuation (the same place on every keyboard, whatever they write: `Semicolon` is the key right of L on a US keyboard, the one of M on a
/// French one) and the locks. Their names are the ones of the web's `KeyboardEvent.code` (the keypad's without the `pad`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extra {
    NumAdd,
    NumSubtract,
    NumMultiply,
    NumDivide,
    NumDecimal,
    Backquote,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    IntlBackslash,
    CapsLock,
    PrintScreen,
    ScrollLock,
    Pause,
    ContextMenu,
}

impl Extra {
    pub const ALL: [(&'static str, Extra); 22] = [
        ("NumAdd", Extra::NumAdd),
        ("NumSubtract", Extra::NumSubtract),
        ("NumMultiply", Extra::NumMultiply),
        ("NumDivide", Extra::NumDivide),
        ("NumDecimal", Extra::NumDecimal),
        ("Backquote", Extra::Backquote),
        ("Minus", Extra::Minus),
        ("Equal", Extra::Equal),
        ("BracketLeft", Extra::BracketLeft),
        ("BracketRight", Extra::BracketRight),
        ("Backslash", Extra::Backslash),
        ("Semicolon", Extra::Semicolon),
        ("Quote", Extra::Quote),
        ("Comma", Extra::Comma),
        ("Period", Extra::Period),
        ("Slash", Extra::Slash),
        ("IntlBackslash", Extra::IntlBackslash),
        ("CapsLock", Extra::CapsLock),
        ("PrintScreen", Extra::PrintScreen),
        ("ScrollLock", Extra::ScrollLock),
        ("Pause", Extra::Pause),
        ("ContextMenu", Extra::ContextMenu),
    ];

    /// Its name, as it is written in a shortcut.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, e)| *e == self)
            .map(|(n, _)| *n)
            .expect("every key is in the table")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Windows / Command / Super key.
    pub meta: bool,
}

/// A shortcut: modifiers and one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub modifiers: Modifiers,
    pub key: Key,
}

const NAMED: &[(&str, Key)] = &[
    ("space", Key::Space),
    ("enter", Key::Enter),
    ("return", Key::Enter),
    ("tab", Key::Tab),
    ("escape", Key::Escape),
    ("esc", Key::Escape),
    ("backspace", Key::Backspace),
    ("insert", Key::Insert),
    ("ins", Key::Insert),
    ("delete", Key::Delete),
    ("del", Key::Delete),
    ("home", Key::Home),
    ("end", Key::End),
    ("pageup", Key::PageUp),
    ("pgup", Key::PageUp),
    ("pagedown", Key::PageDown),
    ("pgdn", Key::PageDown),
    ("up", Key::Up),
    ("down", Key::Down),
    ("left", Key::Left),
    ("right", Key::Right),
];

impl KeyCombo {
    /// Parses `"Ctrl+Shift+F8"`: modifiers (`Ctrl`/`Control`, `Alt`, `Shift`, `Win`/`Meta`/`Super`/`Cmd`) then one
    /// key, separated by `+`, case-insensitive. A shortcut needs a key; a lone modifier is refused.
    pub fn parse(text: &str) -> Result<KeyCombo, String> {
        let mut modifiers = Modifiers::default();
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            if part.is_empty() {
                return Err(format!("{text:?}: empty part (write the key after the +)"));
            }
            let lower = part.to_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "win" | "meta" | "super" | "cmd" | "command" => modifiers.meta = true,
                _ => {
                    if key
                        .replace(
                            parse_key(&lower)
                                .ok_or_else(|| format!("{part:?} is not a key I know"))?,
                        )
                        .is_some()
                    {
                        return Err(format!("{text:?}: only one key besides the modifiers"));
                    }
                }
            }
        }
        let key = key.ok_or_else(|| format!("{text:?}: no key (only modifiers)"))?;
        Ok(KeyCombo { modifiers, key })
    }
}

fn parse_key(lower: &str) -> Option<Key> {
    if let Some((_, k)) = NAMED.iter().find(|(n, _)| *n == lower) {
        return Some(*k);
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=24).contains(&n).then_some(Key::Function(n));
    }
    if let Some(n) = lower
        .strip_prefix("numpad")
        .or_else(|| lower.strip_prefix("num"))
        .and_then(|n| n.parse::<u8>().ok())
    {
        return (n <= 9).then_some(Key::Numpad(n));
    }
    if let Some((_, e)) = Extra::ALL.iter().find(|(n, _)| n.to_lowercase() == lower) {
        return Some(Key::Extra(*e));
    }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(Key::Letter(c.to_ascii_uppercase())),
        (Some(c), None) if c.is_ascii_digit() => Some(Key::Digit(c as u8 - b'0')),
        _ => None,
    }
}

impl fmt::Display for KeyCombo {
    /// The normal form: `Ctrl+Alt+Shift+Win+Key`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.meta, "Win"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
            Key::Function(n) => write!(f, "F{n}"),
            Key::Numpad(n) => write!(f, "Num{n}"),
            Key::Extra(e) => f.write_str(e.name()),
            Key::Space => f.write_str("Space"),
            Key::Enter => f.write_str("Enter"),
            Key::Tab => f.write_str("Tab"),
            Key::Escape => f.write_str("Escape"),
            Key::Backspace => f.write_str("Backspace"),
            Key::Insert => f.write_str("Insert"),
            Key::Delete => f.write_str("Delete"),
            Key::Home => f.write_str("Home"),
            Key::End => f.write_str("End"),
            Key::PageUp => f.write_str("PageUp"),
            Key::PageDown => f.write_str("PageDown"),
            Key::Up => f.write_str("Up"),
            Key::Down => f.write_str("Down"),
            Key::Left => f.write_str("Left"),
            Key::Right => f.write_str("Right"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_keys_in_any_case() {
        let c = KeyCombo::parse("ctrl + SHIFT+f8").unwrap();
        assert!(c.modifiers.ctrl && c.modifiers.shift && !c.modifiers.alt && !c.modifiers.meta);
        assert_eq!(c.key, Key::Function(8));
        assert_eq!(KeyCombo::parse("a").unwrap().key, Key::Letter('A'));
        assert_eq!(KeyCombo::parse("7").unwrap().key, Key::Digit(7));
        assert_eq!(KeyCombo::parse("Num3").unwrap().key, Key::Numpad(3));
        assert_eq!(
            KeyCombo::parse("Numpad5").unwrap().key,
            Key::Numpad(5),
            "the long name works too"
        );
        assert_eq!(KeyCombo::parse("Win+Space").unwrap().key, Key::Space);
        assert_eq!(KeyCombo::parse("F24").unwrap().key, Key::Function(24));
    }

    #[test]
    fn the_other_keys_have_names_and_round_trip() {
        for (name, extra) in Extra::ALL {
            let c = KeyCombo::parse(&format!("ctrl+{}", name.to_uppercase())).unwrap();
            assert_eq!(c.key, Key::Extra(extra), "{name}");
            assert_eq!(c.to_string(), format!("Ctrl+{name}"));
        }
        assert_eq!(
            KeyCombo::parse("NumAdd").unwrap().key,
            Key::Extra(Extra::NumAdd)
        );
    }

    #[test]
    fn the_normal_form_is_stable() {
        for (input, normal) in [
            ("shift+ctrl+f8", "Ctrl+Shift+F8"),
            ("alt+m", "Alt+M"),
            ("esc", "Escape"),
            ("pgup", "PageUp"),
            ("win+num0", "Win+Num0"),
        ] {
            let c = KeyCombo::parse(input).unwrap();
            assert_eq!(c.to_string(), normal, "{input}");
            assert_eq!(
                KeyCombo::parse(normal).unwrap(),
                c,
                "round trip of {normal}"
            );
        }
    }

    #[test]
    fn bad_shortcuts_say_why() {
        for bad in [
            "", "Ctrl", "Ctrl+", "A+B", "F25", "F0", "Num10", "Hyper+A", "++",
        ] {
            assert!(KeyCombo::parse(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(KeyCombo::parse("Ctrl+Shift")
            .unwrap_err()
            .contains("no key"));
        assert!(KeyCombo::parse("A+B").unwrap_err().contains("one key"));
    }
}
