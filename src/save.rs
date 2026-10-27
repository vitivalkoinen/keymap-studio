//! 設定ファイルの保存(SPEC.md §4.8)。
//!
//! 手順:
//! 1. 同じフォルダの `editorTransaction.lock` を排他的に開く(PowerToys の新エディタと同じ方式)
//! 2. 現在のファイルの内容を読み、読み込んだ時点の内容と違えば中止する
//! 3. 現在のファイルを世代バックアップとして残す
//! 4. 同じフォルダの一時ファイルに書いてから rename で置き換える
//!
//! 書き込みが成功したあとのエンジンへの通知は [`signal_settings_changed`] で行う。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::{self, Config};

/// PowerToys の新エディタが書き込みの間に排他的に開くファイル。
pub const LOCK_FILE: &str = "editorTransaction.lock";

/// エンジンに設定の読み直しを促す名前付きイベント(PowerToys の `SettingsEventName`)。
#[cfg(windows)]
pub const SETTINGS_EVENT_NAME: &str = "PowerToys_KeyboardManager_Event_Settings";

/// 既定で残すバックアップの世代数。
pub const DEFAULT_GENERATIONS: usize = 10;

/// ロックを取る再試行の間隔。
#[cfg(windows)]
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// ロックを取れないときに諦めるまでの時間。
#[cfg(windows)]
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// 保存の設定。テストではバックアップ先を一時ディレクトリに差し替える。
#[derive(Debug, Clone)]
pub struct Options {
    /// 世代バックアップを置くフォルダ
    pub backup_dir: PathBuf,
    /// 残すバックアップの世代数
    pub generations: usize,
}

impl Options {
    pub fn new(backup_dir: PathBuf) -> Self {
        Self {
            backup_dir,
            generations: DEFAULT_GENERATIONS,
        }
    }
}

/// 既定のバックアップ先(exe と同じフォルダの `backups`)。exe の場所が分からなければ `None`。
///
/// フォルダは保存のときに作る。作れない・書き込めないときは [`SaveError::Backup`] で保存を中止する。
pub fn default_backup_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("backups"))
}

/// 保存を中止した理由。詳細の文字列は OS などのエラーをそのまま表示するためのもの。
#[derive(Debug)]
pub enum SaveError {
    /// 読み込んだあとにファイルが他のプログラムに変更された(削除も含む)
    Modified,
    /// `editorTransaction.lock` を時間内に取れなかった(最後のエラー)。
    /// Windows 以外ではロックを取らないので作られない
    #[cfg_attr(not(windows), allow(dead_code))]
    LockTimeout(String),
    /// 現在のファイルを読めなかった
    Read(String),
    /// バックアップを作れなかった
    Backup(String),
    /// 書き込みか置き換えに失敗した
    Write(String),
}

/// `cfg` を `path` に保存する。`snapshot` は読み込んだ時点のファイルの内容。
///
/// 成功したら、書き込んだ内容(次の保存で `snapshot` として使う)を返す。
pub fn save(
    path: &Path,
    cfg: &Config,
    snapshot: &[u8],
    opts: &Options,
) -> Result<Vec<u8>, SaveError> {
    let text = config::to_json(cfg).map_err(SaveError::Write)?;
    write_locked(path, text.as_bytes(), Some(snapshot), opts)?;
    Ok(text.into_bytes())
}

/// `path` を `contents` でそのまま置き換える(導入前の設定に戻す操作。SPEC.md §4.7)。
///
/// 手順は [`save`] と同じだが、外部の変更は検知しない(ユーザーが明示的に戻すと決めた操作のため)。
/// 今のファイルがあれば、その内容を世代バックアップに残してから置き換える。今のファイルが
/// なければバックアップせずに作る。
pub fn overwrite(path: &Path, contents: &[u8], opts: &Options) -> Result<(), SaveError> {
    write_locked(path, contents, None, opts)
}

/// ロックを取り、今のファイルを世代バックアップに残してから `contents` で置き換える。
///
/// `snapshot` があれば、今のファイルの内容がそれと違う(削除も含む)ときに中止する。
fn write_locked(
    path: &Path,
    contents: &[u8],
    snapshot: Option<&[u8]>,
    opts: &Options,
) -> Result<(), SaveError> {
    let dir = path.parent().unwrap_or(Path::new(""));
    let _lock = lock(&dir.join(LOCK_FILE))?;

    let current = match fs::read(path) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(SaveError::Read(format!("{}: {e}", path.display()))),
    };
    if let Some(snapshot) = snapshot
        && current.as_deref() != Some(snapshot)
    {
        return Err(SaveError::Modified);
    }

    if let Some(current) = &current {
        backup(path, current, opts, SystemTime::now()).map_err(SaveError::Backup)?;
    }
    write_replace(path, contents).map_err(|e| SaveError::Write(format!("{}: {e}", path.display())))
}

/// 書き込みの間 `editorTransaction.lock` を開いたままにしておくための値。
/// 落とすとファイルが閉じ、ロックが外れる。
struct LockGuard {
    _file: Option<File>,
}

/// `editorTransaction.lock` を共有なし(`FileShare.None` 相当)で開く。
/// 取れなければ 50ms 間隔で再試行し、10 秒でタイムアウトとする。
#[cfg(windows)]
fn lock(path: &Path) -> Result<LockGuard, SaveError> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = retry(LOCK_TIMEOUT, LOCK_RETRY_INTERVAL, || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            // 中身は使わないので、既存のファイルはそのまま開く(.NET の FileMode.OpenOrCreate 相当)
            .truncate(false)
            .share_mode(0)
            .open(path)
    })
    .map_err(|e| SaveError::LockTimeout(format!("{}: {e}", path.display())))?;
    Ok(LockGuard { _file: Some(file) })
}

/// Windows 以外ではロックを取らない。
///
/// 共有モードによる排他は Windows の仕組みで、PowerToys も Windows でしか動かない。
/// Windows 以外で動かすのは開発中の確認のためだけなので、ロックなしで書き込む。
#[cfg(not(windows))]
fn lock(_path: &Path) -> Result<LockGuard, SaveError> {
    Ok(LockGuard { _file: None })
}

/// `f` が成功するまで `interval` 間隔で呼び直す。`timeout` を過ぎたら最後のエラーを返す。
#[cfg(any(windows, test))]
fn retry<T>(
    timeout: Duration,
    interval: Duration,
    f: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    retry_unless(timeout, interval, f, |_| false)
}

/// [`retry`] と同じだが、`give_up` が `true` を返すエラーでは呼び直さずにすぐ返す。
pub fn retry_unless<T>(
    timeout: Duration,
    interval: Duration,
    mut f: impl FnMut() -> io::Result<T>,
    give_up: impl Fn(&io::Error) -> bool,
) -> io::Result<T> {
    let start = std::time::Instant::now();
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if give_up(&e) || start.elapsed() >= timeout => return Err(e),
            Err(_) => std::thread::sleep(interval),
        }
    }
}

/// 同じフォルダの一時ファイルに `contents` を書いてから、rename で `path` を置き換える。
fn write_replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".{}.keymap-studio.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);

    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `contents`(上書きする前の `path` の内容)を世代バックアップとして残し、
/// 古いものを `opts.generations` 世代まで減らす。
///
/// ファイル名は `<元の名前の stem>-<UTC の日時>.json`(例: `default-20261028T123456.789Z.json`)。
/// 同じ時刻のものがあれば `-1`、`-2`… を付ける。
fn backup(path: &Path, contents: &[u8], opts: &Options, now: SystemTime) -> Result<(), String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("{}: no file name", path.display()))?;
    let dir = &opts.backup_dir;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let stamp = timestamp(now);
    for n in 0.. {
        let name = if n == 0 {
            format!("{stem}-{stamp}.json")
        } else {
            format!("{stem}-{stamp}-{n}.json")
        };
        let target = dir.join(name);
        let mut f = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", target.display())),
        };
        let written = f.write_all(contents).and_then(|()| f.sync_all());
        if let Err(e) = written {
            drop(f);
            let _ = fs::remove_file(&target);
            return Err(format!("{}: {e}", target.display()));
        }
        break;
    }

    prune(dir, stem, opts.generations);
    Ok(())
}

/// `stem` のバックアップのうち、新しいものから `keep` 個を残して削除する。
/// 削除に失敗しても保存は続ける(次の保存で再び削除を試みる)。
fn prune(dir: &Path, stem: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<((String, u32), PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((parse_backup_name(&name, stem)?, e.path()))
        })
        .collect();
    if backups.len() <= keep {
        return;
    }
    backups.sort();
    let excess = backups.len() - keep;
    for (_, p) in backups.into_iter().take(excess) {
        let _ = fs::remove_file(p);
    }
}

/// バックアップのファイル名から `(日時, 連番)` を取り出す。`stem` のバックアップでなければ `None`。
fn parse_backup_name(name: &str, stem: &str) -> Option<(String, u32)> {
    let rest = name.strip_prefix(stem)?.strip_prefix('-')?;
    let rest = rest.strip_suffix(".json")?;
    // 日時は "YYYYMMDDTHHMMSS.mmmZ" の 20 文字
    let (stamp, suffix) = (rest.get(..20)?, rest.get(20..)?);
    let shape_ok = stamp.char_indices().all(|(i, c)| match i {
        8 => c == 'T',
        15 => c == '.',
        19 => c == 'Z',
        _ => c.is_ascii_digit(),
    });
    if !shape_ok {
        return None;
    }
    let n = match suffix {
        "" => 0,
        s => {
            let digits = s.strip_prefix('-')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()?
        }
    };
    Some((stamp.to_owned(), n))
}

/// UTC の日時を `YYYYMMDDTHHMMSS.mmmZ` の形にする。
fn timestamp(t: SystemTime) -> String {
    let (y, m, day, h, min, sec, ms) = utc_parts(t);
    format!("{y:04}{m:02}{day:02}T{h:02}{min:02}{sec:02}.{ms:03}Z")
}

/// UTC の日時を ISO 8601 の拡張形式 `YYYY-MM-DDTHH:MM:SS.mmmZ` にする(記録や表示用)。
pub fn iso8601(t: SystemTime) -> String {
    let (y, m, day, h, min, sec, ms) = utc_parts(t);
    format!("{y:04}-{m:02}-{day:02}T{h:02}:{min:02}:{sec:02}.{ms:03}Z")
}

/// UTC の (年, 月, 日, 時, 分, 秒, ミリ秒)。
fn utc_parts(t: SystemTime) -> (i64, u32, u32, u64, u64, u64, u32) {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (days, sod) = ((secs / 86_400) as i64, secs % 86_400);
    let (y, m, day) = civil_from_days(days);
    (
        y,
        m,
        day,
        sod / 3600,
        sod % 3600 / 60,
        sod % 60,
        d.subsec_millis(),
    )
}

/// 1970-01-01 からの日数を (年, 月, 日) にする(グレゴリオ暦。Howard Hinnant の方法)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// エンジンへの通知に失敗した理由。
#[derive(Debug)]
pub enum SignalError {
    /// Windows 以外では通知できない
    #[cfg_attr(windows, allow(dead_code))]
    Unsupported,
    /// イベントがない(PowerToys の Keyboard Manager が動いていない、など)。
    /// Windows 以外では作られない
    #[cfg_attr(not(windows), allow(dead_code))]
    EventNotFound,
    /// OS のエラー。Windows 以外では作られない
    #[cfg_attr(not(windows), allow(dead_code))]
    Os(String),
}

/// 名前付きイベント `PowerToys_KeyboardManager_Event_Settings` をシグナルし、
/// エンジンに設定を読み直させる。
///
/// PowerToys 本体(`SaveSettingsToFile`)は `CreateEvent` で作るか開いてから `SetEvent` するが、
/// ここでは `OpenEventW(EVENT_MODIFY_STATE, FALSE, name)` で既存のイベントだけを開く。
/// `CreateEventW` だと、エンジンが動いておらずイベントがないときに誰も待っていない
/// イベントを新しく作ってシグナルし、成功と区別できない。開けなければ反映されていない
/// 可能性があるものとして失敗を返す。
#[cfg(windows)]
pub fn signal_settings_changed() -> Result<(), SignalError> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND};
    use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};

    let name: Vec<u16> = SETTINGS_EVENT_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `name` は NUL 終端の UTF-16 で、呼び出しの間は生きている。
    // 得たハンドルはこの関数の中で必ず閉じる。
    unsafe {
        let handle = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if handle.is_null() {
            let e = io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
                SignalError::EventNotFound
            } else {
                SignalError::Os(e.to_string())
            });
        }
        let result = if SetEvent(handle) == 0 {
            Err(SignalError::Os(io::Error::last_os_error().to_string()))
        } else {
            Ok(())
        };
        CloseHandle(handle);
        result
    }
}

/// Windows 以外では PowerToys が動かないので通知しない。
#[cfg(not(windows))]
pub fn signal_settings_changed() -> Result<(), SignalError> {
    Err(SignalError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeyCode;
    use crate::edit;
    use serde_json::Value;

    const SAMPLE: &str = r#"{"remapKeys":{"inProcess":[{"originalKeys":"2147483685","newRemapKeys":"100","futureField":1}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]},"topLevelExtra":"x"}"#;

    /// 設定フォルダとバックアップ先を一時ディレクトリに用意する。
    fn setup() -> (tempfile::TempDir, PathBuf, Options) {
        let tmp = tempfile::tempdir().unwrap();
        let km = tmp.path().join("Keyboard Manager");
        fs::create_dir(&km).unwrap();
        let path = km.join("default.json");
        fs::write(&path, SAMPLE).unwrap();
        let opts = Options::new(tmp.path().join("backups"));
        (tmp, path, opts)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn save_replaces_file_and_keeps_unknown_fields() {
        let (_tmp, path, opts) = setup();
        let (mut cfg, snapshot) = config::load(&path).unwrap();
        edit::set(
            &mut cfg.remap_keys.in_process,
            KeyCode(0x41),
            KeyCode(0x100),
        );

        let written = save(&path, &cfg, &snapshot, &opts).unwrap();
        assert_eq!(fs::read(&path).unwrap(), written);
        let out: Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(out["topLevelExtra"], "x");
        assert_eq!(out["remapKeys"]["inProcess"][0]["futureField"], 1);
        assert_eq!(out["remapKeys"]["inProcess"][1]["originalKeys"], "65");
        assert_eq!(out["remapKeys"]["inProcess"][1]["newRemapKeys"], "256");

        // 一時ファイルは残らない(置き換え先のフォルダにあるのは本体とロックだけ)
        let names = files_in(path.parent().unwrap());
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        assert!(names.contains(&"default.json".to_owned()));
    }

    #[test]
    fn write_replace_cleans_up_temp_file_on_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // 置き換え先がフォルダなら rename に失敗する。元のものは残り、一時ファイルは消える
        let target = tmp.path().join("default.json");
        fs::create_dir(&target).unwrap();
        assert!(write_replace(&target, b"{}").is_err());
        assert!(target.is_dir());
        assert_eq!(files_in(tmp.path()), vec!["default.json"]);
    }

    #[test]
    fn save_backs_up_previous_contents() {
        let (_tmp, path, opts) = setup();
        let (cfg, snapshot) = config::load(&path).unwrap();
        save(&path, &cfg, &snapshot, &opts).unwrap();

        let backups = files_in(&opts.backup_dir);
        assert_eq!(backups.len(), 1);
        assert!(
            parse_backup_name(&backups[0], "default").is_some(),
            "{backups:?}"
        );
        assert_eq!(
            fs::read(opts.backup_dir.join(&backups[0])).unwrap(),
            SAMPLE.as_bytes()
        );
    }

    #[test]
    fn consecutive_saves_use_returned_snapshot() {
        let (_tmp, path, opts) = setup();
        let (mut cfg, snapshot) = config::load(&path).unwrap();
        let snapshot = save(&path, &cfg, &snapshot, &opts).unwrap();
        edit::remove(&mut cfg.remap_keys.in_process, KeyCode(0x8000_0025));
        let written = save(&path, &cfg, &snapshot, &opts).unwrap();
        // 2 回目のバックアップは 1 回目に書いた内容
        let backups = files_in(&opts.backup_dir);
        assert_eq!(backups.len(), 2);
        assert_eq!(
            fs::read(opts.backup_dir.join(&backups[1])).unwrap(),
            snapshot
        );
        let out: Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(out["remapKeys"]["inProcess"], serde_json::json!([]));
    }

    #[test]
    fn save_aborts_when_file_changed_externally() {
        let (_tmp, path, opts) = setup();
        let (cfg, snapshot) = config::load(&path).unwrap();
        let external = SAMPLE.replace("\"100\"", "\"101\"");
        fs::write(&path, &external).unwrap();

        assert!(matches!(
            save(&path, &cfg, &snapshot, &opts),
            Err(SaveError::Modified)
        ));
        // 上書きもバックアップもしない
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        assert!(!opts.backup_dir.exists());
    }

    #[test]
    fn save_aborts_when_backup_dir_is_not_writable() {
        let (tmp, path, mut opts) = setup();
        // ファイルの下にはフォルダを作れない
        let blocker = tmp.path().join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        opts.backup_dir = blocker.join("backups");
        let (mut cfg, snapshot) = config::load(&path).unwrap();
        edit::set(&mut cfg.remap_keys.in_process, KeyCode(0x41), KeyCode(0x42));

        assert!(matches!(
            save(&path, &cfg, &snapshot, &opts),
            Err(SaveError::Backup(_))
        ));
        // 上書きしない
        assert_eq!(fs::read_to_string(&path).unwrap(), SAMPLE);
    }

    #[test]
    fn default_backup_dir_is_next_to_exe() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            default_backup_dir(),
            Some(exe.parent().unwrap().join("backups"))
        );
    }

    #[test]
    fn save_aborts_when_file_deleted_externally() {
        let (_tmp, path, opts) = setup();
        let (cfg, snapshot) = config::load(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(matches!(
            save(&path, &cfg, &snapshot, &opts),
            Err(SaveError::Modified)
        ));
        assert!(!path.exists());
    }

    #[test]
    fn backup_keeps_limited_generations() {
        let (tmp, path, mut opts) = setup();
        opts.generations = 3;
        // 別の構成のバックアップと無関係なファイルは消さない
        fs::create_dir_all(&opts.backup_dir).unwrap();
        fs::write(
            opts.backup_dir.join("gaming-20000101T000000.000Z.json"),
            "g",
        )
        .unwrap();
        fs::write(opts.backup_dir.join("default-notes.json"), "n").unwrap();

        let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for i in 0..5u64 {
            let now = base + Duration::from_secs(i);
            backup(&path, format!("v{i}").as_bytes(), &opts, now).unwrap();
        }
        let names = files_in(&opts.backup_dir);
        let ours: Vec<_> = names
            .iter()
            .filter(|n| parse_backup_name(n, "default").is_some())
            .collect();
        assert_eq!(ours.len(), 3, "{names:?}");
        let contents: Vec<String> = ours
            .iter()
            .map(|n| fs::read_to_string(opts.backup_dir.join(n)).unwrap())
            .collect();
        assert_eq!(contents, vec!["v2", "v3", "v4"]);
        assert!(names.contains(&"gaming-20000101T000000.000Z.json".to_owned()));
        assert!(names.contains(&"default-notes.json".to_owned()));
        drop(tmp);
    }

    #[test]
    fn backup_with_same_timestamp_gets_suffix_and_order() {
        let (_tmp, path, mut opts) = setup();
        opts.generations = 2;
        let now = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        for i in 0..3 {
            backup(&path, format!("v{i}").as_bytes(), &opts, now).unwrap();
        }
        let names = files_in(&opts.backup_dir);
        assert_eq!(
            names,
            vec![
                "default-20231114T221320.123Z-1.json",
                "default-20231114T221320.123Z-2.json"
            ]
        );
    }

    #[test]
    fn overwrite_ignores_snapshot_and_backs_up_current() {
        let (_tmp, path, opts) = setup();
        // 読み込んだあとに外部で変更されていても、今の内容をバックアップして置き換える
        let external = SAMPLE.replace("\"100\"", "\"101\"");
        fs::write(&path, &external).unwrap();
        overwrite(&path, b"{}", &opts).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        let backups = files_in(&opts.backup_dir);
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(opts.backup_dir.join(&backups[0])).unwrap(),
            external
        );
        let names = files_in(path.parent().unwrap());
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn overwrite_creates_missing_file_without_backup() {
        let (_tmp, path, opts) = setup();
        fs::remove_file(&path).unwrap();
        overwrite(&path, b"{}", &opts).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        assert!(!opts.backup_dir.exists());
    }

    #[test]
    fn iso8601_is_utc() {
        let t = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(iso8601(t), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn timestamp_is_utc() {
        assert_eq!(timestamp(UNIX_EPOCH), "19700101T000000.000Z");
        let t = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(timestamp(t), "20231114T221320.123Z");
        // うるう日
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(timestamp(t), "20000229T000000.000Z");
    }

    #[test]
    fn parse_backup_name_rejects_other_files() {
        assert_eq!(
            parse_backup_name("default-20231114T221320.123Z.json", "default"),
            Some(("20231114T221320.123Z".to_owned(), 0))
        );
        assert_eq!(
            parse_backup_name("default-20231114T221320.123Z-12.json", "default"),
            Some(("20231114T221320.123Z".to_owned(), 12))
        );
        for bad in [
            "default-20231114T221320.123Z-.json",
            "default-20231114T221320.123Z-x.json",
            "default-20231114X221320.123Z.json",
            "default-2023.json",
            "default-x-20231114T221320.123Z.json",
            "gaming-20231114T221320.123Z.json",
            "default-20231114T221320.123Z.txt",
        ] {
            assert_eq!(parse_backup_name(bad, "default"), None, "{bad}");
        }
    }

    #[test]
    fn retry_succeeds_after_failures() {
        let mut calls = 0;
        let r = retry(Duration::from_secs(5), Duration::from_millis(1), || {
            calls += 1;
            if calls < 3 {
                Err(io::Error::other("busy"))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(r.unwrap(), 3);
    }

    #[test]
    fn retry_unless_gives_up_immediately() {
        let mut calls = 0;
        let start = std::time::Instant::now();
        let r: io::Result<()> = retry_unless(
            Duration::from_secs(5),
            Duration::from_millis(1),
            || {
                calls += 1;
                Err(io::Error::other("fatal"))
            },
            |_| true,
        );
        assert!(r.is_err());
        assert_eq!(calls, 1);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn retry_times_out_with_last_error() {
        let start = std::time::Instant::now();
        let r: io::Result<()> = retry(Duration::from_millis(30), Duration::from_millis(5), || {
            Err(io::Error::other("busy"))
        });
        assert_eq!(r.unwrap_err().to_string(), "busy");
        assert!(start.elapsed() >= Duration::from_millis(30));
    }
}
