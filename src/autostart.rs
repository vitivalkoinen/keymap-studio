//! サインイン時の自動起動の登録(SPEC.md §4.5)。
//!
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` に、exe のフルパスとトレイだけで起動する
//! 引数([`launch::TRAY_ARG`])を [`VALUE_NAME`] の値として入れる。有効なら起動のたびに今の exe の
//! パスに合わせ、無効なら値を消す([`sync`])。
//!
//! 登録内容の組み立て([`command_line`])と、何をするかの判断([`plan`])は純粋な関数にして
//! テストする。レジストリを読み書きするのは Windows だけで、テストでは触らない。

// Windows 以外では使わない部分がある(Windows 専用の機能のため)
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::Path;

use crate::launch;

/// 登録先のキー(HKCU の下)。
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// 登録する値の名前。
pub const VALUE_NAME: &str = "Keymap Studio";

/// 登録する値(`"<exe のフルパス>" --tray`)。パスに空白があってもよいよう引用符で囲む。
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" {}", exe.display(), launch::TRAY_ARG)
}

/// 登録をどう変えるか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// 何もしない
    Keep,
    /// この値で登録する(新しく、または今の exe のパスに合わせて)
    Set(String),
    /// 値を消す
    Remove,
}

/// 自動起動が `enabled` で、今の登録が `current`(なければ `None`)、登録したい値が `desired` の
/// ときにすること。
pub fn plan(enabled: bool, current: Option<&str>, desired: &str) -> Plan {
    match (enabled, current) {
        (true, Some(c)) if c == desired => Plan::Keep,
        (true, _) => Plan::Set(desired.to_owned()),
        (false, Some(_)) => Plan::Remove,
        (false, None) => Plan::Keep,
    }
}

/// 今の登録の値を、比べられる形にする。
///
/// 値の種類が `REG_EXPAND_SZ`(`expandable`)なら、Windows がサインイン時に展開するのと同じく
/// 環境変数を `expand` で展開したものを返す(展開できなければそのまま)。`REG_SZ` はそのまま。
/// 本アプリが書くのは常に `REG_SZ` で、展開しないと今の exe のパスと比べられないため。
pub fn current_value(
    raw: &str,
    expandable: bool,
    expand: impl Fn(&str) -> Option<String>,
) -> String {
    if expandable {
        expand(raw).unwrap_or_else(|| raw.to_owned())
    } else {
        raw.to_owned()
    }
}

/// 登録を設定に合わせた結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Synced {
    /// 変えなかった
    Unchanged,
    /// 登録した(または値を今の exe に合わせた)
    Registered(String),
    /// 消した
    Removed,
    /// この OS では扱わない(Windows 以外。テストでも扱わない)
    Unsupported,
}

/// 登録を `enabled` に合わせる。
#[cfg(windows)]
pub fn sync(enabled: bool) -> Result<Synced, String> {
    // テストでは実際のレジストリに触らない
    if cfg!(test) {
        return Ok(Synced::Unsupported);
    }
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let desired = command_line(&exe);
    let current = registry::read()?
        .map(|(raw, expandable)| current_value(&raw, expandable, registry::expand));
    match plan(enabled, current.as_deref(), &desired) {
        Plan::Keep => Ok(Synced::Unchanged),
        Plan::Set(value) => registry::write(&value).map(|()| Synced::Registered(value)),
        Plan::Remove => registry::remove().map(|()| Synced::Removed),
    }
}

/// Windows 以外では登録しない。
#[cfg(not(windows))]
pub fn sync(_enabled: bool) -> Result<Synced, String> {
    Ok(Synced::Unsupported)
}

#[cfg(windows)]
mod registry {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RRF_NOEXPAND,
        RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    /// `REG_SZ` と `REG_EXPAND_SZ` を読み、`REG_EXPAND_SZ` は展開せずに返させる
    /// (展開は [`expand`] で行う。`RRF_RT_REG_EXPAND_SZ` は `RRF_NOEXPAND` と一緒に指定する必要がある)。
    const READ_FLAGS: u32 = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;

    use super::{RUN_KEY, VALUE_NAME};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn os_error(what: &str, code: u32) -> String {
        format!(
            "{what} HKCU\\{RUN_KEY}\\{VALUE_NAME}: {}",
            io::Error::from_raw_os_error(code as i32)
        )
    }

    /// 環境変数を展開する。失敗したら `None`。
    pub fn expand(raw: &str) -> Option<String> {
        let src = wide(raw);
        // SAFETY: `src` は NUL 終端。1 回目は必要な長さ(NUL を含む文字数)だけを得る。
        let needed = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), std::ptr::null_mut(), 0) };
        if needed == 0 {
            return None;
        }
        let mut buf = vec![0u16; needed as usize];
        // SAFETY: `buf` は `needed` 文字の書き込み先。
        let written = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), needed) };
        if written == 0 || written > needed {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    /// 今の登録の値(展開していないもの)と、`REG_EXPAND_SZ` か。なければ `None`。
    pub fn read() -> Result<Option<(String, bool)>, String> {
        let key = wide(RUN_KEY);
        let name = wide(VALUE_NAME);
        let mut size: u32 = 0;
        // SAFETY: 文字列は NUL 終端で、呼び出しの間は生きている。1 回目は大きさだけを得る。
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                READ_FLAGS,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(os_error("read", status));
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = (buf.len() * 2) as u32;
        let mut kind: REG_VALUE_TYPE = 0;
        // SAFETY: `buf` は `size` バイトの書き込み先で、呼び出しの間は生きている。
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                READ_FLAGS,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(os_error("read", status));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(Some((
            String::from_utf16_lossy(&buf[..len]),
            kind == REG_EXPAND_SZ,
        )))
    }

    pub fn write(value: &str) -> Result<(), String> {
        let key = wide(RUN_KEY);
        let name = wide(VALUE_NAME);
        let data = wide(value);
        // SAFETY: 文字列は NUL 終端で、呼び出しの間は生きている。大きさは NUL を含むバイト数。
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(os_error("write", status))
        }
    }

    pub fn remove() -> Result<(), String> {
        let key = wide(RUN_KEY);
        let name = wide(VALUE_NAME);
        // SAFETY: 文字列は NUL 終端で、呼び出しの間は生きている。
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(os_error("delete", status))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_the_exe_and_adds_tray_argument() {
        let exe = Path::new(r"C:\Users\me\Tools\Keymap Studio\keymap-studio.exe");
        assert_eq!(
            command_line(exe),
            r#""C:\Users\me\Tools\Keymap Studio\keymap-studio.exe" --tray"#
        );
    }

    #[test]
    fn plan_follows_setting_and_exe_path() {
        let desired = r#""C:\new\keymap-studio.exe" --tray"#;
        // 有効: なければ登録、違えば合わせ直す、同じなら何もしない
        assert_eq!(plan(true, None, desired), Plan::Set(desired.to_owned()));
        assert_eq!(
            plan(true, Some(r#""C:\old\keymap-studio.exe" --tray"#), desired),
            Plan::Set(desired.to_owned())
        );
        assert_eq!(plan(true, Some(desired), desired), Plan::Keep);
        // 無効: あれば消す
        assert_eq!(plan(false, Some(desired), desired), Plan::Remove);
        assert_eq!(plan(false, None, desired), Plan::Keep);
    }

    #[test]
    fn expandable_values_are_compared_after_expansion() {
        let expand = |s: &str| Some(s.replace("%LOCALAPPDATA%", r"C:\Users\me\AppData\Local"));
        let raw = r#""%LOCALAPPDATA%\ks\keymap-studio.exe" --tray"#;
        let desired = command_line(Path::new(r"C:\Users\me\AppData\Local\ks\keymap-studio.exe"));
        // REG_EXPAND_SZ は展開して比べるので、同じ exe を指していれば書き換えない
        let current = current_value(raw, true, expand);
        assert_eq!(current, desired);
        assert_eq!(plan(true, Some(&current), &desired), Plan::Keep);
        // REG_SZ は展開しない(Windows も展開しない)ので、違うものとして書き直す
        let current = current_value(raw, false, expand);
        assert_eq!(current, raw);
        assert_eq!(
            plan(true, Some(&current), &desired),
            Plan::Set(desired.clone())
        );
        // 展開できなければそのまま
        assert_eq!(current_value(raw, true, |_| None), raw);
    }

    #[test]
    fn sync_does_not_touch_the_registry_in_tests() {
        assert_eq!(sync(true), Ok(Synced::Unsupported));
        assert_eq!(sync(false), Ok(Synced::Unsupported));
    }
}
