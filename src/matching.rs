//! リマップとキーボード図の照合。UI から切り離した純粋な関数として置く。
//!
//! キーコードはすべて [`keys::normalize`] を通してから比べる。

use std::collections::HashMap;

use crate::config::{KeyCode, Remap};
use crate::keys;
use crate::layout::{KeyDef, Layout};

/// リマップ元のコード(正規化済み)ごとの変換先。
///
/// 同じリマップ元のエントリが複数あれば、変換先をファイルの順に並べる。複数キーの
/// `originalKeys`(`"17;65"` など)は、含まれるコードそれぞれのリマップ元として扱う。
/// エントリの並び順や `originalKeys` の表記・未知のフィールドは含めないので、
/// 「リマップとしての意味」が同じなら等しくなる。
pub type SourceMap = HashMap<KeyCode, Vec<Vec<KeyCode>>>;

/// `remaps` から [`SourceMap`] を作る。
pub fn source_map(remaps: &[Remap]) -> SourceMap {
    let mut map = SourceMap::new();
    for r in remaps {
        let target = r.target();
        for code in r.original() {
            map.entry(keys::normalize(code))
                .or_default()
                .push(target.clone());
        }
    }
    map
}

/// キーボード図での物理キーの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// どのコードもリマップ元でなく、保存済みの内容とも違わない
    Plain,
    /// リマップ元のコードがあり、保存済みの内容と同じ
    Saved,
    /// 保存済みの内容と違うコードがある(追加・変更・削除のいずれも)
    Unsaved,
}

/// 物理キー `key` の状態。`saved` は最後に読み込んだか保存した時点、`current` は今の内容。
pub fn key_state(key: &KeyDef, saved: &SourceMap, current: &SourceMap) -> KeyState {
    if key.codes.iter().any(|c| saved.get(c) != current.get(c)) {
        KeyState::Unsaved
    } else if key.codes.iter().any(|c| current.contains_key(c)) {
        KeyState::Saved
    } else {
        KeyState::Plain
    }
}

/// 配列の全キーの状態(`layout.keys` と同じ順)。
pub fn key_states(layout: &Layout, saved: &SourceMap, current: &SourceMap) -> Vec<KeyState> {
    layout
        .keys
        .iter()
        .map(|k| key_state(k, saved, current))
        .collect()
}

/// キーボード図のキーに併記する変換先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRemapLabel {
    /// 表示するリマップのリマップ元(キーの `codes` のうちのどれか)
    pub source: KeyCode,
    /// その変換先(同じリマップ元のエントリが複数あれば先頭のもの)
    pub target: Vec<KeyCode>,
    /// ほかにリマップ元になっているコードの数
    pub others: usize,
}

impl KeyRemapLabel {
    /// キーの上の行に出す名前。表示しているリマップのリマップ元の名前にする
    /// (テンキーで主でないコードのリマップなら、主のコードではなくそのコードの名前)。
    pub fn source_name(&self) -> String {
        keys::name(self.source)
    }

    /// 下の行に出す変換先の名前。変換先が空か解析できないエントリなら "?"。
    pub fn target_name(&self) -> String {
        if self.target.is_empty() {
            return "?".to_owned();
        }
        self.target
            .iter()
            .map(|&k| keys::name(k))
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// 物理キー `key` に併記する変換先。リマップ元のコードがなければ `None`。
///
/// 主のコードがリマップ元ならそれを、そうでなければ `codes` の順で最初のリマップ元を表示する。
pub fn key_remap_label(key: &KeyDef, current: &SourceMap) -> Option<KeyRemapLabel> {
    let mut remapped = key
        .codes
        .iter()
        .filter_map(|c| current.get(c).and_then(|t| t.first()).map(|t| (*c, t)));
    let (source, target) = remapped.next()?;
    Some(KeyRemapLabel {
        source,
        target: target.clone(),
        others: remapped.count(),
    })
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
    fn key_states_mark_numpad_and_navigation_keys() {
        let layout = Layout::us_ansi();
        let map = source_map(&remaps());
        let states = key_states(&layout, &map, &map);
        for vk in [
            0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6E,
        ] {
            assert_eq!(
                states[key_with_primary(&layout, vk)],
                KeyState::Saved,
                "{vk:#X}"
            );
        }
        // "37" はナビゲーションクラスタの ←
        assert_eq!(states[key_with_primary(&layout, 0x25)], KeyState::Saved);
        // リマップ元でないキー
        assert_eq!(states[key_with_primary(&layout, 0x26)], KeyState::Plain);
        assert_eq!(states[key_with_primary(&layout, 0x0D)], KeyState::Plain);
        assert_eq!(states.iter().filter(|&&s| s == KeyState::Saved).count(), 12);
        assert!(!states.contains(&KeyState::Unsaved));
    }

    #[test]
    fn source_map_normalizes_and_ignores_notation_and_order() {
        let a: Vec<Remap> = serde_json::from_str(
            r#"[{"originalKeys":"65","newRemapKeys":"66","x":1},
                {"originalKeys":"2147483685","newRemapKeys":"100"}]"#,
        )
        .unwrap();
        // 順序・未知のフィールド・フラグで意味の変わらない VK のフラグは比較に影響しない
        let b: Vec<Remap> = serde_json::from_str(
            r#"[{"originalKeys":"2147483685","newRemapKeys":"100"},
                {"originalKeys":"2147483713","newRemapKeys":"66"}]"#,
        )
        .unwrap();
        assert_eq!(source_map(&a), source_map(&b));
        // フラグで別のキーになる VK は区別する
        let c: Vec<Remap> =
            serde_json::from_str(r#"[{"originalKeys":"37","newRemapKeys":"100"}]"#).unwrap();
        assert!(!source_map(&c).contains_key(&KeyCode(2147483685)));
        // 複数キーの originalKeys は含まれるコードそれぞれのリマップ元
        let d: Vec<Remap> =
            serde_json::from_str(r#"[{"originalKeys":"17;65","newRemapKeys":"66"}]"#).unwrap();
        let map = source_map(&d);
        assert_eq!(map[&KeyCode(17)], vec![vec![KeyCode(66)]]);
        assert_eq!(map[&KeyCode(65)], vec![vec![KeyCode(66)]]);
    }

    #[test]
    fn key_state_detects_added_changed_and_removed() {
        use crate::edit;
        let layout = Layout::us_ansi();
        let saved_remaps = remaps();
        let saved = source_map(&saved_remaps);
        let state = |remaps: &[Remap], vk: u32| {
            key_state(
                &layout.keys[key_with_primary(&layout, vk)],
                &saved,
                &source_map(remaps),
            )
        };

        // 追加
        let mut r = saved_remaps.clone();
        edit::set(&mut r, KeyCode(0x41), KeyCode(0x42));
        assert_eq!(state(&r, 0x41), KeyState::Unsaved);
        assert_eq!(state(&r, 0x64), KeyState::Saved);
        // 変更(テンキーの 4 の、主でないコード)
        let mut r = saved_remaps.clone();
        edit::set(&mut r, KeyCode(2147483685), KeyCode(0x61));
        assert_eq!(state(&r, 0x64), KeyState::Unsaved);
        // 削除したキーも未保存
        let mut r = saved_remaps.clone();
        edit::remove(&mut r, KeyCode(37));
        assert_eq!(state(&r, 0x25), KeyState::Unsaved);
        assert_eq!(state(&r, 0x26), KeyState::Plain);
        // 元に戻せば差はなくなる(削除して同じ内容を追加し直すと並び順だけ変わる)
        edit::set(&mut r, KeyCode(37), KeyCode(256));
        assert_eq!(state(&r, 0x25), KeyState::Saved);
        assert_eq!(source_map(&r), saved);
        // 重複したエントリを取り除くのは変更
        let mut dup = saved_remaps.clone();
        dup.push(saved_remaps[0].clone());
        let saved_dup = source_map(&dup);
        edit::set(&mut dup, KeyCode(2147483685), KeyCode(100));
        assert_ne!(source_map(&dup), saved_dup);
    }

    #[test]
    fn target_name_of_empty_or_unparsable_target_is_question_mark() {
        let layout = Layout::us_ansi();
        let key = &layout.keys[key_with_primary(&layout, 0x41)];
        for new_keys in ["", "abc", ";"] {
            let r = vec![Remap {
                original_keys: "65".into(),
                new_remap_keys: new_keys.into(),
                extra: Default::default(),
            }];
            let label = key_remap_label(key, &source_map(&r)).unwrap();
            assert_eq!(label.target_name(), "?", "{new_keys:?}");
        }
        let r: Vec<Remap> =
            serde_json::from_str(r#"[{"originalKeys":"65","newRemapKeys":"17;66"}]"#).unwrap();
        let label = key_remap_label(key, &source_map(&r)).unwrap();
        assert_eq!(label.target_name(), "Ctrl + B");
    }

    #[test]
    fn key_remap_label_prefers_primary_and_counts_others() {
        let layout = Layout::us_ansi();
        let key = |vk: u32| &layout.keys[key_with_primary(&layout, vk)];
        let map = source_map(&remaps());
        // テンキーの 4: 主のコード 0x64 はリマップ元でないので、0x80000025 を表示する
        assert_eq!(
            key_remap_label(key(0x64), &map),
            Some(KeyRemapLabel {
                source: KeyCode(2147483685),
                target: vec![KeyCode(0x64)],
                others: 0,
            })
        );
        // 上の行はリマップ元のコードの名前(主のコードの "Num 4" ではない)
        let label = key_remap_label(key(0x64), &map).unwrap();
        assert_eq!(label.source_name(), "Num Left");
        assert_eq!(
            key_remap_label(key(0x65), &map).unwrap().source_name(),
            "Clear"
        );
        assert_eq!(label.target_name(), "Num 4");
        assert_eq!(key_remap_label(key(0x26), &map), None);

        // 両方のコードがリマップ元なら主のコードを優先し、ほかの数を数える
        let mut r = remaps();
        crate::edit::set(&mut r, KeyCode(0x64), KeyCode(0x41));
        let map = source_map(&r);
        assert_eq!(
            key_remap_label(key(0x64), &map),
            Some(KeyRemapLabel {
                source: KeyCode(0x64),
                target: vec![KeyCode(0x41)],
                others: 1,
            })
        );
        assert_eq!(
            key_remap_label(key(0x64), &map).unwrap().source_name(),
            "Num 4"
        );
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
