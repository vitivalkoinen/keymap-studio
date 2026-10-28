#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_settings;
mod applog;
mod autostart;
mod cloak;
mod config;
mod edit;
mod hotkey;
mod i18n;
mod icon;
mod initial_backup;
mod instance;
mod keycap;
mod keymap;
mod keys;
mod launch;
mod layout;
mod matching;
mod preset;
mod save;
mod tray;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use app_settings::{AppSettings, AssignError, Target};
use applog::Log;
use config::{Config, KeyCode, PathError, PowerToysState, Remap};
use eframe::egui;
use hotkey::Hotkey;
use i18n::{Lang, Msg, fill, tr};
use layout::Layout;
use matching::{KeyRemapLabel, KeyState, Role, SourceMap};

/// PowerToys のインストール方法の公式ページ(§1.1)。
const POWERTOYS_INSTALL_URL: &str = "https://learn.microsoft.com/windows/powertoys/install";

fn main() -> eframe::Result {
    // 引数でパスを指定できる(WSL での開発時は /mnt/c/... を渡す)。
    // 指定がなければ PowerToys の状態を判定し(§1.1)、settings.json の activeConfiguration から
    // 決める。読み込みと保存は同じファイル。`--tray` ならウィンドウを出さずトレイにだけ表示する
    // (§4.5)。
    let launch = launch::parse(std::env::args_os().skip(1));
    let log = Log::default_location();
    let (arg, tray_only) = match launch {
        launch::Launch::File(p) => (Some(p), false),
        launch::Launch::Resident { tray_only } => (None, tray_only),
    };

    // 二重起動の防止(§4.5.2)。引数でファイルを指定した起動は対象外
    let guard = if arg.is_none() {
        match instance::acquire() {
            Ok(instance::Instance::Primary(g)) => Some(g),
            Ok(instance::Instance::AlreadyRunning(running)) => {
                if tray_only {
                    log.write("startup: already running; exiting (tray only)");
                } else {
                    log.write("startup: already running; bringing its window to the front");
                    if let Err(e) = running.activate_running() {
                        log.write(&format!("startup: failed to notify the running app: {e}"));
                    }
                }
                return Ok(());
            }
            Err(e) => {
                // 判定できなくても起動は続ける
                log.write(&format!("startup: single instance check failed: {e}"));
                None
            }
        }
    } else {
        None
    };
    let activation = guard.as_ref().and_then(instance::Guard::activation_event);

    let mut app = App::new(arg, Places::default_locations(), log);
    app.start_hidden = tray_only;
    // ウィンドウのアイコンはトレイと同じ図案にする
    let icon_size = 64;
    let icon = egui::IconData {
        rgba: icon::keyboard_rgba(icon_size),
        width: icon_size,
        height: icon_size,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 600.0])
        .with_icon(icon);
    if tray_only {
        // 最初のフレームを描くまでの一瞬に表示されても、前面を奪わないようにする
        viewport = viewport.with_active(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    // `guard` は main を抜けるまで(ウィンドウを閉じて run_native が戻るまで)持っておく
    eframe::run_native(
        "Keymap Studio",
        options,
        Box::new(move |cc| {
            app.start_resident(&cc.egui_ctx, activation);
            // ウィンドウはできているが、まだ何も描いていない。ここでクロークしておく
            app.cloak_if_starting_hidden(cloak::hwnd_of(cc));
            Ok(Box::new(app))
        }),
    )
}

/// 設定ファイルを読めなかった理由。
enum LoadError {
    /// 既定のパスが分からない(`%LOCALAPPDATA%` が分からない)
    PathNotFound,
    /// PowerToys が入っていない(§1.1)
    PowerToysNotInstalled,
    /// Keyboard Manager の設定がない(§1.1)
    KeyboardManagerNotConfigured,
    /// `settings.json` があるのに読めない・解析できない(詳細はそのまま表示する)
    SettingsUnreadable(String),
    /// `settings.json` の `activeConfiguration` が安全なファイル名でない(値をそのまま持つ)
    UnsafeConfiguration(String),
    /// 読み込みか解析に失敗した(詳細はそのまま表示する)
    Failed(String),
}

impl LoadError {
    fn message(&self, lang: Lang) -> String {
        match self {
            LoadError::PathNotFound => tr(lang, Msg::ConfigPathNotFound).to_owned(),
            LoadError::PowerToysNotInstalled => tr(lang, Msg::PowerToysNotInstalled).to_owned(),
            LoadError::KeyboardManagerNotConfigured => {
                tr(lang, Msg::KeyboardManagerNotConfigured).to_owned()
            }
            LoadError::SettingsUnreadable(detail) => {
                format!("{}: {detail}", tr(lang, Msg::SettingsLoadFailed))
            }
            LoadError::UnsafeConfiguration(value) => {
                format!("{}: {value:?}", tr(lang, Msg::UnsafeActiveConfiguration))
            }
            LoadError::Failed(detail) => format!("{}: {detail}", tr(lang, Msg::ConfigLoadFailed)),
        }
    }
}

impl LoadError {
    /// PowerToys の状態による案内か(§1.1)。案内と「再確認」のボタンを出す
    fn is_powertoys_guidance(&self) -> bool {
        matches!(
            self,
            LoadError::PowerToysNotInstalled | LoadError::KeyboardManagerNotConfigured
        )
    }
}

impl From<PathError> for LoadError {
    fn from(e: PathError) -> Self {
        match e {
            PathError::SettingsUnreadable(detail) => LoadError::SettingsUnreadable(detail),
            PathError::UnsafeConfiguration(value) => LoadError::UnsafeConfiguration(value),
        }
    }
}

/// 導入前の設定のバックアップ(§4.7)の状態。
enum InitialBackup {
    /// 引数でファイルを指定して起動したので扱わない
    SkippedByArgument,
    /// PowerToys の設定が使える状態でない(§1.1)ので、まだ試していない
    NotAttempted,
    /// データフォルダ(exe と同じフォルダ)が分からない
    DataDirNotFound,
    /// 作れなかった、または前に作ったものを読めなかった(詳細はそのまま表示する)。
    /// 次の起動で再び試す
    Failed(String),
    Ready {
        /// バックアップのフォルダ
        dir: PathBuf,
        manifest: initial_backup::Manifest,
        /// 今回の起動で作った
        created: bool,
    },
}

impl InitialBackup {
    /// `dir` にバックアップがなければ `source`(Keyboard Manager の設定フォルダ)から作る。
    /// すでにあれば読む。結果をログに残す。
    fn ensure(dir: Option<&Path>, source: &Path, log: &Log) -> Self {
        let Some(dir) = dir else {
            log.write("initial backup: failed: data folder not found");
            return InitialBackup::DataDirNotFound;
        };
        match initial_backup::ensure(dir, source, std::time::SystemTime::now()) {
            Ok(outcome) => {
                let created = matches!(outcome, initial_backup::Outcome::Created(_));
                let manifest = outcome.manifest().clone();
                log.write(&format!(
                    "initial backup: {} {} (created at {})",
                    if created { "created" } else { "already exists" },
                    dir.display(),
                    manifest.created_at
                ));
                InitialBackup::Ready {
                    dir: dir.to_owned(),
                    manifest,
                    created,
                }
            }
            Err(e) => {
                log.write(&format!("initial backup: failed: {e}"));
                InitialBackup::Failed(e)
            }
        }
    }

    /// 保存を止めるか。初回のバックアップができていない間は、導入前の状態を確実に残すため
    /// 保存しない(§4.7)。引数でファイルを指定して起動したときはバックアップを扱わないので止めない。
    fn blocks_save(&self) -> bool {
        match self {
            InitialBackup::SkippedByArgument | InitialBackup::Ready { .. } => false,
            InitialBackup::NotAttempted
            | InitialBackup::DataDirNotFound
            | InitialBackup::Failed(_) => true,
        }
    }

    /// 起動時に知らせること(エラーか、今回作ったこと、扱わないこと)。知らせることがなければ `None`。
    fn notice(&self, lang: Lang) -> Option<(bool, String)> {
        let failed = |detail: &str| {
            Some((
                true,
                format!("{}: {detail}", tr(lang, Msg::InitialBackupFailed)),
            ))
        };
        match self {
            InitialBackup::SkippedByArgument => Some((
                false,
                tr(lang, Msg::InitialBackupSkippedByArgument).to_owned(),
            )),
            // PowerToys の状態の案内を別に出す
            InitialBackup::NotAttempted => None,
            InitialBackup::DataDirNotFound => failed(tr(lang, Msg::DataDirNotFound)),
            InitialBackup::Failed(d) => failed(d),
            InitialBackup::Ready {
                dir, created: true, ..
            } => Some((
                false,
                format!("{}: {}", tr(lang, Msg::InitialBackupCreated), dir.display()),
            )),
            InitialBackup::Ready { created: false, .. } => None,
        }
    }
}

/// PowerToys の設定と本アプリのデータの場所。テストでは一時ディレクトリに差し替える。
#[derive(Debug, Clone)]
struct Places {
    /// PowerToys の設定フォルダ(`%LOCALAPPDATA%\Microsoft\PowerToys`)
    powertoys_root: Option<PathBuf>,
    /// 導入前の設定のバックアップのフォルダ(exe と同じフォルダの `initial-backup`)
    initial_backup_dir: Option<PathBuf>,
    /// 世代バックアップを置くフォルダ(exe と同じフォルダの `backups`)
    backup_dir: Option<PathBuf>,
    /// キーマップを置くフォルダ(exe と同じフォルダの `keymaps`。§4.5.1)
    keymaps_dir: Option<PathBuf>,
    /// 本アプリの設定ファイル(exe と同じフォルダの `keymap-studio.settings.json`。§4.5.2)
    settings_path: Option<PathBuf>,
}

impl Places {
    fn default_locations() -> Self {
        Self {
            powertoys_root: config::powertoys_root(),
            initial_backup_dir: initial_backup::default_dir(),
            backup_dir: save::default_backup_dir(),
            keymaps_dir: keymap::default_dir(),
            settings_path: app_settings::default_path(),
        }
    }
}

/// 読み書きする設定ファイルと、その読み込み・初回のバックアップの結果。
struct Opened {
    path: Option<PathBuf>,
    doc: Result<Document, LoadError>,
    initial: InitialBackup,
}

impl Opened {
    /// 引数で指定したファイルを開く。PowerToys の状態の判定も初回のバックアップもしない(§1.1)。
    fn argument(path: PathBuf, log: &Log) -> Self {
        log.write(&format!(
            "open: file specified on the command line: {}",
            path.display()
        ));
        Opened {
            doc: Document::load_logged(&path, log),
            path: Some(path),
            initial: InitialBackup::SkippedByArgument,
        }
    }

    /// PowerToys の状態を判定し(§1.1)、通常なら初回のバックアップをしてから読み込む。
    /// 通常でなければ、編集・保存・初回のバックアップをせず案内だけを出す。
    /// 設定フォルダや定義ファイルは作らない。
    fn powertoys(places: &Places, log: &Log) -> Self {
        let not_ready = |doc: LoadError| Opened {
            path: None,
            doc: Err(doc),
            initial: InitialBackup::NotAttempted,
        };
        let Some(root) = &places.powertoys_root else {
            log.write("powertoys: %LOCALAPPDATA% not found");
            return not_ready(LoadError::PathNotFound);
        };
        match config::detect(root) {
            Ok(PowerToysState::NotInstalled) => {
                log.write(&format!("powertoys: not installed ({})", root.display()));
                not_ready(LoadError::PowerToysNotInstalled)
            }
            Ok(PowerToysState::NotConfigured) => {
                log.write(&format!(
                    "powertoys: Keyboard Manager not configured ({})",
                    root.display()
                ));
                not_ready(LoadError::KeyboardManagerNotConfigured)
            }
            Err(e) => {
                log.write(&format!(
                    "powertoys: cannot determine the config file: {e:?}"
                ));
                not_ready(e.into())
            }
            Ok(PowerToysState::Ready { dir, definition }) => {
                log.write(&format!("powertoys: ready ({})", definition.display()));
                let initial =
                    InitialBackup::ensure(places.initial_backup_dir.as_deref(), &dir, log);
                Opened {
                    doc: Document::load_logged(&definition, log),
                    path: Some(definition),
                    initial,
                }
            }
        }
    }
}

/// 読み込んだ設定と編集の状態。
struct Document {
    config: Config,
    /// 読み込んだ(または最後に保存した)時点のファイルの内容。外部の変更の検知に使う
    snapshot: Vec<u8>,
    /// 読み込んだ(または最後に保存した)時点のキー→キーのリマップ。キーボード図の色分けに使う
    saved: SourceMap,
    /// 読み込んだ(または最後に保存した)時点の設定。未保存の変更と「使用中」の判定、適用の元にする
    saved_config: Config,
}

impl Document {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        config::load(path).map(|(config, snapshot)| Document {
            saved: matching::source_map(&config.remap_keys.in_process),
            saved_config: config.clone(),
            config,
            snapshot,
        })
    }

    /// 読み込み、失敗したらログに残す。
    fn load_logged(path: &Path, log: &Log) -> Result<Self, LoadError> {
        Document::load(path).map_err(|e| {
            log.write(&format!("load failed: {e}"));
            LoadError::Failed(e)
        })
    }

    /// 今のキー→キーのリマップ。
    fn current(&self) -> SourceMap {
        matching::source_map(&self.config.remap_keys.in_process)
    }

    /// 保存していない変更があるか。
    ///
    /// 編集のたびに立てるフラグではなく、保存済みの内容と比べて決める。編集を元に戻して
    /// 差がなくなれば未保存ではなくなる。
    ///
    /// 比べ方は [`keymap::same_remaps`]。キー→キーのリマップは解析したキーコードの値で比べるので、
    /// 表記の違い(リマップ元の正規化で同じになるフラグの有無など)・エントリの並び順・
    /// 未知のフィールドは差とみなさない。キーマップを編集に読み込んだときはほかの区分も
    /// 変わりうるので、ほかの 3 つの区分は JSON として比べる。
    fn is_dirty(&self) -> bool {
        !keymap::same_remaps(&self.config, &self.saved_config)
    }

    /// 保存(または適用)して `written` を書き込んだあとの状態にする。
    fn mark_saved(&mut self, written: Vec<u8>) {
        self.snapshot = written;
        self.saved = self.current();
        self.saved_config = self.config.clone();
    }
}

/// 選択中のキーの欄での操作。
enum Action {
    /// リマップ元を変換先へリマップする(追加または変更)
    Set(KeyCode, KeyCode),
    /// リマップ元のリマップを削除する
    Remove(KeyCode),
    /// リマップ元の変換先をキーボード図で選び始める(§4.10)
    StartPick(KeyCode),
}

/// 直前の保存・読み直しの結果。
enum Status {
    Saved,
    /// 保存はできたが、エンジンへの通知に失敗した
    SavedNotApplied(save::SignalError),
    SaveFailed(save::SaveError),
    /// バックアップ先のパスが分からないため保存しなかった
    BackupDirNotFound,
    Reloaded,
    /// 読み直しに失敗した(編集中の内容はそのまま)
    ReloadFailed(String),
    /// 導入前の設定に戻した
    Restored,
    /// 戻したが、エンジンへの通知に失敗した
    RestoredNotApplied(save::SignalError),
    /// 戻したが、読み直しに失敗した(詳細はそのまま表示する)。エンジンへの通知の結果も持つ
    RestoredReloadFailed(String, Result<(), save::SignalError>),
    /// 初回のバックアップができていないため保存しなかった
    SaveBlockedByInitialBackup,
    /// 導入時のバックアップから戻す内容を読めなかった(詳細はそのまま表示する)
    RestoreBackupUnreadable(String),
    /// 書き戻しに失敗した
    RestoreFailed(save::SaveError),
    /// キーマップを新しく保存した(名前)
    KeymapSaved(String),
    /// キーマップを今の内容で上書きした(名前)
    KeymapOverwritten(String),
    /// キーマップの名前を変えた(新しい名前)
    KeymapRenamed(String),
    /// キーマップを削除した(名前)
    KeymapDeleted(String),
    /// キーマップを書き出した(書き出し先)
    KeymapExported(String),
    /// キーマップを取り込んだ(名前)
    KeymapImported(String),
    /// キーマップのファイルの操作に失敗した(詳細はそのまま表示する)
    KeymapFailed(String),
    /// 取り込もうとしたファイルを受け付けなかった
    ImportRejected(keymap::InvalidKeymap),
    /// キーマップのフォルダ(exe と同じフォルダの `keymaps`)が分からない
    KeymapDirNotFound,
    /// キーマップまたはプリセットを適用した(名前)
    Applied(String),
    /// 適用したが、エンジンへの通知に失敗した
    AppliedNotApplied(String, save::SignalError),
    /// キーマップまたはプリセットを編集に読み込んだ(名前)
    LoadedIntoEditor(String),
    /// プリセットを今の編集に重ねた(名前)
    PresetOverlaid(String),
    /// ホットキーを割り当てた(対象の名前、ホットキー)
    HotkeyAssigned(String, String),
    /// ホットキーの割り当てを外した(対象の名前)
    HotkeyRemoved(String),
    /// ホットキーを割り当てなかった(理由)
    HotkeyRejected(String),
    /// 本アプリの設定ファイルを書けなかった(詳細はそのまま表示する)
    AppSettingsSaveFailed(String),
    /// 本アプリの設定ファイルを読めなかったので、上書きしなかった
    AppSettingsUnreadable,
    /// 自動起動の登録を変えられなかった(詳細はそのまま表示する)
    AutoStartFailed(String),
}

impl Status {
    fn is_error(&self) -> bool {
        !matches!(
            self,
            Status::Saved
                | Status::Reloaded
                | Status::Restored
                | Status::KeymapSaved(_)
                | Status::KeymapOverwritten(_)
                | Status::KeymapRenamed(_)
                | Status::KeymapDeleted(_)
                | Status::KeymapExported(_)
                | Status::KeymapImported(_)
                | Status::Applied(_)
                | Status::LoadedIntoEditor(_)
                | Status::PresetOverlaid(_)
                | Status::HotkeyAssigned(..)
                | Status::HotkeyRemoved(_)
        )
    }

    /// 外部の変更を検知して保存を中止したか(読み直しの操作を出す)。
    fn is_modified_externally(&self) -> bool {
        matches!(self, Status::SaveFailed(save::SaveError::Modified))
    }

    fn message(&self, lang: Lang) -> String {
        use save::{SaveError, SignalError};
        let with = |msg: Msg, detail: &str| format!("{}: {detail}", tr(lang, msg));
        match self {
            Status::Saved => tr(lang, Msg::Saved).to_owned(),
            Status::SavedNotApplied(SignalError::Unsupported) => {
                with(Msg::SavedNotApplied, tr(lang, Msg::SignalUnsupported))
            }
            Status::SavedNotApplied(SignalError::EventNotFound) => {
                with(Msg::SavedNotApplied, tr(lang, Msg::SignalEventNotFound))
            }
            Status::SavedNotApplied(SignalError::Os(d)) => with(Msg::SavedNotApplied, d),
            Status::SaveFailed(SaveError::Modified) => {
                tr(lang, Msg::SaveAbortedModified).to_owned()
            }
            Status::SaveFailed(SaveError::LockTimeout(d)) => with(Msg::SaveAbortedLocked, d),
            Status::SaveFailed(SaveError::Read(d)) => with(Msg::SaveFailedRead, d),
            Status::SaveFailed(SaveError::Backup(d)) => with(Msg::SaveFailedBackup, d),
            Status::SaveFailed(SaveError::Write(d)) => with(Msg::SaveFailedWrite, d),
            Status::BackupDirNotFound => tr(lang, Msg::BackupDirNotFound).to_owned(),
            Status::Reloaded => tr(lang, Msg::Reloaded).to_owned(),
            Status::ReloadFailed(d) => with(Msg::ReloadFailed, d),
            Status::Restored => tr(lang, Msg::Restored).to_owned(),
            Status::RestoredNotApplied(e) => with(Msg::RestoredNotApplied, &signal_detail(lang, e)),
            Status::RestoredReloadFailed(d, signal) => {
                let mut text = with(Msg::RestoredReloadFailed, d);
                if let Err(e) = signal {
                    text.push_str(" / ");
                    text.push_str(&with(Msg::RestoredNotApplied, &signal_detail(lang, e)));
                }
                text
            }
            Status::SaveBlockedByInitialBackup => {
                tr(lang, Msg::SaveBlockedByInitialBackup).to_owned()
            }
            Status::RestoreBackupUnreadable(d) => with(Msg::RestoreBackupUnreadable, d),
            Status::RestoreFailed(SaveError::LockTimeout(d)) => with(Msg::RestoreAbortedLocked, d),
            Status::RestoreFailed(SaveError::Backup(d)) => with(Msg::RestoreFailedBackup, d),
            Status::RestoreFailed(SaveError::Read(d) | SaveError::Write(d)) => {
                with(Msg::RestoreFailed, d)
            }
            // 書き戻しでは外部の変更を検知しないので起こらない
            Status::RestoreFailed(SaveError::Modified) => tr(lang, Msg::RestoreFailed).to_owned(),
            Status::KeymapSaved(n) => with(Msg::KeymapSaved, n),
            Status::KeymapOverwritten(n) => with(Msg::KeymapOverwritten, n),
            Status::KeymapRenamed(n) => with(Msg::KeymapRenamed, n),
            Status::KeymapDeleted(n) => with(Msg::KeymapDeleted, n),
            Status::KeymapExported(p) => with(Msg::KeymapExported, p),
            Status::KeymapImported(n) => with(Msg::KeymapImported, n),
            Status::KeymapFailed(d) => with(Msg::KeymapFailed, d),
            Status::ImportRejected(e) => with(Msg::ImportRejected, &invalid_keymap_reason(lang, e)),
            Status::KeymapDirNotFound => tr(lang, Msg::KeymapDirNotFound).to_owned(),
            Status::Applied(n) => with(Msg::Applied, n),
            Status::AppliedNotApplied(n, e) => format!(
                "{}: {n} / {}",
                tr(lang, Msg::Applied),
                with(Msg::SavedNotApplied, &signal_detail(lang, e))
            ),
            Status::LoadedIntoEditor(n) => with(Msg::LoadedIntoEditor, n),
            Status::PresetOverlaid(n) => with(Msg::PresetOverlaid, n),
            Status::HotkeyAssigned(n, h) => format!("{}: {n} ({h})", tr(lang, Msg::HotkeyAssigned)),
            Status::HotkeyRemoved(n) => with(Msg::HotkeyRemoved, n),
            Status::HotkeyRejected(d) => d.clone(),
            Status::AppSettingsSaveFailed(d) => with(Msg::AppSettingsSaveFailed, d),
            Status::AppSettingsUnreadable => tr(lang, Msg::AppSettingsUnreadable).to_owned(),
            Status::AutoStartFailed(d) => with(Msg::AutoStartFailed, d),
        }
    }
}

/// 開いている確認ダイアログ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialog {
    /// 未保存の変更があるときにウィンドウを閉じようとした
    CloseWithUnsaved,
    /// 未保存の変更を破棄してファイルを読み直すか
    DiscardAndReload,
    /// 導入前の設定に戻すか
    Restore,
}

/// 確認ダイアログで選ばれた操作。
enum DialogChoice {
    Save,
    Discard,
    Cancel,
}

/// 適用・編集に読み込むリマップ定義の出どころ。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Source {
    /// 保存したキーマップ(`keymaps` の中のファイル名)
    Keymap(String),
    /// 同梱のプリセット
    Preset(preset::PresetId),
}

impl Source {
    /// 本アプリの設定でホットキーを割り当てる対象としての表し方。
    fn target(&self) -> Target {
        match self {
            Source::Keymap(file) => Target::Keymap(file.clone()),
            Source::Preset(id) => Target::Preset(id.id().to_owned()),
        }
    }

    /// 設定の対象から引く。知らないプリセットの ID なら `None`。
    fn from_target(target: &Target) -> Option<Source> {
        match target {
            Target::Keymap(file) => Some(Source::Keymap(file.clone())),
            Target::Preset(id) => preset::PresetId::from_id(id).map(Source::Preset),
        }
    }
}

/// ホットキーを取り込むダイアログ(§4.5.2)。
struct HotkeyDialog {
    source: Source,
    /// 対象の表示名
    name: String,
    /// 最後に押された組み合わせ(Win は含まない)
    captured: Option<Hotkey>,
    /// Win を加える(Win との組み合わせはウィンドウに届かないことが多いため、チェックで指定する)
    win: bool,
}

impl HotkeyDialog {
    /// 割り当てる組み合わせ。
    fn hotkey(&self) -> Option<Hotkey> {
        self.captured.map(|h| Hotkey { win: self.win, ..h })
    }
}

/// ホットキー・トレイのメニューでの切り替えの結果(§4.5.2)。通知の文言の元にする。
#[derive(Debug, Clone, PartialEq, Eq)]
enum SwitchOutcome {
    Switched {
        name: String,
        /// 未保存の編集を破棄した
        discarded: bool,
        /// 保存したが PowerToys への通知に失敗した(理由)
        not_applied: Option<String>,
    },
    /// 切り替えなかった(理由)
    Failed { name: String, reason: String },
}

impl SwitchOutcome {
    /// 通知の文言。
    fn notification(&self, lang: Lang) -> tray::Notification {
        let title = tr(lang, Msg::NotificationTitle).to_owned();
        match self {
            SwitchOutcome::Switched {
                name,
                discarded,
                not_applied,
            } => {
                let mut body = fill(tr(lang, Msg::SwitchedTo), name);
                if *discarded {
                    body.push('\n');
                    body.push_str(tr(lang, Msg::SwitchDiscardedUnsaved));
                }
                if let Some(reason) = not_applied {
                    body.push('\n');
                    body.push_str(&format!("{}: {reason}", tr(lang, Msg::SavedNotApplied)));
                }
                tray::Notification {
                    title,
                    body,
                    warning: not_applied.is_some(),
                }
            }
            SwitchOutcome::Failed { name, reason } => tray::Notification {
                title,
                body: format!("{}\n{reason}", fill(tr(lang, Msg::SwitchFailed), name)),
                warning: true,
            },
        }
    }

    /// 通知を出すか。通知をオフにしていても、切り替えられなかったことと反映されていない
    /// 可能性は知らせる(ウィンドウを隠していると、ほかに気づく手段がないため)。
    fn should_notify(&self, notifications_enabled: bool) -> bool {
        match self {
            SwitchOutcome::Switched { not_applied, .. } => {
                notifications_enabled || not_applied.is_some()
            }
            SwitchOutcome::Failed { .. } => true,
        }
    }
}

/// キーマップの操作の確認・名前の入力のダイアログ。
enum KeymapDialog {
    /// 今の内容を新しいキーマップとして保存する(名前を入力)
    SaveNew {
        name: String,
        focus: bool,
    },
    /// 名前を変える
    Rename {
        file: String,
        name: String,
        focus: bool,
    },
    ConfirmDelete {
        file: String,
        name: String,
    },
    ConfirmOverwrite {
        file: String,
        name: String,
    },
    /// 未保存の編集があるときに適用する
    ConfirmApply {
        source: Source,
        name: String,
    },
    /// 未保存の編集があるときに編集に読み込む
    ConfirmLoad {
        source: Source,
        name: String,
    },
}

/// キーマップとプリセットの一覧で押された操作。
enum PanelAction {
    SaveNew,
    Import,
    Apply(Source),
    Load(Source),
    Overwrite(String),
    Rename(String),
    Delete(String),
    Export(String),
    Overlay(preset::PresetId),
    /// ホットキーを取り込むダイアログを開く
    SetHotkey(Source),
    RemoveHotkey(Source),
    SetAutoStart(bool),
    SetNotifications(bool),
}

struct App {
    lang: Lang,
    /// 読み込み・保存するファイル
    path: Option<PathBuf>,
    doc: Result<Document, LoadError>,
    layout: Layout,
    /// 選択中の物理キー(`layout.keys` の添字)
    selected: Option<usize>,
    /// リマップ先として選べるキーコード
    choices: Vec<KeyCode>,
    /// リマップ先の一覧の検索欄に入力した文字列
    target_search: String,
    /// キーボード図で変換先を選んでいる間の、リマップ元のコード(§4.10)
    picking: Option<KeyCode>,
    /// 引数でファイルを指定して起動した(PowerToys の状態の判定をしない)
    from_argument: bool,
    /// PowerToys の設定と本アプリのデータの場所
    places: Places,
    /// 導入前の設定のバックアップ
    initial: InitialBackup,
    log: Log,
    /// 保存したキーマップ(名前の順)
    keymaps: Vec<keymap::Entry>,
    /// `keymaps` にあるが読めなかったファイル
    unreadable_keymaps: Vec<keymap::Unreadable>,
    status: Option<Status>,
    dialog: Option<Dialog>,
    /// キーマップの操作の確認・名前の入力のダイアログ
    keymap_dialog: Option<KeymapDialog>,
    /// 未保存の変更があっても閉じてよい(確認ダイアログで保存または破棄を選んだ)
    allow_close: bool,
    /// 本アプリの設定(§4.5.2)。引数でファイルを指定して起動したときは読まず、書かない
    settings: AppSettings,
    /// 本アプリの設定ファイルを読めなかった(詳細)。直すか消すまで上書きしない
    settings_error: Option<String>,
    /// ホットキーを取り込むダイアログ
    hotkey_dialog: Option<HotkeyDialog>,
    /// タスクトレイとホットキー(Windows で、引数でファイルを指定していないとき)
    resident: Option<tray::Resident>,
    /// 常駐できなかった理由(画面に出す)
    resident_error: Option<String>,
    /// 最後にトレイへ送ったメニュー
    sent_menu: Option<tray::Menu>,
    /// 最後に登録を頼んだホットキー
    sent_hotkeys: Option<Vec<(Source, Hotkey)>>,
    /// 登録できなかったホットキー(対象と理由)
    hotkey_failures: Vec<(Source, String)>,
    /// トレイのメニューの「終了」で閉じようとしている(× ボタンでは隠すだけ)
    quitting: bool,
    /// トレイだけで起動したので、最初に描くフレームでウィンドウを隠す
    start_hidden: bool,
    /// トレイだけで起動したときに、ウィンドウを一瞬も見せないための DWM のクローク(§4.5)
    cloak: cloak::Cloak,
}

impl App {
    /// `arg` があればそのファイルを開く。なければ PowerToys の状態を判定して開く。
    fn new(arg: Option<PathBuf>, places: Places, log: Log) -> Self {
        let from_argument = arg.is_some();
        let opened = match arg {
            Some(p) => Opened::argument(p, &log),
            None => Opened::powertoys(&places, &log),
        };
        // 本アプリの設定。読めなければ既定の設定で動き、ファイルは上書きしない
        let (settings, settings_error) = match (&places.settings_path, from_argument) {
            (Some(path), false) => match app_settings::load(path) {
                Ok(s) => (s, None),
                Err(e) => {
                    log.write(&format!("app settings: failed to read: {e}"));
                    (AppSettings::default(), Some(e))
                }
            },
            _ => (AppSettings::default(), None),
        };
        let mut app = Self {
            lang: Lang::default(),
            path: opened.path,
            doc: opened.doc,
            layout: Layout::us_ansi(),
            selected: None,
            choices: keys::target_choices(),
            target_search: String::new(),
            picking: None,
            from_argument,
            places,
            initial: opened.initial,
            log,
            keymaps: Vec::new(),
            unreadable_keymaps: Vec::new(),
            status: None,
            dialog: None,
            keymap_dialog: None,
            allow_close: false,
            settings,
            settings_error,
            hotkey_dialog: None,
            resident: None,
            resident_error: None,
            sent_menu: None,
            sent_hotkeys: None,
            hotkey_failures: Vec::new(),
            quitting: false,
            start_hidden: false,
            cloak: cloak::Cloak::default(),
        };
        app.refresh_keymaps();
        app
    }

    /// PowerToys の状態を判定し直し、通常になっていれば初回のバックアップと読み込みに進む
    /// (「再確認」のボタン)。
    fn recheck(&mut self) {
        if self.from_argument {
            return;
        }
        let opened = Opened::powertoys(&self.places, &self.log);
        self.path = opened.path;
        self.doc = opened.doc;
        self.initial = opened.initial;
        self.status = None;
        self.picking = None;
    }

    /// 結果を `status` に入れ、ログに残す。
    fn set_status(&mut self, what: &str, status: Status) {
        self.log
            .write(&format!("{what}: {}", status.message(Lang::En)));
        self.status = Some(status);
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_ok_and(Document::is_dirty)
    }

    /// 編集中の設定を保存し、結果を `status` に入れる。
    fn save(&mut self) {
        let (Some(path), Ok(doc)) = (&self.path, &mut self.doc) else {
            return;
        };
        if self.initial.blocks_save() {
            self.set_status("save", Status::SaveBlockedByInitialBackup);
            return;
        }
        let Some(backup_dir) = &self.places.backup_dir else {
            self.set_status("save", Status::BackupDirNotFound);
            return;
        };
        let opts = save::Options::new(backup_dir.clone());
        let status = match save::save(path, &doc.config, &doc.snapshot, &opts) {
            Ok(written) => {
                doc.mark_saved(written);
                match save::signal_settings_changed() {
                    Ok(()) => Status::Saved,
                    Err(e) => Status::SavedNotApplied(e),
                }
            }
            Err(e) => Status::SaveFailed(e),
        };
        self.set_status(&format!("save {}", path.display()), status);
    }

    /// ファイルを読み直す。編集中の内容は破棄する。失敗したら編集中の内容を残す。
    ///
    /// 引数で指定したファイルでなければ、PowerToys の状態も判定し直す(§1.1)。通常でなくなって
    /// いれば案内に切り替える。
    fn reload(&mut self) {
        if !self.from_argument {
            let opened = Opened::powertoys(&self.places, &self.log);
            match (&opened.doc, &opened.path) {
                // 同じファイルを読めなかったときは編集中の内容を残す
                (Err(LoadError::Failed(e)), Some(p)) if self.path.as_ref() == Some(p) => {
                    let e = e.clone();
                    self.initial = opened.initial;
                    self.set_status("reload", Status::ReloadFailed(e));
                }
                _ => {
                    let ok = opened.doc.is_ok();
                    self.path = opened.path;
                    self.doc = opened.doc;
                    self.initial = opened.initial;
                    self.picking = None;
                    self.status = None;
                    if ok {
                        self.set_status("reload", Status::Reloaded);
                    }
                }
            }
            return;
        }
        let Some(path) = &self.path else {
            return;
        };
        let status = match Document::load(path) {
            Ok(doc) => {
                self.doc = Ok(doc);
                self.picking = None;
                Status::Reloaded
            }
            Err(e) => Status::ReloadFailed(e),
        };
        self.set_status("reload", status);
    }

    /// キーマップのフォルダ。
    fn store(&self) -> Option<keymap::Store> {
        self.places.keymaps_dir.clone().map(keymap::Store::new)
    }

    /// キーマップの一覧を読み直す。
    fn refresh_keymaps(&mut self) {
        let (entries, unreadable) = match self.store() {
            Some(store) => store.list(),
            None => (Vec::new(), Vec::new()),
        };
        self.keymaps = entries;
        self.unreadable_keymaps = unreadable;
        self.prune_hotkeys();
    }

    /// 削除されたキーマップ(ファイルがなくなったもの)のホットキーの割り当てを外す(§4.5.2)。
    /// ホットキーはファイル名で対応づけているため。
    fn prune_hotkeys(&mut self) {
        if self.from_argument || self.settings_error.is_some() {
            return;
        }
        let Some(dir) = self.places.keymaps_dir.clone() else {
            return;
        };
        // フォルダそのものが無い・一覧を読めないときは、消えたのかどうか分からないので外さない
        if std::fs::read_dir(&dir).is_err() {
            return;
        }
        let mut settings = self.settings.clone();
        let exists = |file: &str| keymap::is_plain_file_name(file) && dir.join(file).is_file();
        if settings.prune_keymaps(exists) {
            self.store_settings("hotkey prune", settings);
        }
    }

    /// 新しく作ったキーマップのファイル `file` に、前に同じ名前だったファイルの割り当てが
    /// 残っていれば外す(新しいキーマップは割り当てなしで始める)。
    fn clear_stale_hotkey(&mut self, file: &str) {
        let target = Target::Keymap(file.to_owned());
        if self.from_argument || self.settings.hotkey(&target).is_none() {
            return;
        }
        let mut settings = self.settings.clone();
        settings.unassign(&target);
        self.store_settings("hotkey clear stale", settings);
    }

    /// キーマップのファイルを操作し、結果を `status` に入れて一覧を読み直す。
    fn keymap_op(&mut self, what: &str, op: impl FnOnce(&keymap::Store) -> Result<Status, String>) {
        let Some(store) = self.store() else {
            self.set_status(what, Status::KeymapDirNotFound);
            return;
        };
        let status = op(&store).unwrap_or_else(Status::KeymapFailed);
        self.set_status(what, status);
        self.refresh_keymaps();
    }

    /// 今の内容(キーボード図の内容。未保存の編集も含む)を新しいキーマップとして保存する。
    fn save_new_keymap(&mut self, name: &str) {
        let Ok(doc) = &self.doc else {
            return;
        };
        let k = keymap::KeymapFile::new(name.trim(), &doc.config);
        let mut created = None;
        self.keymap_op("keymap save", |store| {
            created = Some(store.create(&k)?);
            Ok(Status::KeymapSaved(k.name.clone()))
        });
        if let Some(file) = created {
            self.clear_stale_hotkey(&file);
        }
    }

    /// キーマップ `file` を今の内容で上書きする。
    fn overwrite_keymap(&mut self, file: &str) {
        let Ok(doc) = &self.doc else {
            return;
        };
        let definition = doc.config.clone();
        self.keymap_op("keymap overwrite", |store| {
            let k = store.overwrite(file, &definition)?;
            Ok(Status::KeymapOverwritten(k.name))
        });
    }

    fn rename_keymap(&mut self, file: &str, name: &str) {
        self.keymap_op("keymap rename", |store| {
            let k = store.rename(file, name.trim())?;
            Ok(Status::KeymapRenamed(k.name))
        });
    }

    fn delete_keymap(&mut self, file: &str) {
        let name = self.keymap_name(file);
        self.keymap_op("keymap delete", |store| {
            store.delete(file)?;
            Ok(Status::KeymapDeleted(name))
        });
    }

    fn export_keymap(&mut self, file: &str, dest: &Path) {
        self.keymap_op("keymap export", |store| {
            store.export(file, dest)?;
            Ok(Status::KeymapExported(dest.display().to_string()))
        });
    }

    /// `src` を検証して取り込む。受け付けないときは理由を表示する。
    fn import_keymap(&mut self, src: &Path) {
        let existing = self.keymaps.clone();
        let what = format!("keymap import {}", src.display());
        let mut created = None;
        self.keymap_op(&what, |store| match store.import(src, &existing) {
            Ok(e) => {
                created = Some(e.file_name);
                Ok(Status::KeymapImported(e.keymap.name))
            }
            Err(keymap::ImportError::Invalid(reason)) => Ok(Status::ImportRejected(reason)),
            Err(keymap::ImportError::Io(d)) => Err(d),
        });
        if let Some(file) = created {
            self.clear_stale_hotkey(&file);
        }
    }

    /// 一覧にあるキーマップの名前(なければファイル名)。
    fn keymap_name(&self, file: &str) -> String {
        self.keymaps
            .iter()
            .find(|e| e.file_name == file)
            .map_or_else(|| file.to_owned(), |e| e.keymap.name.clone())
    }

    /// 適用・編集に読み込むリマップ定義と、その名前。キーマップはファイルから読み直す。
    fn source_definition(&mut self, source: &Source) -> Option<(String, Config)> {
        match source {
            Source::Preset(id) => {
                Some((tr(self.lang, preset_msg(*id)).to_owned(), id.definition()))
            }
            Source::Keymap(file) => {
                let store = self.store()?;
                match store.read(file) {
                    Ok(k) => Some((k.name, k.definition)),
                    Err(e) => {
                        self.set_status("keymap read", Status::KeymapFailed(e));
                        self.refresh_keymaps();
                        None
                    }
                }
            }
        }
    }

    /// キーマップまたはプリセットを適用する(§4.5.1)。保存済みの内容のリマップ定義の 4 つの区分を
    /// 丸ごと置き換え、§4.8 の保存処理ですぐ反映する。編集中の内容は破棄する。
    fn apply(&mut self, source: &Source) {
        if self.path.is_none() || self.doc.is_err() {
            return;
        }
        if self.initial.blocks_save() {
            self.set_status("apply", Status::SaveBlockedByInitialBackup);
            return;
        }
        let Some(backup_dir) = self.places.backup_dir.clone() else {
            self.set_status("apply", Status::BackupDirNotFound);
            return;
        };
        let Some((name, definition)) = self.source_definition(source) else {
            return;
        };
        let (Some(path), Ok(doc)) = (&self.path, &mut self.doc) else {
            return;
        };
        let applied = keymap::apply_definition(&doc.saved_config, &definition);
        let opts = save::Options::new(backup_dir);
        let status = match save::save(path, &applied, &doc.snapshot, &opts) {
            Ok(written) => {
                doc.config = applied;
                doc.mark_saved(written);
                match save::signal_settings_changed() {
                    Ok(()) => Status::Applied(name.clone()),
                    Err(e) => Status::AppliedNotApplied(name.clone(), e),
                }
            }
            Err(e) => Status::SaveFailed(e),
        };
        self.picking = None;
        self.set_status(&format!("apply {name} to {}", path.display()), status);
    }

    /// キーマップまたはプリセットを編集に読み込む。保存済みの内容の 4 つの区分を置き換えた状態にし、
    /// Save を押すまで反映しない。編集中の内容は破棄する。
    fn load_into_editor(&mut self, source: &Source) {
        if self.doc.is_err() {
            return;
        }
        let Some((name, definition)) = self.source_definition(source) else {
            return;
        };
        let Ok(doc) = &mut self.doc else {
            return;
        };
        doc.config = keymap::apply_definition(&doc.saved_config, &definition);
        self.picking = None;
        self.set_status("load into editor", Status::LoadedIntoEditor(name));
    }

    /// プリセットを今の編集に重ねる(§4.6)。同じリマップ元はプリセットを優先する。
    fn overlay_preset(&mut self, id: preset::PresetId) {
        let Ok(doc) = &mut self.doc else {
            return;
        };
        let preset = id.definition();
        keymap::overlay(
            &mut doc.config.remap_keys.in_process,
            &preset.remap_keys.in_process,
        );
        self.picking = None;
        let name = tr(self.lang, preset_msg(id)).to_owned();
        self.set_status("overlay preset", Status::PresetOverlaid(name));
    }

    /// 適用できるか。できなければ理由。
    fn apply_unavailable(&self, lang: Lang) -> Option<&'static str> {
        if self.doc.is_err() || self.path.is_none() {
            Some(tr(lang, Msg::NoConfigLoaded))
        } else if self.initial.blocks_save() {
            Some(tr(lang, Msg::SaveBlockedByInitialBackup))
        } else {
            None
        }
    }

    /// 一覧の操作の出どころの表示名。
    fn source_name(&self, source: &Source) -> String {
        match source {
            Source::Keymap(file) => self.keymap_name(file),
            Source::Preset(id) => tr(self.lang, preset_msg(*id)).to_owned(),
        }
    }

    // ---- 常駐・ホットキー・自動起動(§4.5・§4.5.2) ----

    /// 常駐・ホットキー・自動起動の対象か(引数でファイルを指定した起動は対象外)。
    fn is_resident_mode(&self) -> bool {
        !self.from_argument
    }

    /// タスクトレイとホットキーのスレッドを立て、自動起動の登録を設定に合わせる。
    /// `activation` は後から起動した側からの知らせを待つイベント。
    fn start_resident(&mut self, ctx: &egui::Context, activation: Option<usize>) {
        if !self.is_resident_mode() {
            return;
        }
        match tray::Resident::start(ctx.clone(), activation, self.log.clone()) {
            Ok(r) => {
                self.log.write("tray: started");
                self.resident = Some(r);
            }
            Err(e) => {
                self.log.write(&format!("tray: not available: {e}"));
                self.resident_error = Some(e);
                // トレイがないと隠したウィンドウを開く手段がないので、トレイだけの起動でも表示する
                self.start_hidden = false;
            }
        }
        // 設定を読めなかったときは、ユーザーの意図が分からないので登録を変えない
        if self.settings_error.is_none() {
            self.sync_autostart(false);
        }
    }

    /// 自動起動の登録を設定に合わせる(有効なら今の exe のパスにする)。失敗はログに残し、
    /// `report` なら画面にも出す。
    fn sync_autostart(&mut self, report: bool) {
        match autostart::sync(self.settings.auto_start) {
            Ok(autostart::Synced::Registered(v)) => {
                self.log.write(&format!("autostart: registered: {v}"));
            }
            Ok(autostart::Synced::Removed) => self.log.write("autostart: removed"),
            Ok(autostart::Synced::Unchanged | autostart::Synced::Unsupported) => {}
            Err(e) => {
                if report {
                    self.set_status("autostart", Status::AutoStartFailed(e));
                } else {
                    self.log.write(&format!("autostart: failed: {e}"));
                }
            }
        }
    }

    /// 本アプリの設定を `new` にしてファイルに書く。書けたら `true`。
    ///
    /// 引数でファイルを指定した起動では書かない。設定ファイルを読めなかったときは、
    /// 中身を失わないよう上書きしない。
    fn store_settings(&mut self, what: &str, new: AppSettings) -> bool {
        if !self.is_resident_mode() {
            return false;
        }
        if self.settings_error.is_some() {
            self.set_status(what, Status::AppSettingsUnreadable);
            return false;
        }
        let Some(path) = self.places.settings_path.clone() else {
            let detail = tr(Lang::En, Msg::DataDirNotFound).to_owned();
            self.set_status(what, Status::AppSettingsSaveFailed(detail));
            return false;
        };
        match app_settings::store(&path, &new) {
            Ok(()) => {
                self.settings = new;
                self.log.write(&format!("{what}: saved {}", path.display()));
                true
            }
            Err(e) => {
                self.set_status(what, Status::AppSettingsSaveFailed(e));
                false
            }
        }
    }

    /// `source` にホットキーを割り当てる。本アプリの中で重複するものや、修飾キーを含まないものは
    /// 拒否する。
    fn assign_hotkey(&mut self, source: &Source, hotkey: Hotkey) {
        let name = self.source_name(source);
        let mut settings = self.settings.clone();
        match settings.assign(&source.target(), hotkey) {
            Ok(()) => {
                if self.store_settings("hotkey assign", settings) {
                    self.set_status(
                        "hotkey assign",
                        Status::HotkeyAssigned(name, hotkey.format()),
                    );
                }
            }
            Err(e) => {
                let reason = self.assign_error_text(self.lang, &e);
                self.set_status("hotkey assign", Status::HotkeyRejected(reason));
            }
        }
    }

    fn unassign_hotkey(&mut self, source: &Source) {
        let name = self.source_name(source);
        let mut settings = self.settings.clone();
        if settings.unassign(&source.target()) && self.store_settings("hotkey remove", settings) {
            self.set_status("hotkey remove", Status::HotkeyRemoved(name));
        }
    }

    /// 割り当てを受け付けなかった理由の文。
    fn assign_error_text(&self, lang: Lang, e: &AssignError) -> String {
        match e {
            AssignError::Invalid(hotkey::Invalid::NoModifier) => {
                tr(lang, Msg::HotkeyNeedsModifier).to_owned()
            }
            AssignError::Invalid(hotkey::Invalid::UnsupportedKey) => {
                tr(lang, Msg::HotkeyUnsupportedKey).to_owned()
            }
            AssignError::Taken(holder) => {
                format!(
                    "{}: {}",
                    tr(lang, Msg::HotkeyTaken),
                    self.target_name(lang, holder)
                )
            }
        }
    }

    /// ホットキーの割り当ての対象の表示名。キーマップは名前(一覧に無ければファイル名)、
    /// プリセットは名前、知らないプリセットの ID はそのことと ID。
    fn target_name(&self, lang: Lang, target: &Target) -> String {
        match (Source::from_target(target), target) {
            (Some(source), _) => self.source_name(&source),
            (None, Target::Preset(id)) => format!("{}: {id}", tr(lang, Msg::UnknownPreset)),
            (None, Target::Keymap(file)) => file.clone(),
        }
    }

    /// 自動起動の有効・無効を変え、登録を合わせる。
    fn set_auto_start(&mut self, enabled: bool) {
        let mut settings = self.settings.clone();
        settings.auto_start = enabled;
        if self.store_settings("autostart setting", settings) {
            self.sync_autostart(true);
        }
    }

    fn set_notifications(&mut self, enabled: bool) {
        let mut settings = self.settings.clone();
        settings.notifications = enabled;
        self.store_settings("notifications setting", settings);
    }

    /// ホットキー・トレイのメニューから `source` に切り替える(§4.5.2)。
    ///
    /// 「適用」と同じ処理([`App::apply`])を通す。未保存の編集があっても切り替え、破棄する。
    /// 適用できないとき(初回のバックアップに失敗しているなど)は切り替えない。
    ///
    /// 画面の「適用」と違い、適用の直前に定義ファイルを読み直す。外部で変更されたあとも、外部の
    /// 変更の検知で失敗し続けないようにするため(置き換える前の内容は世代バックアップに残る)。
    /// 読み直したあとに切り替えられなかったとき、未保存の編集があれば、切り替える前の状態
    /// (編集と、読み込んだ時点の内容)に丸ごと戻す。編集を残しつつ、あとで Save したときに外部の
    /// 変更の検知(§4.8)が働くようにするため。未保存の編集がなければ、読み直した内容のままにする。
    fn switch_to(&mut self, source: &Source) -> SwitchOutcome {
        let lang = self.lang;
        let name = self.source_name(source);
        if let Some(reason) = self.apply_unavailable(lang) {
            let outcome = SwitchOutcome::Failed {
                name,
                reason: reason.to_owned(),
            };
            self.log.write(&format!("switch: not switched: {reason}"));
            return outcome;
        }
        let dirty = self.is_dirty();
        let Some(path) = self.path.clone() else {
            // apply_unavailable で確かめているので起こらない
            return SwitchOutcome::Failed {
                name,
                reason: tr(lang, Msg::NoConfigLoaded).to_owned(),
            };
        };
        let fresh = match Document::load(&path) {
            Ok(d) => d,
            Err(e) => {
                self.log
                    .write(&format!("switch: not switched: failed to reload: {e}"));
                return SwitchOutcome::Failed {
                    name,
                    reason: format!("{}: {e}", tr(lang, Msg::ReloadFailed)),
                };
            }
        };
        let previous = std::mem::replace(&mut self.doc, Ok(fresh));
        self.status = None;
        self.apply(source);
        let outcome = match &self.status {
            Some(Status::Applied(n)) => SwitchOutcome::Switched {
                name: n.clone(),
                discarded: dirty,
                not_applied: None,
            },
            Some(Status::AppliedNotApplied(n, e)) => SwitchOutcome::Switched {
                name: n.clone(),
                discarded: dirty,
                not_applied: Some(signal_detail(lang, e)),
            },
            Some(other) => SwitchOutcome::Failed {
                name,
                reason: other.message(lang),
            },
            // キーマップのフォルダが分からないと、適用は理由を残さずに戻る
            None => SwitchOutcome::Failed {
                name,
                reason: tr(lang, Msg::KeymapDirNotFound).to_owned(),
            },
        };
        if matches!(outcome, SwitchOutcome::Failed { .. }) && dirty {
            // 切り替えなかったので、未保存の編集を読み込んだ時点の内容ごと戻す
            self.doc = previous;
        }
        if matches!(outcome, SwitchOutcome::Switched { .. }) {
            // 今の内容での上書きなどの確認は、切り替えたあとの内容で実行されないよう閉じる。
            // ホットキーの取り込みダイアログは、開いている間ホットキーが効かないので残す
            self.keymap_dialog = None;
            // 未保存の編集についての確認は意味がなくなった
            if matches!(
                self.dialog,
                Some(Dialog::CloseWithUnsaved | Dialog::DiscardAndReload)
            ) {
                self.dialog = None;
                self.quitting = false;
            }
        }
        outcome
    }

    /// トレイアイコンを出せているか(出せていなければ × で隠さない。§4.5.2)。
    fn tray_visible(&self) -> bool {
        self.resident
            .as_ref()
            .is_some_and(tray::Resident::icon_visible)
    }

    /// ウィンドウを閉じる要求(× ボタン、トレイの「終了」)を扱う。
    ///
    /// トレイアイコンを出せていれば、× はウィンドウを隠してトレイに残す(編集内容は保持する)。
    /// ホットキーの取り込みダイアログは閉じる(開いたままだとホットキーが止まったままになる)。
    /// 「終了」のとき、またはトレイアイコンを出せていないときは、未保存の編集があれば確認する。
    fn on_close_requested(&mut self, ctx: &egui::Context, tray_visible: bool) {
        if tray_visible && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hotkey_dialog = None;
        } else if self.is_dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_window(ctx);
            self.dialog = Some(Dialog::CloseWithUnsaved);
            self.keymap_dialog = None;
            self.hotkey_dialog = None;
        }
    }

    /// トレイだけで起動するとき、最初の表示より前にウィンドウをクロークする(§4.5)。
    /// eframe は最初のフレームのあとに必ずウィンドウを表示するので、そのまま隠すと一瞬見える。
    /// クロークできなければ、今までどおり(一瞬見えることがある)動く。
    fn cloak_if_starting_hidden(&mut self, hwnd: Option<usize>) {
        if !cloak::should_cloak_at_start(self.start_hidden, self.resident.is_some()) {
            return;
        }
        self.cloak = cloak::Cloak::new(hwnd);
        match self.cloak.cloak() {
            Ok(()) => self.log.write("window: cloaked until opened"),
            Err(e) => self.log.write(&format!("window: failed to cloak: {e}")),
        }
    }

    /// ウィンドウを出して前面に持ってくる。
    ///
    /// クロークしていれば、`Visible(true)` を送る前(ウィンドウが隠れている間)に外す。
    /// 最初のフレームより前に開くことになったとき(起動直後に二重起動の知らせが届いたなど)は、
    /// 最初のフレームで隠すのもやめる。
    fn show_window(&mut self, ctx: &egui::Context) {
        self.start_hidden = false;
        if let Err(e) = self.cloak.uncloak() {
            self.log.write(&format!("window: failed to uncloak: {e}"));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// トレイのスレッドからの知らせを処理する。
    fn handle_resident_events(&mut self, ctx: &egui::Context) {
        let mut events = Vec::new();
        if let Some(r) = &self.resident {
            while let Some(e) = r.try_recv() {
                events.push(e);
            }
        }
        for event in events {
            match event {
                tray::Event::OpenWindow => self.show_window(ctx),
                tray::Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                tray::Event::Switch(source) => {
                    let outcome = self.switch_to(&source);
                    if outcome.should_notify(self.settings.notifications)
                        && let Some(r) = &self.resident
                    {
                        r.notify(outcome.notification(self.lang));
                    }
                }
                tray::Event::HotkeysRegistered(results) => {
                    self.hotkey_failures = results
                        .into_iter()
                        .filter_map(|(s, r)| r.err().map(|e| (s, e)))
                        .collect();
                    for (s, e) in &self.hotkey_failures {
                        self.log
                            .write(&format!("hotkey: failed to register for {s:?}: {e}"));
                    }
                }
            }
        }
    }

    /// 登録するホットキー。ホットキーを取り込んでいる間は、押した組み合わせが登録済みの
    /// ホットキーとして奪われないよう、何も登録しない。対象がないもの(読めない設定、知らない
    /// プリセット、なくなったキーマップ)は除く。
    fn active_hotkeys(&self) -> Vec<(Source, Hotkey)> {
        if self.hotkey_dialog.is_some() {
            return Vec::new();
        }
        self.settings
            .assignments()
            .into_iter()
            .filter_map(|(t, h)| Some((Source::from_target(&t)?, h)))
            .filter(|(s, _)| match s {
                Source::Keymap(file) => {
                    self.keymaps.iter().any(|e| &e.file_name == file)
                        || self.unreadable_keymaps.iter().any(|u| &u.file_name == file)
                }
                Source::Preset(_) => true,
            })
            .collect()
    }

    /// トレイのメニューとツールチップ。今の PowerToys の内容と一致するものにチェックを付ける。
    fn tray_menu(&self) -> tray::Menu {
        let lang = self.lang;
        let saved = self.doc.as_ref().ok().map(|d| &d.saved_config);
        let in_use = |def: &Config| saved.is_some_and(|c| keymap::same_remaps(def, c));
        let item = |source: Source, label: String, def: &Config| tray::MenuItem {
            label,
            hotkey: self.settings.hotkey(&source.target()).map(|h| h.format()),
            checked: in_use(def),
            source,
        };
        let keymaps: Vec<tray::MenuItem> = self
            .keymaps
            .iter()
            .map(|e| {
                let source = Source::Keymap(e.file_name.clone());
                item(source, e.keymap.name.clone(), &e.keymap.definition)
            })
            .collect();
        let presets: Vec<tray::MenuItem> = preset::PresetId::ALL
            .iter()
            .map(|&id| {
                let label = tr(lang, preset_msg(id)).to_owned();
                item(Source::Preset(id), label, &id.definition())
            })
            .collect();
        let tooltip = if saved.is_none() {
            tr(lang, Msg::TrayTooltipNotLoaded).to_owned()
        } else {
            let names: Vec<&str> = keymaps
                .iter()
                .chain(&presets)
                .filter(|i| i.checked)
                .map(|i| i.label.as_str())
                .collect();
            if names.is_empty() {
                tr(lang, Msg::TrayTooltipNoneInUse).to_owned()
            } else {
                fill(tr(lang, Msg::TrayTooltipInUse), &names.join(", "))
            }
        };
        tray::Menu {
            tooltip,
            open_window: tr(lang, Msg::TrayOpenWindow).to_owned(),
            exit: tr(lang, Msg::TrayExit).to_owned(),
            sections: vec![
                tray::MenuSection {
                    heading: tr(lang, Msg::KeymapsHeading).to_owned(),
                    empty: tr(lang, Msg::NoKeymaps).to_owned(),
                    items: keymaps,
                },
                tray::MenuSection {
                    heading: tr(lang, Msg::PresetsHeading).to_owned(),
                    empty: String::new(),
                    items: presets,
                },
            ],
        }
    }

    /// トレイのメニューと、登録するホットキーが変わっていればスレッドに送る。
    fn sync_resident(&mut self) {
        if self.resident.is_none() {
            return;
        }
        let hotkeys = self.active_hotkeys();
        let menu = self.tray_menu();
        let Some(r) = &self.resident else {
            return;
        };
        if self.sent_hotkeys.as_ref() != Some(&hotkeys) {
            r.set_hotkeys(hotkeys.clone());
            self.sent_hotkeys = Some(hotkeys);
        }
        if self.sent_menu.as_ref() != Some(&menu) {
            r.set_menu(menu.clone());
            self.sent_menu = Some(menu);
        }
    }

    /// `source` のホットキーが登録できなかった理由。
    fn hotkey_failure(&self, source: &Source) -> Option<&str> {
        self.hotkey_failures
            .iter()
            .find(|(s, _)| s == source)
            .map(|(_, e)| e.as_str())
    }

    /// キーマップとプリセットの一覧(§4.5.1・§4.6)。押された操作を実行する。
    /// `frame` はファイルのダイアログの親にするウィンドウ。
    fn keymaps_panel(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let lang = self.lang;
        let has_doc = self.doc.is_ok();
        let apply_reason = self.apply_unavailable(lang);
        let saved_config = self.doc.as_ref().ok().map(|d| &d.saved_config);
        let in_use = |def: &Config| saved_config.is_some_and(|c| keymap::same_remaps(def, c));
        let resident = self.is_resident_mode();
        let settings = &self.settings;
        let row_hotkey = |ui: &mut egui::Ui, source: Source, actions: &mut Vec<PanelAction>| {
            if resident {
                let hotkey = settings.hotkey(&source.target());
                let failure = self.hotkey_failure(&source);
                hotkey_row(ui, lang, hotkey, failure, source, actions);
            }
        };
        let mut actions = Vec::new();

        ui.heading(tr(lang, Msg::KeymapsHeading));
        ui.horizontal_wrapped(|ui| {
            let new = ui
                .add_enabled(has_doc, egui::Button::new(tr(lang, Msg::SaveAsNewKeymap)))
                .on_disabled_hover_text(tr(lang, Msg::NoConfigLoaded));
            if new.clicked() {
                actions.push(PanelAction::SaveNew);
            }
            if ui.button(tr(lang, Msg::ImportKeymap)).clicked() {
                actions.push(PanelAction::Import);
            }
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("keymaps_list")
            .show(ui, |ui| {
                if self.keymaps.is_empty() {
                    ui.weak(tr(lang, Msg::NoKeymaps));
                }
                for e in &self.keymaps {
                    let file = &e.file_name;
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(&e.keymap.name);
                            if in_use(&e.keymap.definition) {
                                in_use_badge(ui, lang);
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            let source = Source::Keymap(file.clone());
                            if apply_button(ui, lang, apply_reason) {
                                actions.push(PanelAction::Apply(source.clone()));
                            }
                            if small_enabled(ui, has_doc, tr(lang, Msg::LoadIntoEditor), lang) {
                                actions.push(PanelAction::Load(source));
                            }
                            if small_enabled(ui, has_doc, tr(lang, Msg::OverwriteKeymap), lang) {
                                actions.push(PanelAction::Overwrite(file.clone()));
                            }
                            if ui.small_button(tr(lang, Msg::RenameKeymap)).clicked() {
                                actions.push(PanelAction::Rename(file.clone()));
                            }
                            if ui.small_button(tr(lang, Msg::DeleteKeymap)).clicked() {
                                actions.push(PanelAction::Delete(file.clone()));
                            }
                            if ui.small_button(tr(lang, Msg::ExportKeymap)).clicked() {
                                actions.push(PanelAction::Export(file.clone()));
                            }
                        });
                        row_hotkey(ui, Source::Keymap(file.clone()), &mut actions);
                    });
                }
                if !self.unreadable_keymaps.is_empty() {
                    ui.colored_label(ui.visuals().warn_fg_color, tr(lang, Msg::UnreadableKeymaps));
                    for u in &self.unreadable_keymaps {
                        ui.weak(format!(
                            "{}: {}",
                            u.file_name,
                            invalid_keymap_reason(lang, &u.reason)
                        ));
                    }
                }

                ui.add_space(8.0);
                ui.separator();
                ui.heading(tr(lang, Msg::PresetsHeading));
                for &id in preset::PresetId::ALL {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(tr(lang, preset_msg(id)));
                            ui.weak(tr(lang, Msg::ReadOnlyPreset));
                            if in_use(&id.definition()) {
                                in_use_badge(ui, lang);
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if apply_button(ui, lang, apply_reason) {
                                actions.push(PanelAction::Apply(Source::Preset(id)));
                            }
                            if small_enabled(ui, has_doc, tr(lang, Msg::OverlayPreset), lang) {
                                actions.push(PanelAction::Overlay(id));
                            }
                        });
                        row_hotkey(ui, Source::Preset(id), &mut actions);
                    });
                }
                ui.label(
                    egui::RichText::new(tr(lang, Msg::PresetShortcutNote))
                        .small()
                        .color(ui.visuals().warn_fg_color),
                );
                if resident {
                    ui.add_space(8.0);
                    ui.separator();
                    self.resident_settings(ui, &mut actions);
                }
            });

        for action in actions {
            self.panel_action(action, frame);
        }
    }

    /// 一覧で押された操作を実行する。確認や名前の入力が要るものはダイアログを開く。
    fn panel_action(&mut self, action: PanelAction, frame: &eframe::Frame) {
        let lang = self.lang;
        match action {
            PanelAction::SaveNew => {
                self.keymap_dialog = Some(KeymapDialog::SaveNew {
                    name: String::new(),
                    focus: true,
                });
            }
            PanelAction::Import => {
                if let Some(src) = file_dialog(lang, Msg::ImportKeymap, frame).pick_file() {
                    self.import_keymap(&src);
                }
            }
            PanelAction::Apply(source) => {
                if self.is_dirty() {
                    let name = self.source_name(&source);
                    self.keymap_dialog = Some(KeymapDialog::ConfirmApply { source, name });
                } else {
                    self.apply(&source);
                }
            }
            PanelAction::Load(source) => {
                if self.is_dirty() {
                    let name = self.source_name(&source);
                    self.keymap_dialog = Some(KeymapDialog::ConfirmLoad { source, name });
                } else {
                    self.load_into_editor(&source);
                }
            }
            PanelAction::Overwrite(file) => {
                let name = self.keymap_name(&file);
                self.keymap_dialog = Some(KeymapDialog::ConfirmOverwrite { file, name });
            }
            PanelAction::Rename(file) => {
                let name = self.keymap_name(&file);
                self.keymap_dialog = Some(KeymapDialog::Rename {
                    file,
                    name,
                    focus: true,
                });
            }
            PanelAction::Delete(file) => {
                let name = self.keymap_name(&file);
                self.keymap_dialog = Some(KeymapDialog::ConfirmDelete { file, name });
            }
            PanelAction::Export(file) => {
                let dest = file_dialog(lang, Msg::ExportKeymap, frame)
                    .set_file_name(file.clone())
                    .save_file();
                if let Some(dest) = dest {
                    self.export_keymap(&file, &dest);
                }
            }
            PanelAction::Overlay(id) => self.overlay_preset(id),
            PanelAction::SetHotkey(source) => {
                let name = self.source_name(&source);
                let current = self.settings.hotkey(&source.target());
                self.hotkey_dialog = Some(HotkeyDialog {
                    source,
                    name,
                    captured: current.map(|h| Hotkey { win: false, ..h }),
                    win: current.is_some_and(|h| h.win),
                });
            }
            PanelAction::RemoveHotkey(source) => self.unassign_hotkey(&source),
            PanelAction::SetAutoStart(enabled) => self.set_auto_start(enabled),
            PanelAction::SetNotifications(enabled) => self.set_notifications(enabled),
        }
    }

    /// 常駐・自動起動・通知の設定の欄(§4.5・§4.5.2)。
    fn resident_settings(&self, ui: &mut egui::Ui, actions: &mut Vec<PanelAction>) {
        let lang = self.lang;
        ui.strong(tr(lang, Msg::ResidentHeading));
        if let Some(e) = &self.settings_error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("{}: {e}", tr(lang, Msg::AppSettingsUnreadable)),
            );
        }
        if let Some(e) = &self.resident_error {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{}: {e}", tr(lang, Msg::ResidentUnavailable)),
            );
        }
        let mut auto_start = self.settings.auto_start;
        if ui
            .checkbox(&mut auto_start, tr(lang, Msg::AutoStartOption))
            .changed()
        {
            actions.push(PanelAction::SetAutoStart(auto_start));
        }
        let mut notifications = self.settings.notifications;
        if ui
            .checkbox(&mut notifications, tr(lang, Msg::NotificationsOption))
            .changed()
        {
            actions.push(PanelAction::SetNotifications(notifications));
        }
    }

    /// ホットキーを取り込むダイアログ(§4.5.2)。押された組み合わせを取り込み、修飾キーを含まない
    /// ものや本アプリの中で重複するものは理由を出して割り当てられないようにする。
    fn show_hotkey_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        if self.hotkey_dialog.is_none() {
            return;
        }
        // 押された組み合わせ。文字としてのキー(配列に合わせた VK)で引けなければ、物理的な位置で引く
        let pressed = ctx.input(|i| {
            i.events.iter().rev().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => Hotkey::from_egui(*key, *modifiers)
                    .or_else(|| physical_key.and_then(|p| Hotkey::from_egui(p, *modifiers))),
                _ => None,
            })
        });
        let Some(dialog) = &mut self.hotkey_dialog else {
            return;
        };
        if let Some(h) = pressed {
            dialog.captured = Some(h);
        }
        let candidate = dialog.hotkey();
        let target = dialog.source.target();
        let problem = candidate.and_then(|h| match h.validate() {
            Err(e) => Some(AssignError::Invalid(e)),
            Ok(()) => self.settings.holder_of(h, &target).map(AssignError::Taken),
        });
        let problem_text = problem.as_ref().map(|e| self.assign_error_text(lang, e));
        let Some(dialog) = &mut self.hotkey_dialog else {
            return;
        };
        let mut result: Option<bool> = None;
        let response = egui::Modal::new(egui::Id::unique("hotkey_dialog")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(tr(lang, Msg::HotkeyPrompt));
            ui.strong(&dialog.name);
            ui.add_space(8.0);
            match candidate {
                Some(h) => ui.heading(h.format()),
                None => ui.weak(tr(lang, Msg::HotkeyWaiting)),
            };
            ui.checkbox(&mut dialog.win, tr(lang, Msg::HotkeyAddWin))
                .on_hover_text(tr(lang, Msg::HotkeyWinNote));
            if let Some(text) = &problem_text {
                ui.colored_label(ui.visuals().error_fg_color, text);
            }
            ui.label(
                egui::RichText::new(tr(lang, Msg::HotkeysPaused))
                    .small()
                    .weak(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = candidate.is_some() && problem_text.is_none();
                if ui
                    .add_enabled(ok, egui::Button::new(tr(lang, Msg::HotkeyAssign)))
                    .clicked()
                {
                    result = Some(true);
                }
                if ui.button(tr(lang, Msg::Cancel)).clicked() {
                    result = Some(false);
                }
            });
        });
        // Esc や背景のクリックは取り消しとみなす
        if result.is_none() && response.should_close() {
            result = Some(false);
        }
        let Some(ok) = result else {
            return;
        };
        let Some(dialog) = self.hotkey_dialog.take() else {
            return;
        };
        if ok && let Some(h) = dialog.hotkey() {
            self.assign_hotkey(&dialog.source, h);
        }
    }

    /// キーマップの操作のダイアログを表示し、了承されたら実行する。
    fn show_keymap_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let dirty = self.is_dirty();
        let keymaps = &self.keymaps;
        let Some(dialog) = &mut self.keymap_dialog else {
            return;
        };
        let mut result: Option<bool> = None;
        let response = egui::Modal::new(egui::Id::unique("keymap_dialog")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            result = match dialog {
                KeymapDialog::SaveNew { name, focus } => {
                    let mut text = tr(lang, Msg::NewKeymapPrompt).to_owned();
                    if dirty {
                        text.push('\n');
                        text.push_str(tr(lang, Msg::NewKeymapIncludesUnsaved));
                    }
                    name_input(ui, lang, &text, name, focus, keymaps, None, Msg::Save)
                }
                KeymapDialog::Rename { file, name, focus } => {
                    let text = tr(lang, Msg::RenamePrompt);
                    name_input(
                        ui,
                        lang,
                        text,
                        name,
                        focus,
                        keymaps,
                        Some(file),
                        Msg::RenameKeymap,
                    )
                }
                KeymapDialog::ConfirmDelete { name, .. } => {
                    let text = format!("{}\n\n{name}", tr(lang, Msg::DeleteKeymapConfirm));
                    confirm(ui, lang, &text, Msg::DeleteKeymap)
                }
                KeymapDialog::ConfirmOverwrite { name, .. } => {
                    let mut text = format!("{}\n\n{name}", tr(lang, Msg::OverwriteKeymapConfirm));
                    if dirty {
                        text.push_str("\n\n");
                        text.push_str(tr(lang, Msg::NewKeymapIncludesUnsaved));
                    }
                    confirm(ui, lang, &text, Msg::OverwriteKeymap)
                }
                KeymapDialog::ConfirmApply { name, .. } => {
                    let text = format!("{}\n\n{name}", tr(lang, Msg::ApplyDiscardConfirm));
                    confirm(ui, lang, &text, Msg::Apply)
                }
                KeymapDialog::ConfirmLoad { name, .. } => {
                    let text = format!("{}\n\n{name}", tr(lang, Msg::LoadDiscardConfirm));
                    confirm(ui, lang, &text, Msg::LoadIntoEditor)
                }
            };
        });
        // Esc や背景のクリックは取り消しとみなす
        if result.is_none() && response.should_close() {
            result = Some(false);
        }
        let Some(ok) = result else {
            return;
        };
        let Some(dialog) = self.keymap_dialog.take() else {
            return;
        };
        if !ok {
            return;
        }
        match dialog {
            KeymapDialog::SaveNew { name, .. } => self.save_new_keymap(&name),
            KeymapDialog::Rename { file, name, .. } => self.rename_keymap(&file, &name),
            KeymapDialog::ConfirmDelete { file, .. } => self.delete_keymap(&file),
            KeymapDialog::ConfirmOverwrite { file, .. } => self.overwrite_keymap(&file),
            KeymapDialog::ConfirmApply { source, .. } => self.apply(&source),
            KeymapDialog::ConfirmLoad { source, .. } => self.load_into_editor(&source),
        }
    }

    /// 導入前の設定に戻せるか。戻せなければ理由を返す。
    fn restore_unavailable(&self, lang: Lang) -> Option<String> {
        let manifest = match &self.initial {
            InitialBackup::SkippedByArgument => {
                return Some(tr(lang, Msg::RestoreUnavailableArgument).to_owned());
            }
            InitialBackup::Ready { manifest, .. } => manifest,
            _ => return Some(tr(lang, Msg::RestoreUnavailableNoBackup).to_owned()),
        };
        if self.places.backup_dir.is_none() {
            return Some(tr(lang, Msg::BackupDirNotFound).to_owned());
        }
        let Some(path) = &self.path else {
            return Some(tr(lang, Msg::RestoreUnavailableNoPath).to_owned());
        };
        // 今の activeConfiguration がバックアップを取ったときと違えば、どちらへ戻すべきか
        // 決まっていないので戻さない
        if !is_definition_file(path, &manifest.definition.name) {
            return Some(format!(
                "{}: {}",
                tr(lang, Msg::RestoreUnavailableOtherConfiguration),
                manifest.definition.name
            ));
        }
        None
    }

    /// 導入時のバックアップの定義ファイルを、§4.8 の保存処理で書き戻して読み直す。
    ///
    /// 外部の変更は検知しない(今のファイルの内容を世代バックアップに残して置き換える)。
    /// 編集中の内容は破棄する。
    fn restore(&mut self) {
        let (Some(path), Some(backup_dir), InitialBackup::Ready { dir, manifest, .. }) =
            (&self.path, &self.places.backup_dir, &self.initial)
        else {
            return;
        };
        let what = format!("restore {}", path.display());
        let contents = match initial_backup::definition_contents(dir, manifest) {
            Ok(c) => c,
            Err(e) => {
                self.set_status(&what, Status::RestoreBackupUnreadable(e));
                return;
            }
        };
        let opts = save::Options::new(backup_dir.clone());
        if let Err(e) = save::overwrite(path, &contents, &opts) {
            self.set_status(&what, Status::RestoreFailed(e));
            return;
        }
        let signal = save::signal_settings_changed();
        let status = match Document::load(path) {
            Ok(doc) => {
                self.doc = Ok(doc);
                self.picking = None;
                match signal {
                    Ok(()) => Status::Restored,
                    Err(e) => Status::RestoredNotApplied(e),
                }
            }
            Err(e) => Status::RestoredReloadFailed(e, signal),
        };
        self.set_status(&what, status);
    }

    /// 戻す前の確認の文。何が起こるかと、未保存の変更が破棄されることを含める。
    fn restore_confirm_text(&self, lang: Lang) -> String {
        let mut text = tr(lang, Msg::RestoreConfirm).to_owned();
        if let InitialBackup::Ready { manifest, .. } = &self.initial {
            text.push_str(&format!(
                "\n\n{}: {}\n{}: {}",
                tr(lang, Msg::RestoreDefinitionFile),
                manifest.definition.name,
                tr(lang, Msg::RestoreBackedUpAt),
                manifest.created_at
            ));
            if !manifest.definition.existed {
                text.push_str("\n\n");
                text.push_str(tr(lang, Msg::RestoreNoDefinition));
            }
        }
        if self.is_dirty() {
            text.push_str("\n\n");
            text.push_str(tr(lang, Msg::RestoreDiscardsUnsaved));
        }
        text
    }

    /// 確認ダイアログを表示し、選ばれた操作を実行する。
    fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog else {
            return;
        };
        let lang = self.lang;
        let (text, discard) = match dialog {
            Dialog::CloseWithUnsaved => (tr(lang, Msg::CloseConfirm).to_owned(), Msg::DontSave),
            Dialog::DiscardAndReload => (
                tr(lang, Msg::ReloadConfirm).to_owned(),
                Msg::DiscardAndReload,
            ),
            Dialog::Restore => (self.restore_confirm_text(lang), Msg::Restore),
        };
        let mut choice = None;
        let response = egui::Modal::new(egui::Id::unique("confirm_dialog")).show(ctx, |ui| {
            ui.label(text);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if dialog == Dialog::CloseWithUnsaved && ui.button(tr(lang, Msg::Save)).clicked() {
                    choice = Some(DialogChoice::Save);
                }
                if ui.button(tr(lang, discard)).clicked() {
                    choice = Some(DialogChoice::Discard);
                }
                if ui.button(tr(lang, Msg::Cancel)).clicked() {
                    choice = Some(DialogChoice::Cancel);
                }
            });
        });
        // Esc や背景のクリックは取り消しとみなす
        if choice.is_none() && response.should_close() {
            choice = Some(DialogChoice::Cancel);
        }
        let Some(choice) = choice else {
            return;
        };
        self.dialog = None;
        self.dialog_chosen(ctx, dialog, choice);
    }

    /// 確認ダイアログで選ばれた操作を実行する。
    fn dialog_chosen(&mut self, ctx: &egui::Context, dialog: Dialog, choice: DialogChoice) {
        match (dialog, choice) {
            (Dialog::CloseWithUnsaved, DialogChoice::Cancel) => self.quitting = false,
            (_, DialogChoice::Cancel) => {}
            (Dialog::CloseWithUnsaved, DialogChoice::Save) => {
                self.save();
                // 保存に失敗したら閉じずに結果を見せる。「終了」もやめる(× で隠せるように戻す)
                if self.is_dirty() {
                    self.quitting = false;
                } else {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            (Dialog::CloseWithUnsaved, DialogChoice::Discard) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Dialog::DiscardAndReload, DialogChoice::Discard) => self.reload(),
            (Dialog::DiscardAndReload | Dialog::Restore, DialogChoice::Save) => {}
            (Dialog::Restore, DialogChoice::Discard) => self.restore(),
        }
    }
}

impl eframe::App for App {
    /// 閉じる要求を止める。ウィンドウが最小化されているときは `ui` が呼ばれず
    /// こちらだけが呼ばれるため、ここで扱う。確認のダイアログが見えるよう、
    /// 最小化を解いて前面に出す。
    ///
    /// ウィンドウを隠している間も、トレイのスレッドが再描画を要求すると呼ばれるので、トレイと
    /// ホットキーの知らせもここで処理する(§4.5.2)。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_resident_events(ctx);
        self.sync_resident();
        if ctx.input(|i| i.viewport().close_requested()) {
            let tray_visible = self.tray_visible();
            self.on_close_requested(ctx, tray_visible);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let lang = self.lang;
        // トレイだけで起動したとき。eframe は最初のフレームを描いたあとにウィンドウを表示するので、
        // そのフレームの指示で隠す
        if std::mem::take(&mut self.start_hidden) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        // PowerToys の設定が使える状態でないときは案内だけを出す(§1.1)
        if !matches!(self.initial, InitialBackup::NotAttempted) {
            egui::Panel::right("keymaps")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| self.keymaps_panel(ui, frame));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(p) = &self.path {
                ui.label(p.display().to_string());
            }
            if let Some((is_error, notice)) = self.initial.notice(lang) {
                if is_error {
                    ui.colored_label(ui.visuals().error_fg_color, notice);
                } else {
                    ui.label(notice);
                }
            }
            if let Ok(doc) = &self.doc {
                let dirty = doc.is_dirty();
                let blocked = self.initial.blocks_save();
                let mut clicked = false;
                ui.horizontal_wrapped(|ui| {
                    clicked = ui
                        .add_enabled(dirty && !blocked, save_button(ui, lang, dirty))
                        .clicked();
                    if blocked {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            tr(lang, Msg::SaveBlockedByInitialBackup),
                        );
                    }
                    if dirty {
                        ui.colored_label(ui.visuals().warn_fg_color, tr(lang, Msg::UnsavedChanges));
                    } else {
                        ui.weak(tr(lang, Msg::NoUnsavedChanges));
                    }
                });
                if clicked {
                    self.save();
                }
            }
            let unavailable = self.restore_unavailable(lang);
            let mut restore = false;
            // PowerToys の設定が使える状態でないときは案内だけを出す(§1.1)
            if !matches!(self.initial, InitialBackup::NotAttempted) {
                ui.horizontal_wrapped(|ui| {
                    restore = ui
                        .add_enabled(
                            unavailable.is_none(),
                            egui::Button::new(tr(lang, Msg::RestoreButton)),
                        )
                        .clicked();
                    if let Some(reason) = &unavailable {
                        ui.weak(reason);
                    }
                });
            }
            if restore {
                self.dialog = Some(Dialog::Restore);
            }
            if let Some(status) = &self.status {
                let color = if status.is_error() {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                };
                let mut reload = false;
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(color, status.message(lang));
                    if status.is_modified_externally() {
                        reload = ui.button(tr(lang, Msg::Reload)).clicked();
                    }
                });
                if reload {
                    if self.is_dirty() {
                        self.dialog = Some(Dialog::DiscardAndReload);
                    } else {
                        self.reload();
                    }
                }
            }
            ui.separator();
            let doc = match &mut self.doc {
                Err(e) if e.is_powertoys_guidance() => {
                    ui.label(e.message(lang));
                    if matches!(e, LoadError::PowerToysNotInstalled) {
                        ui.hyperlink(POWERTOYS_INSTALL_URL);
                    }
                    ui.add_space(8.0);
                    if ui.button(tr(lang, Msg::Recheck)).clicked() {
                        self.recheck();
                    }
                    return;
                }
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e.message(lang));
                    return;
                }
                Ok(doc) => doc,
            };
            let mut action = None;
            // 変換先をキーボード図で選んでいる間は Esc で抜ける。Esc 1 回で 1 段だけ戻すため、
            // リマップ先の一覧などのポップアップが開いていれば、そちらを閉じるだけにする
            // (ポップアップはこのあと表示するときに Esc で閉じる)
            if self.picking.is_some()
                && self.dialog.is_none()
                && !egui::Popup::is_any_open(ui.ctx())
                && ui.input(|i| i.key_pressed(egui::Key::Escape))
            {
                self.picking = None;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(tr(lang, Msg::KeyboardHeading));
                let remaps = &doc.config.remap_keys.in_process;
                let current = doc.current();
                // 選択モードでは、キーごとにクリックしたら変換先になるコード(選べなければ None)
                let picks: Option<Vec<Option<KeyCode>>> = self.picking.map(|source| {
                    self.layout
                        .keys
                        .iter()
                        .map(|k| edit::pick_target(source, &k.codes, &self.choices))
                        .collect()
                });
                if let Some(source) = self.picking
                    && pick_banner(ui, lang, source)
                {
                    self.picking = None;
                }
                let states = matching::key_states(&self.layout, &doc.saved, &current);
                let labels: Vec<_> = self
                    .layout
                    .keys
                    .iter()
                    .map(|k| matching::key_remap_label(k, &current))
                    .collect();
                // 狭いウィンドウでも右端が切れないよう、図は横スクロールの中に置く。
                let width = ui.available_width();
                egui::ScrollArea::horizontal()
                    .id_salt("keyboard")
                    .show(ui, |ui| {
                        let clicked = keyboard(
                            ui,
                            width,
                            &self.layout,
                            &states,
                            &labels,
                            self.selected,
                            picks.as_deref(),
                        );
                        match (clicked, self.picking, &picks) {
                            (Some(i), Some(source), Some(picks)) => {
                                if let Some(target) = picks[i] {
                                    action = Some(Action::Set(source, target));
                                }
                            }
                            (Some(i), None, _) => self.selected = Some(i),
                            _ => {}
                        }
                    });
                legend(ui, lang);
                ui.separator();

                ui.heading(tr(lang, Msg::SelectedKeyHeading));
                match self.selected {
                    None => {
                        ui.label(tr(lang, Msg::NoKeySelected));
                    }
                    Some(i) => {
                        let codes = &self.layout.keys[i].codes;
                        let picker = TargetPicker {
                            choices: &self.choices,
                            search: &mut self.target_search,
                            picking: self.picking,
                        };
                        if let Some(a) = key_details(ui, lang, codes, remaps, picker) {
                            action = Some(a);
                        }
                    }
                }
                ui.separator();

                ui.heading(tr(lang, Msg::KeyRemapsHeading));
                egui::Grid::new("remap_keys").striped(true).show(ui, |ui| {
                    for r in remaps {
                        ui.label(format_keys(&r.original()));
                        ui.label("→");
                        ui.label(format_keys(&r.target()));
                        ui.end_row();
                    }
                });
            });

            if let Some(action) = action {
                let remaps = &mut doc.config.remap_keys.in_process;
                let changed = match action {
                    Action::Set(source, target) => edit::set(remaps, source, target),
                    Action::Remove(source) => edit::remove(remaps, source),
                    Action::StartPick(source) => {
                        self.picking = Some(source);
                        return;
                    }
                };
                // 編集したら選択モードを抜ける
                self.picking = None;
                if changed {
                    self.status = None;
                }
            }
        });
        self.show_dialog(ui.ctx());
        self.show_keymap_dialog(ui.ctx());
        self.show_hotkey_dialog(ui.ctx());
    }
}

/// 保存ボタン。未保存の変更があるときは未保存のキーと同じ色で塗って目立たせる。
fn save_button(ui: &egui::Ui, lang: Lang, dirty: bool) -> egui::Button<'static> {
    if !dirty {
        return egui::Button::new(tr(lang, Msg::Save));
    }
    let visuals = ui.visuals();
    let text = egui::RichText::new(tr(lang, Msg::Save))
        .strong()
        .color(visuals.strong_text_color());
    egui::Button::new(text).fill(keycap::unsaved_fill(visuals.dark_mode))
}

/// キーボード図の色と印の凡例。
fn legend(ui: &mut egui::Ui, lang: Lang) {
    let dark = ui.visuals().dark_mode;
    let swatch = |ui: &mut egui::Ui, color: egui::Color32| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let stroke = ui.visuals().widgets.inactive.bg_stroke;
        ui.painter()
            .rect(rect, 2.0, color, stroke, egui::StrokeKind::Inside);
    };
    ui.horizontal_wrapped(|ui| {
        swatch(ui, keycap::saved_fill(dark));
        ui.label(tr(lang, Msg::LegendRemapped));
        ui.add_space(12.0);
        swatch(ui, keycap::unsaved_fill(dark));
        ui.label(tr(lang, Msg::LegendUnsaved));
        ui.add_space(12.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let color = ui.visuals().strong_text_color();
        ui.painter().circle_filled(rect.center(), 3.0, color);
        ui.label(tr(lang, Msg::LegendMoreRemaps));
    });
}

/// キーボード図を描き、クリックされたキーの添字を返す。
///
/// `width` は図に使える幅。キーの大きさはこれに合わせるが、下限より狭いときは
/// 図がはみ出す(呼び出し側の横スクロールで見る)。`states` と `labels` は
/// `layout.keys` と同じ順の、各キーの状態と併記する変換先。`selected` は選択中のキー。
///
/// `picks` は変換先をキーボード図で選んでいる間だけ渡す、`layout.keys` と同じ順の
/// 「クリックしたら変換先になるコード」。選べるキーは選択の色の枠で囲み、選べないキーは
/// 薄くして、クリックしても返さない。
fn keyboard(
    ui: &mut egui::Ui,
    width: f32,
    layout: &Layout,
    states: &[KeyState],
    labels: &[Option<KeyRemapLabel>],
    selected: Option<usize>,
    picks: Option<&[Option<KeyCode>]>,
) -> Option<usize> {
    let mut clicked = None;
    let (w, h) = layout.size();
    // 1u の大きさ。幅に合わせて伸縮させるが、極端に小さく・大きくはしない。
    let unit = (width / w).clamp(keycap::MIN_UNIT, keycap::MAX_UNIT);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w * unit, h * unit), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let font = egui::FontId::proportional(keycap::single_size(unit));

    for (i, k) in layout.keys.iter().enumerate() {
        let key_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(k.x * unit, k.y * unit),
            egui::vec2(k.w * unit, k.h * unit),
        )
        .shrink(keycap::KEY_GAP);
        // 選択モードで選べないキーは、クリックもホバーも受け付けない
        let pickable = picks.map(|p| p[i].is_some());
        let sense = if pickable == Some(false) {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        };
        let response = ui.interact(key_rect, ui.scope_id().with(("key", i)), sense);
        let hovered = response.hovered() && pickable != Some(false);
        if response.clicked() {
            clicked = Some(i);
        }

        let fill = keycap::state_fill(states[i], visuals.dark_mode).unwrap_or(if hovered {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            visuals.widgets.inactive.weak_bg_fill
        });
        let stroke = if pickable == Some(true) {
            egui::Stroke::new(
                if hovered { 3.0 } else { 2.0 },
                visuals.selection.stroke.color,
            )
        } else if selected == Some(i) {
            egui::Stroke::new(2.5, visuals.strong_text_color())
        } else if hovered {
            visuals.widgets.hovered.bg_stroke
        } else {
            visuals.widgets.inactive.bg_stroke
        };
        painter.rect(key_rect, 3.0, fill, stroke, egui::StrokeKind::Inside);

        match &labels[i] {
            Some(label) => {
                draw_remapped_label(&painter, &visuals, key_rect, unit, label);
            }
            None => {
                let galley = painter.layout(
                    keys::name(k.primary()),
                    font.clone(),
                    visuals.strong_text_color(),
                    key_rect.width() - 4.0,
                );
                let pos = key_rect.center() - galley.size() / 2.0;
                painter.galley(pos, galley, visuals.strong_text_color());
            }
        }
        if pickable == Some(false) {
            // 背景の色を半透明に重ねて薄くする
            painter.rect_filled(key_rect, 3.0, visuals.panel_fill.gamma_multiply(0.75));
            // 選択中のキー(選択元)の枠は薄くせずに描き直す
            if selected == Some(i) {
                painter.rect_stroke(key_rect, 3.0, stroke, egui::StrokeKind::Inside);
            }
        }
    }
    clicked
}

/// 変換先をキーボード図で選んでいることを示す帯。どのリマップ元のコードについての選択かと、
/// 抜け方を示す。取り消しのボタンが押されたら `true`。
fn pick_banner(ui: &mut egui::Ui, lang: Lang, source: KeyCode) -> bool {
    let mut cancel = false;
    let visuals = ui.visuals().clone();
    egui::Frame::new()
        .fill(visuals.selection.bg_fill.gamma_multiply(0.35))
        .stroke(visuals.selection.stroke)
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!(
                    "{}: {} ({})",
                    tr(lang, Msg::PickBanner),
                    keys::name(source),
                    keys::format_hex(source)
                ));
                cancel = ui.button(tr(lang, Msg::Cancel)).clicked();
            });
            ui.label(tr(lang, Msg::PickBannerHint));
        });
    cancel
}

/// リマップ元のキーに、元の名前(表示するリマップのリマップ元の名前)を上に小さく、
/// 変換先を下に大きく描く。
///
/// 幅が足りなければ変換先の文字を少し縮め、それでも足りなければ "→ " を省き、
/// 最後は末尾を省略する。キーのほかのコードもリマップ元なら右上に印を描く。
fn draw_remapped_label(
    painter: &egui::Painter,
    visuals: &egui::Visuals,
    key_rect: egui::Rect,
    unit: f32,
    label: &KeyRemapLabel,
) {
    let marker = label.others > 0;
    let name_width = keycap::name_max_width(key_rect, unit, marker);
    let target_width = keycap::text_max_width(key_rect);
    let (name_size, target_size) = keycap::two_line_sizes(unit);

    let name = single_line(
        painter,
        label.source_name(),
        name_size,
        name_width,
        visuals.text_color(),
    );

    let target = label.target_name();
    let candidates = [format!("→ {target}"), target];
    let widths: Vec<f32> = candidates
        .iter()
        .map(|t| {
            painter
                .layout_no_wrap(
                    t.clone(),
                    egui::FontId::proportional(target_size),
                    visuals.strong_text_color(),
                )
                .size()
                .x
        })
        .collect();
    let (index, size) = keycap::choose_fit(&widths, target_size, target_width);
    let target = single_line(
        painter,
        candidates[index].clone(),
        size,
        target_width,
        visuals.strong_text_color(),
    );

    let (name_y, target_y) = keycap::stack_two(key_rect, name.size().y, target.size().y);
    let cx = key_rect.center().x;
    for (galley, y) in [(name, name_y), (target, target_y)] {
        let pos = egui::pos2(cx - galley.size().x / 2.0, y - galley.size().y / 2.0);
        painter.galley(pos, galley, visuals.strong_text_color());
    }
    if marker {
        painter.circle_filled(
            keycap::marker_center(key_rect, unit),
            keycap::marker_radius(unit),
            visuals.strong_text_color(),
        );
    }
}

/// 1 行に並べ、`max_width` を超える部分は末尾を "…" にして省く。
fn single_line(
    painter: &egui::Painter,
    text: String,
    size: f32,
    max_width: f32,
    color: egui::Color32,
) -> Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    painter.layout_job(job)
}

/// 選択中のキーの名前と、持っているコード(名前・VK・テンキー由来か)、
/// それぞれに関係するリマップを表示する。`codes` は先頭が主のコード。
///
/// コードごとにリマップ先を選ぶ欄([`target_picker`])、キーボード図で選ぶボタン、削除のボタンを
/// 置き、操作されたらそれを返す。
fn key_details(
    ui: &mut egui::Ui,
    lang: Lang,
    codes: &[KeyCode],
    remaps: &[Remap],
    picker: TargetPicker<'_>,
) -> Option<Action> {
    let mut action = None;
    ui.label(format!(
        "{}: {}",
        tr(lang, Msg::KeyName),
        keys::name(codes[0])
    ));

    egui::Grid::new("selected_key_codes")
        .striped(true)
        .show(ui, |ui| {
            ui.strong(tr(lang, Msg::KeyCode));
            ui.strong(tr(lang, Msg::VirtualKey));
            ui.strong(tr(lang, Msg::NumpadOrigin));
            ui.strong(tr(lang, Msg::RemapTo));
            ui.end_row();
            for &code in codes {
                ui.label(keys::name(code));
                ui.label(format!("0x{:02X}", code.vk()));
                let yes_no = if code.is_numpad_origin() {
                    Msg::Yes
                } else {
                    Msg::No
                };
                ui.label(tr(lang, yes_no));
                let current = edit::find(remaps, code).map(|i| remaps[i].target());
                let text = match &current {
                    Some(target) => format_keys(target),
                    None => tr(lang, Msg::NotRemapped).to_owned(),
                };
                if let Some(target) = target_picker(
                    ui,
                    lang,
                    code,
                    current.as_deref(),
                    text,
                    picker.choices,
                    picker.search,
                ) {
                    action = Some(Action::Set(code, target));
                }
                ui.horizontal(|ui| {
                    if picker.picking == Some(code) {
                        ui.colored_label(
                            ui.visuals().selection.stroke.color,
                            tr(lang, Msg::PickingOnKeyboard),
                        );
                    } else if ui.button(tr(lang, Msg::PickOnKeyboard)).clicked() {
                        action = Some(Action::StartPick(code));
                    }
                    if current.is_some() && ui.button(tr(lang, Msg::RemoveRemap)).clicked() {
                        action = Some(Action::Remove(code));
                    }
                });
                ui.end_row();
            }
        });

    let related = matching::related_remaps(codes, remaps);
    if related.is_empty() {
        ui.label(tr(lang, Msg::NoRemapsForKey));
        return action;
    }
    egui::Grid::new("selected_key_remaps")
        .striped(true)
        .show(ui, |ui| {
            for r in related {
                ui.label(keys::name(r.code));
                let (msg, other) = match r.role {
                    Role::Source => (Msg::RemappedTo, r.remap.target()),
                    Role::Target => (Msg::RemappedFrom, r.remap.original()),
                };
                ui.label(tr(lang, msg));
                ui.label(format_keys(&other));
                ui.end_row();
            }
        });
    action
}

/// エンジンへの通知に失敗した理由の表示。
fn signal_detail(lang: Lang, e: &save::SignalError) -> String {
    match e {
        save::SignalError::Unsupported => tr(lang, Msg::SignalUnsupported).to_owned(),
        save::SignalError::EventNotFound => tr(lang, Msg::SignalEventNotFound).to_owned(),
        save::SignalError::Os(d) => d.clone(),
    }
}

/// `path` のファイル名が `name` か。
fn is_definition_file(path: &Path, name: &str) -> bool {
    path.file_name().is_some_and(|n| n == name)
}

/// リマップ先の一覧に渡す状態。
struct TargetPicker<'a> {
    /// リマップ先として選べるキーコード
    choices: &'a [KeyCode],
    /// 検索欄の文字列(一覧を開くたびに空にする)
    search: &'a mut String,
    /// キーボード図で変換先を選んでいるリマップ元のコード
    picking: Option<KeyCode>,
}

/// リマップ先を選ぶボタンと、押すと開く一覧(§4.10)。選ばれたコードを返す。
///
/// 一覧の上に検索欄を置き(開いたときにフォーカスを当てる)、表示名の一部で絞り込む。
/// 一覧はカテゴリの見出しで区切る。自分自身へのリマップになるものは出さない。
/// egui の ComboBox は一覧全体をスクロールの中に置くため、検索欄が流れないよう
/// ボタンと [`egui::Popup`] で作る。
fn target_picker(
    ui: &mut egui::Ui,
    lang: Lang,
    source: KeyCode,
    current: Option<&[KeyCode]>,
    text: String,
    choices: &[KeyCode],
    search: &mut String,
) -> Option<KeyCode> {
    let button = ui.add(egui::Button::new(text).min_size(egui::vec2(160.0, 0.0)));
    let popup_id = ui.make_persistent_id(("remap_target", source.0));
    let opening = button.clicked() && !egui::Popup::is_id_open(ui.ctx(), popup_id);
    if opening {
        search.clear();
    }
    let mut picked = None;
    egui::Popup::menu(&button)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(button.rect.width().max(220.0))
        .show(|ui| {
            // TextEdit の内側の余白はテーマの `spacing.button_padding` から決まるが、メニューの
            // ポップアップでは egui の `menu_style` がこれを (2, 0) に詰めるため、文字が枠に
            // 重なって見える。ポップアップの外(全体のスタイル)の値を明示して標準の余白に戻す。
            // 幅は一覧に合わせる(余白と枠を含めて利用できる幅に収まる)。
            let margin = ui.ctx().global_style().spacing.button_padding;
            let field = ui.add(
                egui::TextEdit::singleline(search)
                    .hint_text(tr(lang, Msg::SearchKeys))
                    .margin(margin)
                    .desired_width(f32::INFINITY),
            );
            if opening {
                field.request_focus();
            }
            ui.separator();
            let allowed: Vec<KeyCode> = choices
                .iter()
                .copied()
                .filter(|&c| edit::can_target(source, c))
                .collect();
            let groups = keys::group_choices(&allowed, search);
            egui::ScrollArea::vertical()
                .id_salt("remap_target_list")
                .max_height(320.0)
                .show(ui, |ui| {
                    if groups.is_empty() {
                        ui.weak(tr(lang, Msg::NoMatchingKeys));
                    }
                    for (category, codes) in groups {
                        ui.add_space(4.0);
                        category_header(ui, tr(lang, category_msg(category)));
                        // キーの項目は見出しより字下げする
                        ui.indent(("remap_target_group", category), |ui| {
                            for c in codes {
                                let checked = current == Some(&[c][..]);
                                if ui.selectable_label(checked, keys::name(c)).clicked() {
                                    picked = Some(c);
                                }
                            }
                        });
                    }
                });
        });
    if picked.is_some() {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    picked
}

/// リマップ先の一覧のカテゴリの見出し(§4.10)。一覧の幅いっぱいの帯に、小さめの文字で書く。
///
/// 選択肢の一つに見えないよう、ボタンや `SelectableLabel` ではなく painter で描く。
/// `Sense::hover()` で場所を取るだけなので、クリックやホバーに反応せず、キーボード操作の
/// フォーカスも止まらない。
fn category_header(ui: &mut egui::Ui, text: &str) {
    let visuals = ui.visuals();
    let fill = category_header_fill(visuals.dark_mode);
    let color = visuals.strong_text_color();
    let body = egui::TextStyle::Body.resolve(ui.style()).size;
    let font = egui::FontId::proportional((body * 0.85).round());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let padding = egui::vec2(6.0, 3.0);
    let size = egui::vec2(ui.available_width(), galley.size().y + padding.y * 2.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, fill);
    let pos = egui::pos2(
        rect.left() + padding.x,
        rect.center().y - galley.size().y / 2.0,
    );
    // egui の既定のフォントには太字の書体がないため、横に少しずらして重ねて太く見せる
    painter.galley(pos, galley.clone(), color);
    painter.galley(pos + egui::vec2(0.6, 0.0), galley, color);
}

/// カテゴリの見出しの帯の色。
///
/// 一覧の背景(ポップアップの `window_fill`)・ホバーした項目(`widgets.hovered`)・選択した項目
/// (`selection`、青系)のどれとも区別でき、`strong_text_color` の文字が読める無彩色にする。
/// ダークでは背景より暗く、ライトではホバーより暗くする。
fn category_header_fill(dark_mode: bool) -> egui::Color32 {
    if dark_mode {
        egui::Color32::from_gray(12)
    } else {
        egui::Color32::from_gray(190)
    }
}

/// カテゴリの見出しの文字列。
fn category_msg(category: keys::Category) -> Msg {
    use keys::Category::*;
    match category {
        Special => Msg::CategorySpecial,
        Letters => Msg::CategoryLetters,
        Digits => Msg::CategoryDigits,
        Symbols => Msg::CategorySymbols,
        Function => Msg::CategoryFunction,
        Modifiers => Msg::CategoryModifiers,
        Editing => Msg::CategoryEditing,
        Navigation => Msg::CategoryNavigation,
        Numpad => Msg::CategoryNumpad,
        MediaAndOther => Msg::CategoryMediaAndOther,
    }
}

/// 名前の入力欄と、了承・取り消しのボタン。押されたら `Some(了承したか)`。
///
/// 名前が空(空白だけ)か、ほかのキーマップと重なる(大文字と小文字を区別しない。`except` の
/// ファイルは除く)ときは理由を出し、了承できないようにする。Enter でも了承できる。
#[allow(clippy::too_many_arguments)]
fn name_input(
    ui: &mut egui::Ui,
    lang: Lang,
    prompt: &str,
    name: &mut String,
    focus: &mut bool,
    keymaps: &[keymap::Entry],
    except: Option<&str>,
    ok: Msg,
) -> Option<bool> {
    ui.label(prompt);
    let field = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
    if *focus {
        field.request_focus();
        *focus = false;
    }
    let problem = if name.trim().is_empty() {
        Some(Msg::KeymapNameEmpty)
    } else if keymap::name_taken(keymaps, name, except) {
        Some(Msg::KeymapNameTaken)
    } else {
        None
    };
    if let Some(p) = problem {
        ui.colored_label(ui.visuals().error_fg_color, tr(lang, p));
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    let mut result = None;
    ui.horizontal(|ui| {
        let ok_clicked = ui
            .add_enabled(problem.is_none(), egui::Button::new(tr(lang, ok)))
            .clicked();
        if ok_clicked || (enter && problem.is_none()) {
            result = Some(true);
        }
        if ui.button(tr(lang, Msg::Cancel)).clicked() {
            result = Some(false);
        }
    });
    result
}

/// 確認の文と、了承・取り消しのボタン。押されたら `Some(了承したか)`。
fn confirm(ui: &mut egui::Ui, lang: Lang, text: &str, ok: Msg) -> Option<bool> {
    ui.label(text);
    ui.add_space(8.0);
    let mut result = None;
    ui.horizontal(|ui| {
        if ui.button(tr(lang, ok)).clicked() {
            result = Some(true);
        }
        if ui.button(tr(lang, Msg::Cancel)).clicked() {
            result = Some(false);
        }
    });
    result
}

/// 「適用」のボタン。適用できないときは押せず、理由を出す。押されたら `true`。
fn apply_button(ui: &mut egui::Ui, lang: Lang, unavailable: Option<&str>) -> bool {
    let response = ui.add_enabled(
        unavailable.is_none(),
        egui::Button::new(tr(lang, Msg::Apply)).small(),
    );
    match unavailable {
        Some(reason) => response.on_disabled_hover_text(reason).clicked(),
        None => response.clicked(),
    }
}

/// 設定を読み込めているときだけ押せる小さなボタン。押されたら `true`。
fn small_enabled(ui: &mut egui::Ui, enabled: bool, text: &str, lang: Lang) -> bool {
    ui.add_enabled(enabled, egui::Button::new(text).small())
        .on_disabled_hover_text(tr(lang, Msg::NoConfigLoaded))
        .clicked()
}

/// 一覧の行のホットキーの欄。割り当て・解除のボタンと、登録できなかったときの理由を出す。
fn hotkey_row(
    ui: &mut egui::Ui,
    lang: Lang,
    hotkey: Option<Hotkey>,
    failure: Option<&str>,
    source: Source,
    actions: &mut Vec<PanelAction>,
) {
    ui.horizontal_wrapped(|ui| {
        let text = hotkey.map_or_else(|| tr(lang, Msg::HotkeyNone).to_owned(), |h| h.format());
        ui.label(egui::RichText::new(format!("{}: {text}", tr(lang, Msg::HotkeyLabel))).small());
        if ui.small_button(tr(lang, Msg::SetHotkey)).clicked() {
            actions.push(PanelAction::SetHotkey(source.clone()));
        }
        if hotkey.is_some() && ui.small_button(tr(lang, Msg::RemoveHotkey)).clicked() {
            actions.push(PanelAction::RemoveHotkey(source));
        }
    });
    if let Some(reason) = failure {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("{}: {reason}", tr(lang, Msg::HotkeyRegisterFailed)),
        );
    }
}

/// 「使用中」の印。
fn in_use_badge(ui: &mut egui::Ui, lang: Lang) {
    let color = ui.visuals().selection.stroke.color;
    ui.label(
        egui::RichText::new(format!("● {}", tr(lang, Msg::InUse)))
            .small()
            .strong()
            .color(color),
    )
    .on_hover_text(tr(lang, Msg::InUseHint));
}

/// キーマップのファイルを選ぶダイアログ(`.json` で絞り込む)。`parent` のウィンドウの前に出す。
fn file_dialog(lang: Lang, title: Msg, parent: &eframe::Frame) -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_parent(parent)
        .set_title(tr(lang, title))
        .add_filter(tr(lang, Msg::KeymapFileFilter), &["json"])
}

/// 受け付けなかったキーマップのファイルの理由。
fn invalid_keymap_reason(lang: Lang, e: &keymap::InvalidKeymap) -> String {
    use keymap::InvalidKeymap::*;
    let with = |msg: Msg, v: &str| {
        if v.is_empty() {
            tr(lang, msg).to_owned()
        } else {
            format!("{}: {v}", tr(lang, msg))
        }
    };
    match e {
        Malformed(d) => with(Msg::KeymapInvalidMalformed, d),
        WrongFormat(v) => with(Msg::KeymapInvalidFormat, v),
        UnknownVersion(v) => with(Msg::KeymapInvalidVersion, v),
        WrongLayout(v) => with(Msg::KeymapInvalidLayout, v),
    }
}

/// プリセットの表示名。
fn preset_msg(id: preset::PresetId) -> Msg {
    match id {
        preset::PresetId::Onishi => Msg::PresetOnishi,
    }
}

fn format_keys(keys: &[KeyCode]) -> String {
    keys.iter()
        .map(|&k| keys::name(k))
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const DEFAULT_JSON: &str = r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66"}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]}}"#;

    /// 一時ディレクトリの中の PowerToys の設定フォルダとデータフォルダ(PowerToys のフォルダは作らない)。
    struct Env {
        tmp: tempfile::TempDir,
        places: Places,
    }

    impl Env {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let data = tmp.path().join("data");
            fs::create_dir(&data).unwrap();
            let places = Places {
                powertoys_root: Some(tmp.path().join("Microsoft").join("PowerToys")),
                initial_backup_dir: Some(data.join(initial_backup::DIR_NAME)),
                backup_dir: Some(data.join("backups")),
                keymaps_dir: Some(data.join(keymap::DIR_NAME)),
                settings_path: Some(data.join(app_settings::FILE_NAME)),
            };
            Env { tmp, places }
        }

        fn root(&self) -> PathBuf {
            self.places.powertoys_root.clone().unwrap()
        }

        fn km(&self) -> PathBuf {
            self.root().join(config::KEYBOARD_MANAGER_DIR)
        }

        fn log_path(&self) -> PathBuf {
            self.tmp.path().join("data").join(applog::FILE_NAME)
        }

        fn log(&self) -> Log {
            Log::new(Some(self.log_path()), applog::DEFAULT_MAX_BYTES)
        }

        fn log_text(&self) -> String {
            fs::read_to_string(self.log_path()).unwrap_or_default()
        }

        /// Keyboard Manager の設定を用意する(settings.json はなし)。
        fn install(&self) {
            fs::create_dir_all(self.km()).unwrap();
            fs::write(self.km().join("default.json"), DEFAULT_JSON).unwrap();
        }

        fn app(&self, arg: Option<PathBuf>) -> App {
            App::new(arg, self.places.clone(), self.log())
        }
    }

    fn edit_a_to_c(app: &mut App) {
        let doc = app.doc.as_mut().ok().unwrap();
        edit::set(
            &mut doc.config.remap_keys.in_process,
            KeyCode(0x41),
            KeyCode(0x43),
        );
        assert!(app.is_dirty());
    }

    #[test]
    fn category_header_fill_is_distinct_and_readable() {
        // 無彩色の明るさ(r = g = b でないものは平均)
        fn level(c: egui::Color32) -> i32 {
            (i32::from(c.r()) + i32::from(c.g()) + i32::from(c.b())) / 3
        }
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let fill = category_header_fill(visuals.dark_mode);
            let theme = if visuals.dark_mode { "dark" } else { "light" };
            assert_eq!(fill.r(), fill.g(), "{theme}");
            assert_eq!(fill.g(), fill.b(), "{theme}");
            // 一覧の背景と区別できる
            assert!(
                (level(fill) - level(visuals.window_fill)).abs() >= 15,
                "{theme}"
            );
            // ホバーした項目と区別できる
            for hover in [
                visuals.widgets.hovered.weak_bg_fill,
                visuals.widgets.hovered.bg_fill,
            ] {
                assert!((level(fill) - level(hover)).abs() >= 25, "{theme}");
            }
            // 選択した項目(青系)は色みで区別できる
            let sel = visuals.selection.bg_fill;
            assert_ne!((sel.r(), sel.g(), sel.b()), (fill.r(), fill.g(), fill.b()));
            assert!(
                sel.b() > sel.r(),
                "{theme}: selection is expected to be bluish"
            );
            // 文字が読める
            assert!(
                (level(fill) - level(visuals.strong_text_color())).abs() >= 150,
                "{theme}"
            );
        }
    }

    #[test]
    fn save_is_blocked_only_while_initial_backup_is_missing() {
        assert!(!InitialBackup::SkippedByArgument.blocks_save());
        assert!(InitialBackup::NotAttempted.blocks_save());
        assert!(InitialBackup::DataDirNotFound.blocks_save());
        assert!(InitialBackup::Failed("x".into()).blocks_save());

        let env = Env::new();
        env.install();
        let dir = env.places.initial_backup_dir.clone().unwrap();
        let outcome =
            initial_backup::ensure(&dir, &env.km(), std::time::SystemTime::now()).unwrap();
        let ready = InitialBackup::Ready {
            dir,
            manifest: outcome.manifest().clone(),
            created: true,
        };
        assert!(!ready.blocks_save());
    }

    #[test]
    fn not_installed_shows_guidance_and_creates_nothing() {
        let env = Env::new();
        let app = env.app(None);
        assert!(matches!(app.doc, Err(LoadError::PowerToysNotInstalled)));
        assert!(app.path.is_none());
        assert!(matches!(app.initial, InitialBackup::NotAttempted));
        assert!(app.restore_unavailable(Lang::En).is_some());
        assert!(!env.root().exists());
        assert!(!env.places.initial_backup_dir.as_ref().unwrap().exists());
        assert!(env.log_text().contains("powertoys: not installed"));
    }

    #[test]
    fn not_configured_shows_guidance_and_creates_nothing() {
        let env = Env::new();
        fs::create_dir_all(env.root()).unwrap();
        let app = env.app(None);
        assert!(matches!(
            app.doc,
            Err(LoadError::KeyboardManagerNotConfigured)
        ));
        assert!(matches!(app.initial, InitialBackup::NotAttempted));
        assert!(!env.km().exists());
        assert!(!env.places.initial_backup_dir.as_ref().unwrap().exists());

        // フォルダはあるが定義ファイルがない
        fs::create_dir(env.km()).unwrap();
        let app = env.app(None);
        assert!(matches!(
            app.doc,
            Err(LoadError::KeyboardManagerNotConfigured)
        ));
        assert!(!env.km().join("default.json").exists());
        assert!(env.log_text().contains("Keyboard Manager not configured"));
    }

    #[test]
    fn recheck_proceeds_to_backup_and_load() {
        let env = Env::new();
        fs::create_dir_all(env.root()).unwrap();
        let mut app = env.app(None);
        assert!(app.doc.is_err());

        env.install();
        app.recheck();
        assert_eq!(app.path, Some(env.km().join("default.json")));
        assert!(app.doc.is_ok());
        assert!(matches!(
            app.initial,
            InitialBackup::Ready { created: true, .. }
        ));
        assert!(app.restore_unavailable(Lang::En).is_none());
        let log = env.log_text();
        assert!(log.contains("powertoys: ready"), "{log}");
        assert!(log.contains("initial backup: created"), "{log}");
    }

    #[test]
    fn reload_detects_state_again() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        assert!(app.doc.is_ok());
        fs::remove_dir_all(env.km()).unwrap();
        app.reload();
        assert!(matches!(
            app.doc,
            Err(LoadError::KeyboardManagerNotConfigured)
        ));
        assert!(app.path.is_none());
    }

    #[test]
    fn reload_keeps_edits_when_file_cannot_be_parsed() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        edit_a_to_c(&mut app);
        fs::write(env.km().join("default.json"), "broken").unwrap();
        app.reload();
        assert!(matches!(app.status, Some(Status::ReloadFailed(_))));
        assert!(app.is_dirty());
    }

    #[test]
    fn load_failures_are_logged() {
        let env = Env::new();
        // 引数で指定したファイルが解析できない
        let path = env.tmp.path().join("broken.json");
        fs::write(&path, "broken").unwrap();
        let app = env.app(Some(path));
        assert!(matches!(app.doc, Err(LoadError::Failed(_))));
        assert!(env.log_text().contains("load failed: "));

        // PowerToys のフォルダの定義ファイルが解析できない
        env.install();
        fs::write(env.km().join("default.json"), "broken").unwrap();
        let app = env.app(None);
        assert!(matches!(app.doc, Err(LoadError::Failed(_))));
        assert_eq!(env.log_text().matches("load failed: ").count(), 2);
    }

    #[test]
    fn reload_results_are_logged_and_end_picking() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        app.picking = Some(KeyCode(0x41));
        app.reload();
        assert!(matches!(app.status, Some(Status::Reloaded)));
        assert_eq!(app.picking, None);
        assert!(env.log_text().contains("reload: Reloaded"));

        fs::write(env.km().join("default.json"), "broken").unwrap();
        app.reload();
        assert!(matches!(app.status, Some(Status::ReloadFailed(_))));
        assert!(env.log_text().contains("reload: Failed to reload"));

        // 引数で指定したファイルでも残す
        let path = env.tmp.path().join("arg.json");
        fs::write(&path, DEFAULT_JSON).unwrap();
        let mut app = env.app(Some(path.clone()));
        app.reload();
        fs::write(&path, "broken").unwrap();
        app.reload();
        let log = env.log_text();
        assert_eq!(log.matches("reload: Reloaded").count(), 2, "{log}");
        assert_eq!(log.matches("reload: Failed to reload").count(), 2, "{log}");
    }

    #[test]
    fn save_and_restore_in_powertoys_folder() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let path = env.km().join("default.json");

        edit_a_to_c(&mut app);
        app.save();
        assert!(!app.is_dirty());
        assert_ne!(fs::read_to_string(&path).unwrap(), DEFAULT_JSON);

        app.restore();
        assert!(matches!(
            app.status,
            Some(Status::Restored | Status::RestoredNotApplied(_))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_JSON);
        assert!(!app.is_dirty());
        // 保存と復元のそれぞれで世代バックアップが残る
        let generations = fs::read_dir(env.places.backup_dir.as_ref().unwrap())
            .unwrap()
            .count();
        assert_eq!(generations, 2);
        let log = env.log_text();
        assert!(log.contains("save "), "{log}");
        assert!(log.contains("restore "), "{log}");
    }

    /// 保存したキーマップのうち、使用中のものの名前。
    fn in_use_names(app: &App) -> Vec<String> {
        let saved = &app.doc.as_ref().ok().unwrap().saved_config;
        app.keymaps
            .iter()
            .filter(|e| keymap::same_remaps(&e.keymap.definition, saved))
            .map(|e| e.keymap.name.clone())
            .collect()
    }

    #[test]
    fn keymap_save_new_includes_unsaved_edits_and_in_use() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        assert!(app.keymaps.is_empty());

        app.save_new_keymap("  Current  ");
        assert!(matches!(&app.status, Some(Status::KeymapSaved(n)) if n == "Current"));
        assert_eq!(app.keymaps.len(), 1);
        // 今の PowerToys の内容と同じなので使用中
        assert_eq!(in_use_names(&app), vec!["Current"]);

        // 未保存の編集を含めて保存する。PowerToys の内容とは違うので使用中ではない
        edit_a_to_c(&mut app);
        app.save_new_keymap("Edited");
        let edited = app
            .keymaps
            .iter()
            .find(|e| e.keymap.name == "Edited")
            .unwrap();
        let map = matching::source_map(&edited.keymap.definition.remap_keys.in_process);
        assert_eq!(map[&KeyCode(0x41)], vec![vec![KeyCode(0x43)]]);
        assert_eq!(in_use_names(&app), vec!["Current"]);
        // 保存すると使用中が入れ替わる
        app.save();
        assert_eq!(in_use_names(&app), vec!["Edited"]);
        assert!(
            env.log_text()
                .contains("keymap save: Saved the keymap: Current")
        );
    }

    #[test]
    fn keymap_apply_replaces_sections_and_saves() {
        let env = Env::new();
        fs::create_dir_all(env.km()).unwrap();
        let path = env.km().join("default.json");
        fs::write(
            &path,
            r#"{"remapKeys":{"inProcess":[{"originalKeys":"65","newRemapKeys":"66"}]},"remapKeysToText":{"inProcess":[]},"remapShortcuts":{"global":[{"originalKeys":"17;65","newRemapKeys":"66"}],"appSpecific":[]},"remapShortcutsToText":{"global":[],"appSpecific":[]},"topLevel":"keep"}"#,
        )
        .unwrap();
        let mut app = env.app(None);
        // キーマップ: E → F だけ
        let def: Config = serde_json::from_str(
            r#"{"remapKeys":{"inProcess":[{"originalKeys":"69","newRemapKeys":"70"}]}}"#,
        )
        .unwrap();
        let store = app.store().unwrap();
        let file = store.create(&keymap::KeymapFile::new("EF", &def)).unwrap();
        app.refresh_keymaps();
        assert!(in_use_names(&app).is_empty());

        app.apply(&Source::Keymap(file));
        assert!(
            matches!(&app.status, Some(Status::Applied(n) | Status::AppliedNotApplied(n, _)) if n == "EF"),
            "{:?}",
            app.status.as_ref().map(|s| s.message(Lang::En))
        );
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["remapKeys"]["inProcess"][0]["originalKeys"], "69");
        assert_eq!(v["remapKeys"]["inProcess"].as_array().unwrap().len(), 1);
        assert_eq!(v["remapShortcuts"]["global"], serde_json::json!([]));
        assert_eq!(v["topLevel"], "keep");
        assert!(!app.is_dirty());
        assert_eq!(in_use_names(&app), vec!["EF"]);
        // §4.8 の保存処理: 世代バックアップが残る
        assert_eq!(
            fs::read_dir(env.places.backup_dir.as_ref().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn apply_discards_unsaved_edits_and_detects_external_changes() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        edit_a_to_c(&mut app);
        // 未保存の編集は捨て、保存済みの内容を元に置き換える
        app.apply(&Source::Preset(preset::PresetId::Onishi));
        assert!(!app.is_dirty());
        let (cfg, _) = config::load(&env.km().join("default.json")).unwrap();
        assert!(keymap::same_remaps(
            &cfg,
            &preset::PresetId::Onishi.definition()
        ));

        // 読み込んだあとに外部で変更されていたら上書きしない
        fs::write(env.km().join("default.json"), DEFAULT_JSON).unwrap();
        app.apply(&Source::Preset(preset::PresetId::Onishi));
        assert!(matches!(
            app.status,
            Some(Status::SaveFailed(save::SaveError::Modified))
        ));
        assert_eq!(
            fs::read_to_string(env.km().join("default.json")).unwrap(),
            DEFAULT_JSON
        );
    }

    #[test]
    fn apply_is_refused_while_initial_backup_failed() {
        let mut env = Env::new();
        env.install();
        let blocker = env.tmp.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        env.places.initial_backup_dir = Some(blocker.join(initial_backup::DIR_NAME));
        let mut app = env.app(None);
        assert!(app.apply_unavailable(Lang::En).is_some());
        app.apply(&Source::Preset(preset::PresetId::Onishi));
        assert!(matches!(
            app.status,
            Some(Status::SaveBlockedByInitialBackup)
        ));
        assert_eq!(
            fs::read_to_string(env.km().join("default.json")).unwrap(),
            DEFAULT_JSON
        );
    }

    #[test]
    fn load_into_editor_and_overlay_make_unsaved_changes() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        // 編集に読み込む: 未保存になり、ファイルは変えない
        app.load_into_editor(&Source::Preset(preset::PresetId::Onishi));
        assert!(app.is_dirty());
        assert!(matches!(app.status, Some(Status::LoadedIntoEditor(_))));
        assert_eq!(
            fs::read_to_string(env.km().join("default.json")).unwrap(),
            DEFAULT_JSON
        );
        let doc = app.doc.as_ref().ok().unwrap();
        assert!(keymap::same_remaps(
            &doc.config,
            &preset::PresetId::Onishi.definition()
        ));

        // 重ねる: 今の編集(A → B と Num 0 → Num 1)にプリセットを重ね、同じリマップ元はプリセットを優先
        app.reload();
        let doc = app.doc.as_mut().ok().unwrap();
        edit::set(
            &mut doc.config.remap_keys.in_process,
            KeyCode(0x60),
            KeyCode(0x61),
        );
        app.overlay_preset(preset::PresetId::Onishi);
        assert!(app.is_dirty());
        let doc = app.doc.as_ref().ok().unwrap();
        let map = matching::source_map(&doc.config.remap_keys.in_process);
        assert_eq!(map[&KeyCode(0x41)], vec![vec![KeyCode(0x45)]]); // A → E(プリセット)
        assert_eq!(map[&KeyCode(0x60)], vec![vec![KeyCode(0x61)]]); // 残る
        assert_eq!(map[&KeyCode(0x53)], vec![vec![KeyCode(0x49)]]); // S → I
        assert_eq!(
            fs::read_to_string(env.km().join("default.json")).unwrap(),
            DEFAULT_JSON
        );
    }

    #[test]
    fn keymap_rename_delete_export_import_through_app() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        app.save_new_keymap("Mine");
        let file = app.keymaps[0].file_name.clone();

        app.rename_keymap(&file, "Renamed");
        assert_eq!(app.keymaps[0].keymap.name, "Renamed");
        assert_eq!(app.keymaps[0].file_name, file);

        let dest = env.tmp.path().join("exported.json");
        app.export_keymap(&file, &dest);
        assert!(matches!(app.status, Some(Status::KeymapExported(_))));

        // 取り込み: 名前が重なるので連番
        app.import_keymap(&dest);
        assert!(matches!(&app.status, Some(Status::KeymapImported(n)) if n == "Renamed (2)"));
        assert_eq!(app.keymaps.len(), 2);

        // 受け付けないもの
        app.import_keymap(&env.km().join("default.json"));
        assert!(matches!(
            app.status,
            Some(Status::ImportRejected(keymap::InvalidKeymap::WrongFormat(
                _
            )))
        ));
        assert_eq!(app.keymaps.len(), 2);

        app.delete_keymap(&file);
        assert!(matches!(&app.status, Some(Status::KeymapDeleted(n)) if n == "Renamed"));
        assert_eq!(app.keymaps.len(), 1);
        let log = env.log_text();
        assert!(log.contains("keymap import"), "{log}");
        assert!(log.contains("keymap delete"), "{log}");
    }

    #[test]
    fn keymap_overwrite_uses_current_contents() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        app.save_new_keymap("Mine");
        let file = app.keymaps[0].file_name.clone();
        edit_a_to_c(&mut app);
        app.overwrite_keymap(&file);
        assert!(matches!(app.status, Some(Status::KeymapOverwritten(_))));
        let map = matching::source_map(&app.keymaps[0].keymap.definition.remap_keys.in_process);
        assert_eq!(map[&KeyCode(0x41)], vec![vec![KeyCode(0x43)]]);
        // 上書きしても PowerToys のファイルは変えない
        assert_eq!(
            fs::read_to_string(env.km().join("default.json")).unwrap(),
            DEFAULT_JSON
        );
    }

    #[test]
    fn save_is_refused_while_initial_backup_failed() {
        let mut env = Env::new();
        env.install();
        // データフォルダの位置がファイルなので初回のバックアップを作れない
        let blocker = env.tmp.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        env.places.initial_backup_dir = Some(blocker.join(initial_backup::DIR_NAME));
        let mut app = env.app(None);
        assert!(matches!(app.initial, InitialBackup::Failed(_)));
        edit_a_to_c(&mut app);

        app.save();
        assert!(matches!(
            app.status,
            Some(Status::SaveBlockedByInitialBackup)
        ));
        assert!(app.is_dirty());
        let path = env.km().join("default.json");
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_JSON);
        assert!(!env.places.backup_dir.as_ref().unwrap().exists());
        let log = env.log_text();
        assert!(log.contains("initial backup: failed"), "{log}");
        assert!(log.contains("save: "), "{log}");
    }

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap()
    }

    /// 一時ディレクトリの本アプリの設定ファイルの中身(なければ空)。
    fn settings_text(env: &Env) -> String {
        fs::read_to_string(env.places.settings_path.as_ref().unwrap()).unwrap_or_default()
    }

    /// E → F だけのキーマップを作り、そのファイル名を返す。
    fn create_ef_keymap(app: &mut App) -> String {
        let def: Config = serde_json::from_str(
            r#"{"remapKeys":{"inProcess":[{"originalKeys":"69","newRemapKeys":"70"}]}}"#,
        )
        .unwrap();
        let file = app
            .store()
            .unwrap()
            .create(&keymap::KeymapFile::new("EF", &def))
            .unwrap();
        app.refresh_keymaps();
        file
    }

    #[test]
    fn switch_applies_like_apply_and_discards_unsaved_edits() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let file = create_ef_keymap(&mut app);
        edit_a_to_c(&mut app);

        let outcome = app.switch_to(&Source::Keymap(file));
        // テストでは PowerToys への通知をしないので、反映されていない可能性が添えられる
        let SwitchOutcome::Switched {
            name,
            discarded,
            not_applied,
        } = &outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(name, "EF");
        assert!(discarded);
        assert!(not_applied.is_some());
        // 「適用」と同じ: 定義ファイルを置き換え、世代バックアップを残し、未保存の編集は消える
        let (cfg, _) = config::load(&env.km().join("default.json")).unwrap();
        let map = matching::source_map(&cfg.remap_keys.in_process);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&KeyCode(0x45)], vec![vec![KeyCode(0x46)]]);
        assert!(!app.is_dirty());
        assert_eq!(
            fs::read_dir(env.places.backup_dir.as_ref().unwrap())
                .unwrap()
                .count(),
            1
        );
        let n = outcome.notification(Lang::En);
        assert!(n.body.starts_with("Switched to EF"), "{}", n.body);
        assert!(n.body.contains(tr(Lang::En, Msg::SwitchDiscardedUnsaved)));
        assert!(env.log_text().contains("apply EF to "));

        // 未保存の編集がなければ、破棄したとは書かない
        let outcome = app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        assert!(matches!(
            &outcome,
            SwitchOutcome::Switched {
                discarded: false,
                ..
            }
        ));
        let n = outcome.notification(Lang::En);
        assert!(
            n.body.starts_with("Switched to Onishi layout"),
            "{}",
            n.body
        );
        assert!(!n.body.contains(tr(Lang::En, Msg::SwitchDiscardedUnsaved)));
    }

    #[test]
    fn switch_is_refused_when_apply_is_unavailable() {
        let mut env = Env::new();
        env.install();
        let blocker = env.tmp.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        env.places.initial_backup_dir = Some(blocker.join(initial_backup::DIR_NAME));
        let mut app = env.app(None);
        edit_a_to_c(&mut app);
        let outcome = app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        assert_eq!(
            outcome,
            SwitchOutcome::Failed {
                name: "Onishi layout".to_owned(),
                reason: tr(Lang::En, Msg::SaveBlockedByInitialBackup).to_owned(),
            }
        );
        // 切り替えず、未保存の編集も残す
        assert!(app.is_dirty());
        assert_eq!(
            fs::read_to_string(env.km().join("default.json")).unwrap(),
            DEFAULT_JSON
        );
        let n = outcome.notification(Lang::En);
        assert!(n.warning);
        assert!(n.body.starts_with("Could not switch to Onishi layout"));
        assert!(
            n.body
                .contains(tr(Lang::En, Msg::SaveBlockedByInitialBackup))
        );
    }

    #[test]
    fn switch_notification_texts_and_when_to_show() {
        let lang = Lang::En;
        let ok = SwitchOutcome::Switched {
            name: "Gaming".into(),
            discarded: false,
            not_applied: None,
        };
        let n = ok.notification(lang);
        assert_eq!(n.title, "Keymap Studio");
        assert_eq!(n.body, "Switched to Gaming");
        assert!(!n.warning);
        // 通知をオフにすると、うまく切り替えたときは出さない
        assert!(ok.should_notify(true));
        assert!(!ok.should_notify(false));

        let discarded = SwitchOutcome::Switched {
            name: "Gaming".into(),
            discarded: true,
            not_applied: Some("not running".into()),
        };
        let n = discarded.notification(lang);
        assert_eq!(
            n.body,
            format!(
                "Switched to Gaming\n{}\n{}: not running",
                tr(lang, Msg::SwitchDiscardedUnsaved),
                tr(lang, Msg::SavedNotApplied)
            )
        );
        assert!(n.warning);
        // 反映されていない可能性と、切り替えられなかったことはオフでも知らせる
        assert!(discarded.should_notify(false));
        let failed = SwitchOutcome::Failed {
            name: "Gaming".into(),
            reason: "locked".into(),
        };
        assert_eq!(
            failed.notification(lang).body,
            "Could not switch to Gaming\nlocked"
        );
        assert!(failed.should_notify(false));
    }

    #[test]
    fn hotkeys_are_saved_validated_and_deduplicated() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let file = create_ef_keymap(&mut app);
        let km = Source::Keymap(file.clone());
        let onishi = Source::Preset(preset::PresetId::Onishi);

        app.assign_hotkey(&km, hk("Ctrl+Alt+1"));
        assert!(
            matches!(&app.status, Some(Status::HotkeyAssigned(n, h)) if n == "EF" && h == "Ctrl+Alt+1")
        );
        app.assign_hotkey(&onishi, hk("Ctrl+Alt+O"));
        let v: serde_json::Value = serde_json::from_str(&settings_text(&env)).unwrap();
        assert_eq!(v["hotkeys"]["keymaps"][&file], "Ctrl+Alt+1");
        assert_eq!(v["hotkeys"]["presets"]["onishi"], "Ctrl+Alt+O");

        // 本アプリの中で重複するものは拒否し、ファイルは変えない
        let before = settings_text(&env);
        app.assign_hotkey(&onishi, hk("Alt+Ctrl+1"));
        assert!(
            matches!(&app.status, Some(Status::HotkeyRejected(r)) if r.contains("EF")),
            "{:?}",
            app.status.as_ref().map(|s| s.message(Lang::En))
        );
        // 修飾キーのないものも拒否する
        app.assign_hotkey(
            &onishi,
            Hotkey {
                vk: 0x41,
                ..Default::default()
            },
        );
        assert!(
            matches!(&app.status, Some(Status::HotkeyRejected(r)) if r == tr(Lang::En, Msg::HotkeyNeedsModifier))
        );
        assert_eq!(settings_text(&env), before);
        assert_eq!(
            app.settings.hotkey(&onishi.target()),
            Some(hk("Ctrl+Alt+O"))
        );

        // 登録するもの(キーマップとプリセット)。取り込み中は何も登録しない
        assert_eq!(
            app.active_hotkeys(),
            vec![
                (km.clone(), hk("Ctrl+Alt+1")),
                (onishi.clone(), hk("Ctrl+Alt+O"))
            ]
        );
        app.hotkey_dialog = Some(HotkeyDialog {
            source: km.clone(),
            name: "EF".into(),
            captured: None,
            win: false,
        });
        assert!(app.active_hotkeys().is_empty());
        app.hotkey_dialog = None;

        app.unassign_hotkey(&onishi);
        assert!(matches!(&app.status, Some(Status::HotkeyRemoved(n)) if n == "Onishi layout"));
        assert_eq!(app.settings.hotkey(&onishi.target()), None);
        assert!(!settings_text(&env).contains("onishi"));
        assert!(env.log_text().contains("hotkey assign: saved"));
    }

    #[test]
    fn hotkeys_follow_keymap_rename_delete_and_new_files() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        app.save_new_keymap("Mine");
        let file = app.keymaps[0].file_name.clone();
        let target = Target::Keymap(file.clone());
        app.assign_hotkey(&Source::Keymap(file.clone()), hk("Ctrl+Alt+1"));

        // 名前を変えてもファイル名は変わらないので、割り当ては残る
        app.rename_keymap(&file, "Renamed");
        assert_eq!(app.keymaps[0].file_name, file);
        assert_eq!(app.settings.hotkey(&target), Some(hk("Ctrl+Alt+1")));
        assert_eq!(app.tray_menu().sections[0].items[0].label, "Renamed");

        // 取り込んだキーマップは割り当てなしで始まり、ほかの割り当ても変えない
        let dest = env.tmp.path().join("exported.json");
        app.export_keymap(&file, &dest);
        app.import_keymap(&dest);
        assert_eq!(app.keymaps.len(), 2);
        let imported = app
            .keymaps
            .iter()
            .find(|e| e.file_name != file)
            .unwrap()
            .file_name
            .clone();
        assert_eq!(app.settings.hotkey(&Target::Keymap(imported.clone())), None);
        assert_eq!(app.settings.hotkey(&target), Some(hk("Ctrl+Alt+1")));

        // 削除したら割り当ても外す(ファイルにも残さない)
        app.delete_keymap(&file);
        assert_eq!(app.settings.hotkey(&target), None);
        assert!(!settings_text(&env).contains(&file));

        // 外部で消されたファイルと同じ名前で新しく作っても、前の割り当ては引き継がない
        app.assign_hotkey(&Source::Keymap(imported.clone()), hk("Ctrl+Alt+2"));
        fs::remove_file(env.places.keymaps_dir.as_ref().unwrap().join(&imported)).unwrap();
        // 取り込んだもの("Renamed (2)")と同じ名前なので、同じファイル名になる
        app.save_new_keymap("Renamed (2)");
        assert!(app.keymaps.iter().any(|e| e.file_name == imported));
        assert_eq!(app.settings.hotkey(&Target::Keymap(imported)), None);
    }

    #[test]
    fn hotkeys_of_missing_keymaps_are_pruned_at_startup() {
        let env = Env::new();
        env.install();
        fs::create_dir_all(env.places.keymaps_dir.as_ref().unwrap()).unwrap();
        let path = env.places.settings_path.clone().unwrap();
        fs::write(
            &path,
            r#"{"hotkeys":{"keymaps":{"gone.json":"Ctrl+Alt+9"},"presets":{"onishi":"Ctrl+Alt+O","future":"Ctrl+Alt+F"}},"autoStart":false,"other":1}"#,
        )
        .unwrap();
        let app = env.app(None);
        assert_eq!(
            app.settings.hotkey(&Target::Keymap("gone.json".into())),
            None
        );
        let v: serde_json::Value = serde_json::from_str(&settings_text(&env)).unwrap();
        assert_eq!(v["hotkeys"]["keymaps"], serde_json::json!({}));
        // プリセット(知らない ID も)とほかの設定は残す
        assert_eq!(v["hotkeys"]["presets"]["future"], "Ctrl+Alt+F");
        assert_eq!(v["autoStart"], false);
        assert_eq!(v["other"], 1);
        // 知らないプリセットは登録しない
        assert_eq!(
            app.active_hotkeys(),
            vec![(Source::Preset(preset::PresetId::Onishi), hk("Ctrl+Alt+O"))]
        );
    }

    #[test]
    fn unreadable_settings_are_not_overwritten() {
        let env = Env::new();
        env.install();
        let path = env.places.settings_path.clone().unwrap();
        fs::write(&path, "broken").unwrap();
        let mut app = env.app(None);
        assert!(app.settings_error.is_some());
        app.assign_hotkey(&Source::Preset(preset::PresetId::Onishi), hk("Ctrl+Alt+O"));
        assert!(matches!(app.status, Some(Status::AppSettingsUnreadable)));
        app.set_notifications(false);
        assert!(app.settings.notifications);
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
        assert!(env.log_text().contains("app settings: failed to read"));
    }

    #[test]
    fn argument_mode_neither_reads_nor_writes_settings() {
        let env = Env::new();
        let path = env.places.settings_path.clone().unwrap();
        let text = r#"{"hotkeys":{"keymaps":{"gone.json":"Ctrl+Alt+9"}},"notifications":false}"#;
        fs::write(&path, text).unwrap();
        let arg = env.tmp.path().join("default.json");
        fs::write(&arg, DEFAULT_JSON).unwrap();
        let mut app = env.app(Some(arg));
        assert!(!app.is_resident_mode());
        assert!(app.settings.notifications);
        app.save_new_keymap("Mine");
        let file = app.keymaps[0].file_name.clone();
        app.assign_hotkey(&Source::Keymap(file.clone()), hk("Ctrl+Alt+1"));
        app.delete_keymap(&file);
        app.set_auto_start(false);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        // 常駐しないので、トレイやホットキーには何も送らない
        app.sync_resident();
        assert!(app.sent_menu.is_none() && app.sent_hotkeys.is_none());
    }

    #[test]
    fn tray_menu_marks_in_use_and_shows_hotkeys() {
        let env = Env::new();
        let app = env.app(None);
        assert_eq!(
            app.tray_menu().tooltip,
            tr(Lang::En, Msg::TrayTooltipNotLoaded)
        );

        env.install();
        let mut app = env.app(None);
        assert_eq!(
            app.tray_menu().tooltip,
            tr(Lang::En, Msg::TrayTooltipNoneInUse)
        );
        app.save_new_keymap("Current");
        let file = app.keymaps[0].file_name.clone();
        app.assign_hotkey(&Source::Keymap(file.clone()), hk("Ctrl+Alt+1"));
        let menu = app.tray_menu();
        assert_eq!(menu.tooltip, "Keymap Studio: Current");
        let [keymaps, presets] = &menu.sections[..] else {
            panic!("{menu:?}");
        };
        assert_eq!(keymaps.items.len(), 1);
        assert_eq!(keymaps.items[0].source, Source::Keymap(file));
        assert!(keymaps.items[0].checked);
        assert_eq!(keymaps.items[0].hotkey.as_deref(), Some("Ctrl+Alt+1"));
        assert_eq!(presets.items.len(), preset::PresetId::ALL.len());
        assert!(!presets.items[0].checked);
        assert_eq!(presets.items[0].hotkey, None);

        // 切り替えるとチェックとツールチップが移る
        app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        let menu = app.tray_menu();
        assert!(!menu.sections[0].items[0].checked);
        assert!(menu.sections[1].items[0].checked);
        assert_eq!(menu.tooltip, "Keymap Studio: Onishi layout");
    }

    #[test]
    fn source_and_target_convert_both_ways() {
        for source in [
            Source::Keymap("a.json".into()),
            Source::Preset(preset::PresetId::Onishi),
        ] {
            assert_eq!(Source::from_target(&source.target()), Some(source));
        }
        assert_eq!(Source::from_target(&Target::Preset("unknown".into())), None);
    }

    #[test]
    fn hotkey_dialog_combines_captured_keys_with_win() {
        let mut d = HotkeyDialog {
            source: Source::Preset(preset::PresetId::Onishi),
            name: String::new(),
            captured: None,
            win: true,
        };
        assert_eq!(d.hotkey(), None);
        d.captured = Some(Hotkey {
            vk: 0x31,
            ..Default::default()
        });
        // Win だけでも修飾キーを含むものとして受け付ける
        assert_eq!(d.hotkey(), Some(hk("Win+1")));
        d.win = false;
        assert_eq!(
            d.hotkey().unwrap().validate(),
            Err(hotkey::Invalid::NoModifier)
        );
    }

    #[test]
    fn switch_reloads_the_definition_file_before_applying() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let path = env.km().join("default.json");
        // 読み込んだあとに外部で変更された
        let external = r#"{"remapKeys":{"inProcess":[{"originalKeys":"71","newRemapKeys":"72"}]},"topLevel":"external"}"#;
        fs::write(&path, external).unwrap();
        // 画面の「適用」は外部の変更を検知して止める
        app.apply(&Source::Preset(preset::PresetId::Onishi));
        assert!(matches!(
            app.status,
            Some(Status::SaveFailed(save::SaveError::Modified))
        ));
        // ホットキー・トレイからの切り替えは読み直してから適用する
        edit_a_to_c(&mut app);
        let outcome = app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        assert!(
            matches!(
                &outcome,
                SwitchOutcome::Switched {
                    discarded: true,
                    ..
                }
            ),
            "{outcome:?}"
        );
        let (cfg, _) = config::load(&path).unwrap();
        assert!(keymap::same_remaps(
            &cfg,
            &preset::PresetId::Onishi.definition()
        ));
        // 4 つの区分以外は、読み直した内容のものを残す
        assert_eq!(cfg.extra["topLevel"], "external");
        // 置き換える前の(外部で変更された)内容は世代バックアップに残る
        let backups: Vec<_> = fs::read_dir(env.places.backup_dir.as_ref().unwrap())
            .unwrap()
            .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert_eq!(backups, vec![external.to_owned()]);
        assert!(!app.is_dirty());
        let menu = app.tray_menu();
        assert!(menu.sections[1].items[0].checked);
        assert_eq!(menu.tooltip, "Keymap Studio: Onishi layout");
    }

    #[test]
    fn failed_switch_keeps_edits_and_save_still_detects_external_changes() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        app.save_new_keymap("Current");
        let path = env.km().join("default.json");
        let external = r#"{"remapKeys":{"inProcess":[]}}"#;
        // 外部で変更されてから、読めないキーマップへ切り替えようとする
        fs::write(&path, external).unwrap();
        edit_a_to_c(&mut app);
        let outcome = app.switch_to(&Source::Keymap("missing.json".into()));
        assert!(
            matches!(outcome, SwitchOutcome::Failed { .. }),
            "{outcome:?}"
        );
        // 切り替える前の状態に丸ごと戻る: 未保存の編集が残り、保存済みの内容も元のまま
        // (トレイのチェックとツールチップも元の内容に合う)
        let doc = app.doc.as_ref().ok().unwrap();
        assert_eq!(doc.snapshot, DEFAULT_JSON.as_bytes());
        let map = matching::source_map(&doc.config.remap_keys.in_process);
        assert_eq!(map[&KeyCode(0x41)], vec![vec![KeyCode(0x43)]]);
        assert!(app.is_dirty());
        assert_eq!(app.tray_menu().tooltip, "Keymap Studio: Current");
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        // そのあと Save しても、外部の変更を検知して上書きしない
        app.save();
        assert!(matches!(
            app.status,
            Some(Status::SaveFailed(save::SaveError::Modified))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        assert!(app.is_dirty());

        // 未保存の編集がなければ、読み直した内容のままにする
        let env = Env::new();
        env.install();
        let mut clean = env.app(None);
        let path2 = env.km().join("default.json");
        fs::write(&path2, external).unwrap();
        let outcome = clean.switch_to(&Source::Keymap("missing.json".into()));
        assert!(
            matches!(outcome, SwitchOutcome::Failed { .. }),
            "{outcome:?}"
        );
        let doc = clean.doc.as_ref().ok().unwrap();
        assert_eq!(doc.snapshot, external.as_bytes());
        assert!(!clean.is_dirty());
        assert_eq!(
            clean.tray_menu().tooltip,
            tr(Lang::En, Msg::TrayTooltipNoneInUse)
        );

        // 定義ファイルを読めなければ切り替えず、何も変えない
        fs::write(&path, "broken").unwrap();
        let outcome = app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        let SwitchOutcome::Failed { reason, .. } = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(
            reason.starts_with(tr(Lang::En, Msg::ReloadFailed)),
            "{reason}"
        );
        assert!(app.is_dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
    }

    #[test]
    fn successful_switch_closes_keymap_dialogs_but_not_hotkey_capture() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        app.save_new_keymap("Mine");
        let file = app.keymaps[0].file_name.clone();
        app.keymap_dialog = Some(KeymapDialog::ConfirmOverwrite {
            file,
            name: "Mine".into(),
        });
        app.hotkey_dialog = Some(HotkeyDialog {
            source: Source::Preset(preset::PresetId::Onishi),
            name: String::new(),
            captured: None,
            win: false,
        });
        app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        assert!(app.keymap_dialog.is_none());
        assert!(app.hotkey_dialog.is_some());

        // 切り替えられなかったときは閉じない
        let mut env = Env::new();
        env.install();
        let blocker = env.tmp.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        env.places.initial_backup_dir = Some(blocker.join(initial_backup::DIR_NAME));
        let mut app = env.app(None);
        app.keymap_dialog = Some(KeymapDialog::SaveNew {
            name: String::new(),
            focus: false,
        });
        app.switch_to(&Source::Preset(preset::PresetId::Onishi));
        assert!(app.keymap_dialog.is_some());
    }

    #[test]
    fn closing_to_tray_ends_hotkey_capture() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let onishi = Source::Preset(preset::PresetId::Onishi);
        app.assign_hotkey(&onishi, hk("Ctrl+Alt+O"));
        app.hotkey_dialog = Some(HotkeyDialog {
            source: onishi.clone(),
            name: String::new(),
            captured: None,
            win: false,
        });
        assert!(app.active_hotkeys().is_empty());
        edit_a_to_c(&mut app);
        let ctx = egui::Context::default();
        // トレイアイコンがあれば × で隠す。取り込みは終わり、ホットキーは再び登録される
        app.on_close_requested(&ctx, true);
        assert!(app.hotkey_dialog.is_none());
        assert!(app.dialog.is_none());
        assert_eq!(app.active_hotkeys(), vec![(onishi, hk("Ctrl+Alt+O"))]);
        assert!(app.is_dirty());
    }

    #[test]
    fn closing_without_tray_icon_asks_about_unsaved_edits() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let ctx = egui::Context::default();
        // 未保存の編集がなければ何もしない(そのまま閉じる)
        app.on_close_requested(&ctx, false);
        assert!(app.dialog.is_none());
        // トレイアイコンを出せていなければ、隠さずに閉じる確認へ進む
        edit_a_to_c(&mut app);
        app.on_close_requested(&ctx, false);
        assert_eq!(app.dialog, Some(Dialog::CloseWithUnsaved));
        // 「終了」のときも同じ
        app.dialog = None;
        app.quitting = true;
        app.on_close_requested(&ctx, true);
        assert_eq!(app.dialog, Some(Dialog::CloseWithUnsaved));
    }

    #[test]
    fn quitting_is_cancelled_when_saving_fails() {
        let mut env = Env::new();
        env.install();
        let blocker = env.tmp.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        env.places.initial_backup_dir = Some(blocker.join(initial_backup::DIR_NAME));
        let mut app = env.app(None);
        edit_a_to_c(&mut app);
        app.quitting = true;
        let ctx = egui::Context::default();
        app.dialog_chosen(&ctx, Dialog::CloseWithUnsaved, DialogChoice::Save);
        assert!(app.is_dirty());
        assert!(!app.quitting);
        assert!(!app.allow_close);
        // 取り消しでも終了をやめる
        app.quitting = true;
        app.dialog_chosen(&ctx, Dialog::CloseWithUnsaved, DialogChoice::Cancel);
        assert!(!app.quitting);
    }

    #[test]
    fn hotkeys_are_kept_when_keymaps_folder_is_missing() {
        let env = Env::new();
        env.install();
        let path = env.places.settings_path.clone().unwrap();
        let text = r#"{"hotkeys":{"keymaps":{"a.json":"Ctrl+Alt+1"}}}"#;
        fs::write(&path, text).unwrap();
        // keymaps フォルダが無い
        let app = env.app(None);
        assert_eq!(
            app.settings.hotkey(&Target::Keymap("a.json".into())),
            Some(hk("Ctrl+Alt+1"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        // keymaps がファイルで、一覧を読めない
        fs::write(env.places.keymaps_dir.as_ref().unwrap(), "x").unwrap();
        let app = env.app(None);
        assert!(
            app.settings
                .hotkey(&Target::Keymap("a.json".into()))
                .is_some()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn duplicate_messages_name_the_holder_readably() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        let file = create_ef_keymap(&mut app);
        let lang = Lang::En;
        let taken = |t: Target| app.assign_error_text(lang, &AssignError::Taken(t));
        assert_eq!(taken(Target::Keymap(file)), "Already assigned to: EF");
        assert_eq!(
            taken(Target::Preset("onishi".into())),
            "Already assigned to: Onishi layout"
        );
        assert_eq!(
            taken(Target::Preset("future".into())),
            "Already assigned to: Unknown preset: future"
        );
        // 一覧に無いキーマップはファイル名
        assert_eq!(
            taken(Target::Keymap("gone.json".into())),
            "Already assigned to: gone.json"
        );
    }

    #[test]
    fn opening_the_window_removes_the_cloak() {
        let env = Env::new();
        env.install();
        let mut app = env.app(None);
        // トレイが使えない(このテストでは常駐しない)ときはクロークしない
        app.start_hidden = true;
        app.cloak_if_starting_hidden(None);
        assert!(!app.cloak.is_cloaked());

        // クロークして隠している状態から開く: クロークを外し、最初のフレームで隠すのもやめる。
        // クロークを外すのは show_window の中で同期的に行い、Visible(true) は送るだけで
        // eframe が後で処理するので、外すのが表示より先になることは構造上保証される
        app.cloak = cloak::Cloak::cloaked_for_test();
        let ctx = egui::Context::default();
        app.show_window(&ctx);
        assert!(!app.cloak.is_cloaked());
        assert!(!app.start_hidden);

        // 「終了」で未保存の確認を出すときも、ウィンドウを開くのでクロークを外す
        app.cloak = cloak::Cloak::cloaked_for_test();
        edit_a_to_c(&mut app);
        app.quitting = true;
        app.on_close_requested(&ctx, true);
        assert_eq!(app.dialog, Some(Dialog::CloseWithUnsaved));
        assert!(!app.cloak.is_cloaked());

        // × で隠すときはクロークに触らない(すでに外れていれば外れたまま)
        app.quitting = false;
        app.dialog = None;
        app.on_close_requested(&ctx, true);
        assert!(!app.cloak.is_cloaked());

        // 外すのに失敗したら、クロークしたまま残し(ログに残す)、次に開くときにもう一度試す
        app.cloak = cloak::Cloak::unremovable_for_test();
        app.show_window(&ctx);
        assert!(app.cloak.is_cloaked());
        assert!(env.log_text().contains("window: failed to uncloak"));
        app.show_window(&ctx);
        assert!(app.cloak.is_cloaked());
        assert_eq!(
            env.log_text().matches("window: failed to uncloak").count(),
            2
        );
    }

    #[test]
    fn argument_mode_skips_detection_and_backup() {
        let env = Env::new();
        let path = env.tmp.path().join("default.json");
        fs::write(&path, DEFAULT_JSON).unwrap();
        let mut app = env.app(Some(path.clone()));
        assert!(app.doc.is_ok());
        assert!(matches!(app.initial, InitialBackup::SkippedByArgument));
        let lang = Lang::En;
        assert_eq!(
            app.initial.notice(lang),
            Some((
                false,
                tr(lang, Msg::InitialBackupSkippedByArgument).to_owned()
            ))
        );
        assert!(app.restore_unavailable(lang).is_some());
        // 再確認は何もしない。PowerToys のフォルダもバックアップも作らない
        app.recheck();
        assert_eq!(app.path, Some(path));
        assert!(!env.root().exists());
        assert!(!env.places.initial_backup_dir.as_ref().unwrap().exists());

        // 保存は止めない
        edit_a_to_c(&mut app);
        app.save();
        assert!(!app.is_dirty());
    }
}
