//! 本アプリの短いログ。
//!
//! データフォルダ(exe と同じフォルダ)の `keymap-studio.log` に、日時(UTC)付きで 1 行ずつ
//! 追記する。大きくなりすぎないよう、上限を超えたら `keymap-studio.log.old` に回す
//! (前の `.old` は置き換える)。書けなくてもアプリの動作は止めない。

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::save;

/// ログのファイル名。
pub const FILE_NAME: &str = "keymap-studio.log";

/// これを超えたら `.old` に回す大きさ(バイト)。
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;

/// ログの書き込み先。
#[derive(Debug, Clone)]
pub struct Log {
    /// 書き込むファイル。`None` なら何もしない(exe の場所が分からないとき)
    path: Option<PathBuf>,
    max_bytes: u64,
}

impl Log {
    pub fn new(path: Option<PathBuf>, max_bytes: u64) -> Self {
        Self { path, max_bytes }
    }

    /// 既定の書き込み先(exe と同じフォルダの `keymap-studio.log`)。
    pub fn default_location() -> Self {
        let path = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join(FILE_NAME)));
        Self::new(path, DEFAULT_MAX_BYTES)
    }

    /// 書き込まないログ(テスト用)。
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self::new(None, DEFAULT_MAX_BYTES)
    }

    /// 1 行追記する。失敗しても何もしない。
    pub fn write(&self, message: &str) {
        if let Some(path) = &self.path {
            let _ = append(path, self.max_bytes, SystemTime::now(), message);
        }
    }
}

/// `path` に `<日時> <message>` の 1 行を追記する。追記する前に `max_bytes` 以上になっていれば
/// `.old` に回す。`message` の改行は空白にして、必ず 1 行にする。
fn append(path: &Path, max_bytes: u64, now: SystemTime, message: &str) -> io::Result<()> {
    if fs::metadata(path).is_ok_and(|m| m.len() >= max_bytes) {
        // 回せなくても(別の起動が開いている、など)、この行は書く
        let _ = fs::rename(path, old_path(path));
    }
    let one_line: String = message
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(format!("{} {one_line}\n", save::iso8601(now)).as_bytes())
}

/// 回した先のパス(`keymap-studio.log.old`)。
fn old_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn t() -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)
    }

    #[test]
    fn appends_lines_with_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(FILE_NAME);
        append(&path, DEFAULT_MAX_BYTES, t(), "first").unwrap();
        append(&path, DEFAULT_MAX_BYTES, t(), "second\nline\r\n").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "2023-11-14T22:13:20.123Z first\n2023-11-14T22:13:20.123Z second line  \n"
        );
    }

    #[test]
    fn rotates_when_over_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let old = tmp.path().join("keymap-studio.log.old");
        fs::write(&old, "older").unwrap();
        fs::write(&path, "x".repeat(100)).unwrap();

        append(&path, 100, t(), "after").unwrap();
        // 上限に達していたものが .old になり(前の .old は置き換わる)、新しいファイルに書く
        assert_eq!(fs::read_to_string(&old).unwrap(), "x".repeat(100));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "2023-11-14T22:13:20.123Z after\n"
        );

        // 上限未満なら回さない
        append(&path, 100, t(), "more").unwrap();
        assert_eq!(fs::read_to_string(&old).unwrap(), "x".repeat(100));
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
    }

    #[test]
    fn write_failure_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        // 書き込み先の親がファイルなので書けない
        let blocker = tmp.path().join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        let path = blocker.join(FILE_NAME);
        assert!(append(&path, DEFAULT_MAX_BYTES, t(), "x").is_err());
        // Log::write はパニックせずに戻る
        Log::new(Some(path), DEFAULT_MAX_BYTES).write("x");
        Log::disabled().write("x");
    }

    #[test]
    fn default_location_is_next_to_exe() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            Log::default_location().path,
            Some(exe.parent().unwrap().join(FILE_NAME))
        );
    }
}
