//! Горячие клавиши. Платформенно-независимая таблица; перевод событий клавиатуры tao в `Chord` — шаг 3/4.

use crate::command::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Буква или цифра, буквы в нижнем регистре.
    Char(char),
    Tab,
    Left,
    Right,
    PageUp,
    PageDown,
    /// Функциональная клавиша F1..F12.
    F(u8),
    Plus,
    Minus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Chord {
    pub fn new(ctrl: bool, shift: bool, alt: bool, key: Key) -> Self {
        let key = match key {
            Key::Char(c) => Key::Char(c.to_ascii_lowercase()),
            k => k,
        };
        Self {
            ctrl,
            shift,
            alt,
            key,
        }
    }
}

/// Имя клавиши из `KeyboardEvent.key` (интерфейс браузера присылает его как есть).
pub fn key_from_name(name: &str) -> Option<Key> {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some('+'), None) => return Some(Key::Plus),
        (Some('-'), None) => return Some(Key::Minus),
        (Some(c), None) => return Some(Key::Char(c.to_ascii_lowercase())),
        _ => {}
    }
    Some(match name {
        "Tab" => Key::Tab,
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        n if n.starts_with('F') => match n[1..].parse::<u8>() {
            Ok(k @ 1..=12) => Key::F(k),
            _ => return None,
        },
        _ => return None,
    })
}

/// Код виртуальной клавиши Windows (VK_*), который WebView2 присылает в `AcceleratorKeyPressed`.
pub fn key_from_vk(vk: u32) -> Option<Key> {
    Some(match vk {
        0x09 => Key::Tab,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x25 => Key::Left,
        0x27 => Key::Right,
        0x30..=0x39 | 0x41..=0x5A => Key::Char(char::from_u32(vk)?.to_ascii_lowercase()),
        0x60..=0x69 => Key::Char(char::from_u32(vk - 0x60 + 0x30)?), // цифровой блок
        0x6B | 0xBB => Key::Plus,                                    // VK_ADD, VK_OEM_PLUS
        0x6D | 0xBD => Key::Minus,                                   // VK_SUBTRACT, VK_OEM_MINUS
        0x70..=0x7B => Key::F((vk - 0x70 + 1) as u8),
        _ => return None,
    })
}

pub fn lookup(c: Chord) -> Option<Command> {
    use Command::*;
    use Key::*;
    Some(match (c.ctrl, c.shift, c.alt, c.key) {
        (true, false, false, Char('t')) => NewTab,
        (true, true, false, Char('t')) => ReopenClosedTab,
        (true, true, false, Char('n')) => NewPrivateTab,
        (true, false, false, Char('w')) | (true, false, false, F(4)) => CloseActiveTab,
        (true, false, false, Char('l'))
        | (false, false, true, Char('d'))
        | (false, false, false, F(6)) => FocusAddressBar,
        (true, false, false, Char('r')) | (false, false, false, F(5)) => Reload,
        (true, true, false, Char('r')) | (true, false, false, F(5)) => HardReload,
        (false, false, true, Left) => Back,
        (false, false, true, Right) => Forward,
        (true, false, false, Tab) | (true, false, false, PageDown) => NextTab,
        (true, true, false, Tab) | (true, false, false, PageUp) => PrevTab,
        (true, false, false, Char('9')) => SelectLastTab,
        (true, false, false, Char(d @ '1'..='8')) => SelectTab(d as usize - '1' as usize),
        // На большинстве раскладок «+» — это Shift+«=», поэтому shift игнорируем.
        (true, _, false, Plus) | (true, _, false, Char('=')) => ZoomIn,
        (true, false, false, Minus) | (true, false, false, Char('-')) => ZoomOut,
        (true, false, false, Char('0')) => ZoomReset,
        (false, false, false, F(11)) => ToggleFullscreen,
        (true, false, false, Char('d')) => BookmarkPage,
        (true, false, false, Char('h')) => ShowHistory,
        (true, false, false, Char('j')) => ShowDownloads,
        (true, true, false, Char('o')) => ShowBookmarks,
        (true, false, false, Char('p')) => Print,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Command::*;

    fn ctrl(k: Key) -> Chord {
        Chord::new(true, false, false, k)
    }

    #[test]
    fn required_shortcuts() {
        assert_eq!(lookup(ctrl(Key::Char('t'))), Some(NewTab));
        assert_eq!(lookup(ctrl(Key::Char('w'))), Some(CloseActiveTab));
        assert_eq!(lookup(ctrl(Key::Char('l'))), Some(FocusAddressBar));
        assert_eq!(lookup(ctrl(Key::Char('r'))), Some(Reload));
        assert_eq!(
            lookup(Chord::new(true, true, false, Key::Char('t'))),
            Some(ReopenClosedTab)
        );
    }

    #[test]
    fn uppercase_letters_are_normalized() {
        assert_eq!(
            lookup(Chord::new(true, true, false, Key::Char('T'))),
            Some(ReopenClosedTab)
        );
    }

    #[test]
    fn ctrl_shift_t_is_not_new_tab() {
        assert_ne!(
            lookup(Chord::new(true, true, false, Key::Char('t'))),
            Some(NewTab)
        );
    }

    #[test]
    fn tab_selection_by_digit() {
        assert_eq!(lookup(ctrl(Key::Char('1'))), Some(SelectTab(0)));
        assert_eq!(lookup(ctrl(Key::Char('8'))), Some(SelectTab(7)));
        assert_eq!(lookup(ctrl(Key::Char('9'))), Some(SelectLastTab));
    }

    #[test]
    fn zoom_and_navigation_keys() {
        assert_eq!(lookup(ctrl(Key::Char('='))), Some(ZoomIn));
        assert_eq!(
            lookup(Chord::new(true, true, false, Key::Plus)),
            Some(ZoomIn)
        );
        assert_eq!(lookup(ctrl(Key::Minus)), Some(ZoomOut));
        assert_eq!(lookup(ctrl(Key::Char('0'))), Some(ZoomReset));
        assert_eq!(
            lookup(Chord::new(false, false, true, Key::Left)),
            Some(Back)
        );
        assert_eq!(
            lookup(Chord::new(false, false, false, Key::F(11))),
            Some(ToggleFullscreen)
        );
        assert_eq!(
            lookup(Chord::new(false, false, false, Key::F(5))),
            Some(Reload)
        );
    }

    #[test]
    fn key_names_from_the_ui() {
        assert_eq!(key_from_name("T"), Some(Key::Char('t')));
        assert_eq!(key_from_name("+"), Some(Key::Plus));
        assert_eq!(key_from_name("-"), Some(Key::Minus));
        assert_eq!(key_from_name("ArrowLeft"), Some(Key::Left));
        assert_eq!(key_from_name("F11"), Some(Key::F(11)));
        assert_eq!(key_from_name("F13"), None);
        assert_eq!(key_from_name("Enter"), None);
        assert_eq!(key_from_name("Fx"), None);
        assert_eq!(key_from_name(""), None);
    }

    #[test]
    fn virtual_keys_from_webview2() {
        assert_eq!(key_from_vk(0x54), Some(Key::Char('t')));
        assert_eq!(key_from_vk(0x31), Some(Key::Char('1')));
        assert_eq!(key_from_vk(0x61), Some(Key::Char('1')));
        assert_eq!(key_from_vk(0xBB), Some(Key::Plus));
        assert_eq!(key_from_vk(0xBD), Some(Key::Minus));
        assert_eq!(key_from_vk(0x7A), Some(Key::F(11)));
        assert_eq!(key_from_vk(0x09), Some(Key::Tab));
        assert_eq!(key_from_vk(0x0D), None);
    }

    #[test]
    fn vk_and_name_agree_on_shortcuts() {
        let c = |k| Chord::new(true, false, false, k);
        assert_eq!(
            lookup(c(key_from_vk(0x54).unwrap())),
            lookup(c(key_from_name("t").unwrap()))
        );
        assert_eq!(lookup(c(key_from_vk(0xBB).unwrap())), Some(Command::ZoomIn));
    }

    #[test]
    fn unknown_chords_are_ignored() {
        assert_eq!(
            lookup(Chord::new(false, false, false, Key::Char('t'))),
            None
        );
        assert_eq!(lookup(ctrl(Key::Char('q'))), None);
    }
}
