//! トレイだけで起動したときに、ウィンドウを一瞬も見せないための DWM のクローク(SPEC.md §4.5)。
//!
//! eframe 0.37 は最初のフレームを描いたあと、ウィンドウを必ず表示する
//! (`native/epi_integration.rs` の `post_rendering` が `set_visible(true)` を呼ぶ)。アプリはその
//! フレームの指示で `Visible(false)` を送って隠すが、その間に一瞬見えてしまう。
//!
//! そこで、アプリを作るとき(ウィンドウはできているが、まだ何も描いていない)にウィンドウを
//! `DwmSetWindowAttribute(DWMWA_CLOAK, TRUE)` でクロークしておく。クロークしたウィンドウは表示中でも
//! 画面に合成されない。クロークしたまま `Visible(false)` で隠し、ユーザーがウィンドウを開くときに、
//! 隠れている間に([`Cloak::uncloak`])クロークを外してから `Visible(true)` を送る。
//! 隠れている間に外すので、外したときにも何も見えない。
//!
//! クロークできなかったときは、今までどおり(一瞬見えることがある)動く。Windows 以外では何もしない。

/// 起動時にクロークするか。トレイだけで起動し、トレイが使える(隠したウィンドウを開く手段がある)
/// ときだけ。
pub fn should_cloak_at_start(start_hidden: bool, tray_started: bool) -> bool {
    start_hidden && tray_started
}

/// メインのウィンドウのクロークの状態。
#[derive(Debug, Default)]
pub struct Cloak {
    /// ウィンドウのハンドル(HWND を整数にしたもの)。取れなければ `None`
    hwnd: Option<usize>,
    cloaked: bool,
}

impl Cloak {
    pub fn new(hwnd: Option<usize>) -> Self {
        Self {
            hwnd,
            cloaked: false,
        }
    }

    /// クロークした状態(テスト用。ウィンドウはない)。
    #[cfg(test)]
    pub fn cloaked_for_test() -> Self {
        Self {
            hwnd: None,
            cloaked: true,
        }
    }

    /// クロークを外せない状態(テスト用)。無効なハンドルなので、外す呼び出しは必ず失敗する
    /// (Windows 以外ではクロークの操作そのものが失敗する)。
    #[cfg(test)]
    pub fn unremovable_for_test() -> Self {
        Self {
            hwnd: Some(1),
            cloaked: true,
        }
    }

    /// クロークしているか(テスト用)。
    #[cfg(test)]
    pub fn is_cloaked(&self) -> bool {
        self.cloaked
    }

    /// クロークする。できなければ理由を返し、クロークしていない状態のままにする。
    pub fn cloak(&mut self) -> Result<(), String> {
        let hwnd = self.hwnd.ok_or("the window handle is not available")?;
        set_cloaked(hwnd, true)?;
        self.cloaked = true;
        Ok(())
    }

    /// クロークを外す(していなければ何もしない)。ウィンドウを隠している間に呼ぶこと。
    ///
    /// 外せなかったときはクロークした状態のままにし、理由を返す(ログに残す)。次にウィンドウを
    /// 開くときにもう一度試す。外したものとして扱うと、ウィンドウが見えないままになるため。
    pub fn uncloak(&mut self) -> Result<(), String> {
        if !self.cloaked {
            return Ok(());
        }
        if let Some(hwnd) = self.hwnd {
            set_cloaked(hwnd, false)?;
        }
        self.cloaked = false;
        Ok(())
    }
}

/// アプリを作るときのウィンドウのハンドル。
#[cfg(windows)]
pub fn hwnd_of(cc: &eframe::CreationContext<'_>) -> Option<usize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as usize),
        _ => None,
    }
}

/// Windows 以外ではクロークしない。
#[cfg(not(windows))]
pub fn hwnd_of(_cc: &eframe::CreationContext<'_>) -> Option<usize> {
    None
}

#[cfg(windows)]
fn set_cloaked(hwnd: usize, cloaked: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};

    let value: i32 = i32::from(cloaked);
    // SAFETY: `hwnd` は eframe のウィンドウのハンドルで、`value` は DWMWA_CLOAK が求める BOOL。
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_CLOAK as u32,
            (&raw const value).cast(),
            size_of::<i32>() as u32,
        )
    };
    if hr >= 0 {
        Ok(())
    } else {
        Err(format!(
            "DwmSetWindowAttribute(DWMWA_CLOAK): HRESULT 0x{hr:08X}"
        ))
    }
}

#[cfg(not(windows))]
fn set_cloaked(_hwnd: usize, _cloaked: bool) -> Result<(), String> {
    Err("cloaking is only supported on Windows".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloaks_only_when_starting_hidden_with_a_tray() {
        assert!(should_cloak_at_start(true, true));
        // トレイがなければ隠さないので、クロークもしない
        assert!(!should_cloak_at_start(true, false));
        assert!(!should_cloak_at_start(false, true));
        assert!(!should_cloak_at_start(false, false));
    }

    #[test]
    fn cloak_without_window_handle_fails_and_stays_uncloaked() {
        let mut c = Cloak::new(None);
        assert!(c.cloak().is_err());
        assert!(!c.is_cloaked());
        // していなければ外すのは何もしない
        assert_eq!(c.uncloak(), Ok(()));
    }

    #[test]
    fn uncloak_happens_once() {
        let mut c = Cloak::cloaked_for_test();
        assert!(c.is_cloaked());
        assert_eq!(c.uncloak(), Ok(()));
        assert!(!c.is_cloaked());
        assert_eq!(c.uncloak(), Ok(()));
    }

    #[test]
    fn failed_uncloak_stays_cloaked_and_is_retried() {
        let mut c = Cloak::unremovable_for_test();
        assert!(c.uncloak().is_err());
        // 外せなかったのでクロークしたまま。次に開くときにもう一度試す(また失敗する)
        assert!(c.is_cloaked());
        assert!(c.uncloak().is_err());
        assert!(c.is_cloaked());
    }
}
