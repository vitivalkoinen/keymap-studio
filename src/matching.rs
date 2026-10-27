//! リマップとキーボード図の照合。UI から切り離した純粋な関数として置く。
//!
//! キーコードはすべて [`keys::normalize`] を通してから比べる。

use std::collections::HashSet;

use crate::config::{KeyCode, Remap};
use crate::keys;
use crate::layout::Layout;

/// リマップ元になっている物理キーの添字(`layout.keys` の添字)。
///
/// キーが持つコードのどれか 1 つでもリマップ元なら含める。配列にないコードは無視する。
pub fn remapped_keys(layout: &Layout, remaps: &[Remap]) -> HashSet<usize> {
    remaps
        .iter()
        .flat_map(|r| r.original())
        .filter_map(|c| layout.find(c))
        .collect()
}

/// あるコードがリマップのどちら側に出てくるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// リマップ元(このコードを別のキーへ変える)
    Source,
    /// リマップ先(別のキーがこのコードになる)
    Target,
}

/// 選択したキーに関係するリマップ 1 件。
#[derive(Debug, Clone, Copy)]
pub struct Related<'a> {
    /// 関係するコード(`codes` のうちのどれか)
    pub code: KeyCode,
    pub role: Role,
    pub remap: &'a Remap,
}

/// `codes` のいずれかに関係するリマップを、`codes` の順に並べて返す。
pub fn related_remaps<'a>(codes: &[KeyCode], remaps: &'a [Remap]) -> Vec<Related<'a>> {
    let mut out = Vec::new();
    for &code in codes {
        let code = keys::normalize(code);
        for remap in remaps {
            let hits = |list: Vec<KeyCode>| list.into_iter().any(|c| keys::normalize(c) == code);
            if hits(remap.original()) {
                out.push(Related {
                    code,
                    role: Role::Source,
                    remap,
                });
            }
            if hits(remap.target()) {
                out.push(Related {
                    code,
                    role: Role::Target,
                    remap,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// ユーザーの実際の default.json と同じ値に、ナビゲーションクラスタの ← → Disable を足したもの。
    const SAMPLE: &str = r#"{"remapKeys":{"inProcess":[
        {"originalKeys":"2147483685","newRemapKeys":"100"},
        {"originalKeys":"2147483693","newRemapKeys":"96"},
        {"originalKeys":"2147483683","newRemapKeys":"97"},
        {"originalKeys":"2147483688","newRemapKeys":"98"},
        {"originalKeys":"2147483682","newRemapKeys":"99"},
        {"originalKeys":"2147483684","newRemapKeys":"103"},
        {"originalKeys":"12","newRemapKeys":"101"},
        {"originalKeys":"2147483687","newRemapKeys":"102"},
        {"originalKeys":"2147483686","newRemapKeys":"104"},
        {"originalKeys":"2147483681","newRemapKeys":"105"},
        {"originalKeys":"2147483694","newRemapKeys":"110"},
        {"originalKeys":"37","newRemapKeys":"256"}
    ]}}"#;

    fn remaps() -> Vec<Remap> {
        let cfg: Config = serde_json::from_str(SAMPLE).unwrap();
        cfg.remap_keys.in_process
    }

    /// 主のコードが `vk` の物理キーの添字。
    fn key_with_primary(layout: &Layout, vk: u32) -> usize {
        layout
            .keys
            .iter()
            .position(|k| k.primary() == KeyCode(vk))
            .unwrap()
    }

    #[test]
    fn numpad_origin_codes_map_to_numpad_keys() {
        let layout = Layout::us_ansi();
        assert_eq!(
            layout.find(KeyCode(2147483685)),
            Some(key_with_primary(&layout, 0x64))
        );
        assert_eq!(
            layout.find(KeyCode(2147483693)),
            Some(key_with_primary(&layout, 0x60))
        );
        assert_eq!(
            layout.find(KeyCode(12)),
            Some(key_with_primary(&layout, 0x65))
        );
        assert_eq!(
            layout.find(KeyCode(2147483694)),
            Some(key_with_primary(&layout, 0x6E))
        );
        assert_eq!(
            layout.find(KeyCode(37)),
            Some(key_with_primary(&layout, 0x25))
        );
        assert_eq!(keys::name(KeyCode(256)), "Disable");
    }

    #[test]
    fn remapped_keys_marks_numpad_and_navigation_keys() {
        let layout = Layout::us_ansi();
        let marked = remapped_keys(&layout, &remaps());
        for vk in [
            0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6E,
        ] {
            assert!(marked.contains(&key_with_primary(&layout, vk)), "{vk:#X}");
        }
        // "37" はナビゲーションクラスタの ←
        assert!(marked.contains(&key_with_primary(&layout, 0x25)));
        // リマップ元でないキー
        assert!(!marked.contains(&key_with_primary(&layout, 0x26)));
        assert!(!marked.contains(&key_with_primary(&layout, 0x0D)));
        assert_eq!(marked.len(), 12);
    }

    #[test]
    fn related_remaps_cover_all_codes_of_a_key() {
        let layout = Layout::us_ansi();
        let remaps = remaps();
        let num4 = &layout.keys[key_with_primary(&layout, 0x64)];
        let related = related_remaps(&num4.codes, &remaps);
        // VK_NUMPAD4 はリマップ先、0x80000025 はリマップ元
        assert_eq!(related.len(), 2);
        assert_eq!(related[0].code, KeyCode(0x64));
        assert_eq!(related[0].role, Role::Target);
        assert_eq!(related[0].remap.original(), vec![KeyCode(2147483685)]);
        assert_eq!(related[1].code, KeyCode(2147483685));
        assert_eq!(related[1].role, Role::Source);
        assert_eq!(related[1].remap.target(), vec![KeyCode(0x64)]);

        let num5 = &layout.keys[key_with_primary(&layout, 0x65)];
        let related = related_remaps(&num5.codes, &remaps);
        assert_eq!(related.len(), 2);
        assert_eq!(related[1].code, KeyCode(0x0C));
        assert_eq!(related[1].role, Role::Source);
    }

    #[test]
    fn related_remaps_for_navigation_left() {
        let layout = Layout::us_ansi();
        let remaps = remaps();
        let left = &layout.keys[key_with_primary(&layout, 0x25)];
        let related = related_remaps(&left.codes, &remaps);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].role, Role::Source);
        assert_eq!(keys::name(related[0].remap.target()[0]), "Disable");
    }
}
