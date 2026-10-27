#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod edit;
mod i18n;
mod keycap;
mod keys;
mod layout;
mod matching;
mod save;

use std::{path::PathBuf, sync::Arc};

use config::{Config, KeyCode, PathError, Remap};
use eframe::egui;
use i18n::{Lang, Msg, tr};
use layout::Layout;
use matching::{KeyRemapLabel, KeyState, Role, SourceMap};

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
        self.doc.as_ref().is_ok_and(Document::is_dirty)
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
                let dirty = doc.is_dirty();
                let mut clicked = false;
                ui.horizontal(|ui| {
                    clicked = ui
                        .add_enabled(dirty, save_button(ui, lang, dirty))
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

fn format_keys(keys: &[KeyCode]) -> String {
    keys.iter()
        .map(|&k| keys::name(k))
        .collect::<Vec<_>>()
        .join(" + ")
}
