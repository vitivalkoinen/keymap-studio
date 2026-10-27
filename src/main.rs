#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod applog;
mod config;
mod edit;
mod i18n;
mod initial_backup;
mod keycap;
mod keys;
mod layout;
mod matching;
mod save;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use applog::Log;
use config::{Config, KeyCode, PathError, PowerToysState, Remap};
use eframe::egui;
use i18n::{Lang, Msg, tr};
use layout::Layout;
use matching::{KeyRemapLabel, KeyState, Role, SourceMap};

/// PowerToys のインストール方法の公式ページ(§1.1)。
const POWERTOYS_INSTALL_URL: &str = "https://learn.microsoft.com/windows/powertoys/install";

fn main() -> eframe::Result {
    // 引数でパスを指定できる(WSL での開発時は /mnt/c/... を渡す)。
    // 指定がなければ PowerToys の状態を判定し(§1.1)、settings.json の activeConfiguration から
    // 決める。読み込みと保存は同じファイル。
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    let log = Log::default_location();
    let app = App::new(arg, Places::default_locations(), log);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("Keymap Studio", options, Box::new(|_cc| Ok(Box::new(app))))
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
}

impl Places {
    fn default_locations() -> Self {
        Self {
            powertoys_root: config::powertoys_root(),
            initial_backup_dir: initial_backup::default_dir(),
            backup_dir: save::default_backup_dir(),
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
    /// 読み込んだ(または最後に保存した)時点のキー→キーのリマップ。未保存の変更の判定に使う
    saved: SourceMap,
}

impl Document {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        config::load(path).map(|(config, snapshot)| Document {
            saved: matching::source_map(&config.remap_keys.in_process),
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
    /// 差がなくなれば未保存ではなくなる。比べるのはキー→キーのリマップだけ
    /// (編集できるのはそこだけのため)。
    ///
    /// 比べる単位は、`originalKeys` と `newRemapKeys` を解析したキーコードの値
    /// ([`matching::source_map`])。ファイル上の文字列は比べないので、表記の違い
    /// (リマップ元の正規化で同じになるフラグの有無など)・エントリの並び順・未知のフィールドは
    /// 差とみなさない。
    fn is_dirty(&self) -> bool {
        self.current() != self.saved
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
}

impl Status {
    fn is_error(&self) -> bool {
        !matches!(self, Status::Saved | Status::Reloaded | Status::Restored)
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
    status: Option<Status>,
    dialog: Option<Dialog>,
    /// 未保存の変更があっても閉じてよい(確認ダイアログで保存または破棄を選んだ)
    allow_close: bool,
}

impl App {
    /// `arg` があればそのファイルを開く。なければ PowerToys の状態を判定して開く。
    fn new(arg: Option<PathBuf>, places: Places, log: Log) -> Self {
        let from_argument = arg.is_some();
        let opened = match arg {
            Some(p) => Opened::argument(p, &log),
            None => Opened::powertoys(&places, &log),
        };
        Self {
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
            status: None,
            dialog: None,
            allow_close: false,
        }
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
                doc.snapshot = written;
                doc.saved = doc.current();
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
        match (dialog, choice) {
            (_, DialogChoice::Cancel) => {}
            (Dialog::CloseWithUnsaved, DialogChoice::Save) => {
                self.save();
                // 保存に失敗したら閉じずに結果を見せる
                if !self.is_dirty() {
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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && self.is_dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.dialog = Some(Dialog::CloseWithUnsaved);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let lang = self.lang;
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
            let field =
                ui.add(egui::TextEdit::singleline(search).hint_text(tr(lang, Msg::SearchKeys)));
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
                        ui.strong(tr(lang, category_msg(category)));
                        for c in codes {
                            let checked = current == Some(&[c][..]);
                            if ui.selectable_label(checked, keys::name(c)).clicked() {
                                picked = Some(c);
                            }
                        }
                    }
                });
        });
    if picked.is_some() {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    picked
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
