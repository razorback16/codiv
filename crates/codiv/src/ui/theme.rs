use ratatui::style::Color;

use crate::ui::terminal::TerminalColors;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorDepth {
    TrueColor,
    Color256,
    Color16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub depth: ColorDepth,

    // Ratatui Color fields
    pub gutter_ai: Color,
    pub gutter_cmd: Color,
    pub ai_bullet: Color,
    pub thinking_text: Color,
    pub tool_bullet: Color,
    pub spinner: Color,
    pub separator: Color,
    pub hint_text: Color,
    pub completion_selected_bg: Color,
    pub completion_selected_fg: Color,
    pub completion_unselected_bg: Color,
    pub completion_unselected_fg: Color,
    pub completion_border: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub tool_modal_border: Color,
    pub tool_modal_scroll_hint: Color,

    // ANSI escape string fields
    pub ansi_tool_pending: &'static str,
    pub ansi_tool_done: &'static str,
    pub ansi_tool_done_suffix: &'static str,
    pub ansi_tool_denied: &'static str,
    pub ansi_tool_denied_suffix: &'static str,
    pub ansi_prompt_selected: &'static str,
    pub ansi_prompt_unselected: &'static str,
    pub ansi_thinking: &'static str,
    pub ansi_user_prompt: &'static str,
    pub ansi_exit_success: &'static str,
    pub ansi_exit_failure: &'static str,
}

impl Theme {
    /// Detect the appropriate theme based on environment variables and terminal
    /// colors.
    pub fn detect(terminal_colors: &TerminalColors) -> Theme {
        let depth = Self::detect_color_depth();
        let mode = Self::detect_theme_mode(terminal_colors);
        match mode {
            ThemeMode::Dark => Self::dark(depth),
            ThemeMode::Light => Self::light(depth),
        }
    }

    /// Construct the dark theme with the given color depth.
    pub fn dark(depth: ColorDepth) -> Theme {
        Theme {
            mode: ThemeMode::Dark,
            depth,
            gutter_ai: Color::Cyan,
            gutter_cmd: Color::White,
            ai_bullet: Color::White,
            thinking_text: Color::DarkGray,
            tool_bullet: Color::Green,
            spinner: Color::Yellow,
            separator: Color::DarkGray,
            hint_text: Color::DarkGray,
            completion_selected_bg: Color::Blue,
            completion_selected_fg: Color::White,
            completion_unselected_bg: Color::DarkGray,
            completion_unselected_fg: Color::White,
            completion_border: Color::Gray,
            selection_fg: Color::Black,
            selection_bg: Color::White,
            tool_modal_border: Color::Cyan,
            tool_modal_scroll_hint: Color::DarkGray,
            ansi_tool_pending: "\x1b[1m\x1b[33m",
            ansi_tool_done: "\x1b[1m\x1b[32m",
            ansi_tool_done_suffix: "\x1b[32m",
            ansi_tool_denied: "\x1b[1m\x1b[31m",
            ansi_tool_denied_suffix: "\x1b[31m",
            ansi_prompt_selected: "\x1b[1;37m",
            ansi_prompt_unselected: "\x1b[37m",
            ansi_thinking: "\x1b[90m",
            ansi_user_prompt: "\x1b[1m",
            ansi_exit_success: "\x1b[32m",
            ansi_exit_failure: "\x1b[31m",
        }
    }

    /// Construct the light theme with the given color depth.
    pub fn light(depth: ColorDepth) -> Theme {
        Theme {
            mode: ThemeMode::Light,
            depth,
            gutter_ai: Color::Cyan,
            gutter_cmd: Color::Black,
            ai_bullet: Color::Black,
            thinking_text: Color::Gray,
            tool_bullet: Color::Green,
            spinner: Color::Yellow,
            separator: Color::Gray,
            hint_text: Color::Gray,
            completion_selected_bg: Color::Blue,
            completion_selected_fg: Color::White,
            completion_unselected_bg: Color::Gray,
            completion_unselected_fg: Color::Black,
            completion_border: Color::Gray,
            selection_fg: Color::White,
            selection_bg: Color::Black,
            tool_modal_border: Color::Cyan,
            tool_modal_scroll_hint: Color::Gray,
            ansi_tool_pending: "\x1b[1m\x1b[33m",
            ansi_tool_done: "\x1b[1m\x1b[32m",
            ansi_tool_done_suffix: "\x1b[32m",
            ansi_tool_denied: "\x1b[1m\x1b[31m",
            ansi_tool_denied_suffix: "\x1b[31m",
            ansi_prompt_selected: "\x1b[1;30m",
            ansi_prompt_unselected: "\x1b[30m",
            ansi_thinking: "\x1b[90m",
            ansi_user_prompt: "\x1b[1m",
            ansi_exit_success: "\x1b[32m",
            ansi_exit_failure: "\x1b[31m",
        }
    }

    fn detect_color_depth() -> ColorDepth {
        if let Ok(val) = std::env::var("CODIV_COLOR_DEPTH") {
            match val.to_lowercase().as_str() {
                "truecolor" => return ColorDepth::TrueColor,
                "256" => return ColorDepth::Color256,
                "16" => return ColorDepth::Color16,
                _ => {}
            }
        }

        if let Ok(val) = std::env::var("COLORTERM") {
            let lower = val.to_lowercase();
            if lower.contains("truecolor") || lower.contains("24bit") {
                return ColorDepth::TrueColor;
            }
        }

        if let Ok(val) = std::env::var("TERM") {
            if val.contains("256color") {
                return ColorDepth::Color256;
            }
        }

        ColorDepth::Color16
    }

    fn detect_theme_mode(terminal_colors: &TerminalColors) -> ThemeMode {
        if let Ok(val) = std::env::var("CODIV_THEME") {
            match val.to_lowercase().as_str() {
                "light" => return ThemeMode::Light,
                "dark" => return ThemeMode::Dark,
                _ => {}
            }
        }

        if let Some(mode) = Self::parse_bg_luminance(&terminal_colors.bg) {
            return mode;
        }

        if let Ok(val) = std::env::var("COLORFGBG") {
            if let Some(last) = val.rsplit(';').next() {
                if let Ok(n) = last.trim().parse::<u32>() {
                    if n >= 8 {
                        return ThemeMode::Light;
                    }
                }
            }
        }

        ThemeMode::Dark
    }

    /// Parse a terminal background color string in the format
    /// `"rgb:RRRR/GGGG/BBBB"` and determine if the background is light or
    /// dark based on luminance.
    fn parse_bg_luminance(bg: &str) -> Option<ThemeMode> {
        let hex = bg.strip_prefix("rgb:")?;
        let mut parts = hex.split('/');

        let r_hex = parts.next()?;
        let g_hex = parts.next()?;
        let b_hex = parts.next()?;

        // Each component is a 4-digit hex value (0000-FFFF). Take the first 2
        // digits (divide by 256) to get a 0-255 range value.
        let r = u16::from_str_radix(r_hex.get(..2)?, 16).ok()? as f64;
        let g = u16::from_str_radix(g_hex.get(..2)?, 16).ok()? as f64;
        let b = u16::from_str_radix(b_hex.get(..2)?, 16).ok()? as f64;

        let luminance = 0.299 * r + 0.587 * g + 0.114 * b;

        if luminance > 128.0 {
            Some(ThemeMode::Light)
        } else {
            Some(ThemeMode::Dark)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dark_background() {
        let mode = Theme::parse_bg_luminance("rgb:1a1a/1a1a/2e2e");
        assert_eq!(mode, Some(ThemeMode::Dark));
    }

    #[test]
    fn parse_light_background() {
        let mode = Theme::parse_bg_luminance("rgb:ffff/ffff/ffff");
        assert_eq!(mode, Some(ThemeMode::Light));
    }

    #[test]
    fn parse_invalid_background() {
        assert_eq!(Theme::parse_bg_luminance(""), None);
        assert_eq!(Theme::parse_bg_luminance("invalid"), None);
    }

    #[test]
    fn dark_theme_has_correct_mode() {
        let theme = Theme::dark(ColorDepth::TrueColor);
        assert_eq!(theme.mode, ThemeMode::Dark);
        assert_eq!(theme.depth, ColorDepth::TrueColor);
    }

    #[test]
    fn light_theme_has_correct_mode() {
        let theme = Theme::light(ColorDepth::Color256);
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.depth, ColorDepth::Color256);
    }
}
