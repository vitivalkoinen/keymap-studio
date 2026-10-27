//! 仮想キーコード(VK)と表示名の対応。
//!
//! 最上位ビットは PowerToys の「テンキー由来」フラグ([`NUMPAD_ORIGIN_FLAG`])。
//! フラグの有無で指すキーが変わる VK(ナビゲーション系と Enter)は [`FLAG_DEPENDENT`] で
//! 区別する。それ以外の VK はフラグを無視して同じキーとみなす。
//! 表示名はローカライズの対象外で、常に英語。
//!
//! [`NUMPAD_ORIGIN_FLAG`]: crate::config::NUMPAD_ORIGIN_FLAG

use crate::config::KeyCode;

/// テンキー由来フラグの有無で別のキーになる VK。`(VK, フラグなしの名前, フラグありの名前)`。
///
/// フラグなしはナビゲーションクラスタ(とメインの Enter)、フラグありは
/// NumLock オフ時のテンキー(とテンキーの Enter)。
const FLAG_DEPENDENT: &[(u32, &str, &str)] = &[
    (0x0D, "Enter", "Num Enter"),
    (0x21, "Page Up", "Num Page Up"),
    (0x22, "Page Down", "Num Page Down"),
    (0x23, "End", "Num End"),
    (0x24, "Home", "Num Home"),
    (0x25, "Left", "Num Left"),
    (0x26, "Up", "Num Up"),
    (0x27, "Right", "Num Right"),
    (0x28, "Down", "Num Down"),
    (0x2D, "Insert", "Num Insert"),
    (0x2E, "Delete", "Num Delete"),
];

/// テンキー由来フラグに関係なく同じキーを指す VK と表示名。
///
/// テンキーの /(0x6F)はフラグが付きうるが、どちらでもテンキーの / なのでここに置く。
const NAMES: &[(u32, &str)] = &[
    // マウスボタン
    (0x01, "Left Button"),
    (0x02, "Right Button"),
    (0x04, "Middle Button"),
    (0x05, "X Button 1"),
    (0x06, "X Button 2"),
    // 制御キー
    (0x03, "Break"),
    (0x08, "Backspace"),
    (0x09, "Tab"),
    (0x0C, "Clear"),
    (0x10, "Shift"),
    (0x11, "Ctrl"),
    (0x12, "Alt"),
    (0x13, "Pause"),
    (0x14, "Caps Lock"),
    (0x15, "Kana"),
    (0x19, "Kanji"),
    (0x1B, "Esc"),
    (0x1C, "Convert"),
    (0x1D, "Non Convert"),
    (0x20, "Space"),
    (0x2C, "Print Screen"),
    // 数字
    (0x30, "0"),
    (0x31, "1"),
    (0x32, "2"),
    (0x33, "3"),
    (0x34, "4"),
    (0x35, "5"),
    (0x36, "6"),
    (0x37, "7"),
    (0x38, "8"),
    (0x39, "9"),
    // 英字
    (0x41, "A"),
    (0x42, "B"),
    (0x43, "C"),
    (0x44, "D"),
    (0x45, "E"),
    (0x46, "F"),
    (0x47, "G"),
    (0x48, "H"),
    (0x49, "I"),
    (0x4A, "J"),
    (0x4B, "K"),
    (0x4C, "L"),
    (0x4D, "M"),
    (0x4E, "N"),
    (0x4F, "O"),
    (0x50, "P"),
    (0x51, "Q"),
    (0x52, "R"),
    (0x53, "S"),
    (0x54, "T"),
    (0x55, "U"),
    (0x56, "V"),
    (0x57, "W"),
    (0x58, "X"),
    (0x59, "Y"),
    (0x5A, "Z"),
    // Windows キー
    (0x5B, "Left Win"),
    (0x5C, "Right Win"),
    (0x5D, "Menu"),
    (0x5F, "Sleep"),
    // テンキー
    (0x60, "Num 0"),
    (0x61, "Num 1"),
    (0x62, "Num 2"),
    (0x63, "Num 3"),
    (0x64, "Num 4"),
    (0x65, "Num 5"),
    (0x66, "Num 6"),
    (0x67, "Num 7"),
    (0x68, "Num 8"),
    (0x69, "Num 9"),
    (0x6A, "Num *"),
    (0x6B, "Num +"),
    (0x6C, "Num Separator"),
    (0x6D, "Num -"),
    (0x6E, "Num ."),
    (0x6F, "Num /"),
    // ファンクションキー
    (0x70, "F1"),
    (0x71, "F2"),
    (0x72, "F3"),
    (0x73, "F4"),
    (0x74, "F5"),
    (0x75, "F6"),
    (0x76, "F7"),
    (0x77, "F8"),
    (0x78, "F9"),
    (0x79, "F10"),
    (0x7A, "F11"),
    (0x7B, "F12"),
    (0x7C, "F13"),
    (0x7D, "F14"),
    (0x7E, "F15"),
    (0x7F, "F16"),
    (0x80, "F17"),
    (0x81, "F18"),
    (0x82, "F19"),
    (0x83, "F20"),
    (0x84, "F21"),
    (0x85, "F22"),
    (0x86, "F23"),
    (0x87, "F24"),
    // ロックキー
    (0x90, "Num Lock"),
    (0x91, "Scroll Lock"),
    // 左右を区別する修飾キー
    (0xA0, "Left Shift"),
    (0xA1, "Right Shift"),
    (0xA2, "Left Ctrl"),
    (0xA3, "Right Ctrl"),
    (0xA4, "Left Alt"),
    (0xA5, "Right Alt"),
    // ブラウザ・メディアキー
    (0xA6, "Browser Back"),
    (0xA7, "Browser Forward"),
    (0xA8, "Browser Refresh"),
    (0xA9, "Browser Stop"),
    (0xAA, "Browser Search"),
    (0xAB, "Browser Favorites"),
    (0xAC, "Browser Home"),
    (0xAD, "Volume Mute"),
    (0xAE, "Volume Down"),
    (0xAF, "Volume Up"),
    (0xB0, "Next Track"),
    (0xB1, "Previous Track"),
    (0xB2, "Stop Media"),
    (0xB3, "Play/Pause"),
    (0xB4, "Mail"),
    (0xB5, "Select Media"),
    (0xB6, "Launch App 1"),
    (0xB7, "Launch App 2"),
    // 記号キー(US 配列での刻印)
    (0xBA, ";"),
    (0xBB, "="),
    (0xBC, ","),
    (0xBD, "-"),
    (0xBE, "."),
    (0xBF, "/"),
    (0xC0, "`"),
    (0xDB, "["),
    (0xDC, "\\"),
    (0xDD, "]"),
    (0xDE, "'"),
    (0xE2, "OEM 102"),
    // PowerToys 独自の値(shared_constants.h)
    (0x100, "Disable"),
    (0x104, "Win"),
];

fn flag_dependent(vk: u32) -> Option<(&'static str, &'static str)> {
    FLAG_DEPENDENT
        .iter()
        .find(|(v, _, _)| *v == vk)
        .map(|(_, plain, numpad)| (*plain, *numpad))
}

fn plain_name(vk: u32) -> Option<&'static str> {
    NAMES.iter().find(|(v, _)| *v == vk).map(|(_, name)| *name)
}

/// 比較用にキーコードを正規化する。
///
/// フラグで意味が変わらない既知の VK はフラグを落とす。フラグで意味が変わる VK と、
/// 表にない VK はそのまま返す(表にない VK はフラグの意味が分からないため区別を残す)。
pub fn normalize(code: KeyCode) -> KeyCode {
    if flag_dependent(code.vk()).is_none() && plain_name(code.vk()).is_some() {
        KeyCode(code.vk())
    } else {
        code
    }
}

/// 表にあるキーの表示名。表にない場合は `None`。
pub fn lookup(code: KeyCode) -> Option<&'static str> {
    let vk = code.vk();
    match flag_dependent(vk) {
        Some((plain, numpad)) => Some(if code.is_numpad_origin() {
            numpad
        } else {
            plain
        }),
        None => plain_name(vk),
    }
}

/// キーの表示名。表にない VK は `VK 0x..` の 16 進表記にする。
pub fn name(code: KeyCode) -> String {
    match lookup(code) {
        Some(n) => n.to_owned(),
        None => format_hex(code),
    }
}

/// VK の 16 進表記。テンキー由来フラグがあれば ` (numpad)` を付ける。
pub fn format_hex(code: KeyCode) -> String {
    let suffix = if code.is_numpad_origin() {
        " (numpad)"
    } else {
        ""
    };
    format!("VK 0x{:02X}{suffix}", code.vk())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NUMPAD_ORIGIN_FLAG;

    fn np(vk: u32) -> KeyCode {
        KeyCode(vk | NUMPAD_ORIGIN_FLAG)
    }

    #[test]
    fn distinguishes_numpad_origin_navigation_keys() {
        assert_eq!(name(KeyCode(0x25)), "Left");
        assert_eq!(name(np(0x25)), "Num Left");
        assert_eq!(name(KeyCode(0x2D)), "Insert");
        assert_eq!(name(np(0x2D)), "Num Insert");
        assert_eq!(name(KeyCode(0x2E)), "Delete");
        assert_eq!(name(np(0x2E)), "Num Delete");
        assert_eq!(name(KeyCode(0x24)), "Home");
        assert_eq!(name(KeyCode(0x23)), "End");
        assert_eq!(name(KeyCode(0x21)), "Page Up");
        assert_eq!(name(KeyCode(0x22)), "Page Down");
        assert_eq!(name(np(0x22)), "Num Page Down");
    }

    #[test]
    fn distinguishes_numpad_enter() {
        assert_eq!(name(KeyCode(0x0D)), "Enter");
        assert_eq!(name(np(0x0D)), "Num Enter");
    }

    #[test]
    fn flag_independent_keys_ignore_flag() {
        assert_eq!(name(KeyCode(0xA3)), "Right Ctrl");
        assert_eq!(name(KeyCode(0xA5)), "Right Alt");
        assert_eq!(name(KeyCode(0x6F)), "Num /");
        assert_eq!(name(np(0x6F)), "Num /");
        assert_eq!(name(KeyCode(0x41)), "A");
        assert_eq!(normalize(np(0x6F)), KeyCode(0x6F));
        assert_eq!(normalize(np(0x5B)), KeyCode(0x5B));
        assert_eq!(normalize(np(0x25)), np(0x25));
        assert_eq!(normalize(KeyCode(0x25)), KeyCode(0x25));
    }

    #[test]
    fn powertoys_special_values() {
        assert_eq!(name(KeyCode(0x100)), "Disable");
        assert_eq!(name(KeyCode(0x104)), "Win");
        assert_eq!(name(KeyCode(0x6C)), "Num Separator");
    }

    #[test]
    fn unknown_vk_falls_back_to_hex() {
        assert_eq!(lookup(KeyCode(0xFF)), None);
        assert_eq!(name(KeyCode(0xFF)), "VK 0xFF");
        assert_eq!(name(np(0xFF)), "VK 0xFF (numpad)");
        assert_eq!(name(KeyCode(0x1FF)), "VK 0x1FF");
        // 表にない VK はフラグを残す
        assert_eq!(normalize(np(0xFF)), np(0xFF));
    }

    #[test]
    fn tables_have_no_duplicate_vk() {
        let mut seen = std::collections::HashSet::new();
        for vk in NAMES
            .iter()
            .map(|(v, _)| *v)
            .chain(FLAG_DEPENDENT.iter().map(|(v, _, _)| *v))
        {
            assert!(seen.insert(vk), "duplicate VK 0x{vk:02X}");
        }
    }
}
