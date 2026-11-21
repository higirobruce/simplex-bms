// Task 5 will call these formatting functions; until then they are unused.
#[allow(dead_code)]
mod fmt;

// Later tasks consume the full token set; until then parts of it are unused.
#[allow(dead_code)]
mod theme;

// Task 3: Device traits and simulated implementations. Task 2 polls scanners;
// later tasks will use the printer and drawer.
#[allow(dead_code)]
mod device;

use eframe::egui;

struct LaneApp;

impl LaneApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        Self
    }
}

impl eframe::App for LaneApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |_ui| {});
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Simplex Lane")
            .with_inner_size([1280.0, 1024.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Simplex Lane",
        options,
        Box::new(|cc| Ok(Box::new(LaneApp::new(cc)))),
    )
}
