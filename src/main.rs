use codex_dirigent::{PRODUCT_NAME, app::CodexDirigentApp};
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(PRODUCT_NAME)
            .with_inner_size([1_180.0, 760.0])
            .with_min_inner_size([860.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        PRODUCT_NAME,
        options,
        Box::new(|creation_context| {
            codex_dirigent::theme::apply(&creation_context.egui_ctx);
            Ok(Box::new(CodexDirigentApp::load()))
        }),
    )
}
