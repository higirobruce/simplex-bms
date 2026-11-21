//! The "Cloud & Azure" skin for the lane. The web back office's
//! `src/app/globals.css` is the single source of truth; the drift tests at the
//! bottom of this file parse it, so either side moving fails the build.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals,
};

/// Corner radius for panels and widgets, in points (`--radius` on the web).
pub const RADIUS: f32 = 14.0;

/// The palette. Every field is a `Color32`.
///
/// **The two azures carry different contracts.** `accent` is dark enough to sit
/// behind white text at body size and is the only azure that may. `hi_vis` is the
/// bright brand blue and is a ground for dark text only, never text itself on a
/// light surface. Mixing them up is the one way to break this palette's contrast
/// guarantees.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub canvas: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub ink: Color32,
    pub ink_soft: Color32,
    pub ink_mute: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub hi_vis: Color32,
    pub danger: Color32,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "part of the full token set; nothing paints success yet"
        )
    )]
    pub success: Color32,
    pub warning: Color32,
}

pub const TOKENS: Tokens = Tokens {
    canvas: Color32::from_rgb(0xF1, 0xF6, 0xFB),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface_2: Color32::from_rgb(0xF7, 0xFA, 0xFD),
    ink: Color32::from_rgb(0x0F, 0x1B, 0x2A),
    ink_soft: Color32::from_rgb(0x41, 0x57, 0x6E),
    ink_mute: Color32::from_rgb(0x54, 0x6A, 0x7E),
    line: Color32::from_rgb(0xE2, 0xEB, 0xF4),
    accent: Color32::from_rgb(0x17, 0x65, 0x96),
    accent_hover: Color32::from_rgb(0x12, 0x52, 0x7A),
    accent_soft: Color32::from_rgb(0xDD, 0xEB, 0xF6),
    hi_vis: Color32::from_rgb(0x3F, 0xA9, 0xE5),
    danger: Color32::from_rgb(0xB5, 0x27, 0x1F),
    success: Color32::from_rgb(0x15, 0x71, 0x3F),
    warning: Color32::from_rgb(0x85, 0x54, 0x0A),
};

/// Family names for the weights egui has no built-in axis for.
pub const INTER_SEMIBOLD: &str = "Inter-SemiBold";
pub const INTER_BOLD: &str = "Inter-Bold";
pub const MONO_BOLD: &str = "JetBrainsMono-Bold";

/// Install fonts and visuals on the context.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals(visuals());
}

fn fonts() -> FontDefinitions {
    let faces: [(&str, &'static [u8]); 5] = [
        (
            "Inter-Regular",
            include_bytes!("../assets/fonts/Inter-Regular.ttf"),
        ),
        (
            INTER_SEMIBOLD,
            include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
        ),
        (INTER_BOLD, include_bytes!("../assets/fonts/Inter-Bold.ttf")),
        (
            "JetBrainsMono-Regular",
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
        ),
        (
            MONO_BOLD,
            include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
        ),
    ];
    let mut defs = FontDefinitions::default();
    for (name, bytes) in faces {
        defs.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    }
    // Regular faces lead their family; egui's built-ins stay behind as glyph fallback.
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter-Regular".to_owned());
    defs.families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono-Regular".to_owned());
    // Heavier weights are separate named families, used as `FontFamily::Name(..)`.
    for (family, face) in [
        (INTER_SEMIBOLD, INTER_SEMIBOLD),
        (INTER_BOLD, INTER_BOLD),
        (MONO_BOLD, MONO_BOLD),
    ] {
        defs.families
            .insert(FontFamily::Name(family.into()), vec![face.to_owned()]);
    }
    defs
}

fn visuals() -> Visuals {
    let t = TOKENS;
    let radius = CornerRadius::same(RADIUS as u8);
    let hairline = Stroke::new(1.0_f32, t.line);
    let mut v = Visuals::light();

    v.panel_fill = t.canvas;
    v.window_fill = t.surface;
    v.window_stroke = hairline;
    v.window_corner_radius = radius;
    v.menu_corner_radius = radius;
    v.extreme_bg_color = t.surface;
    v.faint_bg_color = t.surface_2;
    v.code_bg_color = t.surface_2;
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.danger;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0_f32, t.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.surface;
    w.noninteractive.weak_bg_fill = t.surface;
    w.noninteractive.bg_stroke = hairline;
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.ink);

    w.inactive.bg_fill = t.surface;
    w.inactive.weak_bg_fill = t.surface;
    w.inactive.bg_stroke = hairline;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, t.ink);

    w.hovered.bg_fill = t.accent_soft;
    w.hovered.weak_bg_fill = t.accent_soft;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, t.accent);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, t.ink);

    w.active.bg_fill = t.accent;
    w.active.weak_bg_fill = t.accent;
    w.active.bg_stroke = Stroke::new(1.0_f32, t.accent_hover);
    w.active.fg_stroke = Stroke::new(2.0_f32, t.surface);

    w.open.bg_fill = t.accent_soft;
    w.open.weak_bg_fill = t.accent_soft;
    w.open.bg_stroke = Stroke::new(1.0_f32, t.accent);
    w.open.fg_stroke = Stroke::new(1.0_f32, t.ink);

    for s in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        s.corner_radius = radius;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../src/app/globals.css");

    fn css() -> String {
        std::fs::read_to_string(CSS_PATH).expect("globals.css is readable")
    }

    /// Raw value of a custom property, e.g. `--radius` -> `14px`.
    fn hex_of_raw(css: &str, name: &str) -> String {
        let needle = format!("{name}:");
        let line = css
            .lines()
            .map(str::trim_start)
            .find(|l| l.starts_with(&needle))
            .unwrap_or_else(|| panic!("{name} not found in globals.css"));
        line[needle.len()..]
            .split([';', '/'])
            .next()
            .expect("value")
            .trim()
            .to_owned()
    }

    /// Value of a colour custom property, upper-cased, e.g. `#F1F6FB`.
    fn hex_of(css: &str, name: &str) -> String {
        hex_of_raw(css, name).to_uppercase()
    }

    fn to_hex(c: Color32) -> String {
        format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
    }

    #[test]
    fn every_token_matches_the_web_skin() {
        let css = css();
        let t = TOKENS;
        for (name, colour) in [
            ("--bg", t.canvas),
            ("--surface", t.surface),
            ("--surface-2", t.surface_2),
            ("--ink", t.ink),
            ("--ink-soft", t.ink_soft),
            ("--ink-mute", t.ink_mute),
            ("--line", t.line),
            ("--accent", t.accent),
            ("--accent-hover", t.accent_hover),
            ("--accent-soft", t.accent_soft),
            ("--hi-vis", t.hi_vis),
            ("--danger", t.danger),
            ("--success", t.success),
            ("--warning", t.warning),
        ] {
            assert_eq!(
                hex_of(&css, name),
                to_hex(colour),
                "{name} drifted from globals.css"
            );
        }
    }

    #[test]
    fn radius_matches_the_web_skin() {
        assert_eq!(hex_of_raw(&css(), "--radius"), format!("{RADIUS}px"));
    }

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// The colour egui paints a plain-string label with on a pressed (accent) widget:
    /// `override_text_color` wins when set, otherwise the widget's `fg_stroke`.
    #[test]
    fn text_on_accent_fill_meets_wcag_aa() {
        let v = visuals();
        assert_eq!(v.widgets.active.bg_fill, TOKENS.accent);
        assert_eq!(v.widgets.active.weak_bg_fill, TOKENS.accent);
        let text = v
            .override_text_color
            .unwrap_or(v.widgets.active.fg_stroke.color);
        let ratio = contrast(text, TOKENS.accent);
        assert!(ratio >= 4.5, "text on accent is {ratio:.2}:1, needs 4.5:1");
    }

    #[test]
    fn fonts_register_every_family() {
        let defs = fonts();
        assert_eq!(defs.families[&FontFamily::Proportional][0], "Inter-Regular");
        assert_eq!(
            defs.families[&FontFamily::Monospace][0],
            "JetBrainsMono-Regular"
        );
        for name in [INTER_SEMIBOLD, INTER_BOLD, MONO_BOLD] {
            assert!(defs.families.contains_key(&FontFamily::Name(name.into())));
        }
    }
}
