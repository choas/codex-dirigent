use codex_dirigent::{PRODUCT_NAME, app::CodexDirigentApp};
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(PRODUCT_NAME)
            .with_app_id("com.openai.codex-dirigent")
            .with_inner_size([1_180.0, 760.0])
            .with_min_inner_size([860.0, 560.0])
            .with_icon(std::sync::Arc::new(app_icon())),
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

fn app_icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = (y * SIZE + x) * 4;
            let border = !(4..60).contains(&x) || !(4..60).contains(&y);
            let baton = (15..49).contains(&x) && (29..35).contains(&y);
            let hub = (x.abs_diff(32).pow(2) + y.abs_diff(32).pow(2)) < 65;
            let color = if border {
                [16, 24, 22, 255]
            } else if baton || hub {
                [16, 163, 127, 255]
            } else {
                [29, 39, 36, 255]
            };
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
    egui::IconData {
        rgba,
        width: 64,
        height: 64,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_icon_has_rgba_pixels() {
        let icon = super::app_icon();
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
