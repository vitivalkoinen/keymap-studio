#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;

use std::path::PathBuf;

use config::Config;

fn main() -> eframe::Result {
    // 引数でパスを指定できる(WSL での開発時は /mnt/c/... を渡す)。
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(config::default_path);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Keymap Studio",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(path)))),
    )
}

struct App {
    path: Option<PathBuf>,
    config: Result<Config, String>,
}

impl App {
    fn new(path: Option<PathBuf>) -> Self {
        let config = match &path {
            Some(p) => config::load(p),
            None => Err("config path not found".to_owned()),
        };
        Self { path, config }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ui, |ui| {
            if let Some(p) = &self.path {
                ui.label(p.display().to_string());
            }
            ui.separator();
            match &self.config {
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                Ok(cfg) => {
                    ui.heading("Key remaps");
                    eframe::egui::Grid::new("remap_keys")
                        .striped(true)
                        .show(ui, |ui| {
                            for r in &cfg.remap_keys.in_process {
                                ui.label(format_keys(&r.original()));
                                ui.label("→");
                                ui.label(format_keys(&r.target()));
                                ui.end_row();
                            }
                        });
                }
            }
        });
    }
}

fn format_keys(keys: &[config::KeyCode]) -> String {
    keys.iter()
        .map(|k| {
            let ext = if k.is_extended() { " (ext)" } else { "" };
            format!("0x{:02X}{ext}", k.vk())
        })
        .collect::<Vec<_>>()
        .join(" + ")
}
