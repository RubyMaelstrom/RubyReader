use std::fmt::Write;
use trust::render::PaintColor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Sepia,
    Dark,
}

// Shared by every embedded document and by native text, focus and scroll paint.
// Keep the persisted light/sepia/dark names compatible with existing libraries.
// Role                         Light       Sepia       Dark
const COLORS: &[(&str, [u32; 3])] = &[
    ("canvas", [0xfff9f5, 0xe8d9ba, 0x1c1821]),
    ("surface", [0xfffaf5, 0xf0e3c9, 0x292230]),
    ("sidebar", [0xfff5ed, 0xeaddc3, 0x27212d]),
    ("paper", [0xfffdf9, 0xf6ead3, 0x241c2a]),
    ("ink", [0x58273f, 0x4b382e, 0xeee5ed]),
    ("reading-ink", [0x302935, 0x4b382e, 0xeee5ed]),
    ("muted", [0x83546c, 0x685242, 0xbfb0c2]),
    ("accent", [0xac2358, 0x873e4d, 0xf3accd]),
    ("hover-ink", [0x99164a, 0x71323f, 0xffd4e8]),
    ("border", [0xb76689, 0xac9272, 0x795d7a]),
    ("rule", [0xe4cbd7, 0xc9b494, 0x58445c]),
    ("shadow", [0xcf86a4, 0xc3aa88, 0x130f18]),
    ("hover", [0xfce3ed, 0xe4d0ae, 0x3d3045]),
    ("selected", [0xf0c7df, 0xddc39d, 0x523652]),
    ("selected-ink", [0x802347, 0x583823, 0xffe0ee]),
    ("selected-border", [0xc27da2, 0xa48057, 0xae7ca5]),
    ("button-top", [0xfffdf4, 0xf5ead5, 0x49354d]),
    ("button-bottom", [0xf8dfec, 0xe4ceb0, 0x36283d]),
    ("button-edge", [0xa65274, 0x9e795a, 0x956b8c]),
    ("primary-top", [0xc12e66, 0x865544, 0x9f416c]),
    ("primary-bottom", [0xa91d50, 0x694334, 0x803154]),
    ("primary-ink", [0xfff9f5, 0xfff5e3, 0xfff0f8]),
    ("banner", [0x99254d, 0x704736, 0x40263b]),
    ("banner-edge", [0xf59eba, 0xac8763, 0x976b89]),
    ("logo", [0xb51552, 0x865137, 0xf2b4d0]),
    ("logo-secondary", [0x722344, 0x5d4030, 0xe0bdd7]),
    ("logo-highlight", [0xfff5df, 0xf6ead3, 0x41243c]),
    ("logo-shadow", [0xec92b4, 0xc4a17e, 0x774564]),
    ("badge", [0xfff5dc, 0xeddcb9, 0x372c3b]),
    ("stripe", [0xfcf3f6, 0xedddc1, 0x2e2533]),
    ("table-head", [0xf6e5ee, 0xe3d0ae, 0x34283c]),
    ("board-start", [0xffe0ed, 0xe5d0af, 0x39283e]),
    ("board-mid", [0xf2e6fb, 0xe1cfae, 0x2d2940]),
    ("board-end", [0xe0f7ef, 0xd3cfaf, 0x233638]),
    ("chain", [0xfefaf2, 0xf0e0bf, 0x856f91]),
    ("status", [0xf6bed4, 0xddc39e, 0x342638]),
    ("field", [0xfffefb, 0xfbf0dc, 0x1d1823]),
    ("note", [0xfff1d9, 0xebd9b4, 0x392c3b]),
    ("code", [0xfaedf1, 0xeaddc2, 0x322638]),
    ("overlay", [0x4b1631, 0x39271a, 0x100c16]),
    ("find", [0xf09d2e, 0x9d681d, 0xe4b866]),
    ("find-soft", [0xf9d362, 0xce9e3b, 0xdab778]),
];

impl Theme {
    pub fn from_name(name: &str) -> Self {
        match name {
            "sepia" => Self::Sepia,
            "dark" => Self::Dark,
            _ => Self::Light,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Sepia => "Sepia",
            Self::Dark => "Dark",
        }
    }

    pub fn window_theme(self) -> winit::window::Theme {
        match self {
            Self::Dark => winit::window::Theme::Dark,
            _ => winit::window::Theme::Light,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Light => 0,
            Self::Sepia => 1,
            Self::Dark => 2,
        }
    }

    pub fn paint(self, role: &str, alpha: u8) -> PaintColor {
        let value = COLORS
            .iter()
            .find(|(name, _)| *name == role)
            .expect("Known theme color role")
            .1[self.index()];
        PaintColor::Rgba((value >> 16) as u8, (value >> 8) as u8, value as u8, alpha)
    }

    pub fn css(self) -> String {
        let mut css = String::from(":root{");
        for (role, values) in COLORS {
            write!(css, "--{role}:#{:06x};", values[self.index()]).unwrap();
        }
        let wallpaper = match self {
            Self::Light => "gingham.svg",
            Self::Sepia => "gingham-sepia.svg",
            Self::Dark => "gingham-dark.svg",
        };
        write!(
            css,
            "--wallpaper:url('{}{wallpaper}');}}",
            crate::ui::ASSET_BASE
        )
        .unwrap();
        css
    }
}
