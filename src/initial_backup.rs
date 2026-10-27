//! 導入前の設定のバックアップ(SPEC.md §4.7)。
//!
//! 本アプリの初回起動時に、PowerToys Keyboard Manager の `settings.json`・`editorSettings.json`・
//! リマップの定義ファイル(`activeConfiguration` のもの)をデータフォルダの `initial-backup`
//! に写す。一度作ったら上書きしない。
//!
//! 形式:
//! - `initial-backup/manifest.json`: 取ったときの状態([`Manifest`])
//! - `initial-backup/settings.json`・`initial-backup/editorSettings.json`: 元のファイルの写し
//! - `initial-backup/config/<構成名>.json`: リマップの定義ファイルの写し(構成名が `settings`
//!   などでも上の 2 つと名前がぶつからないよう、フォルダを分ける)
//!
//! 存在しなかったファイルは写しを作らず、`manifest.json` に `existed: false` と記録する。
//!
//! 完了の判定: すべてを一時フォルダ(`initial-backup.tmp-<プロセス ID>`)に書き、最後に
//! `manifest.json` を書いてから、フォルダごと rename で `initial-backup` にする。途中で
//! 失敗・中断しても `initial-backup` はできないので、作成済みとはみなさない。読むときも
//! `manifest.json` を解析できなければ作成済みとはみなさない。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::config::{self, Config, PathError};
use crate::save;

/// データフォルダの中のバックアップのフォルダ名。
pub const DIR_NAME: &str = "initial-backup";

/// 取ったときの状態を記録するファイル。
const MANIFEST: &str = "manifest.json";

/// リマップの定義ファイルの写しを置くフォルダ。
const DEFINITION_DIR: &str = "config";

const SETTINGS: &str = "settings.json";
const EDITOR_SETTINGS: &str = "editorSettings.json";

/// 作成中の一時フォルダの名前の接頭辞。
const TMP_PREFIX: &str = "initial-backup.tmp-";

/// フォルダの rename を再試行する間隔。Windows では作ったばかりのファイルを Defender や
/// インデクサーが開いていて、一時的にアクセス拒否になることがある。
const RENAME_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// フォルダの rename を諦めるまでの時間。
const RENAME_TIMEOUT: Duration = Duration::from_secs(2);

/// `manifest.json` の形式の版。
const FORMAT_VERSION: u32 = 1;

/// バックアップを取ったときの状態。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format_version: u32,
    /// 取った日時(UTC、ISO 8601)
    pub created_at: String,
    /// 写した元のフォルダ(記録用。戻すときには使わない)
    pub source_dir: String,
    /// `activeConfiguration` の値(なければ `default`)
    pub configuration_name: String,
    pub settings: FileRecord,
    pub editor_settings: FileRecord,
    /// リマップの定義ファイル(`<configuration_name>.json`)
    pub definition: FileRecord,
}

/// バックアップの対象のファイル 1 つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    /// 元のファイル名
    pub name: String,
    /// 取ったときに存在したか。存在しなければ写しはない
    pub existed: bool,
}

/// [`ensure`] の結果。
#[derive(Debug)]
pub enum Outcome {
    /// 今回作った
    Created(Manifest),
    /// 前に作ったものがあった(上書きしていない)
    Existing(Manifest),
}

impl Outcome {
    pub fn manifest(&self) -> &Manifest {
        match self {
            Outcome::Created(m) | Outcome::Existing(m) => m,
        }
    }
}

/// 既定のバックアップの場所(exe と同じフォルダの `initial-backup`)。exe の場所が分からなければ `None`。
pub fn default_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(DIR_NAME))
}

/// `backup_dir` にバックアップがなければ、`source_dir`(Keyboard Manager の設定フォルダ)から作る。
///
/// すでにあれば読んで返し、上書きしない。`backup_dir` はあるのに `manifest.json` を読めないときは
/// 作成済みとはみなさずエラーにする(中身が分からないものを上書きしない)。
pub fn ensure(backup_dir: &Path, source_dir: &Path, now: SystemTime) -> Result<Outcome, String> {
    if let Some(m) = load(backup_dir)? {
        return Ok(Outcome::Existing(m));
    }
    create(backup_dir, source_dir, now).map(Outcome::Created)
}

/// 作成済みのバックアップの記録を読む。`backup_dir` がなければ `Ok(None)`。
pub fn load(backup_dir: &Path) -> Result<Option<Manifest>, String> {
    if !backup_dir.exists() {
        return Ok(None);
    }
    let path = backup_dir.join(MANIFEST);
    let raw = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let m: Manifest =
        serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    validate(&m).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(m))
}

/// 記録の内容が使えるものか。定義ファイルの名前はパスの組み立てに使うので、安全な名前に限る。
fn validate(m: &Manifest) -> Result<(), String> {
    if m.format_version != FORMAT_VERSION {
        return Err(format!("unsupported formatVersion {}", m.format_version));
    }
    if !config::is_safe_configuration_name(&m.configuration_name)
        || m.definition.name != format!("{}.json", m.configuration_name)
    {
        return Err(format!(
            "invalid configuration name {:?}",
            m.configuration_name
        ));
    }
    Ok(())
}

/// 一時フォルダにすべてを書いてから `backup_dir` へ rename する。失敗したら一時フォルダを消す。
///
/// `source_dir` そのものがない(フォルダでない)ときは、空のバックアップを固定しないよう失敗にする。
fn create(backup_dir: &Path, source_dir: &Path, now: SystemTime) -> Result<Manifest, String> {
    match fs::metadata(source_dir) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(format!("{}: not a folder", source_dir.display())),
        Err(e) => return Err(format!("{}: {e}", source_dir.display())),
    }
    let definition_path = config::active_config_path(source_dir).map_err(path_error)?;
    let definition_name = definition_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: no file name", definition_path.display()))?
        .to_owned();
    let configuration_name = definition_name
        .strip_suffix(".json")
        .unwrap_or(&definition_name)
        .to_owned();

    let parent = backup_dir.parent().unwrap_or(Path::new(""));
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    remove_stale_tmp_dirs(parent);
    let tmp = parent.join(format!("{TMP_PREFIX}{}", std::process::id()));

    let result = (|| {
        fs::create_dir(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let settings = copy_if_exists(&source_dir.join(SETTINGS), &tmp.join(SETTINGS))?;
        let editor_settings = copy_if_exists(
            &source_dir.join(EDITOR_SETTINGS),
            &tmp.join(EDITOR_SETTINGS),
        )?;
        let definition_dir = tmp.join(DEFINITION_DIR);
        fs::create_dir(&definition_dir)
            .map_err(|e| format!("{}: {e}", definition_dir.display()))?;
        let definition = copy_if_exists(&definition_path, &definition_dir.join(&definition_name))?;

        let manifest = Manifest {
            format_version: FORMAT_VERSION,
            created_at: save::iso8601(now),
            source_dir: source_dir.display().to_string(),
            configuration_name,
            settings: FileRecord {
                name: SETTINGS.to_owned(),
                existed: settings,
            },
            editor_settings: FileRecord {
                name: EDITOR_SETTINGS.to_owned(),
                existed: editor_settings,
            },
            definition: FileRecord {
                name: definition_name,
                existed: definition,
            },
        };
        let text = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        let manifest_path = tmp.join(MANIFEST);
        write_synced(&manifest_path, &text)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;

        rename_dir(&tmp, backup_dir, RENAME_TIMEOUT, RENAME_RETRY_INTERVAL)
            .map_err(|e| format!("{}: {e}", backup_dir.display()))?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// `from` のフォルダを `to` へ rename する。失敗したら `interval` 間隔で `timeout` まで再試行する。
/// ただし `to` がすでにあるとき(別の起動が先に作った、など)は再試行しない。
fn rename_dir(from: &Path, to: &Path, timeout: Duration, interval: Duration) -> io::Result<()> {
    save::retry_unless(timeout, interval, || fs::rename(from, to), |_| to.exists())
}

/// 前回の起動で作りかけのまま残った一時フォルダを消す(失敗しても続ける)。
fn remove_stale_tmp_dirs(parent: &Path) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for e in entries.filter_map(|e| e.ok()) {
        let is_tmp = e
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(TMP_PREFIX));
        if is_tmp {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

/// `from` があれば `to` に写して `true`。なければ何もせず `false`。読めなければエラー。
fn copy_if_exists(from: &Path, to: &Path) -> Result<bool, String> {
    let contents = match fs::read(from) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", from.display())),
    };
    write_synced(to, &contents).map_err(|e| format!("{}: {e}", to.display()))?;
    Ok(true)
}

/// 新しいファイルに書いてディスクへ書き出す。
fn write_synced(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut f = fs::File::create_new(path)?;
    f.write_all(contents)?;
    f.sync_all()
}

fn path_error(e: PathError) -> String {
    match e {
        PathError::DirNotFound => "Keyboard Manager folder not found".to_owned(),
        PathError::SettingsUnreadable(d) => d,
        PathError::UnsafeConfiguration(v) => {
            format!("activeConfiguration is not a safe file name: {v:?}")
        }
    }
}

/// 戻すときに書き込む、リマップの定義ファイルの内容。
///
/// 取ったときに定義ファイルがなかったときは、リマップが 1 つもない構成(`remapKeys` などが空)。
/// 写しがリマップの定義として解析できなければエラーにする(戻したあとに読み込めなくなるため)。
pub fn definition_contents(backup_dir: &Path, m: &Manifest) -> Result<Vec<u8>, String> {
    validate(m)?;
    if !m.definition.existed {
        return config::to_json(&Config::default()).map(String::into_bytes);
    }
    let path = backup_dir.join(DEFINITION_DIR).join(&m.definition.name);
    let raw = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice::<Config>(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    const DEFAULT_JSON: &str = r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66"}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#;
    const SETTINGS_JSON: &str = r#"{"properties":{"activeConfiguration":{"value":"default"},"keyboardConfigurations":{"value":["default"]},"useNewEditor":true},"name":"Keyboard Manager","version":"1.0"}"#;
    const EDITOR_JSON: &str = r#"{"ShortcutSettingsDictionary":{}}"#;

    /// 一時ディレクトリに Keyboard Manager の設定フォルダとデータフォルダを用意する。
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let km = tmp.path().join("Keyboard Manager");
        fs::create_dir(&km).unwrap();
        fs::write(km.join(SETTINGS), SETTINGS_JSON).unwrap();
        fs::write(km.join(EDITOR_SETTINGS), EDITOR_JSON).unwrap();
        fs::write(km.join("default.json"), DEFAULT_JSON).unwrap();
        let backup = tmp.path().join("data").join(DIR_NAME);
        (tmp, km, backup)
    }

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn creates_backup_on_first_run() {
        let (_tmp, km, backup) = setup();
        let outcome = ensure(&backup, &km, now()).unwrap();
        let Outcome::Created(m) = outcome else {
            panic!("not created: {outcome:?}");
        };
        assert_eq!(m.created_at, "2023-11-14T22:13:20.123Z");
        assert_eq!(m.configuration_name, "default");
        assert_eq!(m.source_dir, km.display().to_string());
        assert!(m.settings.existed && m.editor_settings.existed && m.definition.existed);
        assert_eq!(m.definition.name, "default.json");

        assert_eq!(
            fs::read_to_string(backup.join(SETTINGS)).unwrap(),
            SETTINGS_JSON
        );
        assert_eq!(
            fs::read_to_string(backup.join(EDITOR_SETTINGS)).unwrap(),
            EDITOR_JSON
        );
        assert_eq!(
            fs::read_to_string(backup.join("config").join("default.json")).unwrap(),
            DEFAULT_JSON
        );
        assert_eq!(load(&backup).unwrap(), Some(m));
        // 一時フォルダは残らない
        assert_eq!(names_in(backup.parent().unwrap()), vec![DIR_NAME]);
        // 元のファイルは変えない
        assert_eq!(
            fs::read_to_string(km.join("default.json")).unwrap(),
            DEFAULT_JSON
        );
    }

    #[test]
    fn uses_active_configuration() {
        let (_tmp, km, backup) = setup();
        fs::write(
            km.join(SETTINGS),
            r#"{"properties":{"activeConfiguration":{"value":"gaming"}}}"#,
        )
        .unwrap();
        fs::write(km.join("gaming.json"), "{}").unwrap();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        assert_eq!(m.configuration_name, "gaming");
        assert_eq!(m.definition.name, "gaming.json");
        assert_eq!(
            fs::read_to_string(backup.join("config").join("gaming.json")).unwrap(),
            "{}"
        );
        assert!(!backup.join("config").join("default.json").exists());
    }

    #[test]
    fn does_not_overwrite_existing_backup() {
        let (_tmp, km, backup) = setup();
        let first = ensure(&backup, &km, now()).unwrap().manifest().clone();

        fs::write(km.join("default.json"), "{}").unwrap();
        fs::remove_file(km.join(SETTINGS)).unwrap();
        let later = now() + Duration::from_secs(60);
        let outcome = ensure(&backup, &km, later).unwrap();
        let Outcome::Existing(m) = outcome else {
            panic!("overwritten: {outcome:?}");
        };
        assert_eq!(m, first);
        assert_eq!(
            fs::read_to_string(backup.join("config").join("default.json")).unwrap(),
            DEFAULT_JSON
        );
        assert_eq!(
            fs::read_to_string(backup.join(SETTINGS)).unwrap(),
            SETTINGS_JSON
        );
    }

    #[test]
    fn records_missing_files() {
        let (_tmp, km, backup) = setup();
        fs::remove_file(km.join(SETTINGS)).unwrap();
        fs::remove_file(km.join(EDITOR_SETTINGS)).unwrap();
        fs::remove_file(km.join("default.json")).unwrap();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        assert!(!m.settings.existed);
        assert!(!m.editor_settings.existed);
        assert!(!m.definition.existed);
        // settings.json がないので構成名は default
        assert_eq!(m.definition.name, "default.json");
        assert!(!backup.join(SETTINGS).exists());
        assert!(!backup.join(EDITOR_SETTINGS).exists());
        assert!(!backup.join("config").join("default.json").exists());

        // 記録にも existed: false として残る
        let raw: serde_json::Value =
            serde_json::from_slice(&fs::read(backup.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(raw["settings"]["existed"], false);
        assert_eq!(raw["editorSettings"]["existed"], false);
        assert_eq!(raw["definition"]["existed"], false);
        assert_eq!(raw["definition"]["name"], "default.json");
    }

    #[test]
    fn records_partially_missing_files() {
        let (_tmp, km, backup) = setup();
        fs::remove_file(km.join(EDITOR_SETTINGS)).unwrap();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        assert!(m.settings.existed);
        assert!(!m.editor_settings.existed);
        assert!(m.definition.existed);
    }

    #[test]
    fn failure_midway_is_not_treated_as_created() {
        let (_tmp, km, backup) = setup();
        // settings.json を写したあと、editorSettings.json を読むところで失敗させる
        fs::remove_file(km.join(EDITOR_SETTINGS)).unwrap();
        fs::create_dir(km.join(EDITOR_SETTINGS)).unwrap();
        assert!(ensure(&backup, &km, now()).is_err());
        assert!(!backup.exists());
        assert_eq!(load(&backup).unwrap(), None);
        // 作りかけの一時フォルダは残らない
        assert!(names_in(backup.parent().unwrap()).is_empty());

        // 原因がなくなれば次の起動で作られる
        fs::remove_dir(km.join(EDITOR_SETTINGS)).unwrap();
        fs::write(km.join(EDITOR_SETTINGS), EDITOR_JSON).unwrap();
        assert!(matches!(
            ensure(&backup, &km, now()),
            Ok(Outcome::Created(_))
        ));
    }

    #[test]
    fn missing_source_dir_fails_without_creating() {
        let (tmp, km, backup) = setup();
        fs::remove_dir_all(&km).unwrap();
        assert!(ensure(&backup, &km, now()).is_err());
        assert!(!backup.exists());
        assert_eq!(load(&backup).unwrap(), None);

        // ファイルがフォルダの位置にあるときも失敗
        fs::write(&km, "x").unwrap();
        assert!(ensure(&backup, &km, now()).is_err());
        assert!(!backup.exists());

        // フォルダができれば次の起動で作られる(中のファイルがなくてもよい)
        fs::remove_file(&km).unwrap();
        fs::create_dir(&km).unwrap();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        assert!(!m.settings.existed && !m.editor_settings.existed && !m.definition.existed);
        drop(tmp);
    }

    #[test]
    fn rename_dir_does_not_retry_when_target_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from");
        let to = tmp.path().join("to");
        fs::create_dir(&from).unwrap();
        fs::create_dir(&to).unwrap();
        fs::write(to.join("x"), "x").unwrap();
        let start = std::time::Instant::now();
        assert!(
            rename_dir(
                &from,
                &to,
                Duration::from_secs(5),
                Duration::from_millis(10)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(from.exists());
        assert_eq!(names_in(&to), vec!["x"]);
    }

    #[test]
    fn rename_dir_retries_until_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from");
        fs::create_dir(&from).unwrap();
        // 置き換え先の親がないので失敗し続ける
        let to = tmp.path().join("missing").join("to");
        let timeout = Duration::from_millis(60);
        let start = std::time::Instant::now();
        assert!(rename_dir(&from, &to, timeout, Duration::from_millis(5)).is_err());
        assert!(start.elapsed() >= timeout);

        let to = tmp.path().join("to");
        rename_dir(&from, &to, timeout, Duration::from_millis(5)).unwrap();
        assert!(to.is_dir() && !from.exists());
    }

    #[test]
    fn unreadable_settings_fails_without_creating() {
        let (_tmp, km, backup) = setup();
        fs::write(km.join(SETTINGS), "not json").unwrap();
        assert!(ensure(&backup, &km, now()).is_err());
        assert!(!backup.exists());
    }

    #[test]
    fn leftover_tmp_dir_is_not_treated_as_created() {
        let (_tmp, km, backup) = setup();
        // 前回の起動で中断した作りかけ(manifest.json まで書いていても rename 前)
        let parent = backup.parent().unwrap();
        let stale = parent.join(format!("{TMP_PREFIX}999999"));
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join(SETTINGS), "partial").unwrap();
        assert_eq!(load(&backup).unwrap(), None);

        assert!(matches!(
            ensure(&backup, &km, now()),
            Ok(Outcome::Created(_))
        ));
        assert_eq!(names_in(parent), vec![DIR_NAME]);
    }

    #[test]
    fn backup_dir_without_manifest_is_error_and_not_overwritten() {
        let (_tmp, km, backup) = setup();
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("note.txt"), "x").unwrap();
        assert!(load(&backup).is_err());
        assert!(ensure(&backup, &km, now()).is_err());
        assert_eq!(names_in(&backup), vec!["note.txt"]);
    }

    #[test]
    fn unwritable_data_dir_fails() {
        let (tmp, km, _) = setup();
        let blocker = tmp.path().join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        let backup = blocker.join(DIR_NAME);
        assert!(ensure(&backup, &km, now()).is_err());
    }

    #[test]
    fn manifest_with_unsafe_name_is_rejected() {
        let (_tmp, km, backup) = setup();
        let mut m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        m.configuration_name = "../evil".to_owned();
        m.definition.name = "../evil.json".to_owned();
        fs::write(backup.join(MANIFEST), serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(load(&backup).is_err());
        assert!(definition_contents(&backup, &m).is_err());
    }

    #[test]
    fn definition_contents_returns_copy_or_empty_config() {
        let (_tmp, km, backup) = setup();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        assert_eq!(
            definition_contents(&backup, &m).unwrap(),
            DEFAULT_JSON.as_bytes()
        );

        let mut missing = m.clone();
        missing.definition.existed = false;
        let empty = definition_contents(&backup, &missing).unwrap();
        assert_eq!(
            String::from_utf8(empty).unwrap(),
            r#"{"remapKeys":{"inProcess":[]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#
        );
    }

    #[test]
    fn definition_contents_rejects_invalid_copy() {
        let (_tmp, km, backup) = setup();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        fs::write(backup.join("config").join("default.json"), "broken").unwrap();
        assert!(definition_contents(&backup, &m).is_err());
    }

    #[test]
    fn restore_writes_back_definition_with_generation_backup() {
        let (tmp, km, backup) = setup();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        let path = km.join("default.json");
        // 導入後に変更された定義ファイル
        let changed = r#"{"remapKeys":{"inProcess":[{"originalKeys":"67","newRemapKeys":"68"}]}}"#;
        fs::write(&path, changed).unwrap();
        fs::write(km.join(EDITOR_SETTINGS), "{\"changed\":1}").unwrap();

        let opts = save::Options::new(tmp.path().join("data").join("backups"));
        let contents = definition_contents(&backup, &m).unwrap();
        save::overwrite(&path, &contents, &opts).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_JSON);
        // 世代バックアップに戻す前の内容が残る
        let generations = names_in(&opts.backup_dir);
        assert_eq!(generations.len(), 1, "{generations:?}");
        assert_eq!(
            fs::read_to_string(opts.backup_dir.join(&generations[0])).unwrap(),
            changed
        );
        // ほかの 2 つは書き戻さない
        assert_eq!(
            fs::read_to_string(km.join(EDITOR_SETTINGS)).unwrap(),
            "{\"changed\":1}"
        );
        // 導入時のバックアップは変わらない
        assert_eq!(load(&backup).unwrap(), Some(m));
    }

    #[test]
    fn restore_of_missing_definition_writes_empty_config() {
        let (tmp, km, backup) = setup();
        let path = km.join("default.json");
        fs::remove_file(&path).unwrap();
        let m = ensure(&backup, &km, now()).unwrap().manifest().clone();
        fs::write(&path, DEFAULT_JSON).unwrap();

        let opts = save::Options::new(tmp.path().join("data").join("backups"));
        let contents = definition_contents(&backup, &m).unwrap();
        save::overwrite(&path, &contents, &opts).unwrap();
        let (cfg, _) = config::load(&path).unwrap();
        assert!(cfg.remap_keys.in_process.is_empty());
        assert_eq!(names_in(&opts.backup_dir).len(), 1);
    }

    #[test]
    fn default_dir_is_next_to_exe() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(default_dir(), Some(exe.parent().unwrap().join(DIR_NAME)));
    }
}
