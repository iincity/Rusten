use nsh_core::prelude::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    Mint,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::Mint];

    pub fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Mint => "mint",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "明亮",
            Self::Dark => "暗黑",
            Self::Mint => "淡绿色",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Mint,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ColorRole(usize);

pub const BACKGROUND: ColorRole = ColorRole(0);
pub const PANEL: ColorRole = ColorRole(1);
pub const SURFACE: ColorRole = ColorRole(2);
pub const BORDER: ColorRole = ColorRole(3);
pub const TEXT: ColorRole = ColorRole(4);
pub const MUTED: ColorRole = ColorRole(5);
pub const ACCENT: ColorRole = ColorRole(6);
pub const WARNING: ColorRole = ColorRole(7);
pub const DANGER: ColorRole = ColorRole(8);
pub const TITLE_BAR: ColorRole = ColorRole(9);
pub const ON_ACCENT: ColorRole = ColorRole(10);

impl ColorRole {
    pub fn value(self, mode: ThemeMode) -> u32 {
        let palette = match mode {
            ThemeMode::Light => [
                0xf5f6f8, 0xffffff, 0xe9edf3, 0xd3d9e2, 0x20252d, 0x566171, 0x245ac4, 0x925500,
                0xbe3035, 0xe7ebf0, 0xffffff,
            ],
            ThemeMode::Dark => [
                0x09090b, 0x111114, 0x232329, 0x383840, 0xf4f4f5, 0xaaaab5, 0x82b1ff, 0xf6bd60,
                0xff9699, 0x202127, 0x111114,
            ],
            ThemeMode::Mint => [
                0xeff8f1, 0xf9fdfa, 0xddeee2, 0xbfd6c6, 0x20392d, 0x496554, 0x256347, 0x865313,
                0xb52f3a, 0xd8ebdf, 0xffffff,
            ],
        };
        palette[self.0]
    }
}

pub fn color(role: ColorRole) -> impl Fn() -> nsh_core::color::Rgba {
    let mode = use_context::<Memo<ThemeMode>>();
    move || rgb(role.value(mode.map_or_else(ThemeMode::default, |mode| mode.get())))
}

pub fn tile_color(
    category: crate::space_analysis::model::FileCategory,
    marked: bool,
) -> impl Fn() -> nsh_core::color::Hsla {
    let mode = use_context::<Memo<ThemeMode>>();
    move || {
        let mode = mode.map_or_else(ThemeMode::default, |mode| mode.get());
        let base: nsh_core::color::Hsla = rgb(PANEL.value(mode)).into();
        let tint: nsh_core::color::Hsla = rgb(if marked {
            WARNING.value(mode)
        } else {
            use crate::space_analysis::model::FileCategory::*;
            match category {
                Code => 0x448aff,
                Git => 0xa970ef,
                Cache => 0xd89a35,
                Build => 0xe5656b,
                Toolchain => 0x36a7bc,
                Media => 0x3eae7e,
                Document => 0x8185e8,
                Archive => 0xd38b54,
                Other => 0x82929c,
            }
        })
        .into();
        base.mix(tint, if mode == ThemeMode::Dark { 0.75 } else { 0.82 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_default_to_mint_and_keep_text_readable() {
        assert_eq!(ThemeMode::from_key(""), ThemeMode::Mint);
        assert_eq!(ThemeMode::from_key("unknown"), ThemeMode::Mint);
        for mode in ThemeMode::ALL {
            let ink: nsh_core::color::Hsla = rgb(ON_ACCENT.value(mode)).into();
            let paper: nsh_core::color::Hsla = rgb(ACCENT.value(mode)).into();
            assert!(
                ink.contrast_ratio(paper) >= 4.5,
                "{} primary button",
                mode.key()
            );
            assert_eq!(ThemeMode::from_key(mode.key()), mode);
            assert_ne!(TITLE_BAR.value(mode), PANEL.value(mode));
            assert_ne!(TITLE_BAR.value(mode), BACKGROUND.value(mode));
            for surface in [BACKGROUND, PANEL, SURFACE, TITLE_BAR] {
                for foreground in [TEXT, MUTED, ACCENT, WARNING, DANGER] {
                    let ink: nsh_core::color::Hsla = rgb(foreground.value(mode)).into();
                    let paper: nsh_core::color::Hsla = rgb(surface.value(mode)).into();
                    assert!(ink.contrast_ratio(paper) >= 4.5, "{}", mode.key());
                }
            }
        }
    }
}
