//! Colour contrast of both themes, measured with the WCAG 2 formula.
//! Body text needs 4.5:1 against what it sits on; large or bold labels need 3:1.

use bilby::{build, Options, Theme};
use bilby_core::install::Dirs;
use bilby_core::Platform;
use slint::{Color, ComponentHandle};

fn luminance(c: Color) -> f64 {
    let channel = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.red()) + 0.7152 * channel(c.green()) + 0.0722 * channel(c.blue())
}

fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `over` drawn at `alpha` on top of `under`, as a tinted chip or banner is.
fn blend(over: Color, alpha: f64, under: Color) -> Color {
    let mix = |o: u8, u: u8| (o as f64 * alpha + u as f64 * (1.0 - alpha)).round() as u8;
    Color::from_rgb_u8(mix(over.red(), under.red()), mix(over.green(), under.green()), mix(over.blue(), under.blue()))
}

#[test]
fn text_is_readable_in_both_themes() {
    i_slint_backend_testing::init_no_event_loop();
    let root = std::env::temp_dir().join(format!("bilby-contrast-{}", std::process::id()));
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-debian"),
        dirs: Some(Dirs {
            downloads: root.join("d"),
            applications: root.join("a"),
            menu: root.join("menu"),
            flatpak: root.join("flatpak-program"),
            elevate: root.join("elevate-program"),
            terminal: None,
            flatpak_roots: vec![root.join("flatpak")],
            programs: Vec::new(),
            cache: root.join("c"),
            config: root.join("cfg"),
            desktop: None,
        }),
        ..Default::default()
    };
    let (ui, _app) = build(&options).unwrap();
    let theme = ui.global::<Theme>();

    for (mode, name) in [(1, "light"), (2, "dark")] {
        theme.set_mode(mode);
        let (bg, surface, surface2) = (theme.get_bg(), theme.get_surface(), theme.get_surface_2());
        let mut failures = Vec::new();
        let mut need = |what: &str, fg: Color, on: Color, minimum: f64| {
            let ratio = contrast(fg, on);
            if ratio < minimum {
                failures.push(format!("{what}: {ratio:.2}:1, needs {minimum}:1"));
            }
        };
        for (surface_name, on) in [("page", bg), ("card", surface), ("hovered card", surface2)] {
            need(&format!("text on {surface_name}"), theme.get_text(), on, 4.5);
            need(&format!("secondary text on {surface_name}"), theme.get_text_dim(), on, 4.5);
            need(&format!("links on {surface_name}"), theme.get_accent(), on, 4.5);
            need(&format!("success text on {surface_name}"), theme.get_ok(), on, 4.5);
            need(&format!("warning text on {surface_name}"), theme.get_warn(), on, 4.5);
            need(&format!("error text on {surface_name}"), theme.get_bad(), on, 4.5);
        }
        need("primary button label", theme.get_on_accent(), theme.get_accent(), 4.5);
        // Chips: coloured text on a 14% tint of itself over a card.
        for (chip, colour) in [
            ("neutral chip", theme.get_text_dim()),
            ("accent chip", theme.get_accent()),
            ("success chip", theme.get_ok()),
            ("warning chip", theme.get_warn()),
        ] {
            need(chip, colour, blend(colour, 0.14, surface), 4.5);
            need(&format!("{chip} on a hovered card"), colour, blend(colour, 0.14, surface2), 4.5);
        }
        // The keyboard focus ring and control outlines are graphics: 3:1.
        need("focus ring on page", theme.get_focus(), bg, 3.0);
        need("focus ring on card", theme.get_focus(), surface, 3.0);
        need("toast text", bg, theme.get_text(), 4.5);
        assert!(failures.is_empty(), "{name} theme:\n  {}", failures.join("\n  "));
    }
}
