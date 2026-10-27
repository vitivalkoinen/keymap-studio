#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

use config::{Config, KeyCode, PathError, Remap};
use eframe::egui;
use i18n::{Lang, Msg, tr};
use layout::Layout;
use matching::{KeyRemapLabel, KeyState, Role, SourceMap};

fn main() -> eframe::Result {
    // 引数でパスを指定できる(WSL での開発時は /mnt/c/... を渡す)。
    // 指定がなければ settings.json の activeConfiguration から決める。読み込みと保存は同じファイル。
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    // 導入前の設定のバックアップ(§4.7)は PowerToys の実際の設定フォルダだけを対象にする。
    // 引数で指定したファイルは開発・テスト用なので、バックアップも復元もしない。
    let initial = if arg.is_some() {
        InitialBackup::SkippedByArgument
    } else {
        InitialBackup::ensure()
    };
    let path = config::resolve_path(arg);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Keymap Studio",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(path, initial)))),
    )
}

/// 設定ファイルを読めなかった理由。
enum LoadError {
    /// 既定のパスが分からない
    PathNotFound,
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

impl From<PathError> for LoadError {
    fn from(e: PathError) -> Self {
        match e {
            PathError::DirNotFound => LoadError::PathNotFound,
            PathError::SettingsUnreadable(detail) => LoadError::SettingsUnreadable(detail),
            PathError::UnsafeConfiguration(value) => LoadError::UnsafeConfiguration(value),
        }
    }
}

/// 導入前の設定のバックアップ(§4.7)の状態。
enum InitialBackup {
    /// 引数でファイルを指定して起動したので扱わない
    SkippedByArgument,
    /// データフォルダ(exe と同じフォルダ)が分からない
    DataDirNotFound,
    /// Keyboard Manager の設定フォルダが分からない
    SourceDirNotFound,
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
    /// バックアップがなければ作る。すでにあれば読む。
    fn ensure() -> Self {
        let Some(dir) = initial_backup::default_dir() else {
            return InitialBackup::DataDirNotFound;
        };
        let Some(source) = config::powertoys_dir() else {
            return InitialBackup::SourceDirNotFound;
        };
        match initial_backup::ensure(&dir, &source, std::time::SystemTime::now()) {
            Ok(outcome) => {
                let created = matches!(outcome, initial_backup::Outcome::Created(_));
                InitialBackup::Ready {
                    dir,
                    manifest: outcome.manifest().clone(),
                    created,
                }
            }
            Err(e) => InitialBackup::Failed(e),
        }
    }

    /// 保存を止めるか。初回のバックアップができていない間は、導入前の状態を確実に残すため
    /// 保存しない(§4.7)。引数でファイルを指定して起動したときはバックアップを扱わないので止めない。
    fn blocks_save(&self) -> bool {
        match self {
            InitialBackup::SkippedByArgument | InitialBackup::Ready { .. } => false,
            InitialBackup::DataDirNotFound
            | InitialBackup::SourceDirNotFound
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
            InitialBackup::DataDirNotFound => failed(tr(lang, Msg::DataDirNotFound)),
            InitialBackup::SourceDirNotFound => failed(tr(lang, Msg::ConfigPathNotFound)),
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

/// キー→キーのリマップへの編集操作。
enum Edit {
    /// リマップ元を変換先へリマップする(追加または変更)
    Set(KeyCode, KeyCode),
    /// リマップ元のリマップを削除する
    Remove(KeyCode),
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
    /// 世代バックアップを置くフォルダ(exe と同じフォルダの `backups`)
    backup_dir: Option<PathBuf>,
    /// 導入前の設定のバックアップ
    initial: InitialBackup,
    status: Option<Status>,
    dialog: Option<Dialog>,
    /// 未保存の変更があっても閉じてよい(確認ダイアログで保存または破棄を選んだ)
    allow_close: bool,
}

impl App {
    fn new(path: Result<PathBuf, PathError>, initial: InitialBackup) -> Self {
        let (path, doc) = match path {
            Ok(p) => {
                let doc = Document::load(&p).map_err(LoadError::Failed);
                (Some(p), doc)
            }
            Err(e) => (None, Err(e.into())),
        };
        Self {
            lang: Lang::default(),
            path,
            doc,
            layout: Layout::us_ansi(),
            selected: None,
            choices: keys::target_choices(),
            backup_dir: save::default_backup_dir(),
            initial,
            status: None,
            dialog: None,
            allow_close: false,
        }
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
            self.status = Some(Status::SaveBlockedByInitialBackup);
            return;
        }
        let Some(backup_dir) = &self.backup_dir else {
            self.status = Some(Status::BackupDirNotFound);
            return;
        };
        let opts = save::Options::new(backup_dir.clone());
        self.status = Some(match save::save(path, &doc.config, &doc.snapshot, &opts) {
            Ok(written) => {
                doc.snapshot = written;
                doc.saved = doc.current();
                match save::signal_settings_changed() {
                    Ok(()) => Status::Saved,
                    Err(e) => Status::SavedNotApplied(e),
                }
            }
            Err(e) => Status::SaveFailed(e),
        });
    }

    /// ファイルを読み直す。編集中の内容は破棄する。失敗したら編集中の内容を残す。
    fn reload(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        self.status = Some(match Document::load(path) {
            Ok(doc) => {
                self.doc = Ok(doc);
                Status::Reloaded
            }
            Err(e) => Status::ReloadFailed(e),
        });
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
        if self.backup_dir.is_none() {
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
            (&self.path, &self.backup_dir, &self.initial)
        else {
            return;
        };
        let contents = match initial_backup::definition_contents(dir, manifest) {
            Ok(c) => c,
            Err(e) => {
                self.status = Some(Status::RestoreBackupUnreadable(e));
                return;
            }
        };
        let opts = save::Options::new(backup_dir.clone());
        if let Err(e) = save::overwrite(path, &contents, &opts) {
            self.status = Some(Status::RestoreFailed(e));
            return;
        }
        let signal = save::signal_settings_changed();
        self.status = Some(match Document::load(path) {
            Ok(doc) => {
                self.doc = Ok(doc);
                match signal {
                    Ok(()) => Status::Restored,
                    Err(e) => Status::RestoredNotApplied(e),
                }
            }
            Err(e) => Status::RestoredReloadFailed(e, signal),
        });
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
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e.message(lang));
                    return;
                }
                Ok(doc) => doc,
            };
            let mut action = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(tr(lang, Msg::KeyboardHeading));
                let remaps = &doc.config.remap_keys.in_process;
                let current = doc.current();
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
                        keyboard(
                            ui,
                            width,
                            &self.layout,
                            &states,
                            &labels,
                            &mut self.selected,
                        );
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
                        action = key_details(ui, lang, codes, remaps, &self.choices);
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
                    Edit::Set(source, target) => edit::set(remaps, source, target),
                    Edit::Remove(source) => edit::remove(remaps, source),
                };
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

/// キーボード図を描き、クリックされたキーの添字を `selected` に入れる。
///
/// `width` は図に使える幅。キーの大きさはこれに合わせるが、下限より狭いときは
/// 図がはみ出す(呼び出し側の横スクロールで見る)。`states` と `labels` は
/// `layout.keys` と同じ順の、各キーの状態と併記する変換先。
fn keyboard(
    ui: &mut egui::Ui,
    width: f32,
    layout: &Layout,
    states: &[KeyState],
    labels: &[Option<KeyRemapLabel>],
    selected: &mut Option<usize>,
) {
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
        let response = ui.interact(
            key_rect,
            ui.scope_id().with(("key", i)),
            egui::Sense::click(),
        );
        if response.clicked() {
            *selected = Some(i);
        }

        let fill =
            keycap::state_fill(states[i], visuals.dark_mode).unwrap_or(if response.hovered() {
                visuals.widgets.hovered.weak_bg_fill
            } else {
                visuals.widgets.inactive.weak_bg_fill
            });
        let stroke = if *selected == Some(i) {
            egui::Stroke::new(2.5, visuals.strong_text_color())
        } else if response.hovered() {
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
    }
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
/// コードごとにリマップ先を選ぶ欄と削除のボタンを置き、操作されたらその編集を返す。
/// `choices` はリマップ先として選べるキーコード。
fn key_details(
    ui: &mut egui::Ui,
    lang: Lang,
    codes: &[KeyCode],
    remaps: &[Remap],
    choices: &[KeyCode],
) -> Option<Edit> {
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
                egui::ComboBox::from_id_salt(("remap_target", code.0))
                    .selected_text(text)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for &c in choices.iter().filter(|&&c| edit::can_target(code, c)) {
                            let checked = current.as_deref() == Some(&[c][..]);
                            if ui.selectable_label(checked, keys::name(c)).clicked() {
                                action = Some(Edit::Set(code, c));
                            }
                        }
                    });
                if current.is_some() && ui.button(tr(lang, Msg::RemoveRemap)).clicked() {
                    action = Some(Edit::Remove(code));
                }
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

fn format_keys(keys: &[KeyCode]) -> String {
    keys.iter()
        .map(|&k| keys::name(k))
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_is_blocked_only_while_initial_backup_is_missing() {
        assert!(!InitialBackup::SkippedByArgument.blocks_save());
        assert!(InitialBackup::DataDirNotFound.blocks_save());
        assert!(InitialBackup::SourceDirNotFound.blocks_save());
        assert!(InitialBackup::Failed("x".into()).blocks_save());

        let tmp = tempfile::tempdir().unwrap();
        let km = tmp.path().join("Keyboard Manager");
        std::fs::create_dir(&km).unwrap();
        let dir = tmp.path().join(initial_backup::DIR_NAME);
        let outcome = initial_backup::ensure(&dir, &km, std::time::SystemTime::now()).unwrap();
        let ready = InitialBackup::Ready {
            dir,
            manifest: outcome.manifest().clone(),
            created: true,
        };
        assert!(!ready.blocks_save());
    }

    #[test]
    fn save_is_refused_while_initial_backup_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("default.json");
        let original = r#"{"remapKeys":{"inProcess":[]}}"#;
        std::fs::write(&path, original).unwrap();
        let mut app = App::new(Ok(path.clone()), InitialBackup::Failed("x".into()));
        app.backup_dir = Some(tmp.path().join("backups"));
        let doc = app.doc.as_mut().ok().unwrap();
        edit::set(
            &mut doc.config.remap_keys.in_process,
            KeyCode(0x41),
            KeyCode(0x42),
        );
        assert!(app.is_dirty());

        app.save();
        assert!(matches!(
            app.status,
            Some(Status::SaveBlockedByInitialBackup)
        ));
        assert!(app.is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert!(!tmp.path().join("backups").exists());
    }

    #[test]
    fn argument_mode_shows_notice_and_disables_restore() {
        let lang = Lang::En;
        let notice = InitialBackup::SkippedByArgument.notice(lang);
        assert_eq!(
            notice,
            Some((
                false,
                tr(lang, Msg::InitialBackupSkippedByArgument).to_owned()
            ))
        );
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("default.json");
        std::fs::write(&path, "{}").unwrap();
        let app = App::new(Ok(path), InitialBackup::SkippedByArgument);
        assert!(app.restore_unavailable(lang).is_some());
    }
}
