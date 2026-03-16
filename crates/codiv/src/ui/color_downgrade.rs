//! ANSI color downgrading from 24-bit true color to 256-color and 16-color terminals.
//!
//! The main entry point is [`downgrade_ansi`], which scans a byte stream for
//! 24-bit ANSI color escape sequences and converts them to the requested color
//! depth while preserving all other escape parameters.

use super::theme::ColorDepth;

/// Downgrade 24-bit true-color ANSI escape sequences in `input` to the given
/// color depth. Non-color escapes and plain text are passed through unchanged.
pub fn downgrade_ansi(input: &[u8], depth: ColorDepth) -> Vec<u8> {
    match depth {
        ColorDepth::TrueColor => input.to_vec(),
        ColorDepth::Color256 | ColorDepth::Color16 => rewrite_sequences(input, depth),
    }
}

// ---------------------------------------------------------------------------
// RGB → 256-color palette
// ---------------------------------------------------------------------------

/// Map an RGB triple to the nearest xterm-256 color index.
///
/// - Grayscale values (r == g == b) are mapped to the 24-step grayscale ramp
///   (indices 232–255), with extremes snapped to indices 16 (black) and 231
///   (white).
/// - All other colors are mapped to the 6×6×6 color cube (indices 16–231).
pub(crate) fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        // Grayscale shortcut.
        if r < 8 {
            return 16; // black
        }
        if r > 248 {
            return 231; // white
        }
        return ((r - 8) as f64 / 247.0 * 24.0).round() as u8 + 232;
    }

    // 6×6×6 color cube.
    let r_idx = ((r as f64 / 255.0) * 5.0).round() as u8;
    let g_idx = ((g as f64 / 255.0) * 5.0).round() as u8;
    let b_idx = ((b as f64 / 255.0) * 5.0).round() as u8;
    16 + 36 * r_idx + 6 * g_idx + b_idx
}

// ---------------------------------------------------------------------------
// RGB → 16-color palette
// ---------------------------------------------------------------------------

/// Standard 16 ANSI colors with their canonical RGB values.
const ANSI_16: [(u8, u8, u8); 16] = [
    (0, 0, 0),       //  0  Black
    (128, 0, 0),     //  1  Red
    (0, 128, 0),     //  2  Green
    (128, 128, 0),   //  3  Yellow
    (0, 0, 128),     //  4  Blue
    (128, 0, 128),   //  5  Magenta
    (0, 128, 128),   //  6  Cyan
    (192, 192, 192), //  7  White
    (128, 128, 128), //  8  Bright Black
    (255, 0, 0),     //  9  Bright Red
    (0, 255, 0),     // 10  Bright Green
    (255, 255, 0),   // 11  Bright Yellow
    (0, 0, 255),     // 12  Bright Blue
    (255, 0, 255),   // 13  Bright Magenta
    (0, 255, 255),   // 14  Bright Cyan
    (255, 255, 255), // 15  Bright White
];

/// Map an RGB triple to the nearest of the 16 standard ANSI colors by
/// Euclidean distance in RGB space.  Returns an index 0–15.
pub(crate) fn rgb_to_16(r: u8, g: u8, b: u8) -> u8 {
    let mut best = 0u8;
    let mut best_dist = u32::MAX;
    for (i, &(cr, cg, cb)) in ANSI_16.iter().enumerate() {
        let dr = (r as i32 - cr as i32).unsigned_abs();
        let dg = (g as i32 - cg as i32).unsigned_abs();
        let db = (b as i32 - cb as i32).unsigned_abs();
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best = i as u8;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Escape-sequence scanner / rewriter
// ---------------------------------------------------------------------------

/// Scan `input` for CSI sequences (ESC [ … m) and rewrite any 24-bit color
/// parameters found within them.
fn rewrite_sequences(input: &[u8], depth: ColorDepth) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        // Look for ESC (0x1b).
        if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'[' {
            // Start of a CSI sequence.  Collect everything up to and including
            // the final byte (we only care about 'm').
            let seq_start = i;
            i += 2; // skip ESC [

            // Gather parameter bytes (digits, semicolons, colons).
            let params_start = i;
            while i < input.len() && (input[i].is_ascii_digit() || input[i] == b';') {
                i += 1;
            }
            let params_end = i;

            if i < input.len() && input[i] == b'm' {
                // This is an SGR sequence — parse and potentially rewrite.
                let params_slice = &input[params_start..params_end];
                let rewritten = rewrite_sgr_params(params_slice, depth);
                out.extend_from_slice(b"\x1b[");
                out.extend_from_slice(&rewritten);
                out.push(b'm');
                i += 1; // skip 'm'
            } else {
                // Not an SGR sequence — copy verbatim.
                if i < input.len() {
                    i += 1; // include the final byte
                }
                out.extend_from_slice(&input[seq_start..i]);
            }
        } else {
            out.push(input[i]);
            i += 1;
        }
    }

    out
}

/// Parse the semicolon-separated parameters of an SGR sequence and rewrite
/// any `38;2;R;G;B` (foreground) or `48;2;R;G;B` (background) sub-sequences.
fn rewrite_sgr_params(params: &[u8], depth: ColorDepth) -> Vec<u8> {
    // Parse all numeric parameters.
    let nums: Vec<u32> = if params.is_empty() {
        vec![0]
    } else {
        params
            .split(|&b| b == b';')
            .map(|s| {
                s.iter().fold(0u32, |acc, &b| acc * 10 + (b - b'0') as u32)
            })
            .collect()
    };

    let mut out_parts: Vec<String> = Vec::new();
    let mut j = 0;

    while j < nums.len() {
        // Check for 38;2;R;G;B or 48;2;R;G;B.
        if (nums[j] == 38 || nums[j] == 48)
            && j + 4 < nums.len()
            && nums[j + 1] == 2
        {
            let is_fg = nums[j] == 38;
            let r = nums[j + 2] as u8;
            let g = nums[j + 3] as u8;
            let b = nums[j + 4] as u8;

            match depth {
                ColorDepth::Color256 => {
                    let idx = rgb_to_256(r, g, b);
                    if is_fg {
                        out_parts.push(format!("38;5;{idx}"));
                    } else {
                        out_parts.push(format!("48;5;{idx}"));
                    }
                }
                ColorDepth::Color16 => {
                    let idx = rgb_to_16(r, g, b);
                    out_parts.push(ansi16_code(idx, is_fg));
                }
                ColorDepth::TrueColor => unreachable!(),
            }
            j += 5;
        } else {
            out_parts.push(nums[j].to_string());
            j += 1;
        }
    }

    out_parts.join(";").into_bytes()
}

/// Produce the SGR parameter string for a 16-color index.
///
/// - Foreground (was `38;2;…`): indices 0–7 → `3X`, indices 8–15 → `9{X-8}`
/// - Background (was `48;2;…`): indices 0–7 → `4X`, indices 8–15 → `10{X-8}`
fn ansi16_code(index: u8, is_fg: bool) -> String {
    if is_fg {
        if index < 8 {
            format!("{}", 30 + index)
        } else {
            format!("{}", 90 + (index - 8))
        }
    } else if index < 8 {
        format!("{}", 40 + index)
    } else {
        format!("{}", 100 + (index - 8))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_truecolor() {
        let input = b"\x1b[38;2;255;100;50mhello\x1b[0m";
        let result = downgrade_ansi(input, ColorDepth::TrueColor);
        assert_eq!(result, input.to_vec());
    }

    #[test]
    fn test_downgrade_fg_to_256() {
        let input = b"\x1b[38;2;255;0;0mred\x1b[0m";
        let result = downgrade_ansi(input, ColorDepth::Color256);
        // Should produce \x1b[38;5;196m (bright red in 256 palette)
        assert!(result.starts_with(b"\x1b[38;5;"));
        assert!(result.ends_with(b"red\x1b[0m"));
    }

    #[test]
    fn test_downgrade_bg_to_256() {
        let input = b"\x1b[48;2;0;128;0mgreen bg\x1b[0m";
        let result = downgrade_ansi(input, ColorDepth::Color256);
        assert!(result.starts_with(b"\x1b[48;5;"));
    }

    #[test]
    fn test_downgrade_to_16() {
        let input = b"\x1b[38;2;255;0;0mred\x1b[0m";
        let result = downgrade_ansi(input, ColorDepth::Color16);
        // Should produce \x1b[91m (bright red) or \x1b[31m (red)
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("\x1b[91m") || result_str.contains("\x1b[31m"));
    }

    #[test]
    fn test_preserves_non_color_escapes() {
        let input = b"\x1b[1m\x1b[38;2;100;200;50mbold green\x1b[0m";
        let result = downgrade_ansi(input, ColorDepth::Color256);
        // Bold escape should be preserved
        assert!(result.starts_with(b"\x1b[1m"));
    }

    #[test]
    fn test_plain_text_passthrough() {
        let input = b"no ansi here";
        let result = downgrade_ansi(input, ColorDepth::Color16);
        assert_eq!(result, input.to_vec());
    }

    #[test]
    fn test_rgb_to_256_grayscale() {
        assert_eq!(rgb_to_256(128, 128, 128), 244); // mid gray
    }

    #[test]
    fn test_rgb_to_256_pure_red() {
        assert_eq!(rgb_to_256(255, 0, 0), 196); // bright red
    }

    #[test]
    fn test_rgb_to_16_nearest() {
        assert_eq!(rgb_to_16(255, 0, 0), 9); // bright red
        assert_eq!(rgb_to_16(0, 0, 0), 0);   // black
        assert_eq!(rgb_to_16(255, 255, 255), 15); // bright white
    }
}
