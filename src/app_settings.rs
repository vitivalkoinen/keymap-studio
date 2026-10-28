//! 本アプリの設定ファイル `keymap-studio.settings.json`(SPEC.md §4.5.2)。
//!
//! データフォルダ(exe と同じフォルダ)に置く。内容:
//!
//! ```json
//! {
//!   "hotkeys": {
//!     "keymaps": { "gaming.json": "Ctrl+Alt+1" },
//!     "presets": { "onishi": "Ctrl+Alt+O" }
//!   },
//!   "autoStart": true,
//!   "notifications": true
//! }
//! ```
//!
//! - `hotkeys.keymaps`: キーマップのファイル名(`keymaps` の中)→ ホットキー
//! - `hotkeys.presets`: プリセットの ID([`crate::preset::PresetId::id`])→ ホットキー
//! - `autoStart`: サインイン時の自動起動(既定 `true`)
//! - `notifications`: 切り替えたときの通知(既定 `true`)
//!
//! ホットキーの書き方は [`Hotkey::format`]。読めない値は割り当てなしとみなすが、ファイルからは
//! 消さない(その対象の割り当てを変えるまで)。未知のフィールドはトップレベルも `hotkeys` の中も
//! 保持する。書き込みは一時ファイルを経由して置き換える。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::hotkey::{self, Hotkey};
use crate::save;

/// 設定ファイルの名前。
pub const FILE_NAME: &str = "keymap-studio.settings.json";

/// ホットキーを割り当てる対象。キーマップはファイル名、プリセットは ID で指す。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Keymap(String),
    Preset(String),
}

/// 本アプリの設定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub hotkeys: Hotkeys,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default = "default_true")]
    pub notifications: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// ホットキーの割り当て。値は [`Hotkey::format`] の文字列(読めない値もそのまま持つ)。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Hotkeys {
    #[serde(default)]
    pub keymaps: Map<String, Value>,
    #[serde(default)]
    pub presets: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkeys: Hotkeys::default(),
            auto_start: true,
            notifications: true,
            extra: Map::new(),
        }
    }
}

/// 割り当てを受け付けなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignError {
    /// 組み合わせを受け付けない
    Invalid(hotkey::Invalid),
    /// 本アプリの中でほかの対象に割り当て済み
    Taken(Target),
}

impl AppSettings {
    fn map(&self, target: &Target) -> (&Map<String, Value>, String) {
        match target {
            Target::Keymap(f) => (&self.hotkeys.keymaps, f.clone()),
            Target::Preset(id) => (&self.hotkeys.presets, id.clone()),
        }
    }

    fn map_mut(&mut self, target: &Target) -> (&mut Map<String, Value>, String) {
        match target {
            Target::Keymap(f) => (&mut self.hotkeys.keymaps, f.clone()),
            Target::Preset(id) => (&mut self.hotkeys.presets, id.clone()),
        }
    }

    /// `target` に割り当てたホットキー。割り当てがないか、読めない値なら `None`。
    pub fn hotkey(&self, target: &Target) -> Option<Hotkey> {
        let (map, key) = self.map(target);
        map.get(&key)
            .and_then(Value::as_str)
            .and_then(Hotkey::parse)
    }

    /// 読める割り当てのすべて(キーマップ、プリセットの順。それぞれファイルに書いた順)。
    pub fn assignments(&self) -> Vec<(Target, Hotkey)> {
        let parse = |map: &Map<String, Value>, make: fn(String) -> Target| {
            map.iter()
                .filter_map(|(k, v)| Some((make(k.clone()), Hotkey::parse(v.as_str()?)?)))
                .collect::<Vec<_>>()
        };
        let mut all = parse(&self.hotkeys.keymaps, Target::Keymap);
        all.extend(parse(&self.hotkeys.presets, Target::Preset));
        all
    }

    /// `hotkey` を割り当てているほかの対象(`target` 以外)。
    pub fn holder_of(&self, hotkey: Hotkey, except: &Target) -> Option<Target> {
        self.assignments()
            .into_iter()
            .find(|(t, h)| *h == hotkey && t != except)
            .map(|(t, _)| t)
    }

    /// `target` に `hotkey` を割り当てる(前の割り当ては置き換える)。修飾キーを含まない組み合わせと、
    /// 本アプリの中でほかの対象に割り当て済みの組み合わせは拒否する。
    pub fn assign(&mut self, target: &Target, hotkey: Hotkey) -> Result<(), AssignError> {
        hotkey.validate().map_err(AssignError::Invalid)?;
        if let Some(holder) = self.holder_of(hotkey, target) {
            return Err(AssignError::Taken(holder));
        }
        let (map, key) = self.map_mut(target);
        map.insert(key, Value::String(hotkey.format()));
        Ok(())
    }

    /// `target` の割り当てを外す(読めない値も消す)。外したら `true`。
    pub fn unassign(&mut self, target: &Target) -> bool {
        let (map, key) = self.map_mut(target);
        map.shift_remove(&key).is_some()
    }

    /// `exists` が `false` を返すキーマップのファイルの割り当てを外す(削除されたキーマップ)。
    /// 外したら `true`。
    pub fn prune_keymaps(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let before = self.hotkeys.keymaps.len();
        self.hotkeys.keymaps.retain(|file, _| exists(file));
        self.hotkeys.keymaps.len() != before
    }
}

/// `path` から読む。ファイルがなければ既定の設定。読めない・解析できないときはエラー
/// (呼び出し側は既定の設定で動き、ファイルを上書きしない)。
pub fn load(path: &Path) -> Result<AppSettings, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `path` に書く(同じフォルダの一時ファイルに書いてから置き換える)。
pub fn store(path: &Path, settings: &AppSettings) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    save::write_replace(path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// 既定の場所(exe と同じフォルダ)。exe の場所が分からなければ `None`。
pub fn default_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    fn keymap(f: &str) -> Target {
        Target::Keymap(f.to_owned())
    }

    #[test]
    fn missing_file_gives_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load(&tmp.path().join(FILE_NAME)).unwrap();
        assert_eq!(s, AppSettings::default());
        assert!(s.auto_start);
        assert!(s.notifications);
        assert!(s.assignments().is_empty());
    }

    #[test]
    fn round_trip_keeps_unknown_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(FILE_NAME);
        fs::write(
            &path,
            r#"{"future":{"a":1},"hotkeys":{"keymaps":{"a.json":"Ctrl+Alt+1","bad.json":42},"presets":{"onishi":"Alt+Shift+O"},"later":[1]},"autoStart":false,"notifications":false,"zzz":"keep"}"#,
        )
        .unwrap();
        let mut s = load(&path).unwrap();
        assert!(!s.auto_start);
        assert!(!s.notifications);
        assert_eq!(s.hotkey(&keymap("a.json")), Some(hk("Ctrl+Alt+1")));
        // 読めない値は割り当てなしとみなす
        assert_eq!(s.hotkey(&keymap("bad.json")), None);
        assert_eq!(
            s.hotkey(&Target::Preset("onishi".into())),
            Some(hk("Alt+Shift+O"))
        );

        s.assign(&keymap("b.json"), hk("Ctrl+Alt+2")).unwrap();
        store(&path, &s).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["future"]["a"], 1);
        assert_eq!(v["zzz"], "keep");
        assert_eq!(v["hotkeys"]["later"], serde_json::json!([1]));
        assert_eq!(v["hotkeys"]["keymaps"]["bad.json"], 42);
        assert_eq!(v["hotkeys"]["keymaps"]["b.json"], "Ctrl+Alt+2");
        assert_eq!(v["autoStart"], false);
        assert_eq!(v["notifications"], false);
        // 一時ファイルは残らない
        let names: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
        assert_eq!(load(&path).unwrap(), s);
    }

    #[test]
    fn unreadable_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(FILE_NAME);
        fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_err());
        fs::write(&path, r#"{"autoStart":"yes"}"#).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn assign_rejects_invalid_and_duplicates() {
        let mut s = AppSettings::default();
        let a = keymap("a.json");
        let onishi = Target::Preset("onishi".into());
        // 修飾キーなし
        let bare = Hotkey {
            vk: 0x41,
            ..Default::default()
        };
        assert_eq!(
            s.assign(&a, bare),
            Err(AssignError::Invalid(hotkey::Invalid::NoModifier))
        );
        s.assign(&a, hk("Ctrl+Alt+1")).unwrap();
        // ほかの対象(プリセットも含む)と重なるものは拒否
        assert_eq!(
            s.assign(&onishi, hk("Alt+Ctrl+1")),
            Err(AssignError::Taken(a.clone()))
        );
        assert_eq!(
            s.assign(&keymap("b.json"), hk("Ctrl+Alt+1")),
            Err(AssignError::Taken(a.clone()))
        );
        // 自分自身に同じものを割り当て直すのはよい。置き換えもできる
        s.assign(&a, hk("Ctrl+Alt+1")).unwrap();
        s.assign(&a, hk("Ctrl+Alt+2")).unwrap();
        s.assign(&onishi, hk("Ctrl+Alt+1")).unwrap();
        assert_eq!(
            s.assignments(),
            vec![
                (a.clone(), hk("Ctrl+Alt+2")),
                (onishi.clone(), hk("Ctrl+Alt+1"))
            ]
        );
        assert!(s.unassign(&a));
        assert!(!s.unassign(&a));
        assert_eq!(s.hotkey(&a), None);
    }

    #[test]
    fn prune_removes_only_missing_keymaps() {
        let mut s = AppSettings::default();
        s.assign(&keymap("a.json"), hk("Ctrl+1")).unwrap();
        s.assign(&keymap("gone.json"), hk("Ctrl+2")).unwrap();
        s.assign(&Target::Preset("onishi".into()), hk("Ctrl+3"))
            .unwrap();
        assert!(s.prune_keymaps(|f| f == "a.json"));
        assert!(!s.prune_keymaps(|f| f == "a.json"));
        assert_eq!(s.hotkey(&keymap("a.json")), Some(hk("Ctrl+1")));
        assert_eq!(s.hotkey(&keymap("gone.json")), None);
        assert_eq!(
            s.hotkey(&Target::Preset("onishi".into())),
            Some(hk("Ctrl+3"))
        );
    }
}
