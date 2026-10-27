#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod i18n;
mod keys;
mod layout;
mod matching;

use std::{collections::HashSet, path::PathBuf};

use config::{Config, KeyCode, Remap};
use eframe::egui;
use i18n::{Lang, Msg, tr};
use layout::Layout;
use matching::Role;

fn main() -> eframe::Result {
    // 引数でパスを指定できる(WSL での開発時は /mnt/c/... を渡す)。
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(config::default_path);

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
    /// 読み込みか解析に失敗した(詳細はそのまま表示する)
    Failed(String),
}

impl LoadError {
    fn message(&self, lang: Lang) -> String {
        match self {
            LoadError::PathNotFound => tr(lang, Msg::ConfigPathNotFound).to_owned(),
            LoadError::Failed(detail) => format!("{}: {detail}", tr(lang, Msg::ConfigLoadFailed)),
        }
    }
}

struct App {
    lang: Lang,
    path: Option<PathBuf>,
    config: Result<Config, LoadError>,
    layout: Layout,
    /// 選択中の物理キー(`layout.keys` の添字)
    selected: Option<usize>,
}

impl App {
    fn new(path: Option<PathBuf>) -> Self {
        let config = match &path {
            Some(p) => config::load(p).map_err(LoadError::Failed),
            None => Err(LoadError::PathNotFound),
        };
        Self {
            lang: Lang::default(),
            path,
            config,
            layout: Layout::us_ansi(),
            selected: None,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let lang = self.lang;
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(p) = &self.path {
                ui.label(p.display().to_string());
            }
            ui.separator();
            let cfg = match &self.config {
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e.message(lang));
                    return;
                }
                Ok(cfg) => cfg,
            };
            let remaps = &cfg.remap_keys.in_process;
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(tr(lang, Msg::KeyboardHeading));
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
                    Some(i) => key_details(ui, lang, &self.layout.keys[i].codes, remaps),
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
        });
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
fn key_details(ui: &mut egui::Ui, lang: Lang, codes: &[KeyCode], remaps: &[Remap]) {
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
                ui.end_row();
            }
        });

    let related = matching::related_remaps(codes, remaps);
    if related.is_empty() {
        ui.label(tr(lang, Msg::NoRemapsForKey));
        return;
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
}

fn format_keys(keys: &[KeyCode]) -> String {
    keys.iter()
        .map(|&k| keys::name(k))
        .collect::<Vec<_>>()
        .join(" + ")
}
