//! ホットキーの組み合わせ(SPEC.md §4.5.2)。
//!
//! 修飾キー(Ctrl / Alt / Shift / Win)と 1 つのキー(VK)の組。本アプリの設定ファイルには
//! `Ctrl+Alt+1` のような文字列で書く。キーの名前は [`keys::name`] の表示名(英語)。
//!
//! 受け付けるキーは、ウィンドウで押して取り込める([`Hotkey::from_egui`])もの([`KEYS`])に限る。
//! 修飾キーを 1 つ以上含むものだけを有効とする([`Hotkey::validate`])。

use eframe::egui;

use crate::config::KeyCode;
use crate::keys;

/// ホットキーにできるキーの VK と、egui で押されたときのキー。
///
/// egui はテンキーの数字をメインの数字と区別しないので、テンキーは含めない。
const KEYS: &[(u32, egui::Key)] = {
    use egui::Key::*;
    &[
        (0x08, Backspace),
        (0x09, Tab),
        (0x0D, Enter),
        (0x1B, Escape),
        (0x20, Space),
        (0x21, PageUp),
        (0x22, PageDown),
        (0x23, End),
        (0x24, Home),
        (0x25, ArrowLeft),
        (0x26, ArrowUp),
        (0x27, ArrowRight),
        (0x28, ArrowDown),
        (0x2D, Insert),
        (0x2E, Delete),
        (0x30, Num0),
        (0x31, Num1),
        (0x32, Num2),
        (0x33, Num3),
        (0x34, Num4),
        (0x35, Num5),
        (0x36, Num6),
        (0x37, Num7),
        (0x38, Num8),
        (0x39, Num9),
        (0x41, A),
        (0x42, B),
        (0x43, C),
        (0x44, D),
        (0x45, E),
        (0x46, F),
        (0x47, G),
        (0x48, H),
        (0x49, I),
        (0x4A, J),
        (0x4B, K),
        (0x4C, L),
        (0x4D, M),
        (0x4E, N),
        (0x4F, O),
        (0x50, P),
        (0x51, Q),
        (0x52, R),
        (0x53, S),
        (0x54, T),
        (0x55, U),
        (0x56, V),
        (0x57, W),
        (0x58, X),
        (0x59, Y),
        (0x5A, Z),
        (0x70, F1),
        (0x71, F2),
        (0x72, F3),
        (0x73, F4),
        (0x74, F5),
        (0x75, F6),
        (0x76, F7),
        (0x77, F8),
        (0x78, F9),
        (0x79, F10),
        (0x7A, F11),
        (0x7B, F12),
        (0x7C, F13),
        (0x7D, F14),
        (0x7E, F15),
        (0x7F, F16),
        (0x80, F17),
        (0x81, F18),
        (0x82, F19),
        (0x83, F20),
        (0x84, F21),
        (0x85, F22),
        (0x86, F23),
        (0x87, F24),
        (0xBA, Semicolon),
        (0xBB, Equals),
        (0xBC, Comma),
        (0xBD, Minus),
        (0xBE, Period),
        (0xBF, Slash),
        (0xC0, Backtick),
        (0xDB, OpenBracket),
        (0xDC, Backslash),
        (0xDD, CloseBracket),
        (0xDE, Quote),
    ]
};

/// 修飾キーの文字列での名前(書く順)。
const CTRL: &str = "Ctrl";
const ALT: &str = "Alt";
const SHIFT: &str = "Shift";
const WIN: &str = "Win";

/// ホットキーの組み合わせ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// 修飾キー以外のキー(VK)
    pub vk: u32,
}

/// 受け付けない組み合わせの理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    /// 修飾キーを含まない
    NoModifier,
    /// ホットキーにできないキー
    UnsupportedKey,
}

impl Hotkey {
    /// 修飾キーを 1 つ以上含むか。
    pub fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }

    /// 受け付けるか(修飾キーを 1 つ以上含み、キーが [`KEYS`] にある)。
    pub fn validate(&self) -> Result<(), Invalid> {
        if !KEYS.iter().any(|(vk, _)| *vk == self.vk) {
            Err(Invalid::UnsupportedKey)
        } else if !self.has_modifier() {
            Err(Invalid::NoModifier)
        } else {
            Ok(())
        }
    }

    /// 設定ファイルと画面に出す文字列(例: `Ctrl+Alt+1`)。修飾キーは Ctrl・Alt・Shift・Win の順。
    pub fn format(&self) -> String {
        let mut parts: Vec<String> = [
            (self.ctrl, CTRL),
            (self.alt, ALT),
            (self.shift, SHIFT),
            (self.win, WIN),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| (*name).to_owned())
        .collect();
        parts.push(keys::name(KeyCode(self.vk)));
        parts.join("+")
    }

    /// [`Hotkey::format`] の文字列を読む。修飾キーの順と大文字小文字は問わない。
    /// 受け付けない組み合わせ([`Hotkey::validate`])や読めないものは `None`。
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut h = Hotkey::default();
        let mut rest = s.trim();
        // キーの名前に `+` を含むもの(`Num +` など)もあるので、先頭の修飾キーを順に外す
        while let Some((head, tail)) = rest.split_once('+') {
            let flag = match head.trim().to_ascii_lowercase().as_str() {
                "ctrl" => &mut h.ctrl,
                "alt" => &mut h.alt,
                "shift" => &mut h.shift,
                "win" => &mut h.win,
                _ => break,
            };
            if *flag {
                return None;
            }
            *flag = true;
            rest = tail;
        }
        let rest = rest.trim();
        let vk = KEYS
            .iter()
            .map(|(vk, _)| *vk)
            .find(|&vk| keys::name(KeyCode(vk)).eq_ignore_ascii_case(rest))?;
        h.vk = vk;
        h.validate().ok().map(|()| h)
    }

    /// egui で押されたキーと修飾キーから作る。ホットキーにできないキーなら `None`。
    ///
    /// Win キーは egui の修飾キーに含まれない(Windows では Win との組み合わせの多くをシェルが
    /// 先に取る)ので、`win` は `false` にする。呼び出し側で別に指定する。
    pub fn from_egui(key: egui::Key, modifiers: egui::Modifiers) -> Option<Hotkey> {
        let vk = KEYS.iter().find(|(_, k)| *k == key).map(|(vk, _)| *vk)?;
        Some(Hotkey {
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            win: false,
            vk,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u32) -> Hotkey {
        Hotkey {
            ctrl,
            alt,
            shift,
            win,
            vk,
        }
    }

    #[test]
    fn format_and_parse_round_trip() {
        let all = hk(true, true, true, true, 0x31);
        assert_eq!(all.format(), "Ctrl+Alt+Shift+Win+1");
        assert_eq!(Hotkey::parse("Ctrl+Alt+Shift+Win+1"), Some(all));
        for (vk, _) in KEYS {
            let h = hk(true, false, false, false, *vk);
            assert_eq!(Hotkey::parse(&h.format()), Some(h), "{}", h.format());
        }
        assert_eq!(hk(false, true, false, false, 0x21).format(), "Alt+Page Up");
        assert_eq!(hk(true, false, false, false, 0xBD).format(), "Ctrl+-");
    }

    #[test]
    fn parse_is_lenient_about_order_case_and_spaces() {
        let h = hk(true, true, false, false, 0x4F);
        assert_eq!(Hotkey::parse("alt + CTRL + o"), Some(h));
        assert_eq!(
            Hotkey::parse("Shift+Page Down"),
            Some(hk(false, false, true, false, 0x22))
        );
    }

    #[test]
    fn parse_rejects_invalid() {
        // 修飾キーがない
        assert_eq!(Hotkey::parse("A"), None);
        // 修飾キーだけ
        assert_eq!(Hotkey::parse("Ctrl+Alt"), None);
        // 同じ修飾キーが 2 回
        assert_eq!(Hotkey::parse("Ctrl+Ctrl+A"), None);
        // 知らないキー・扱わないキー(テンキー)
        assert_eq!(Hotkey::parse("Ctrl+Nope"), None);
        assert_eq!(Hotkey::parse("Ctrl+Num 1"), None);
        assert_eq!(Hotkey::parse(""), None);
    }

    #[test]
    fn validate_requires_modifier_and_supported_key() {
        assert_eq!(
            hk(false, false, false, false, 0x41).validate(),
            Err(Invalid::NoModifier)
        );
        assert_eq!(hk(false, false, false, true, 0x41).validate(), Ok(()));
        assert_eq!(hk(false, false, true, false, 0x70).validate(), Ok(()));
        // 修飾キーそのものやテンキーは受け付けない
        assert_eq!(
            hk(true, false, false, false, 0x11).validate(),
            Err(Invalid::UnsupportedKey)
        );
        assert_eq!(
            hk(true, false, false, false, 0x60).validate(),
            Err(Invalid::UnsupportedKey)
        );
    }

    #[test]
    fn from_egui_maps_keys_and_modifiers() {
        let m = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            Hotkey::from_egui(egui::Key::Num1, m),
            Some(hk(true, true, false, false, 0x31))
        );
        assert_eq!(
            Hotkey::from_egui(egui::Key::F5, egui::Modifiers::SHIFT),
            Some(hk(false, false, true, false, 0x74))
        );
        // 修飾キーなしでも作る(受け付けるかは validate で決める)
        let bare = Hotkey::from_egui(egui::Key::A, egui::Modifiers::NONE).unwrap();
        assert_eq!(bare.validate(), Err(Invalid::NoModifier));
        // 扱わないキー
        assert_eq!(Hotkey::from_egui(egui::Key::Copy, m), None);
    }

    #[test]
    fn key_names_are_unique() {
        let mut names: Vec<String> = KEYS
            .iter()
            .map(|(vk, _)| keys::name(KeyCode(*vk)).to_lowercase())
            .collect();
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len);
    }
}
