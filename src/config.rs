//! PowerToys Keyboard Manager の `default.json` の読み書き。
//!
//! 未知のフィールドは `extra` に保持し、書き戻しても失われないようにする。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 拡張キー(矢印キーとテンキーの区別など)を示すフラグ。
pub const EXTENDED_FLAG: u32 = 0x8000_0000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub remap_keys: InProcess,
    #[serde(default)]
    pub remap_keys_to_text: InProcess,
    #[serde(default)]
    pub remap_shortcuts: Shortcuts,
    #[serde(default)]
    pub remap_shortcuts_to_text: Shortcuts,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InProcess {
    #[serde(default)]
    pub in_process: Vec<Remap>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcuts {
    #[serde(default)]
    pub global: Vec<Remap>,
    #[serde(default)]
    pub app_specific: Vec<Remap>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 1件のリマップ。キーは `;` 区切りの数値文字列で保存されている。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remap {
    pub original_keys: String,
    #[serde(default)]
    pub new_remap_keys: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Remap {
    pub fn original(&self) -> Vec<KeyCode> {
        KeyCode::parse_list(&self.original_keys)
    }

    pub fn target(&self) -> Vec<KeyCode> {
        KeyCode::parse_list(&self.new_remap_keys)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCode(pub u32);

impl KeyCode {
    pub fn parse_list(s: &str) -> Vec<KeyCode> {
        s.split(';')
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .map(KeyCode)
            .collect()
    }

    pub fn vk(self) -> u32 {
        self.0 & !EXTENDED_FLAG
    }

    pub fn is_extended(self) -> bool {
        self.0 & EXTENDED_FLAG != 0
    }
}

/// Windows 上の既定の設定ファイルパス。
pub fn default_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| {
        d.join("Microsoft")
            .join("PowerToys")
            .join("Keyboard Manager")
            .join("default.json")
    })
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"remapKeys":{"inProcess":[{"originalKeys":"2147483685","newRemapKeys":"100"},{"originalKeys":"12","newRemapKeys":"101"}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#;

    #[test]
    fn parses_extended_keys() {
        let cfg: Config = serde_json::from_str(SAMPLE).unwrap();
        let keys = &cfg.remap_keys.in_process;
        assert_eq!(keys.len(), 2);
        let k = keys[0].original()[0];
        assert!(k.is_extended());
        assert_eq!(k.vk(), 0x25);
        assert!(!keys[1].original()[0].is_extended());
    }

    #[test]
    fn round_trip_preserves_unknown_fields() {
        let src = r#"{"remapKeys":{"inProcess":[]},"futureField":1,"remapShortcuts":{"global":[{"originalKeys":"17;65","newRemapKeys":"","unicodeText":"hi"}],"appSpecific":[]}}"#;
        let cfg: Config = serde_json::from_str(src).unwrap();
        let out: Value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(out["futureField"], 1);
        assert_eq!(out["remapShortcuts"]["global"][0]["unicodeText"], "hi");
    }
}
