//! 本アプリ独自のキーマップの保存形式と管理(SPEC.md §4.5.1)。
//!
//! 1 キーマップ = 1 ファイルで、データフォルダの `keymaps` に置く。ファイルの内容は
//! [`KeymapFile`](`format`・`formatVersion`・`name`・`layout`・`definition`)。
//! 未知のフィールドは、トップレベルも `definition` の中も保持する。
//!
//! 表示名(`name`)とファイル名は独立している。ファイル名は作るときに名前から安全な文字だけで
//! 作り([`file_stem_for`])、重複したら連番を付ける([`unique_file_name`])。名前を変えても
//! ファイル名は変えない。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::{self, Config, Remap};
use crate::{edit, matching, save};

/// `format` の値。
pub const FORMAT: &str = "keymap-studio.keymap";

/// 対応する `formatVersion`。
pub const FORMAT_VERSION: u64 = 1;

/// 対応する物理配列(US 配列)。
pub const LAYOUT_US: &str = "us";

/// データフォルダの中のキーマップのフォルダ名。
pub const DIR_NAME: &str = "keymaps";

/// ファイル名の stem の長さの上限(文字数)。
const MAX_STEM_CHARS: usize = 64;

/// 名前から安全な文字が 1 つも残らないときの stem。
const FALLBACK_STEM: &str = "keymap";

/// キーマップのファイルの内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeymapFile {
    pub format: String,
    pub format_version: u64,
    /// 表示名
    pub name: String,
    /// 作成時の物理配列
    pub layout: String,
    /// PowerToys のリマップ定義と同じ形
    pub definition: Config,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl KeymapFile {
    /// `definition` のリマップ定義(4 つの区分だけ)で新しいキーマップを作る。
    pub fn new(name: &str, definition: &Config) -> Self {
        Self {
            format: FORMAT.to_owned(),
            format_version: FORMAT_VERSION,
            name: name.to_owned(),
            layout: LAYOUT_US.to_owned(),
            definition: sections_of(definition),
            extra: Map::new(),
        }
    }

    /// ファイルに書く内容(字下げした JSON。手で読めて、ほかの環境へ持ち出しやすいように)。
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        Ok(text.into_bytes())
    }
}

/// 読み込み・取り込みで受け付けないキーマップのファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidKeymap {
    /// JSON として読めない、または必要なフィールドがない・型が違う(詳細)
    Malformed(String),
    /// `format` が本アプリのキーマップでない(値。ないときは空)
    WrongFormat(String),
    /// `formatVersion` が未知(値をそのまま表示用の文字列にしたもの)
    UnknownVersion(String),
    /// `layout` が対応していない(値)
    WrongLayout(String),
}

/// キーマップのファイルを解析し、検証する。形式・版・配列を先に確かめ、
/// それが合っていれば残りを解析する。
pub fn parse(bytes: &[u8]) -> Result<KeymapFile, InvalidKeymap> {
    let v: Value =
        serde_json::from_slice(bytes).map_err(|e| InvalidKeymap::Malformed(e.to_string()))?;
    match v.get("format").and_then(Value::as_str) {
        Some(FORMAT) => {}
        Some(other) => return Err(InvalidKeymap::WrongFormat(other.to_owned())),
        None => return Err(InvalidKeymap::WrongFormat(String::new())),
    }
    match v.get("formatVersion") {
        Some(n) if n.as_u64() == Some(FORMAT_VERSION) => {}
        Some(other) => return Err(InvalidKeymap::UnknownVersion(other.to_string())),
        None => return Err(InvalidKeymap::UnknownVersion(String::new())),
    }
    match v.get("layout").and_then(Value::as_str) {
        Some(LAYOUT_US) => {}
        Some(other) => return Err(InvalidKeymap::WrongLayout(other.to_owned())),
        None => return Err(InvalidKeymap::WrongLayout(String::new())),
    }
    serde_json::from_value(v).map_err(|e| InvalidKeymap::Malformed(e.to_string()))
}

/// リマップ定義のうち 4 つの区分だけを取り出したもの(`extra` は空)。
pub fn sections_of(cfg: &Config) -> Config {
    Config {
        remap_keys: cfg.remap_keys.clone(),
        remap_keys_to_text: cfg.remap_keys_to_text.clone(),
        remap_shortcuts: cfg.remap_shortcuts.clone(),
        remap_shortcuts_to_text: cfg.remap_shortcuts_to_text.clone(),
        extra: Map::new(),
    }
}

/// `base` の 4 つの区分を `definition` のもので丸ごと置き換えたもの。
/// `base` のそれ以外のトップレベルのフィールドは残す(適用。§4.5.1)。
pub fn apply_definition(base: &Config, definition: &Config) -> Config {
    Config {
        remap_keys: definition.remap_keys.clone(),
        remap_keys_to_text: definition.remap_keys_to_text.clone(),
        remap_shortcuts: definition.remap_shortcuts.clone(),
        remap_shortcuts_to_text: definition.remap_shortcuts_to_text.clone(),
        extra: base.extra.clone(),
    }
}

/// `remaps`(キー→キーのリマップ)に `preset` のリマップを重ねる。同じリマップ元は `preset` を
/// 優先する(今の編集に重ねる。§4.6)。リマップ元が 1 つのキーでないものは扱わない。
/// 変更があれば `true`。
pub fn overlay(remaps: &mut Vec<Remap>, preset: &[Remap]) -> bool {
    let mut changed = false;
    for r in preset {
        let target = r.target();
        let (Some(source), [target]) = (r.single_original(), target.as_slice()) else {
            continue;
        };
        changed |= edit::set(remaps, source, *target);
    }
    changed
}

/// 2 つのリマップ定義が、リマップとして同じか。
///
/// キー→キーのリマップは解析したキーコードの値で比べる([`matching::source_map`]。表記の違い・
/// 並び順・未知のフィールドは差とみなさない。§4.9 の未保存の判定と同じ)。ほかの 3 つの区分は
/// 本アプリでは解釈しないので、JSON として同じかで比べる。4 つの区分以外は比べない。
pub fn same_remaps(a: &Config, b: &Config) -> bool {
    matching::source_map(&a.remap_keys.in_process) == matching::source_map(&b.remap_keys.in_process)
        && to_json(&a.remap_keys_to_text) == to_json(&b.remap_keys_to_text)
        && to_json(&a.remap_shortcuts) == to_json(&b.remap_shortcuts)
        && to_json(&a.remap_shortcuts_to_text) == to_json(&b.remap_shortcuts_to_text)
}

fn to_json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// 名前からファイル名の stem を作る。
///
/// 文字・数字(Unicode を含む)と `-` `_` だけを残し、それ以外(空白や記号)は `-` にする。
/// 続く `-` は 1 つにまとめ、前後の `-` は除く。長すぎるときは切り詰める。何も残らない、または
/// Windows の予約デバイス名になるときは `keymap` を使う(または後ろに `-keymap` を付ける)。
pub fn file_stem_for(name: &str) -> String {
    let mut stem = String::new();
    for c in name.chars() {
        let c = if c.is_alphanumeric() || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if c == '-' && (stem.is_empty() || stem.ends_with('-')) {
            continue;
        }
        stem.push(c);
    }
    let stem: String = stem
        .trim_end_matches('-')
        .chars()
        .take(MAX_STEM_CHARS)
        .collect();
    let stem = stem.trim_end_matches('-').to_owned();
    if stem.is_empty() {
        FALLBACK_STEM.to_owned()
    } else if !config::is_safe_configuration_name(&stem) {
        // 予約デバイス名(CON など)
        format!("{stem}-{FALLBACK_STEM}")
    } else {
        stem
    }
}

/// `stem` から、`taken`(使用中のファイル名。小文字にしたもの)と重ならないファイル名を作る。
/// `stem.json`、`stem-2.json`、`stem-3.json`… の順に試す。Windows は大文字と小文字を
/// 区別しないので、比べるときは小文字にする。
pub fn unique_file_name(stem: &str, taken: &HashSet<String>) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                format!("{stem}.json")
            } else {
                format!("{stem}-{n}.json")
            }
        })
        .find(|f| !taken.contains(&f.to_lowercase()))
        .expect("an unused file name exists")
}

/// `name` を、`existing`(既存のキーマップの名前)と重ならない名前にする(取り込み)。
/// 重なれば `name (2)`、`name (3)`… とする。大文字と小文字は区別しない。
pub fn unique_name(name: &str, existing: &[&str]) -> String {
    let taken: HashSet<String> = existing.iter().map(|n| n.to_lowercase()).collect();
    (1..)
        .map(|n| {
            if n == 1 {
                name.to_owned()
            } else {
                format!("{name} ({n})")
            }
        })
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .expect("an unused name exists")
}

/// `name` が既存の名前(大文字と小文字を区別しない)と重なるか。`except` のファイルは除く
/// (名前の変更で自分自身と比べないため)。
pub fn name_taken(entries: &[Entry], name: &str, except: Option<&str>) -> bool {
    let name = name.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| Some(e.file_name.as_str()) != except)
        .any(|e| e.keymap.name.trim().to_lowercase() == name)
}

/// 一覧の 1 件。
#[derive(Debug, Clone)]
pub struct Entry {
    /// `keymaps` の中のファイル名(識別に使う)
    pub file_name: String,
    pub keymap: KeymapFile,
}

/// 一覧に出せなかったファイル。
#[derive(Debug, Clone)]
pub struct Unreadable {
    pub file_name: String,
    pub reason: InvalidKeymap,
}

/// `keymaps` のフォルダ。
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `keymaps` の `.json` を読み、名前の順に並べる。読めないファイルは別に返す。
    /// フォルダがなければ空。
    pub fn list(&self) -> (Vec<Entry>, Vec<Unreadable>) {
        let mut entries = Vec::new();
        let mut unreadable = Vec::new();
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return (entries, unreadable);
        };
        for e in dir.filter_map(|e| e.ok()) {
            let Ok(file_name) = e.file_name().into_string() else {
                continue;
            };
            if !file_name.to_lowercase().ends_with(".json") || !e.path().is_file() {
                continue;
            }
            let result = fs::read(e.path())
                .map_err(|err| InvalidKeymap::Malformed(err.to_string()))
                .and_then(|bytes| parse(&bytes));
            match result {
                Ok(keymap) => entries.push(Entry { file_name, keymap }),
                Err(reason) => unreadable.push(Unreadable { file_name, reason }),
            }
        }
        entries.sort_by(|a, b| {
            (a.keymap.name.to_lowercase(), &a.file_name)
                .cmp(&(b.keymap.name.to_lowercase(), &b.file_name))
        });
        unreadable.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        (entries, unreadable)
    }

    /// `keymaps` の中の `file_name` のパス。`file_name` が 1 つのファイル名でなければ
    /// (パス区切りや `..` を含むなど。[`is_plain_file_name`])、フォルダの外を指さないよう拒否する。
    fn path(&self, file_name: &str) -> Result<PathBuf, String> {
        if is_plain_file_name(file_name) {
            Ok(self.dir.join(file_name))
        } else {
            Err(format!("invalid keymap file name: {file_name:?}"))
        }
    }

    /// 使用中のファイル名(小文字)。読めないファイルも含める(上書きしないため)。
    fn taken_names(&self) -> HashSet<String> {
        fs::read_dir(&self.dir)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .map(|n| n.to_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 新しいファイルに保存し、そのファイル名を返す。フォルダがなければ作る。
    /// 既存のファイルは置き換えない。
    pub fn create(&self, keymap: &KeymapFile) -> Result<String, String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        self.create_avoiding(keymap, self.taken_names())
    }

    /// `taken`(使用中とわかっているファイル名。小文字)を避けて新しいファイルを作る
    /// ([`save::write_new`])。一覧を取ったあとに別のプログラムが同じ名前のファイルを
    /// 作っていたら、次の連番で作り直す。
    fn create_avoiding(
        &self,
        keymap: &KeymapFile,
        mut taken: HashSet<String>,
    ) -> Result<String, String> {
        const MAX_ATTEMPTS: usize = 1000;
        let stem = file_stem_for(&keymap.name);
        let bytes = keymap.to_bytes()?;
        for _ in 0..MAX_ATTEMPTS {
            let file_name = unique_file_name(&stem, &taken);
            let path = self.path(&file_name)?;
            match save::write_new(&path, &bytes) {
                Ok(()) => return Ok(file_name),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    taken.insert(file_name.to_lowercase());
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err(format!(
            "{}: no unused file name for {stem:?}",
            self.dir.display()
        ))
    }

    /// 既存の `file_name` に書く(一時ファイルを経由して置き換える)。
    fn write(&self, file_name: &str, keymap: &KeymapFile) -> Result<(), String> {
        let path = self.path(file_name)?;
        let bytes = keymap.to_bytes()?;
        save::write_replace(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// ファイルから読み直す(上書き・名前の変更で、ほかのプログラムが加えたフィールドも保つため)。
    pub fn read(&self, file_name: &str) -> Result<KeymapFile, String> {
        let path = self.path(file_name)?;
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        parse(&bytes).map_err(|e| format!("{}: {e:?}", path.display()))
    }

    /// `file_name` のリマップ定義の 4 つの区分を `definition` のもので置き換える。
    /// 名前・未知のフィールド(`definition` の中も)は残す。
    pub fn overwrite(&self, file_name: &str, definition: &Config) -> Result<KeymapFile, String> {
        let mut keymap = self.read(file_name)?;
        let extra = std::mem::take(&mut keymap.definition.extra);
        keymap.definition = Config {
            extra,
            ..sections_of(definition)
        };
        self.write(file_name, &keymap)?;
        Ok(keymap)
    }

    /// 名前を変える。ファイル名は変えない。
    pub fn rename(&self, file_name: &str, name: &str) -> Result<KeymapFile, String> {
        let mut keymap = self.read(file_name)?;
        keymap.name = name.to_owned();
        self.write(file_name, &keymap)?;
        Ok(keymap)
    }

    pub fn delete(&self, file_name: &str) -> Result<(), String> {
        let path = self.path(file_name)?;
        fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// `file_name` のファイルを `dest` に書き出す(中身はそのまま)。
    ///
    /// [`save::write_replace`] で、`dest` と同じフォルダの一時ファイル
    /// (`dest.with_file_name(...)`)に書いてから置き換える。途中で失敗しても、`dest` に前から
    /// あったファイルは壊れない。
    pub fn export(&self, file_name: &str, dest: &Path) -> Result<(), String> {
        let path = self.path(file_name)?;
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        save::write_replace(dest, &bytes).map_err(|e| format!("{}: {e}", dest.display()))
    }

    /// `src` のファイルを検証して取り込む。名前が既存と重なれば連番を付ける。名前が空なら
    /// 元のファイル名の stem を使う。取り込んだもの(ファイル名と内容)を返す。
    pub fn import(&self, src: &Path, existing: &[Entry]) -> Result<Entry, ImportError> {
        let bytes =
            fs::read(src).map_err(|e| ImportError::Io(format!("{}: {e}", src.display())))?;
        let mut keymap = parse(&bytes).map_err(ImportError::Invalid)?;
        let name = match keymap.name.trim() {
            "" => src
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(FALLBACK_STEM)
                .to_owned(),
            n => n.to_owned(),
        };
        let names: Vec<&str> = existing.iter().map(|e| e.keymap.name.as_str()).collect();
        keymap.name = unique_name(&name, &names);
        let file_name = self.create(&keymap).map_err(ImportError::Io)?;
        Ok(Entry { file_name, keymap })
    }
}

/// `name` が 1 つのファイル名か(フォルダの外や別のフォルダを指さないか)。
///
/// 空のもの、`..` を含むもの、パス区切り(`/` `\\`)やドライブ・ストリームの指定に使う `:` を
/// 含むもの、`Path::file_name()` で取り出したものと一致しないもの(`.` など)は拒否する。
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && !name.contains(['/', '\\', ':'])
        && Path::new(name).file_name() == Some(std::ffi::OsStr::new(name))
}

/// 取り込めなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// 読み書きに失敗した(詳細)
    Io(String),
    /// キーマップのファイルとして受け付けない
    Invalid(InvalidKeymap),
}

/// データフォルダ(exe と同じフォルダ)の `keymaps`。exe の場所が分からなければ `None`。
pub fn default_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(DIR_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeyCode;

    const DEFINITION: &str = r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66","x":1}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[{"originalKeys":"17;65","newRemapKeys":"66"}],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#;

    fn definition() -> Config {
        serde_json::from_str(DEFINITION).unwrap()
    }

    fn keymap_json(extra: &str) -> String {
        format!(
            r#"{{"format":"keymap-studio.keymap","formatVersion":1,"name":"Gaming","layout":"us","definition":{DEFINITION}{extra}}}"#
        )
    }

    #[test]
    fn round_trip_keeps_unknown_fields() {
        let src = r#"{"format":"keymap-studio.keymap","formatVersion":1,"name":"Gaming","layout":"us","definition":{"remapKeys":{"inProcess":[]},"futureSection":{"a":1}},"note":"hi","nested":{"z":1,"a":2}}"#;
        let k = parse(src.as_bytes()).unwrap();
        assert_eq!(k.name, "Gaming");
        let out: Value = serde_json::from_slice(&k.to_bytes().unwrap()).unwrap();
        assert_eq!(out["note"], "hi");
        assert_eq!(out["nested"]["z"], 1);
        assert_eq!(out["definition"]["futureSection"]["a"], 1);
        assert_eq!(out["format"], FORMAT);
        assert_eq!(out["formatVersion"], 1);
        assert_eq!(out["layout"], "us");
        // 区分がなくても 4 つとも書く
        assert_eq!(
            out["definition"]["remapShortcuts"]["global"],
            Value::Array(vec![])
        );
        // 決まったフィールドが先、未知のものは読んだ順
        let keys: Vec<&String> = out.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "format",
                "formatVersion",
                "name",
                "layout",
                "definition",
                "note",
                "nested"
            ]
        );
    }

    #[test]
    fn new_keymap_has_only_the_four_sections() {
        let mut cfg = definition();
        cfg.extra.insert("topLevel".into(), Value::Bool(true));
        let k = KeymapFile::new("Mine", &cfg);
        let out: Value = serde_json::from_slice(&k.to_bytes().unwrap()).unwrap();
        assert_eq!(out["name"], "Mine");
        assert!(out["definition"].get("topLevel").is_none());
        assert_eq!(out["definition"]["remapKeys"]["inProcess"][0]["x"], 1);
        assert_eq!(parse(&k.to_bytes().unwrap()).unwrap().name, "Mine");
    }

    #[test]
    fn parse_rejects_invalid_files() {
        assert!(matches!(
            parse(b"not json"),
            Err(InvalidKeymap::Malformed(_))
        ));
        assert_eq!(
            parse(br#"{"remapKeys":{}}"#).unwrap_err(),
            InvalidKeymap::WrongFormat(String::new())
        );
        let wrong_format = keymap_json("").replace("keymap-studio.keymap", "other");
        assert_eq!(
            parse(wrong_format.as_bytes()).unwrap_err(),
            InvalidKeymap::WrongFormat("other".into())
        );
        for (version, shown) in [("2", "2"), ("\"1\"", "\"1\""), ("1.5", "1.5")] {
            let s = keymap_json("").replace(
                r#""formatVersion":1"#,
                &format!(r#""formatVersion":{version}"#),
            );
            assert_eq!(
                parse(s.as_bytes()).unwrap_err(),
                InvalidKeymap::UnknownVersion(shown.into()),
                "{s}"
            );
        }
        let jis = keymap_json("").replace(r#""layout":"us""#, r#""layout":"jis""#);
        assert_eq!(
            parse(jis.as_bytes()).unwrap_err(),
            InvalidKeymap::WrongLayout("jis".into())
        );
        let no_layout = keymap_json("").replace(r#""layout":"us","#, "");
        assert_eq!(
            parse(no_layout.as_bytes()).unwrap_err(),
            InvalidKeymap::WrongLayout(String::new())
        );
        // 必要なフィールドがない・型が違う
        let no_name = keymap_json("").replace(r#""name":"Gaming","#, "");
        assert!(matches!(
            parse(no_name.as_bytes()),
            Err(InvalidKeymap::Malformed(_))
        ));
        let bad_def = r#"{"format":"keymap-studio.keymap","formatVersion":1,"name":"x","layout":"us","definition":[]}"#;
        assert!(matches!(
            parse(bad_def.as_bytes()),
            Err(InvalidKeymap::Malformed(_))
        ));
        // 正しいもの
        assert!(parse(keymap_json(r#","extra":1"#).as_bytes()).is_ok());
    }

    #[test]
    fn file_stems_use_only_safe_characters() {
        for (name, stem) in [
            ("Gaming", "Gaming"),
            ("My keymap 2", "My-keymap-2"),
            ("  a / b \\ c : d  ", "a-b-c-d"),
            ("テンキー固定+大西", "テンキー固定-大西"),
            ("a..b", "a-b"),
            ("<>:\"|?*", "keymap"),
            ("", "keymap"),
            ("   ", "keymap"),
            ("CON", "CON-keymap"),
            ("nul", "nul-keymap"),
            ("snake_case-name", "snake_case-name"),
            ("tab\there\nnewline", "tab-here-newline"),
        ] {
            assert_eq!(file_stem_for(name), stem, "{name:?}");
            assert!(config::is_safe_configuration_name(&file_stem_for(name)));
        }
        let long = "x".repeat(200);
        assert_eq!(file_stem_for(&long).chars().count(), MAX_STEM_CHARS);
    }

    #[test]
    fn unique_file_names_get_numbers() {
        let taken: HashSet<String> = ["gaming.json", "gaming-2.json", "other.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(unique_file_name("Gaming", &taken), "Gaming-3.json");
        assert_eq!(unique_file_name("New", &taken), "New.json");
    }

    #[test]
    fn unique_names_get_numbers() {
        assert_eq!(unique_name("Gaming", &["Other"]), "Gaming");
        assert_eq!(unique_name("Gaming", &["gaming"]), "Gaming (2)");
        assert_eq!(
            unique_name("Gaming", &["Gaming", "Gaming (2)"]),
            "Gaming (3)"
        );
    }

    #[test]
    fn store_create_list_overwrite_rename_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join(DIR_NAME));
        assert!(store.list().0.is_empty());

        let a = store
            .create(&KeymapFile::new("Gaming", &definition()))
            .unwrap();
        let b = store
            .create(&KeymapFile::new("gaming", &Config::default()))
            .unwrap();
        assert_eq!(a, "Gaming.json");
        assert_eq!(b, "gaming-2.json");
        // 読めないファイルは別に返す
        fs::write(store.dir.join("broken.json"), "x").unwrap();
        fs::write(store.dir.join("notes.txt"), "x").unwrap();
        let (entries, unreadable) = store.list();
        assert_eq!(entries.len(), 2);
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].file_name, "broken.json");

        // 上書き: 4 つの区分だけを置き換え、名前と未知のフィールドは残す
        let path = store.dir.join(&a);
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v["custom"] = Value::from(1);
        v["definition"]["futureSection"] = Value::from(2);
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let k = store.overwrite(&a, &Config::default()).unwrap();
        assert_eq!(k.name, "Gaming");
        assert!(k.definition.remap_keys.in_process.is_empty());
        let v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["custom"], 1);
        assert_eq!(v["definition"]["futureSection"], 2);
        assert_eq!(
            v["definition"]["remapShortcuts"]["global"],
            Value::Array(vec![])
        );

        // 名前の変更: ファイル名は変えない
        let k = store.rename(&a, "Work").unwrap();
        assert_eq!(k.name, "Work");
        assert!(path.exists());
        assert_eq!(parse(&fs::read(&path).unwrap()).unwrap().name, "Work");
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap()["custom"],
            1
        );

        store.delete(&b).unwrap();
        let names: Vec<String> = store.list().0.into_iter().map(|e| e.keymap.name).collect();
        assert_eq!(names, vec!["Work"]);
        // 一時ファイルは残らない
        assert!(
            fs::read_dir(&store.dir).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }

    #[test]
    fn name_taken_ignores_case_and_self() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join(DIR_NAME));
        let a = store
            .create(&KeymapFile::new("Gaming", &Config::default()))
            .unwrap();
        let entries = store.list().0;
        assert!(name_taken(&entries, " gaming ", None));
        assert!(!name_taken(&entries, "gaming", Some(&a)));
        assert!(!name_taken(&entries, "Work", None));
    }

    #[test]
    fn import_validates_and_numbers_names() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join(DIR_NAME));
        store
            .create(&KeymapFile::new("Gaming", &Config::default()))
            .unwrap();
        let src = tmp.path().join("shared.json");
        fs::write(&src, keymap_json(r#","author":"me""#)).unwrap();

        let existing = store.list().0;
        let e = store.import(&src, &existing).unwrap();
        assert_eq!(e.keymap.name, "Gaming (2)");
        assert_eq!(e.file_name, "Gaming-2.json");
        let v: Value =
            serde_json::from_slice(&fs::read(store.dir.join(&e.file_name)).unwrap()).unwrap();
        assert_eq!(v["author"], "me");
        assert_eq!(v["name"], "Gaming (2)");
        assert_eq!(v["definition"]["remapKeys"]["inProcess"][0]["x"], 1);

        // 名前が空なら元のファイル名
        let unnamed = tmp.path().join("from-friend.json");
        fs::write(
            &unnamed,
            keymap_json("").replace(r#""name":"Gaming""#, r#""name":"  ""#),
        )
        .unwrap();
        let e = store.import(&unnamed, &store.list().0).unwrap();
        assert_eq!(e.keymap.name, "from-friend");

        // 受け付けないものは取り込まない
        let before = fs::read_dir(&store.dir).unwrap().count();
        let jis = tmp.path().join("jis.json");
        fs::write(
            &jis,
            keymap_json("").replace(r#""layout":"us""#, r#""layout":"jis""#),
        )
        .unwrap();
        assert_eq!(
            store.import(&jis, &store.list().0).unwrap_err(),
            ImportError::Invalid(InvalidKeymap::WrongLayout("jis".into()))
        );
        let v2 = tmp.path().join("v2.json");
        fs::write(
            &v2,
            keymap_json("").replace(r#""formatVersion":1"#, r#""formatVersion":2"#),
        )
        .unwrap();
        assert_eq!(
            store.import(&v2, &store.list().0).unwrap_err(),
            ImportError::Invalid(InvalidKeymap::UnknownVersion("2".into()))
        );
        let powertoys = tmp.path().join("default.json");
        fs::write(&powertoys, DEFINITION).unwrap();
        assert_eq!(
            store.import(&powertoys, &store.list().0).unwrap_err(),
            ImportError::Invalid(InvalidKeymap::WrongFormat(String::new()))
        );
        assert!(matches!(
            store.import(&tmp.path().join("missing.json"), &[]),
            Err(ImportError::Io(_))
        ));
        assert_eq!(fs::read_dir(&store.dir).unwrap().count(), before);
    }

    #[test]
    fn export_writes_file_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join(DIR_NAME));
        let a = store
            .create(&KeymapFile::new("Gaming", &definition()))
            .unwrap();
        // 別のフォルダへ書き出す。既存のファイルは置き換える
        let out_dir = tmp.path().join("elsewhere");
        fs::create_dir(&out_dir).unwrap();
        let dest = out_dir.join("out.json");
        fs::write(&dest, "old").unwrap();
        store.export(&a, &dest).unwrap();
        assert_eq!(
            fs::read(&dest).unwrap(),
            fs::read(store.dir.join(&a)).unwrap()
        );
        assert!(parse(&fs::read(&dest).unwrap()).is_ok());
        // 一時ファイルはどちらのフォルダにも残らない
        let names = |d: &Path| -> Vec<String> {
            let mut v: Vec<String> = fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            v.sort();
            v
        };
        assert_eq!(names(&out_dir), vec!["out.json"]);
        assert_eq!(names(&store.dir), vec![a.clone()]);
        // 書き出せないとき(書き出し先がフォルダ)は、書き出し先を壊さず一時ファイルも残さない
        let dir_dest = out_dir.join("is-a-dir.json");
        fs::create_dir(&dir_dest).unwrap();
        assert!(store.export(&a, &dir_dest).is_err());
        assert!(dir_dest.is_dir());
        assert_eq!(names(&out_dir), vec!["is-a-dir.json", "out.json"]);
    }

    #[test]
    fn plain_file_names_only() {
        for ok in [
            "Gaming.json",
            "a-b_c.json",
            "テンキー.json",
            "x.y.json",
            "noext",
        ] {
            assert!(is_plain_file_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../x.json",
            "..\\x.json",
            "a/b.json",
            "a\\b.json",
            "a..b.json",
            "/abs.json",
            "C:x.json",
            "x.json:stream",
            "dir/",
        ] {
            assert!(!is_plain_file_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn store_rejects_paths_outside_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join(DIR_NAME));
        store
            .create(&KeymapFile::new("Gaming", &definition()))
            .unwrap();
        let outside = tmp.path().join("outside.json");
        fs::write(
            &outside,
            KeymapFile::new("Out", &definition()).to_bytes().unwrap(),
        )
        .unwrap();
        for bad in ["../outside.json", "..\\outside.json", "sub/x.json", ""] {
            assert!(store.read(bad).is_err(), "{bad}");
            assert!(store.rename(bad, "x").is_err(), "{bad}");
            assert!(store.overwrite(bad, &Config::default()).is_err(), "{bad}");
            assert!(store.delete(bad).is_err(), "{bad}");
            assert!(
                store.export(bad, &tmp.path().join("e.json")).is_err(),
                "{bad}"
            );
        }
        // 外のファイルは変わらない
        assert_eq!(parse(&fs::read(&outside).unwrap()).unwrap().name, "Out");
        assert!(!tmp.path().join("e.json").exists());
    }

    #[test]
    fn create_never_replaces_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join(DIR_NAME));
        fs::create_dir_all(&store.dir).unwrap();
        // 一覧を取ったあとに別のプログラムが作ったファイル(taken に入っていない)
        fs::write(store.dir.join("Gaming.json"), "someone else").unwrap();
        fs::write(store.dir.join("Gaming-2.json"), "someone else 2").unwrap();
        let k = KeymapFile::new("Gaming", &definition());
        let file = store.create_avoiding(&k, HashSet::new()).unwrap();
        assert_eq!(file, "Gaming-3.json");
        assert_eq!(
            fs::read_to_string(store.dir.join("Gaming.json")).unwrap(),
            "someone else"
        );
        assert_eq!(
            fs::read_to_string(store.dir.join("Gaming-2.json")).unwrap(),
            "someone else 2"
        );
        assert_eq!(
            parse(&fs::read(store.dir.join(&file)).unwrap())
                .unwrap()
                .name,
            "Gaming"
        );
        // 一時ファイルは残らない
        assert!(
            fs::read_dir(&store.dir).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }

    #[test]
    fn apply_replaces_four_sections_and_keeps_other_fields() {
        let base: Config = serde_json::from_str(
            r#"{"remapKeys":{"inProcess":[{"originalKeys":"67","newRemapKeys":"68"}],"inner":1},"remapKeysToText":{"inProcess":[{"originalKeys":"69","unicodeText":"x"}]},"remapShortcuts":{"global":[{"originalKeys":"17;67","newRemapKeys":"68"}],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]},"topLevel":"keep","other":[1]}"#,
        )
        .unwrap();
        let def = definition();
        let out = apply_definition(&base, &def);
        let v: Value = serde_json::to_value(&out).unwrap();
        // 4 つの区分は丸ごと置き換わる(区分の中の未知のフィールドも)
        assert_eq!(
            v["remapKeys"],
            serde_json::to_value(&def.remap_keys).unwrap()
        );
        assert!(v["remapKeys"].get("inner").is_none());
        assert_eq!(v["remapKeysToText"]["inProcess"], Value::Array(vec![]));
        assert_eq!(v["remapShortcuts"]["global"][0]["originalKeys"], "17;65");
        // ほかのトップレベルのフィールドは残る
        assert_eq!(v["topLevel"], "keep");
        assert_eq!(v["other"][0], 1);
        // 定義側のトップレベルの未知のフィールドは持ち込まない
        let mut def2 = def.clone();
        def2.extra.insert("fromKeymap".into(), Value::from(1));
        let v: Value = serde_json::to_value(apply_definition(&base, &def2)).unwrap();
        assert!(v.get("fromKeymap").is_none());
    }

    #[test]
    fn overlay_prefers_preset() {
        let r = |o: u32, n: u32| Remap {
            original_keys: o.to_string(),
            new_remap_keys: n.to_string(),
            ..Default::default()
        };
        let mut remaps = vec![r(0x41, 0x42), r(0x60, 0x61)];
        let preset = vec![r(0x41, 0x45), r(0x53, 0x49)];
        assert!(overlay(&mut remaps, &preset));
        let map = matching::source_map(&remaps);
        assert_eq!(map[&KeyCode(0x41)], vec![vec![KeyCode(0x45)]]);
        assert_eq!(map[&KeyCode(0x53)], vec![vec![KeyCode(0x49)]]);
        assert_eq!(map[&KeyCode(0x60)], vec![vec![KeyCode(0x61)]]);
        assert_eq!(remaps.len(), 3);
        // もう一度重ねても変わらない
        assert!(!overlay(&mut remaps, &preset));
    }

    #[test]
    fn same_remaps_compares_meaning() {
        let a = definition();
        // 表記・並び順・未知のフィールドの違いは同じとみなす
        let mut b = a.clone();
        b.remap_keys.in_process[0].extra.clear();
        b.extra.insert("other".into(), Value::from(1));
        assert!(same_remaps(&a, &b));
        let reordered: Config = serde_json::from_str(
            r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66"},{"originalKeys":"67","newRemapKeys":"68"}]},"remapShortcuts":{"global":[{"originalKeys":"17;65","newRemapKeys":"66"}],"appSpecific":[]}}"#,
        )
        .unwrap();
        let reordered2: Config = serde_json::from_str(
            r#"{"remapKeys":{"inProcess":[{"originalKeys":"67","newRemapKeys":"68"},{"originalKeys":"65","newRemapKeys":"66"}]},"remapShortcuts":{"global":[{"originalKeys":"17;65","newRemapKeys":"66"}],"appSpecific":[]}}"#,
        )
        .unwrap();
        assert!(same_remaps(&reordered, &reordered2));
        // キー→キーが違う
        let mut c = a.clone();
        c.remap_keys.in_process[0].new_remap_keys = "67".into();
        assert!(!same_remaps(&a, &c));
        // ショートカットが違う
        let mut d = a.clone();
        d.remap_shortcuts.global.clear();
        assert!(!same_remaps(&a, &d));
        assert!(same_remaps(&Config::default(), &Config::default()));
    }

    #[test]
    fn default_dir_is_next_to_exe() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(default_dir(), Some(exe.parent().unwrap().join(DIR_NAME)));
    }
}
