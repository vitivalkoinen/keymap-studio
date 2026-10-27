//! タスクトレイへの常駐と、システム全体のホットキー(SPEC.md §4.5・§4.5.2)。
//!
//! Windows では専用のスレッドを 1 つ立て、その中で次のものを扱う。
//!
//! - 見えないウィンドウ(メッセージの受け口)と、`Shell_NotifyIconW` のトレイアイコン・ツールチップ・
//!   メニュー・バルーン通知
//! - `RegisterHotKey` で登録したホットキー(`WM_HOTKEY` がこのウィンドウに届く)
//! - 後から起動した側からの知らせ(名前付きイベント。[`crate::instance`])
//!
//! eframe のイベントループとは独立しているので、メインのウィンドウを隠している間も動く。
//! 起きたことは [`Event`] としてチャンネルに送り、`egui::Context::request_repaint` で eframe を
//! 起こす。eframe 0.37 は隠したウィンドウでも、再描画の要求があれば `App::logic` を呼ぶ
//! (Windows では隠したウィンドウに `RedrawRequested` が来ないため、eframe が直接呼ぶ。
//! `eframe::native::run` の `check_redraw_requests`)。アプリはそこで [`Resident::try_recv`] する。
//!
//! アプリからの指示([`Resident::set_menu`] など)はチャンネルに入れ、スレッドのウィンドウへ
//! メッセージを送って起こす。ホットキーの登録・解除はそのウィンドウのスレッドで行い、結果を
//! [`Event::HotkeysRegistered`] で返す。
//!
//! Windows 以外では何もしない([`Resident::start`] が失敗する)。

// Windows 以外では使わない部分がある(Windows 専用の機能のため)
#![cfg_attr(not(windows), allow(dead_code))]

use eframe::egui;

use crate::Source;
use crate::applog::Log;
use crate::hotkey::Hotkey;

/// トレイのメニューとツールチップの内容。
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub tooltip: String,
    /// 「ウィンドウを開く」の文字列
    pub open_window: String,
    /// 「終了」の文字列
    pub exit: String,
    pub sections: Vec<MenuSection>,
}

/// メニューの区切り(キーマップ、プリセット)。
#[derive(Debug, Clone, PartialEq)]
pub struct MenuSection {
    /// 見出し(選べない項目として出す)
    pub heading: String,
    /// 項目がないときに出す文字列
    pub empty: String,
    pub items: Vec<MenuItem>,
}

/// 選ぶと切り替える項目。
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// 割り当てたホットキー(右寄せで添える)
    pub hotkey: Option<String>,
    /// 使用中か(チェックを付ける)
    pub checked: bool,
    pub source: Source,
}

/// Windows の通知(トレイアイコンのバルーン)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// 警告のアイコンで出す(切り替えられなかった、など)
    pub warning: bool,
}

/// スレッドからアプリへ知らせること。
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// ウィンドウを開く(メニュー、アイコンのダブルクリック、後から起動した側からの知らせ)
    OpenWindow,
    /// 終了する(メニュー)
    Quit,
    /// 切り替える(メニュー、ホットキー)
    Switch(Source),
    /// ホットキーを登録し直した結果(登録しようとしたものすべて)
    HotkeysRegistered(Vec<(Source, Result<(), String>)>),
}

/// アプリからスレッドへの指示。
enum Command {
    SetMenu(Menu),
    SetHotkeys(Vec<(Source, Hotkey)>),
    Notify(Notification),
}

/// 常駐の部品。落とすとトレイアイコンを消し、ホットキーを解除してスレッドを止める。
pub struct Resident {
    commands: std::sync::mpsc::Sender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
    #[cfg(windows)]
    thread: Option<win::Thread>,
}

impl Resident {
    /// スレッドを立て、トレイアイコンを出す。`activation` は後から起動した側からの知らせを待つ
    /// イベントのハンドル([`crate::instance::Guard::activation_event`])。
    #[cfg(windows)]
    pub fn start(ctx: egui::Context, activation: Option<usize>, log: Log) -> Result<Self, String> {
        let (commands, commands_rx) = std::sync::mpsc::channel();
        let (events_tx, events) = std::sync::mpsc::channel();
        let thread = win::Thread::spawn(ctx, log, commands_rx, events_tx, activation)?;
        Ok(Self {
            commands,
            events,
            thread: Some(thread),
        })
    }

    /// Windows 以外では常駐しない。
    #[cfg(not(windows))]
    pub fn start(
        _ctx: egui::Context,
        _activation: Option<usize>,
        _log: Log,
    ) -> Result<Self, String> {
        Err("the tray icon and hotkeys are only supported on Windows".to_owned())
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            #[cfg(windows)]
            if let Some(t) = &self.thread {
                t.wake();
            }
        }
    }

    /// メニューとツールチップを置き換える。
    pub fn set_menu(&self, menu: Menu) {
        self.send(Command::SetMenu(menu));
    }

    /// ホットキーをこれだけに登録し直す。結果は [`Event::HotkeysRegistered`] で届く。
    pub fn set_hotkeys(&self, hotkeys: Vec<(Source, Hotkey)>) {
        self.send(Command::SetHotkeys(hotkeys));
    }

    /// 通知を出す。
    pub fn notify(&self, notification: Notification) {
        self.send(Command::Notify(notification));
    }

    /// トレイアイコンを出せているか(Explorer がまだない・出し直しを諦めたときは `false`)。
    pub fn icon_visible(&self) -> bool {
        #[cfg(windows)]
        {
            self.thread.as_ref().is_some_and(win::Thread::icon_visible)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// 届いている知らせを 1 つ取り出す。
    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

#[cfg(windows)]
impl Drop for Resident {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            t.stop();
        }
    }
}

/// `text` を NUL を含めて `N` 個に収まる UTF-16 にして `dest` に入れる(はみ出す分は切る)。
fn fill_wide<const N: usize>(dest: &mut [u16; N], text: &str) {
    let mut units: Vec<u16> = text.encode_utf16().take(N - 1).collect();
    // サロゲートペアの片方だけが残らないようにする
    if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
        units.pop();
    }
    dest.fill(0);
    dest[..units.len()].copy_from_slice(&units);
}

/// メニューの文字列。`&` はアクセスキーの印になるので `&&` にし、ホットキーはタブで右に寄せる。
fn menu_label(label: &str, hotkey: Option<&str>) -> String {
    let mut text = label.replace('&', "&&");
    if let Some(h) = hotkey {
        text.push('\t');
        text.push_str(&h.replace('&', "&&"));
    }
    text
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::io;
    use std::ptr::{null, null_mut};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::thread::JoinHandle;

    use eframe::egui;
    use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::INFINITE;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    };
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NINF_KEY, NOTIFYICON_VERSION_4,
        NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
        DestroyMenu, DispatchMessageW, GetSystemMetrics, HICON, HMENU, KillTimer, MF_CHECKED,
        MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, MsgWaitForMultipleObjects, PM_REMOVE,
        PeekMessageW, PostMessageW, PostQuitMessage, QS_ALLINPUT, RegisterClassW,
        RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetTimer, TPM_NONOTIFY,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE,
        WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_NULL, WM_QUIT, WM_TIMER,
        WNDCLASSW, WS_EX_TOOLWINDOW,
    };

    use super::{Command, Event, Menu, Notification, fill_wide, menu_label};
    use crate::Source;
    use crate::applog::Log;
    use crate::hotkey::Hotkey;
    use crate::icon;

    /// トレイアイコンからのメッセージ。
    const WM_TRAY: u32 = WM_APP + 1;
    /// アプリからの指示が届いた。
    const WM_COMMANDS: u32 = WM_APP + 2;

    /// キーボードでトレイアイコンを選んだ(Enter など)。
    const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

    /// トレイアイコンの ID(1 つだけ)。
    const ICON_ID: u32 = 1;

    /// トレイアイコンを出し直すタイマー(サインイン直後はタスクバーがまだないことがある)。
    const RETRY_TIMER: usize = 1;
    const RETRY_INTERVAL_MS: u32 = 2000;
    const RETRY_LIMIT: u32 = 60;

    /// メニューの項目の ID。
    const ID_OPEN: usize = 1;
    const ID_EXIT: usize = 2;
    const ID_FIRST_SOURCE: usize = 100;

    /// Explorer が起動し直したときに送られるメッセージ(`TaskbarCreated`)。
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// スレッドの中の状態。ウィンドウプロシージャから使うので thread_local に置く。
    struct State {
        hwnd: HWND,
        icon: HICON,
        log: Log,
        ctx: egui::Context,
        commands: Receiver<Command>,
        events: Sender<Event>,
        menu: Option<Menu>,
        /// 登録中のホットキー(添字 + 1 が ID)
        hotkeys: Vec<Source>,
        /// トレイアイコンを出せている
        icon_added: bool,
        /// `icon_added` をアプリのスレッドにも見せる
        shown: Arc<AtomicBool>,
        /// トレイアイコンを出し直した回数
        retries: u32,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    /// 状態を借りて `f` を呼ぶ。状態がない(終了した)か、借用中(メニューの表示中などに入れ子で
    /// 呼ばれた)なら呼ばない。
    fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
        STATE.with(|cell| {
            let mut guard = cell.try_borrow_mut().ok()?;
            guard.as_mut().map(f)
        })
    }

    impl State {
        fn emit(&self, event: Event) {
            if self.events.send(event).is_ok() {
                self.ctx.request_repaint();
            }
        }

        fn set_icon_added(&mut self, added: bool) {
            self.icon_added = added;
            self.shown.store(added, Ordering::Relaxed);
        }

        fn base_data(&self) -> NOTIFYICONDATAW {
            NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: ICON_ID,
                ..Default::default()
            }
        }

        fn tooltip(&self) -> String {
            self.menu
                .as_ref()
                .map_or_else(|| "Keymap Studio".to_owned(), |m| m.tooltip.clone())
        }

        /// トレイアイコンを出す。出せたら `true`。
        fn add_icon(&mut self) -> bool {
            let mut data = self.base_data();
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            data.uCallbackMessage = WM_TRAY;
            data.hIcon = self.icon;
            fill_wide(&mut data.szTip, &self.tooltip());
            // SAFETY: `data` は正しく初期化した構造体で、呼び出しの間は生きている。
            let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
            if added {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                // SAFETY: 同上。
                unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
            }
            self.set_icon_added(added);
            added
        }

        fn remove_icon(&mut self) {
            if self.icon_added {
                let data = self.base_data();
                // SAFETY: `data` は正しく初期化した構造体で、呼び出しの間は生きている。
                unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                self.set_icon_added(false);
            }
        }

        fn update_tooltip(&self) {
            if !self.icon_added {
                return;
            }
            let mut data = self.base_data();
            data.uFlags = NIF_TIP | NIF_SHOWTIP;
            fill_wide(&mut data.szTip, &self.tooltip());
            // SAFETY: `data` は正しく初期化した構造体で、呼び出しの間は生きている。
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
        }

        fn notify(&self, n: &Notification) {
            if !self.icon_added {
                self.log
                    .write("tray: notification not shown: the tray icon is not available");
                return;
            }
            let mut data = self.base_data();
            // NOTIFYICON_VERSION_4 では、NIM_MODIFY に NIF_SHOWTIP がないと標準のツールチップが
            // 出なくなるので、ツールチップも一緒に渡す
            data.uFlags = NIF_INFO | NIF_TIP | NIF_SHOWTIP;
            fill_wide(&mut data.szTip, &self.tooltip());
            fill_wide(&mut data.szInfoTitle, &n.title);
            fill_wide(&mut data.szInfo, &n.body);
            data.dwInfoFlags = if n.warning { NIIF_WARNING } else { NIIF_INFO };
            // SAFETY: `data` は正しく初期化した構造体で、呼び出しの間は生きている。
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
                self.log.write("tray: failed to show a notification");
            }
        }

        fn unregister_hotkeys(&mut self) {
            for id in 1..=self.hotkeys.len() {
                // SAFETY: `hwnd` はこのスレッドのウィンドウ。登録していない ID でも害はない。
                unsafe { UnregisterHotKey(self.hwnd, id as i32) };
            }
            self.hotkeys.clear();
        }

        fn register_hotkeys(&mut self, list: Vec<(Source, Hotkey)>) {
            self.unregister_hotkeys();
            let mut results = Vec::new();
            for (source, h) in list {
                let mut mods = MOD_NOREPEAT;
                for (on, m) in [
                    (h.ctrl, MOD_CONTROL),
                    (h.alt, MOD_ALT),
                    (h.shift, MOD_SHIFT),
                    (h.win, MOD_WIN),
                ] {
                    if on {
                        mods |= m;
                    }
                }
                let id = (self.hotkeys.len() + 1) as i32;
                // SAFETY: `hwnd` はこのスレッドのウィンドウ。
                let ok = unsafe { RegisterHotKey(self.hwnd, id, mods, h.vk) } != 0;
                let result = if ok {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error().to_string())
                };
                // 登録できなかったものも ID の並びを保つため入れておく(解除は害がない)
                self.hotkeys.push(source.clone());
                results.push((source, result));
            }
            self.emit(Event::HotkeysRegistered(results));
        }

        fn handle_commands(&mut self) {
            while let Ok(command) = self.commands.try_recv() {
                match command {
                    Command::SetMenu(menu) => {
                        self.menu = Some(menu);
                        self.update_tooltip();
                    }
                    Command::SetHotkeys(list) => self.register_hotkeys(list),
                    Command::Notify(n) => self.notify(&n),
                }
            }
        }

        /// メニューを作る。項目の ID と切り替える対象の対応も返す。
        fn build_menu(&self) -> Option<(HMENU, Vec<Source>)> {
            let menu = self.menu.as_ref()?;
            // SAFETY: 作ったメニューは呼び出し側が DestroyMenu で壊す。文字列は呼び出しの間は生きている。
            unsafe {
                let hmenu = CreatePopupMenu();
                if hmenu.is_null() {
                    return None;
                }
                let append = |flags, id: usize, text: &str| {
                    let w = wide(text);
                    AppendMenuW(hmenu, flags, id, w.as_ptr());
                };
                append(MF_STRING, ID_OPEN, &menu_label(&menu.open_window, None));
                let mut sources = Vec::new();
                for section in &menu.sections {
                    AppendMenuW(hmenu, MF_SEPARATOR, 0, null());
                    append(
                        MF_STRING | MF_GRAYED,
                        0,
                        &menu_label(&section.heading, None),
                    );
                    if section.items.is_empty() {
                        append(MF_STRING | MF_GRAYED, 0, &menu_label(&section.empty, None));
                    }
                    for item in &section.items {
                        let flags = if item.checked {
                            MF_STRING | MF_CHECKED
                        } else {
                            MF_STRING
                        };
                        let id = ID_FIRST_SOURCE + sources.len();
                        append(flags, id, &menu_label(&item.label, item.hotkey.as_deref()));
                        sources.push(item.source.clone());
                    }
                }
                AppendMenuW(hmenu, MF_SEPARATOR, 0, null());
                append(MF_STRING, ID_EXIT, &menu_label(&menu.exit, None));
                Some((hmenu, sources))
            }
        }
    }

    /// メニューを `(x, y)` に出し、選ばれたものを知らせる。
    ///
    /// メニューを出している間はメッセージがウィンドウプロシージャに入れ子で届くので、
    /// 状態を借りたままにしない。
    fn show_menu(hwnd: HWND, x: i32, y: i32) {
        let Some(Some((hmenu, sources))) = with_state(|s| s.build_menu()) else {
            return;
        };
        // SAFETY: `hwnd` はこのスレッドのウィンドウ、`hmenu` は今作ったメニューで、ここで壊す。
        // メニューの外をクリックしたときに閉じるよう、前面に出してから表示し、あとで WM_NULL を送る
        // (TrackPopupMenu の説明にある手順)。
        let command = unsafe {
            SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(
                hmenu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                x,
                y,
                0,
                hwnd,
                null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(hmenu);
            command as usize
        };
        let event = match command {
            0 => None,
            ID_OPEN => Some(Event::OpenWindow),
            ID_EXIT => Some(Event::Quit),
            n => n
                .checked_sub(ID_FIRST_SOURCE)
                .and_then(|i| sources.get(i))
                .cloned()
                .map(Event::Switch),
        };
        if let Some(e) = event {
            with_state(|s| s.emit(e));
        }
    }

    /// `WPARAM` の下位・上位の 16 ビットを符号付きの座標として読む(`GET_X_LPARAM` と同じ)。
    fn point_of(wparam: WPARAM) -> (i32, i32) {
        let x = (wparam & 0xFFFF) as u16 as i16 as i32;
        let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
        (x, y)
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TRAY => {
                // NOTIFYICON_VERSION_4 では下位 16 ビットが出来事、wParam が位置
                match (lparam as u32) & 0xFFFF {
                    WM_CONTEXTMENU => {
                        let (x, y) = point_of(wparam);
                        show_menu(hwnd, x, y);
                    }
                    WM_LBUTTONDBLCLK | NIN_KEYSELECT => {
                        with_state(|s| s.emit(Event::OpenWindow));
                    }
                    _ => {}
                }
                0
            }
            WM_COMMANDS => {
                with_state(State::handle_commands);
                0
            }
            WM_HOTKEY => {
                let index = wparam.wrapping_sub(1);
                with_state(|s| {
                    if let Some(source) = s.hotkeys.get(index).cloned() {
                        s.emit(Event::Switch(source));
                    }
                });
                0
            }
            WM_TIMER if wparam == RETRY_TIMER => {
                with_state(|s| {
                    s.retries += 1;
                    if s.add_icon() || s.retries >= RETRY_LIMIT {
                        if !s.icon_added {
                            s.log.write("tray: gave up adding the tray icon");
                        }
                        // SAFETY: このスレッドのウィンドウのタイマー。
                        unsafe { KillTimer(hwnd, RETRY_TIMER) };
                    }
                });
                0
            }
            WM_DESTROY => {
                with_state(|s| {
                    s.unregister_hotkeys();
                    s.remove_icon();
                });
                // SAFETY: このスレッドのメッセージループを終わらせる。
                unsafe { PostQuitMessage(0) };
                0
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                // Explorer が起動し直すとアイコンが消えるので出し直す
                with_state(|s| {
                    s.set_icon_added(false);
                    if !s.add_icon() {
                        s.log.write("tray: failed to add the tray icon again");
                    }
                });
                0
            }
            // SAFETY: 扱わないメッセージは既定の処理に渡す。
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// RGBA の画像から HICON を作る。
    fn create_icon(size: i32) -> HICON {
        let size = size.clamp(16, 256);
        let rgba = icon::keyboard_rgba(size as u32);
        // CreateIcon の 32 ビットの色は BGRA の順
        let bgra: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect();
        // AND マスクは 1 ビット/ピクセルで、行を 2 バイト境界に揃える。透明度は色の α で決まるので 0
        let mask = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        // SAFETY: バッファの大きさは幅・高さ・ビット数と合っていて、呼び出しの間は生きている。
        unsafe {
            CreateIcon(
                GetModuleHandleW(null()),
                size,
                size,
                1,
                32,
                mask.as_ptr(),
                bgra.as_ptr(),
            )
        }
    }

    /// トレイのスレッド。
    pub struct Thread {
        hwnd: usize,
        handle: Option<JoinHandle<()>>,
        /// トレイアイコンを出せているか
        shown: Arc<AtomicBool>,
    }

    impl Thread {
        pub fn spawn(
            ctx: egui::Context,
            log: Log,
            commands: Receiver<Command>,
            events: Sender<Event>,
            activation: Option<usize>,
        ) -> Result<Self, String> {
            let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
            let shown = Arc::new(AtomicBool::new(false));
            let shown_in_thread = Arc::clone(&shown);
            let handle = std::thread::Builder::new()
                .name("tray".to_owned())
                .spawn(move || {
                    run(
                        ctx,
                        log,
                        commands,
                        events,
                        activation,
                        shown_in_thread,
                        ready_tx,
                    )
                })
                .map_err(|e| format!("spawn: {e}"))?;
            match ready_rx.recv() {
                Ok(Ok(hwnd)) => Ok(Self {
                    hwnd,
                    handle: Some(handle),
                    shown,
                }),
                Ok(Err(e)) => {
                    let _ = handle.join();
                    Err(e)
                }
                Err(_) => {
                    let _ = handle.join();
                    Err("the tray thread stopped".to_owned())
                }
            }
        }

        pub fn icon_visible(&self) -> bool {
            self.shown.load(Ordering::Relaxed)
        }

        /// 指示が届いたことを知らせる。
        pub fn wake(&self) {
            // SAFETY: ウィンドウが壊れたあとなら失敗するだけ。
            unsafe { PostMessageW(self.hwnd as HWND, WM_COMMANDS, 0, 0) };
        }

        /// ウィンドウを閉じ(アイコンを消し、ホットキーを解除する)、スレッドの終わりを待つ。
        pub fn stop(mut self) {
            // SAFETY: ウィンドウが壊れたあとなら失敗するだけ。
            unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    fn run(
        ctx: egui::Context,
        log: Log,
        commands: Receiver<Command>,
        events: Sender<Event>,
        activation: Option<usize>,
        shown: Arc<AtomicBool>,
        ready: std::sync::mpsc::SyncSender<Result<usize, String>>,
    ) {
        // SAFETY: Win32 の呼び出し。文字列は呼び出しの間は生きている。作ったウィンドウとアイコンは
        // このスレッドの中で壊す。
        unsafe {
            let instance = GetModuleHandleW(null());
            let class_name = wide("KeymapStudioTray");
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            // すでに登録済み(2 回目の起動)でも CreateWindowExW は成功する
            RegisterClassW(&class);
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                Ordering::Relaxed,
            );
            let title = wide("Keymap Studio tray");
            // 見えないトップレベルのウィンドウ(メッセージ専用のウィンドウには TaskbarCreated が
            // 届かないため)
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                let _ = ready.send(Err(format!(
                    "CreateWindowExW: {}",
                    io::Error::last_os_error()
                )));
                return;
            }
            let icon = create_icon(GetSystemMetrics(SM_CXSMICON));
            STATE.with(|cell| {
                *cell.borrow_mut() = Some(State {
                    hwnd,
                    icon,
                    log: log.clone(),
                    ctx: ctx.clone(),
                    commands,
                    events,
                    menu: None,
                    hotkeys: Vec::new(),
                    icon_added: false,
                    shown,
                    retries: 0,
                });
            });
            if with_state(State::add_icon) != Some(true) {
                log.write("tray: failed to add the tray icon; retrying");
                SetTimer(hwnd, RETRY_TIMER, RETRY_INTERVAL_MS, None);
            }
            let _ = ready.send(Ok(hwnd as usize));

            let handles: Vec<HANDLE> = activation.into_iter().map(|h| h as HANDLE).collect();
            let mut msg: MSG = std::mem::zeroed();
            'outer: loop {
                let r = MsgWaitForMultipleObjects(
                    handles.len() as u32,
                    if handles.is_empty() {
                        null()
                    } else {
                        handles.as_ptr()
                    },
                    0,
                    INFINITE,
                    QS_ALLINPUT,
                );
                if !handles.is_empty() && r == WAIT_OBJECT_0 {
                    // 後から起動した側からの知らせ
                    with_state(|s| s.emit(Event::OpenWindow));
                    continue;
                }
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if msg.message == WM_QUIT {
                        break 'outer;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            STATE.with(|cell| cell.borrow_mut().take());
            DestroyIcon(icon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_wide_truncates_and_terminates() {
        let mut buf = [0xFFFFu16; 8];
        fill_wide(&mut buf, "abc");
        assert_eq!(&buf[..4], &[97, 98, 99, 0]);
        assert!(buf[3..].iter().all(|&u| u == 0));
        fill_wide(&mut buf, "0123456789");
        assert_eq!(String::from_utf16_lossy(&buf[..7]), "0123456");
        assert_eq!(buf[7], 0);
        // サロゲートペアの途中で切らない(6 文字 + 絵文字 1 つは 8 単位)
        fill_wide(&mut buf, "abcdef\u{1F600}");
        assert_eq!(String::from_utf16_lossy(&buf[..6]), "abcdef");
        assert_eq!(buf[6], 0);
    }

    #[test]
    fn menu_label_escapes_ampersands_and_adds_hotkey() {
        assert_eq!(menu_label("A&B", None), "A&&B");
        assert_eq!(
            menu_label("Gaming", Some("Ctrl+Alt+1")),
            "Gaming\tCtrl+Alt+1"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn start_fails_outside_windows() {
        let ctx = egui::Context::default();
        assert!(Resident::start(ctx, None, Log::disabled()).is_err());
    }
}
