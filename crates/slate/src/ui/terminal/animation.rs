use std::time::Duration;

/// Spinner frame characters (Braille pattern).
const SPINNER_FRAMES: &[char] = &[
    '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}',
    '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}',
];

/// Tick interval for the animation timer.
pub(crate) const TICK_INTERVAL: Duration = Duration::from_millis(80);

/// Tracks animation state across ticks.
pub(crate) struct AnimationState {
    tick_count: u64,
    has_active_animations: bool,
}

impl AnimationState {
    pub(crate) fn new() -> Self {
        Self {
            tick_count: 0,
            has_active_animations: false,
        }
    }

    /// Advance one tick. Returns true if a re-render is needed.
    pub(crate) fn tick(&mut self) -> bool {
        self.tick_count = self.tick_count.wrapping_add(1);
        self.has_active_animations
    }

    /// Update which animations are active based on current UI state.
    pub(crate) fn update_active(
        &mut self,
        is_executing: bool,
        agent_streaming: bool,
        is_thinking: bool,
        has_pending_tool: bool,
    ) {
        self.has_active_animations = is_executing || agent_streaming || is_thinking || has_pending_tool;
    }

    pub(crate) fn spinner_char(&self) -> char {
        let idx = (self.tick_count as usize) % SPINNER_FRAMES.len();
        SPINNER_FRAMES[idx]
    }

    pub(crate) fn thinking_dots(&self) -> &'static str {
        match (self.tick_count / 4) % 3 {
            0 => ".",
            1 => "..",
            _ => "...",
        }
    }

    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.has_active_animations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_without_active_animations_returns_false() {
        let mut anim = AnimationState::new();
        assert!(!anim.tick());
    }

    #[test]
    fn tick_with_active_animations_returns_true() {
        let mut anim = AnimationState::new();
        anim.update_active(true, false, false, false);
        assert!(anim.tick());
    }

    #[test]
    fn spinner_cycles_through_frames() {
        let mut anim = AnimationState::new();
        let first = anim.spinner_char();
        anim.tick_count = 1;
        let second = anim.spinner_char();
        assert_ne!(first, second);

        anim.tick_count = 8;
        assert_eq!(anim.spinner_char(), first);
    }

    #[test]
    fn update_active_reflects_state() {
        let mut anim = AnimationState::new();
        assert!(!anim.is_active());

        anim.update_active(true, false, false, false);
        assert!(anim.is_active());

        anim.update_active(false, true, false, false);
        assert!(anim.is_active());

        anim.update_active(false, false, false, false);
        assert!(!anim.is_active());
    }

    #[test]
    fn thinking_dots_cycles() {
        let mut anim = AnimationState::new();
        assert_eq!(anim.thinking_dots(), ".");
        anim.tick_count = 4;
        assert_eq!(anim.thinking_dots(), "..");
        anim.tick_count = 8;
        assert_eq!(anim.thinking_dots(), "...");
        anim.tick_count = 12;
        assert_eq!(anim.thinking_dots(), ".");
    }
}
