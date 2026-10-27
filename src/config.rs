//! PowerToys Keyboard Manager の `default.json` の読み書き。
//!
//! 未知のフィールドは `extra` に保持し、書き戻しても失われないようにする。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// テンキー由来のキーであることを示すフラグ(PowerToys の `numpadOriginBit`)。
///
/// Windows の「拡張キー」とは意味が逆で、フラグ付きの 0x25 は NumLock オフ時のテンキーの 4、
/// フラグのない 0x25 はナビゲーションクラスタの ← を表す。
pub const NUMPAD_ORIGIN_FLAG: u32 = 0x8000_0000;

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
    /// `originalKeys` の全体が 1 つの整数(10 進の数字だけ)なら、そのキーコード。
    ///
    /// `"65;"` や `"65;abc"`、空白を含むものなどは `None`。編集の対象を決めるのに使う。
    pub fn single_original(&self) -> Option<KeyCode> {
        let s = self.original_keys.as_str();
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok().map(KeyCode)
    }

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
        self.0 & !NUMPAD_ORIGIN_FLAG
    }

    pub fn is_numpad_origin(self) -> bool {
        self.0 & NUMPAD_ORIGIN_FLAG != 0
    }
}

/// `settings.json` の `activeConfiguration` がないときの構成名。
pub const DEFAULT_CONFIGURATION: &str = "default";

/// PowerToys の設定フォルダの中の Keyboard Manager のフォルダ名。
pub const KEYBOARD_MANAGER_DIR: &str = "Keyboard Manager";

/// Windows 上の PowerToys の設定フォルダ(`%LOCALAPPDATA%\Microsoft\PowerToys`)。
pub fn powertoys_root() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("Microsoft").join("PowerToys"))
}

/// PowerToys の状態(SPEC.md §1.1)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerToysState {
    /// PowerToys が入っていない(PowerToys の設定フォルダがない)
    NotInstalled,
    /// PowerToys は入っているが、Keyboard Manager のフォルダかリマップの定義ファイルがない
    NotConfigured,
    /// 通常
    Ready {
        /// Keyboard Manager の設定フォルダ
        dir: PathBuf,
        /// リマップの定義ファイル(`activeConfiguration` のもの)
        definition: PathBuf,
    },
}

/// `root`(PowerToys の設定フォルダ)を見て、PowerToys の状態を判定する。ファイルは作らない。
///
/// 定義ファイルは [`active_config_path`] で決める(`settings.json` がなければ `default.json`)。
/// `settings.json` があるのに読めない、値が安全でないときはそのエラーを返す。
pub fn detect(root: &Path) -> Result<PowerToysState, PathError> {
    if !root.is_dir() {
        return Ok(PowerToysState::NotInstalled);
    }
    let dir = root.join(KEYBOARD_MANAGER_DIR);
    if !dir.is_dir() {
        return Ok(PowerToysState::NotConfigured);
    }
    let definition = active_config_path(&dir)?;
    if !definition.is_file() {
        return Ok(PowerToysState::NotConfigured);
    }
    Ok(PowerToysState::Ready { dir, definition })
}

/// 読み書きする設定ファイルのパスを決められなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// `settings.json` があるのに読めない、または JSON として解析できない(詳細はそのまま表示する)
    SettingsUnreadable(String),
    /// `settings.json` の `activeConfiguration` がファイル名として安全でない(値をそのまま持つ)
    UnsafeConfiguration(String),
}

/// `dir` の `settings.json` の `activeConfiguration` に `.json` を付けたパス。
///
/// `settings.json` がない、または値がないときは `default.json`。`settings.json` があるのに
/// 読めない・解析できないとき、値が安全なファイル名でないとき([`is_safe_configuration_name`])は、
/// 別のファイルへ読み書きしないようにエラーにする。
pub fn active_config_path(dir: &Path) -> Result<PathBuf, PathError> {
    let settings = dir.join("settings.json");
    let unreadable = |e: &dyn std::fmt::Display| {
        PathError::SettingsUnreadable(format!("{}: {e}", settings.display()))
    };
    let name = match std::fs::read_to_string(&settings) {
        Ok(text) => {
            let v: Value = serde_json::from_str(&text).map_err(|e| unreadable(&e))?;
            active_configuration(&v)?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(unreadable(&e)),
    };
    let name = name.unwrap_or_else(|| DEFAULT_CONFIGURATION.to_owned());
    Ok(dir.join(format!("{name}.json")))
}

/// 解析した `settings.json` から `activeConfiguration` を取り出す。
///
/// 実際のファイルでは `{"properties":{"activeConfiguration":{"value":"default"}}}` の形で入っている。
/// 値がないときは `Ok(None)`。値が文字列でないか安全なファイル名でなければ
/// [`PathError::UnsafeConfiguration`](値は表示用の文字列にする)。
fn active_configuration(v: &Value) -> Result<Option<String>, PathError> {
    let Some(value) = v
        .get("properties")
        .and_then(|p| p.get("activeConfiguration"))
        .and_then(|a| a.get("value"))
    else {
        return Ok(None);
    };
    match value.as_str() {
        Some(name) if is_safe_configuration_name(name) => Ok(Some(name.to_owned())),
        Some(name) => Err(PathError::UnsafeConfiguration(name.to_owned())),
        None => Err(PathError::UnsafeConfiguration(value.to_string())),
    }
}

/// 構成名がファイル名として安全か(SPEC.md §3.3)。
///
/// 次のどれにも当たらないこと。
/// - 空
/// - パス区切り(`\` `/`)・`..`・ドライブ指定などに使う `:`
/// - Windows のファイル名に使えない `< > " | ? *` と制御文字
/// - Windows の予約デバイス名(CON / PRN / AUX / NUL / COM0–9 / LPT0–9。大文字・小文字を
///   区別しない)。Windows は最初の `.` より前(末尾の空白を除く)で判定するので、
///   `CON.backup` のように拡張子が付いた形も拒否する
pub fn is_safe_configuration_name(name: &str) -> bool {
    const FORBIDDEN: &[char] = &['\\', '/', ':', '<', '>', '"', '|', '?', '*'];
    if name.is_empty()
        || name.contains(FORBIDDEN)
        || name.contains("..")
        || name.chars().any(char::is_control)
    {
        return false;
    }
    let base = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    !is_reserved_device_name(base)
}

/// Windows の予約デバイス名か(大文字・小文字を区別しない)。
fn is_reserved_device_name(base: &str) -> bool {
    let upper = base.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => match upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
        {
            Some(n) => n.len() == 1 && n.as_bytes()[0].is_ascii_digit(),
            None => false,
        },
    }
}

/// 設定ファイルを読む。解析した内容と、読んだ時点のファイルの中身(外部の変更の検知に使う)を返す。
pub fn load(path: &Path) -> Result<(Config, Vec<u8>), String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let cfg = serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((cfg, raw))
}

/// 設定ファイルに書く内容。PowerToys と同じく改行や字下げのない JSON にする。
pub fn to_json(cfg: &Config) -> Result<String, String> {
    serde_json::to_string(cfg).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"remapKeys":{"inProcess":[{"originalKeys":"2147483685","newRemapKeys":"100"},{"originalKeys":"12","newRemapKeys":"101"}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#;

    #[test]
    fn parses_numpad_origin_keys() {
        let cfg: Config = serde_json::from_str(SAMPLE).unwrap();
        let keys = &cfg.remap_keys.in_process;
        assert_eq!(keys.len(), 2);
        let k = keys[0].original()[0];
        assert!(k.is_numpad_origin());
        assert_eq!(k.vk(), 0x25);
        assert!(!keys[1].original()[0].is_numpad_origin());
    }

    #[test]
    fn round_trip_preserves_unknown_fields() {
        let src = r#"{"remapKeys":{"inProcess":[]},"futureField":1,"remapShortcuts":{"global":[{"originalKeys":"17;65","newRemapKeys":"","unicodeText":"hi"}],"appSpecific":[]}}"#;
        let cfg: Config = serde_json::from_str(src).unwrap();
        let out: Value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(out["futureField"], 1);
        assert_eq!(out["remapShortcuts"]["global"][0]["unicodeText"], "hi");
    }

    #[test]
    fn to_json_round_trips_unknown_fields() {
        let src = r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66","x":{"y":[1]}}],"z":null},"futureField":"f"}"#;
        let cfg: Config = serde_json::from_str(src).unwrap();
        let out: Value = serde_json::from_str(&to_json(&cfg).unwrap()).unwrap();
        assert_eq!(out["futureField"], "f");
        assert_eq!(out["remapKeys"]["z"], Value::Null);
        assert_eq!(out["remapKeys"]["inProcess"][0]["x"]["y"][0], 1);
    }

    #[test]
    fn to_json_preserves_field_order() {
        // PowerToys の書いたファイルと同じ形なら、読んで書き戻すと同じ文字列になる
        let src = r#"{"remapKeys":{"inProcess":[{"originalKeys":"2147483685","newRemapKeys":"100"},{"originalKeys":"12","newRemapKeys":"101"}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#;
        let cfg: Config = serde_json::from_str(src).unwrap();
        assert_eq!(to_json(&cfg).unwrap(), src);

        // 未知のフィールドは読み込んだときの順序のまま(アルファベット順に並べ替えない)
        let src = r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66","zeta":1,"alpha":2,"mid":3}],"zz":0,"aa":1},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]},"zFuture":{"b":1,"a":2},"aFuture":true}"#;
        let cfg: Config = serde_json::from_str(src).unwrap();
        assert_eq!(to_json(&cfg).unwrap(), src);
    }

    #[test]
    fn load_returns_raw_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("default.json");
        std::fs::write(&path, SAMPLE).unwrap();
        let (cfg, raw) = load(&path).unwrap();
        assert_eq!(raw, SAMPLE.as_bytes());
        assert_eq!(cfg.remap_keys.in_process.len(), 2);
    }

    #[test]
    fn active_config_path_uses_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        // settings.json がなければ default.json
        assert_eq!(
            active_config_path(dir.path()),
            Ok(dir.path().join("default.json"))
        );

        let real = r#"{"properties":{"activeConfiguration":{"value":"gaming"},"keyboardConfigurations":{"value":["default","gaming"]},"useNewEditor":true},"name":"Keyboard Manager","version":"1.0"}"#;
        std::fs::write(&settings, real).unwrap();
        assert_eq!(
            active_config_path(dir.path()),
            Ok(dir.path().join("gaming.json"))
        );

        // 値がないときは default.json
        for missing in [
            r#"{"properties":{"activeConfiguration":"gaming"}}"#,
            r#"{"properties":{}}"#,
            "{}",
        ] {
            std::fs::write(&settings, missing).unwrap();
            assert_eq!(
                active_config_path(dir.path()),
                Ok(dir.path().join("default.json")),
                "{missing}"
            );
        }
    }

    #[test]
    fn active_config_path_errors_when_settings_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        // JSON として解析できない
        for bad in ["not json", "", r#"{"properties":"#] {
            std::fs::write(&settings, bad).unwrap();
            assert!(
                matches!(
                    active_config_path(dir.path()),
                    Err(PathError::SettingsUnreadable(_))
                ),
                "{bad:?}"
            );
        }
        // UTF-8 でない
        std::fs::write(&settings, [0xFF, 0xFE, 0x00]).unwrap();
        assert!(matches!(
            active_config_path(dir.path()),
            Err(PathError::SettingsUnreadable(_))
        ));
        // ファイルでなくフォルダ(NotFound 以外の読み込みエラー)
        std::fs::remove_file(&settings).unwrap();
        std::fs::create_dir(&settings).unwrap();
        assert!(matches!(
            active_config_path(dir.path()),
            Err(PathError::SettingsUnreadable(_))
        ));
    }

    #[test]
    fn active_config_path_rejects_unsafe_names() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        for (value, shown) in [
            (r#""""#, ""),
            (r#""..""#, ".."),
            (r#""a..b""#, "a..b"),
            (r#""../default""#, "../default"),
            (r#""sub/default""#, "sub/default"),
            (r#""sub\\default""#, "sub\\default"),
            (r#""C:default""#, "C:default"),
            (r#""C:\\x\\default""#, "C:\\x\\default"),
            ("1", "1"),
            ("null", "null"),
        ] {
            let text = format!(r#"{{"properties":{{"activeConfiguration":{{"value":{value}}}}}}}"#);
            std::fs::write(&settings, &text).unwrap();
            assert_eq!(
                active_config_path(dir.path()),
                Err(PathError::UnsafeConfiguration(shown.to_owned())),
                "{text}"
            );
        }
    }

    #[test]
    fn safe_configuration_names() {
        for ok in [
            "default",
            "gaming",
            "my config",
            "a.b",
            "日本語",
            "CONFIG",
            "console",
            "COM10",
            "LPT",
            "COMX",
            "nul-ish",
            "aux1",
        ] {
            assert!(is_safe_configuration_name(ok), "{ok}");
        }
        for bad in [
            "", "..", "a/b", "a\\b", "c:", "x..y", "a<b", "a>b", "a\"b", "a|b", "a?b", "a*b",
            "a\tb", "a\nb", "a\u{0}b", "a\u{7f}b",
        ] {
            assert!(!is_safe_configuration_name(bad), "{bad:?}");
        }
        // 予約デバイス名(大文字・小文字を区別しない。拡張子付き・末尾の空白も)
        for bad in [
            "CON",
            "con",
            "Prn",
            "AUX",
            "nul",
            "COM0",
            "com1",
            "COM9",
            "LPT0",
            "lpt5",
            "LPT9",
            "CON.backup",
            "nul.txt",
            "Com3.x.y",
            "CON ",
            "AUX .old",
        ] {
            assert!(!is_safe_configuration_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn single_original_requires_one_integer() {
        let r = |s: &str| Remap {
            original_keys: s.into(),
            ..Default::default()
        };
        assert_eq!(r("65").single_original(), Some(KeyCode(65)));
        assert_eq!(
            r("2147483759").single_original(),
            Some(KeyCode(0x8000_006F))
        );
        for bad in [
            "",
            "65;",
            "65;abc",
            "17;65",
            " 65",
            "65 ",
            "+65",
            "-1",
            "abc",
            "4294967296",
        ] {
            assert_eq!(r(bad).single_original(), None, "{bad:?}");
        }
    }

    /// 一時ディレクトリの中の PowerToys の設定フォルダ(まだ作らない)。
    fn powertoys_root_in(tmp: &tempfile::TempDir) -> PathBuf {
        tmp.path().join("Microsoft").join("PowerToys")
    }

    #[test]
    fn detect_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = powertoys_root_in(&tmp);
        assert_eq!(detect(&root), Ok(PowerToysState::NotInstalled));
        // フォルダの位置にファイルがあっても入っていないとみなす
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        std::fs::write(&root, "x").unwrap();
        assert_eq!(detect(&root), Ok(PowerToysState::NotInstalled));
        // 何も作らない
        assert!(root.is_file());
    }

    #[test]
    fn detect_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let root = powertoys_root_in(&tmp);
        std::fs::create_dir_all(&root).unwrap();
        // Keyboard Manager のフォルダがない
        assert_eq!(detect(&root), Ok(PowerToysState::NotConfigured));
        assert!(!root.join(KEYBOARD_MANAGER_DIR).exists());

        // フォルダはあるが定義ファイルがない(settings.json もない → default.json を探す)
        let km = root.join(KEYBOARD_MANAGER_DIR);
        std::fs::create_dir(&km).unwrap();
        assert_eq!(detect(&root), Ok(PowerToysState::NotConfigured));
        // settings.json だけあって、activeConfiguration の定義ファイルがない
        std::fs::write(
            km.join("settings.json"),
            r#"{"properties":{"activeConfiguration":{"value":"gaming"}}}"#,
        )
        .unwrap();
        std::fs::write(km.join("default.json"), "{}").unwrap();
        assert_eq!(detect(&root), Ok(PowerToysState::NotConfigured));
        // 何も作らない
        assert!(!km.join("gaming.json").exists());
    }

    #[test]
    fn detect_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let root = powertoys_root_in(&tmp);
        let km = root.join(KEYBOARD_MANAGER_DIR);
        std::fs::create_dir_all(&km).unwrap();
        // settings.json がなければ default.json を使う
        std::fs::write(km.join("default.json"), "{}").unwrap();
        assert_eq!(
            detect(&root),
            Ok(PowerToysState::Ready {
                dir: km.clone(),
                definition: km.join("default.json"),
            })
        );
        // settings.json があれば activeConfiguration のもの
        std::fs::write(
            km.join("settings.json"),
            r#"{"properties":{"activeConfiguration":{"value":"gaming"}}}"#,
        )
        .unwrap();
        std::fs::write(km.join("gaming.json"), "{}").unwrap();
        assert_eq!(
            detect(&root),
            Ok(PowerToysState::Ready {
                dir: km.clone(),
                definition: km.join("gaming.json"),
            })
        );
    }

    #[test]
    fn detect_reports_unreadable_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let root = powertoys_root_in(&tmp);
        let km = root.join(KEYBOARD_MANAGER_DIR);
        std::fs::create_dir_all(&km).unwrap();
        std::fs::write(km.join("default.json"), "{}").unwrap();
        std::fs::write(km.join("settings.json"), "not json").unwrap();
        assert!(matches!(
            detect(&root),
            Err(PathError::SettingsUnreadable(_))
        ));
        std::fs::write(
            km.join("settings.json"),
            r#"{"properties":{"activeConfiguration":{"value":"../x"}}}"#,
        )
        .unwrap();
        assert_eq!(
            detect(&root),
            Err(PathError::UnsafeConfiguration("../x".to_owned()))
        );
    }
}
