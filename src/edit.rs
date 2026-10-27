//! キー→キーのリマップ(`remapKeys.inProcess`)の追加・変更・削除。
//!
//! リマップ元は [`keys::normalize`] を通して比べる。`originalKeys` の全体が 1 つの整数である
//! エントリ([`Remap::single_original`])だけを対象とし、同じリマップ元のエントリが 2 件以上に
//! ならないようにする。リマップ元と同じキーへのリマップ(自分自身へのリマップ)は作らない。

use serde_json::Map;

use crate::config::{KeyCode, Remap};
use crate::keys;

/// `remap` のリマップ元が `source` だけか。
fn is_source(remap: &Remap, source: KeyCode) -> bool {
    remap
        .single_original()
        .is_some_and(|code| keys::normalize(code) == keys::normalize(source))
}

/// `target` を `source` のリマップ先にできるか。正規化して同じキーになるもの
/// (自分自身へのリマップ)は不可。
pub fn can_target(source: KeyCode, target: KeyCode) -> bool {
    keys::normalize(source) != keys::normalize(target)
}

/// `source` をリマップ元とするエントリの添字(先頭のもの)。
pub fn find(remaps: &[Remap], source: KeyCode) -> Option<usize> {
    remaps.iter().position(|r| is_source(r, source))
}

/// `source` を `target` へリマップする。変更があれば `true`。
///
/// 既存のエントリがあれば、その `newRemapKeys` だけを書き換える(`originalKeys` の表記と
/// `extra` はそのまま)。同じリマップ元のエントリがほかにもあれば取り除く。
/// なければ末尾に追加する。自分自身へのリマップ([`can_target`] が `false`)なら何もしない。
pub fn set(remaps: &mut Vec<Remap>, source: KeyCode, target: KeyCode) -> bool {
    if !can_target(source, target) {
        return false;
    }
    let new_keys = target.0.to_string();
    let Some(i) = find(remaps, source) else {
        remaps.push(Remap {
            original_keys: source.0.to_string(),
            new_remap_keys: new_keys,
            extra: Map::new(),
        });
        return true;
    };
    let mut changed = false;
    if remaps[i].new_remap_keys != new_keys {
        remaps[i].new_remap_keys = new_keys;
        changed = true;
    }
    // i より後ろの重複を取り除く
    let before = remaps.len();
    let mut index = 0;
    remaps.retain(|r| {
        let keep = index <= i || !is_source(r, source);
        index += 1;
        keep
    });
    changed || remaps.len() != before
}

/// `source` をリマップ元とするエントリをすべて取り除く。変更があれば `true`。
pub fn remove(remaps: &mut Vec<Remap>, source: KeyCode) -> bool {
    let before = remaps.len();
    remaps.retain(|r| !is_source(r, source));
    remaps.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, NUMPAD_ORIGIN_FLAG};
    use serde_json::Value;

    const SAMPLE: &str = r#"{"remapKeys":{"inProcess":[
        {"originalKeys":"2147483685","newRemapKeys":"100","futureField":"keep"},
        {"originalKeys":"12","newRemapKeys":"101"},
        {"originalKeys":"17;65","newRemapKeys":"66"}
    ],"inProcessExtra":true},"topLevelExtra":[1,2]}"#;

    fn config() -> Config {
        serde_json::from_str(SAMPLE).unwrap()
    }

    fn pairs(remaps: &[Remap]) -> Vec<(&str, &str)> {
        remaps
            .iter()
            .map(|r| (r.original_keys.as_str(), r.new_remap_keys.as_str()))
            .collect()
    }

    #[test]
    fn add_appends_new_entry() {
        let mut cfg = config();
        let remaps = &mut cfg.remap_keys.in_process;
        assert!(set(remaps, KeyCode(0x41), KeyCode(0x100)));
        assert_eq!(remaps.len(), 4);
        assert_eq!(pairs(&remaps[3..]), vec![("65", "256")]);
        assert!(remaps[3].extra.is_empty());
    }

    #[test]
    fn change_keeps_original_keys_and_extra() {
        let mut cfg = config();
        let remaps = &mut cfg.remap_keys.in_process;
        let source = KeyCode(0x25 | NUMPAD_ORIGIN_FLAG);
        assert!(set(remaps, source, KeyCode(0x61)));
        assert_eq!(remaps.len(), 3);
        assert_eq!(pairs(&remaps[..1]), vec![("2147483685", "97")]);
        assert_eq!(remaps[0].extra["futureField"], "keep");
        // 同じ値なら変更なし
        assert!(!set(remaps, source, KeyCode(0x61)));
    }

    #[test]
    fn change_matches_normalized_source() {
        // フラグで意味が変わらない VK は、フラグの有無に関係なく同じキー
        let mut remaps = vec![Remap {
            original_keys: (0x41 | NUMPAD_ORIGIN_FLAG).to_string(),
            new_remap_keys: "65".into(),
            extra: Map::new(),
        }];
        assert!(set(&mut remaps, KeyCode(0x41), KeyCode(0x42)));
        assert_eq!(remaps.len(), 1);
        assert_eq!(remaps[0].new_remap_keys, "66");
        // フラグで別のキーになる VK は区別する
        assert!(set(&mut remaps, KeyCode(0x25), KeyCode(0x42)));
        assert!(set(
            &mut remaps,
            KeyCode(0x25 | NUMPAD_ORIGIN_FLAG),
            KeyCode(0x43)
        ));
        assert_eq!(remaps.len(), 3);
    }

    #[test]
    fn numpad_divide_is_written_with_flag() {
        // テンキーの / は配列上フラグ付きのコードなので、フラグ付きで書く(SPEC.md §3.1)
        let layout = crate::layout::Layout::us_ansi();
        let key = &layout.keys[layout.find(KeyCode(0x6F | NUMPAD_ORIGIN_FLAG)).unwrap()];
        let mut remaps = Vec::new();
        assert!(set(&mut remaps, key.primary(), KeyCode(0x41)));
        assert_eq!(pairs(&remaps), vec![("2147483759", "65")]);
        // フラグなしの 0x6F とは別のエントリ
        assert!(set(&mut remaps, KeyCode(0x6F), KeyCode(0x42)));
        assert_eq!(remaps.len(), 2);
    }

    #[test]
    fn only_single_integer_original_keys_are_edited() {
        let mut remaps: Vec<Remap> = serde_json::from_str(
            r#"[{"originalKeys":"65;","newRemapKeys":"66"},
                {"originalKeys":"65;abc","newRemapKeys":"67"},
                {"originalKeys":" 65","newRemapKeys":"68"},
                {"originalKeys":"17;65","newRemapKeys":"69"}]"#,
        )
        .unwrap();
        assert_eq!(find(&remaps, KeyCode(65)), None);
        assert!(!remove(&mut remaps, KeyCode(65)));
        assert_eq!(remaps.len(), 4);
        // 既存のものは変えずに新しく追加する
        assert!(set(&mut remaps, KeyCode(65), KeyCode(70)));
        assert_eq!(remaps.len(), 5);
        assert_eq!(
            pairs(&remaps[..4]),
            vec![
                ("65;", "66"),
                ("65;abc", "67"),
                (" 65", "68"),
                ("17;65", "69")
            ]
        );
        assert_eq!(find(&remaps, KeyCode(65)), Some(4));
    }

    #[test]
    fn self_remap_is_rejected() {
        let mut remaps = Vec::new();
        assert!(!can_target(KeyCode(0x41), KeyCode(0x41)));
        // 正規化して同じキー(フラグに関係しない VK)
        assert!(!can_target(
            KeyCode(0x41),
            KeyCode(0x41 | NUMPAD_ORIGIN_FLAG)
        ));
        assert!(!set(&mut remaps, KeyCode(0x41), KeyCode(0x41)));
        assert!(remaps.is_empty());
        // フラグで別のキーになる VK は別物
        assert!(can_target(
            KeyCode(0x25 | NUMPAD_ORIGIN_FLAG),
            KeyCode(0x25)
        ));
        // 既存のエントリも自分自身へは変えない
        assert!(set(&mut remaps, KeyCode(0x41), KeyCode(0x42)));
        assert!(!set(&mut remaps, KeyCode(0x41), KeyCode(0x41)));
        assert_eq!(pairs(&remaps), vec![("65", "66")]);
    }

    #[test]
    fn set_removes_duplicate_sources() {
        let mut remaps: Vec<Remap> = serde_json::from_str(
            r#"[{"originalKeys":"65","newRemapKeys":"66","a":1},
                {"originalKeys":"67","newRemapKeys":"68"},
                {"originalKeys":"65","newRemapKeys":"69"}]"#,
        )
        .unwrap();
        // 値が同じでも重複を取り除けば変更あり
        assert!(set(&mut remaps, KeyCode(0x41), KeyCode(0x42)));
        assert_eq!(pairs(&remaps), vec![("65", "66"), ("67", "68")]);
        assert_eq!(remaps[0].extra["a"], 1);
    }

    #[test]
    fn remove_deletes_all_entries_for_source() {
        let mut cfg = config();
        let remaps = &mut cfg.remap_keys.in_process;
        remaps.push(Remap {
            original_keys: "12".into(),
            new_remap_keys: "13".into(),
            extra: Map::new(),
        });
        assert!(remove(remaps, KeyCode(0x0C)));
        assert_eq!(pairs(remaps), vec![("2147483685", "100"), ("17;65", "66")]);
        assert!(!remove(remaps, KeyCode(0x0C)));
        // 複数キーのエントリは対象外
        assert!(!remove(remaps, KeyCode(17)));
        assert_eq!(remaps.len(), 2);
    }

    #[test]
    fn edits_preserve_unknown_fields_on_serialize() {
        let mut cfg = config();
        set(&mut cfg.remap_keys.in_process, KeyCode(0x41), KeyCode(0x42));
        remove(&mut cfg.remap_keys.in_process, KeyCode(0x0C));
        let out: Value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(out["topLevelExtra"], serde_json::json!([1, 2]));
        assert_eq!(out["remapKeys"]["inProcessExtra"], true);
        let list = out["remapKeys"]["inProcess"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["futureField"], "keep");
        assert_eq!(list[2]["originalKeys"], "65");
        assert_eq!(list[2]["newRemapKeys"], "66");
    }
}
