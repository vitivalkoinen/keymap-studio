//! 同梱のプリセット(SPEC.md §4.6)。exe に埋め込み、読み取り専用。
//!
//! 初期の同梱は大西配列(US 配列用)だけ。

use serde_json::Map;

use crate::config::{Config, InProcess, KeyCode, Remap};

/// プリセットの種類。表示名は UI の言語ごとの対応表から引く。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresetId {
    /// 大西配列
    Onishi,
}

impl PresetId {
    /// 一覧での表示順。
    pub const ALL: &[PresetId] = &[PresetId::Onishi];

    /// 本アプリの設定ファイルでプリセットを指す ID(ホットキーの割り当て。§4.5.2)。変えないこと。
    pub fn id(self) -> &'static str {
        match self {
            PresetId::Onishi => "onishi",
        }
    }

    /// [`PresetId::id`] から引く。知らない ID なら `None`。
    pub fn from_id(id: &str) -> Option<PresetId> {
        PresetId::ALL.iter().copied().find(|p| p.id() == id)
    }

    /// プリセットのリマップ定義(4 つの区分だけ)。
    pub fn definition(self) -> Config {
        match self {
            PresetId::Onishi => onishi(),
        }
    }
}

/// 大西配列の定義(出典: https://o24.works/layout/ 、2026-10-28 確認)。
/// US 配列の物理キー(QWERTY での刻印)と、そこに割り当てる文字。左から右の順。
const ONISHI_ROWS: &[(&str, &str)] = &[
    ("QWERTYUIOP", "QLU,.FWRYP"),
    ("ASDFGHJKL;", "EIAO-KTNSH"),
    ("ZXCVBNM,./", "ZXCV;GDMJB"),
];

/// US 配列の刻印の文字から VK。英字と、大西配列に出てくる記号だけを扱う。
fn vk_of(c: char) -> Option<u32> {
    match c {
        'A'..='Z' => Some(c as u32),
        ';' => Some(0xBA),
        ',' => Some(0xBC),
        '-' => Some(0xBD),
        '.' => Some(0xBE),
        '/' => Some(0xBF),
        _ => None,
    }
}

/// 大西配列をキー→キーのリマップで表したもの。物理キーと同じ文字になるキーにはリマップを作らない。
fn onishi() -> Config {
    let mut remaps = Vec::new();
    for (physical, assigned) in ONISHI_ROWS {
        for (p, a) in physical.chars().zip(assigned.chars()) {
            if p == a {
                continue;
            }
            let (Some(from), Some(to)) = (vk_of(p), vk_of(a)) else {
                unreachable!("ONISHI_ROWS has only supported characters: {p} {a}");
            };
            remaps.push(Remap {
                original_keys: KeyCode(from).0.to_string(),
                new_remap_keys: KeyCode(to).0.to_string(),
                extra: Map::new(),
            });
        }
    }
    Config {
        remap_keys: InProcess {
            in_process: remaps,
            extra: Map::new(),
        },
        ..Config::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keys, matching};

    #[test]
    fn ids_round_trip_and_are_unique() {
        for &p in PresetId::ALL {
            assert_eq!(PresetId::from_id(p.id()), Some(p));
        }
        assert_eq!(PresetId::Onishi.id(), "onishi");
        assert_eq!(PresetId::from_id("unknown"), None);
    }

    #[test]
    fn onishi_rows_are_well_formed() {
        for (physical, assigned) in ONISHI_ROWS {
            assert_eq!(physical.chars().count(), 10);
            assert_eq!(assigned.chars().count(), 10);
            assert!(
                physical
                    .chars()
                    .chain(assigned.chars())
                    .all(|c| vk_of(c).is_some())
            );
        }
    }

    #[test]
    fn onishi_remaps_match_the_spec_table() {
        let cfg = PresetId::Onishi.definition();
        let map = matching::source_map(&cfg.remap_keys.in_process);
        let by_name = |name: &str| -> KeyCode {
            keys::target_choices()
                .into_iter()
                .find(|&c| keys::name(c) == name)
                .unwrap_or_else(|| panic!("{name}"))
        };
        // (物理キー, 変換先)。SPEC.md §4.6 の表の 30 キーのうちリマップするもの
        let expected = [
            ("W", "L"),
            ("E", "U"),
            ("R", ","),
            ("T", "."),
            ("Y", "F"),
            ("U", "W"),
            ("I", "R"),
            ("O", "Y"),
            ("A", "E"),
            ("S", "I"),
            ("D", "A"),
            ("F", "O"),
            ("G", "-"),
            ("H", "K"),
            ("J", "T"),
            ("K", "N"),
            ("L", "S"),
            (";", "H"),
            ("B", ";"),
            ("N", "G"),
            ("M", "D"),
            (",", "M"),
            (".", "J"),
            ("/", "B"),
        ];
        for (from, to) in expected {
            assert_eq!(
                map.get(&by_name(from)),
                Some(&vec![vec![by_name(to)]]),
                "{from} -> {to}"
            );
        }
        // 物理キーと同じ文字になるキーにはリマップを作らない
        for same in ["Q", "P", "Z", "X", "C", "V"] {
            assert!(!map.contains_key(&by_name(same)), "{same}");
        }
        // 30 キー以外(数字の段、元の - キーなど)は変えない
        assert_eq!(cfg.remap_keys.in_process.len(), expected.len());
        for other in ["1", "0", "-", "=", "[", "]", "'", "\\", "`", "Space"] {
            assert!(!map.contains_key(&by_name(other)), "{other}");
        }
        // キー→キーのリマップだけ。ほかの区分は空
        assert!(cfg.remap_keys_to_text.in_process.is_empty());
        assert!(cfg.remap_shortcuts.global.is_empty());
        assert!(cfg.remap_shortcuts_to_text.global.is_empty());
        // 1 つの整数のリマップ元・リマップ先だけ
        for r in &cfg.remap_keys.in_process {
            assert!(r.single_original().is_some());
            assert_eq!(r.target().len(), 1);
        }
    }
}
