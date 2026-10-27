#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod edit;
mod i18n;
mod keys;
mod layout;
mod matching;
mod save;

use std::{collections::HashSet, path::PathBuf};

use config::{Config, KeyCode, PathError, Remap};
use eframe::egui;
use i18n::{Lang, Msg, tr};
use layout::Layout;
use matching::Role;

fn main() -> eframe::Result {
    // 引数でパスを指定できる(WSL での開発時は /mnt/c/... を渡す)。
    // 指定がなければ settings.json の activeConfiguration から決める。読み込みと保存は同じファイル。
    let path = config::resolve_path(std::env::args_os().nth(1).map(PathBuf::from));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Keymap Studio",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(path)))),
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

/// 読み込んだ設定と編集の状態。
struct Document {
    config: Config,
    /// 読み込んだ(または最後に保存した)時点のファイルの内容。外部の変更の検知に使う
    snapshot: Vec<u8>,
    /// 保存していない変更があるか
    dirty: bool,
}

impl Document {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        config::load(path).map(|(config, snapshot)| Document {
            config,
            snapshot,
            dirty: false,
        })
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
}

impl Status {
    fn is_error(&self) -> bool {
        !matches!(self, Status::Saved | Status::Reloaded)
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
    status: Option<Status>,
    dialog: Option<Dialog>,
    /// 未保存の変更があっても閉じてよい(確認ダイアログで保存または破棄を選んだ)
    allow_close: bool,
}

impl App {
    fn new(path: Result<PathBuf, PathError>) -> Self {
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
            status: None,
            dialog: None,
            allow_close: false,
        }
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_ok_and(|d| d.dirty)
    }

    /// 編集中の設定を保存し、結果を `status` に入れる。
    fn save(&mut self) {
        let (Some(path), Ok(doc)) = (&self.path, &mut self.doc) else {
            return;
        };
        let Some(backup_dir) = &self.backup_dir else {
            self.status = Some(Status::BackupDirNotFound);
            return;
        };
        let opts = save::Options::new(backup_dir.clone());
        self.status = Some(match save::save(path, &doc.config, &doc.snapshot, &opts) {
            Ok(written) => {
                doc.snapshot = written;
                doc.dirty = false;
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

    /// 確認ダイアログを表示し、選ばれた操作を実行する。
    fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog else {
            return;
        };
        let lang = self.lang;
        let (text, discard) = match dialog {
            Dialog::CloseWithUnsaved => (Msg::CloseConfirm, Msg::DontSave),
            Dialog::DiscardAndReload => (Msg::ReloadConfirm, Msg::DiscardAndReload),
        };
        let mut choice = None;
        let response = egui::Modal::new(egui::Id::unique("confirm_dialog")).show(ctx, |ui| {
            ui.label(tr(lang, text));
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
            (Dialog::DiscardAndReload, DialogChoice::Save) => {}
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
            if let Ok(doc) = &self.doc {
                let dirty = doc.dirty;
                let mut clicked = false;
                ui.horizontal(|ui| {
                    clicked = ui
                        .add_enabled(dirty, egui::Button::new(tr(lang, Msg::Save)))
                        .clicked();
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
                let marked = matching::remapped_keys(&self.layout, remaps);
                // 狭いウィンドウでも右端が切れないよう、図は横スクロールの中に置く。
                let width = ui.available_width();
                egui::ScrollArea::horizontal()
                    .id_salt("keyboard")
                    .show(ui, |ui| {
                        keyboard(ui, width, &self.layout, &marked, &mut self.selected);
                    });
                ui.label(tr(lang, Msg::LegendRemapSource));
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
                    doc.dirty = true;
                    self.status = None;
                }
            }
        });
        self.show_dialog(ui.ctx());
    }
}

/// キーボード図を描き、クリックされたキーの添字を `selected` に入れる。
///
/// `width` は図に使える幅。キーの大きさはこれに合わせるが、下限より狭いときは
/// 図がはみ出す(呼び出し側の横スクロールで見る)。
fn keyboard(
    ui: &mut egui::Ui,
    width: f32,
    layout: &Layout,
    marked: &HashSet<usize>,
    selected: &mut Option<usize>,
) {
    let (w, h) = layout.size();
    // 1u の大きさ。幅に合わせて伸縮させるが、極端に小さく・大きくはしない。
    let unit = (width / w).clamp(28.0, 52.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w * unit, h * unit), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let source_fill = if visuals.dark_mode {
        egui::Color32::from_rgb(0xA0, 0x55, 0x1E)
    } else {
        egui::Color32::from_rgb(0xF5, 0xB0, 0x6B)
    };
    let font = egui::FontId::proportional((unit * 0.26).max(9.0));

    for (i, k) in layout.keys.iter().enumerate() {
        let key_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(k.x * unit, k.y * unit),
            egui::vec2(k.w * unit, k.h * unit),
        )
        .shrink(2.0);
        let response = ui.interact(
            key_rect,
            ui.scope_id().with(("key", i)),
            egui::Sense::click(),
        );
        if response.clicked() {
            *selected = Some(i);
        }

        let fill = if marked.contains(&i) {
            source_fill
        } else if response.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            visuals.widgets.inactive.weak_bg_fill
        };
        let stroke = if *selected == Some(i) {
            egui::Stroke::new(2.5, visuals.strong_text_color())
        } else if response.hovered() {
            visuals.widgets.hovered.bg_stroke
        } else {
            visuals.widgets.inactive.bg_stroke
        };
        painter.rect(key_rect, 3.0, fill, stroke, egui::StrokeKind::Inside);

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

fn format_keys(keys: &[KeyCode]) -> String {
    keys.iter()
        .map(|&k| keys::name(k))
        .collect::<Vec<_>>()
        .join(" + ")
}
