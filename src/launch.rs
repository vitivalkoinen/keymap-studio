//! コマンドライン引数の解析(SPEC.md §4.5・§4.5.2)。
//!
//! - 引数なし: 通常の起動(常駐・ホットキー・自動起動の登録・二重起動の防止の対象)
//! - `--tray`([`TRAY_ARG`]): 通常の起動と同じだが、ウィンドウを出さずタスクトレイにだけ
//!   表示する(自動起動の登録に使う)
//! - それ以外の最初の引数: 読み書きするファイルのパス(開発・テスト用)。常駐などの対象外で、
//!   `--tray` を一緒に指定しても無視する

use std::ffi::OsString;
use std::path::PathBuf;

/// トレイだけで起動する引数。
pub const TRAY_ARG: &str = "--tray";

/// 起動のしかた。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// 引数でファイルを指定した
    File(PathBuf),
    /// 通常の起動。`tray_only` ならウィンドウを出さない
    Resident { tray_only: bool },
}

/// 引数(プログラム名を除いたもの)を解析する。
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Launch {
    let mut tray_only = false;
    for arg in args {
        if arg == TRAY_ARG {
            tray_only = true;
        } else {
            return Launch::File(PathBuf::from(arg));
        }
    }
    Launch::Resident { tray_only }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Launch {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_is_resident_with_window() {
        assert_eq!(parse_strs(&[]), Launch::Resident { tray_only: false });
    }

    #[test]
    fn tray_argument_starts_tray_only() {
        assert_eq!(
            parse_strs(&["--tray"]),
            Launch::Resident { tray_only: true }
        );
    }

    #[test]
    fn file_argument_is_not_resident() {
        assert_eq!(
            parse_strs(&["/mnt/c/x/default.json"]),
            Launch::File(PathBuf::from("/mnt/c/x/default.json"))
        );
        // ファイルの指定が優先し、トレイの引数は無視する
        assert_eq!(
            parse_strs(&["--tray", "a.json"]),
            Launch::File(PathBuf::from("a.json"))
        );
        assert_eq!(
            parse_strs(&["a.json", "--tray"]),
            Launch::File(PathBuf::from("a.json"))
        );
    }
}
