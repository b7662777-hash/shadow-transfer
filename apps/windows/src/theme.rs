use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle,
};

pub const BG: Color32 = Color32::from_rgb(7, 11, 20);
pub const NAV_BG: Color32 = Color32::from_rgb(8, 13, 23);
pub const PANEL: Color32 = Color32::from_rgb(11, 18, 32);
pub const CARD: Color32 = Color32::from_rgb(16, 26, 44);
pub const CARD_HOVER: Color32 = Color32::from_rgb(22, 36, 58);
pub const BORDER: Color32 = Color32::from_rgb(30, 44, 68);
pub const TEXT: Color32 = Color32::from_rgb(232, 238, 249);
pub const MUTED: Color32 = Color32::from_rgb(143, 160, 184);
pub const ACCENT: Color32 = Color32::from_rgb(96, 165, 250);
pub const PRIMARY: Color32 = Color32::from_rgb(37, 99, 235);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(23, 42, 74);
pub const CYAN: Color32 = Color32::from_rgb(34, 211, 238);
pub const GREEN: Color32 = Color32::from_rgb(52, 211, 153);
pub const RED: Color32 = Color32::from_rgb(248, 113, 113);
pub const YELLOW: Color32 = Color32::from_rgb(251, 191, 36);

pub fn configure(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(5, 9, 16);
    visuals.faint_bg_color = NAV_BG;
    visuals.selection.bg_fill = PRIMARY;
    visuals.selection.stroke = Stroke::new(1.0_f32, CYAN);
    visuals.window_rounding = Rounding::same(16.0);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.bg_fill = CARD_HOVER;
    visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.active.bg_fill = PRIMARY;
    visuals.widgets.active.weak_bg_fill = PRIMARY;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.noninteractive,
    ] {
        widget.rounding = Rounding::same(8.0);
    }
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(24.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );
    ctx.set_style(style);
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf") {
        fonts
            .font_data
            .insert("segoe".into(), FontData::from_owned(bytes));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe".into());
    }
    if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\msyh.ttc") {
        fonts
            .font_data
            .insert("msyh".into(), FontData::from_owned(bytes));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("msyh".into());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("msyh".into());
    }
    ctx.set_fonts(fonts);
}
