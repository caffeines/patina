//! Colors, fonts and base visuals from the Patina design.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

pub const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8)
}

// Surfaces
pub const BG: Color32 = hex(0x0F0E0D);
pub const SIDEBAR: Color32 = hex(0x131211);
pub const HAIRLINE: Color32 = hex(0x211F1D);
pub const CARD: Color32 = hex(0x151413);
pub const CARD_EDGE: Color32 = hex(0x24221F);
pub const ENGINE_CARD: Color32 = hex(0x171614);
pub const NAV_ON: Color32 = hex(0x1D1B18);
pub const NAV_ON_EDGE: Color32 = hex(0x2A2724);
pub const ROW_HOVER: Color32 = hex(0x161412);
pub const ROW_SEL: Color32 = hex(0x1A1816);
pub const ROW_SEL_EDGE: Color32 = hex(0x2B2825);
pub const TERM: Color32 = hex(0x0B0A09);
pub const TRACK: Color32 = hex(0x24221F);
pub const RING_TRACK: Color32 = hex(0x2A2724);
pub const IB_HOVER: Color32 = hex(0x24211E);
pub const BTN_EDGE: Color32 = hex(0x34302D);
pub const BTN_EDGE_SOFT: Color32 = hex(0x2E2B28);
pub const MENU: Color32 = hex(0x1F1C1A);
pub const MENU_HOVER: Color32 = hex(0x2A2622);
pub const DIALOG: Color32 = hex(0x1A1816);
pub const FIELD: Color32 = hex(0x131211);
pub const FIELD_ERR: Color32 = hex(0x8A3B2E);
pub const SEARCH: Color32 = hex(0x141312);
pub const SEARCH_FOCUS: Color32 = hex(0x5A544E);
pub const SEG_ON: Color32 = hex(0x26231F);
pub const SWITCH_OFF: Color32 = hex(0x3A3632);
pub const KNOB: Color32 = hex(0xF4F0EA);
pub const TOAST: Color32 = hex(0x26231F);
pub const TOAST_EDGE: Color32 = hex(0x3A3632);
pub const DASH: Color32 = hex(0x3A3632);
pub const SKEL_A: Color32 = hex(0x1A1816);
pub const SKEL_B: Color32 = hex(0x27241F);

// Ink
pub const TEXT: Color32 = hex(0xEDE8E2);
pub const TEXT2: Color32 = hex(0xAEA8A0);
pub const MUTED: Color32 = hex(0x8C867E);
pub const DIM: Color32 = hex(0x5A544E);
pub const CODE: Color32 = hex(0xC9C3BB);
pub const CTA_INK: Color32 = hex(0x1A120B);

// Accents: copper and its verdigris patina.
pub const COPPER: Color32 = hex(0xD4915C);
pub const VERDIGRIS: Color32 = hex(0x6FC2B0);
pub const DANGER: Color32 = hex(0xE0705A);
pub const DANGER_TEXT: Color32 = hex(0xE8836E);
pub const COPPER_TEXT: Color32 = hex(0xDDA57C);
pub const WARN_EDGE: Color32 = hex(0x4E3824);
pub const ERR_EDGE: Color32 = hex(0x5A2A22);
pub const ERR_BG: Color32 = hex(0x261613);
pub const WARN_BG: Color32 = hex(0x221A12);
pub const BANNER: Color32 = hex(0x1A1714);
pub const DIRTY_BG: Color32 = hex(0x1F1A14);
pub const BAR_PAUSED: Color32 = hex(0x3F6E64);
pub const BAR_WARM: Color32 = hex(0x7A5538);
pub const BAR_IDLE: Color32 = hex(0x34302D);
pub const ASSET_TRACK: Color32 = hex(0x211F1D);
pub const POP_EDGE: Color32 = hex(0x2A2724);
pub const POP_ROW_HOVER: Color32 = hex(0x191715);

pub fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Blend two colors in sRGB space.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let a = a.to_srgba_unmultiplied();
    let b = b.to_srgba_unmultiplied();
    let l = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(0), l(1), l(2), l(3))
}

/// CSS `filter: brightness(k)`.
pub fn brighten(c: Color32, k: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let f = |v: u8| ((v as f32) * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(r), f(g), f(b), a)
}

pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("is400", include_bytes!("../assets/fonts/InstrumentSans-400.ttf"));
    add("is500", include_bytes!("../assets/fonts/InstrumentSans-500.ttf"));
    add("is600", include_bytes!("../assets/fonts/InstrumentSans-600.ttf"));
    add("dm400", include_bytes!("../assets/fonts/DMMono-400.ttf"));
    add("dm500", include_bytes!("../assets/fonts/DMMono-500.ttf"));

    // egui's bundled fonts stay in the chain as fallbacks, plus the macOS
    // symbol font when it exists (for glyphs like ⌘ and →).
    let mut fallback: Vec<String> = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for path in ["/System/Library/Fonts/Apple Symbols.ttf", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"] {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("sys:{path}");
            f.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
            fallback.push(name);
            break;
        }
    }
    let chain = |first: &str| {
        let mut v = vec![first.to_owned()];
        v.extend(fallback.iter().cloned());
        v
    };
    f.families.insert(FontFamily::Proportional, chain("is400"));
    f.families.insert(FontFamily::Name("medium".into()), chain("is500"));
    f.families.insert(FontFamily::Name("semibold".into()), chain("is600"));
    f.families.insert(FontFamily::Monospace, chain("dm400"));
    f.families.insert(FontFamily::Name("mono-medium".into()), chain("dm500"));
    ctx.set_fonts(f);
}

pub fn install_style(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = DIALOG;
    v.extreme_bg_color = FIELD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = COPPER;
    v.selection.bg_fill = COPPER.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.text_cursor.stroke = Stroke::new(1.5_f32, COPPER);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, HAIRLINE);
    v.widgets.inactive.bg_fill = hex(0x2A2724);
    v.widgets.hovered.bg_fill = hex(0x3A3632);
    v.widgets.active.bg_fill = hex(0x5A544E);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(0.0, 0.0);
        s.spacing.scroll = egui::style::ScrollStyle::thin();
        s.interaction.selectable_labels = false;
        s.animation_time = 0.2;
    });
}
