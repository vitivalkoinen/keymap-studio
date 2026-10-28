//! 二重起動の防止(SPEC.md §4.5.2)。
//!
//! 名前付きの Mutex([`MUTEX_NAME`])で、すでに動いているかを判定する。動いている方への知らせには
//! 名前付きのイベント([`ACTIVATE_NAME`]、自動リセット)を使う。動いている方はタスクトレイの
//! スレッドでこのイベントを待ち([`crate::tray`])、シグナルされたらウィンドウを前面に出す。
//!
//! 名前は `Local\` の下に置き、同じサインインのセッションの中だけで 1 つにする。
//! Windows 以外では判定せず、常に最初の起動とみなす。

// Windows 以外では使わない部分がある(Windows 専用の機能のため)
#![cfg_attr(not(windows), allow(dead_code))]

/// 起動中であることを示す Mutex の名前。
pub const MUTEX_NAME: &str = r"Local\KeymapStudio.SingleInstance";

/// 動いている方にウィンドウを前面に出させるイベントの名前。
pub const ACTIVATE_NAME: &str = r"Local\KeymapStudio.Activate";

/// 判定の結果。
pub enum Instance {
    /// 最初の起動。プロセスが終わるまで持っておく
    Primary(Guard),
    /// すでに動いている
    AlreadyRunning(Secondary),
}

#[cfg(windows)]
pub use imp::{Guard, Secondary, acquire};

#[cfg(not(windows))]
pub use stub::{Guard, Secondary, acquire};

#[cfg(windows)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

    use super::{ACTIVATE_NAME, Instance, MUTEX_NAME};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 最初の起動が持つハンドル。落とすと閉じる。
    pub struct Guard {
        mutex: HANDLE,
        event: HANDLE,
    }

    impl Guard {
        /// 知らせを待つイベントのハンドル(スレッドに渡すため整数にする)。
        /// `Guard` が生きている間だけ有効。
        pub fn activation_event(&self) -> Option<usize> {
            Some(self.event as usize)
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: どちらも acquire で得た有効なハンドルで、ここでだけ閉じる。
            unsafe {
                CloseHandle(self.mutex);
                CloseHandle(self.event);
            }
        }
    }

    /// 後から起動した側。動いている方へ知らせられる。
    pub struct Secondary {
        event: HANDLE,
    }

    impl Secondary {
        /// 動いている方にウィンドウを前面に出させる。
        ///
        /// 前面に出す権利は、ユーザーが起動した(今前面にいる)こちらにあるので、
        /// `AllowSetForegroundWindow` で動いている方に譲ってからシグナルする。
        pub fn activate_running(&self) -> Result<(), String> {
            // SAFETY: `event` は acquire で得た有効なハンドル。
            unsafe {
                AllowSetForegroundWindow(ASFW_ANY);
                if SetEvent(self.event) == 0 {
                    return Err(format!("SetEvent: {}", io::Error::last_os_error()));
                }
            }
            Ok(())
        }
    }

    impl Drop for Secondary {
        fn drop(&mut self) {
            // SAFETY: acquire で得た有効なハンドルで、ここでだけ閉じる。
            unsafe {
                CloseHandle(self.event);
            }
        }
    }

    /// すでに動いているかを判定する。
    ///
    /// 先にイベントを作って(または開いて)から Mutex を作る。Mutex がすでにあれば後から起動した側。
    pub fn acquire() -> Result<Instance, String> {
        let event_name = wide(ACTIVATE_NAME);
        let mutex_name = wide(MUTEX_NAME);
        // SAFETY: 名前は NUL 終端で、呼び出しの間は生きている。得たハンドルは Guard・Secondary が
        // 閉じるか、ここで閉じる。
        unsafe {
            let event = CreateEventW(std::ptr::null(), 0, 0, event_name.as_ptr());
            if event.is_null() {
                return Err(format!("CreateEventW: {}", io::Error::last_os_error()));
            }
            let mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
            let last = GetLastError();
            if mutex.is_null() {
                let e = io::Error::from_raw_os_error(last as i32);
                CloseHandle(event);
                return Err(format!("CreateMutexW: {e}"));
            }
            if last == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                return Ok(Instance::AlreadyRunning(Secondary { event }));
            }
            Ok(Instance::Primary(Guard { mutex, event }))
        }
    }
}

#[cfg(not(windows))]
mod stub {
    use super::Instance;

    pub struct Guard;

    impl Guard {
        pub fn activation_event(&self) -> Option<usize> {
            None
        }
    }

    pub struct Secondary;

    impl Secondary {
        pub fn activate_running(&self) -> Result<(), String> {
            Ok(())
        }
    }

    /// Windows 以外では判定しない。
    pub fn acquire() -> Result<Instance, String> {
        Ok(Instance::Primary(Guard))
    }
}
