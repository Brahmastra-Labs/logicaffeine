#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEffect {
    XpGain,
    CriticalHit,
    ComboUp,
    ComboBreak,
    Achievement,
    LevelUp,
    StreakSaved,
    StreakLost,
    Correct,
    Incorrect,
}

impl SoundEffect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::XpGain => "xp_gain",
            Self::CriticalHit => "critical",
            Self::ComboUp => "combo_up",
            Self::ComboBreak => "combo_break",
            Self::Achievement => "achievement",
            Self::LevelUp => "level_up",
            Self::StreakSaved => "streak_saved",
            Self::StreakLost => "streak_lost",
            Self::Correct => "correct",
            Self::Incorrect => "incorrect",
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::SoundEffect;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = window, js_name = playSound)]
        fn play_sound_js(effect: &str);
    }

    pub fn play_sound(effect: SoundEffect) {
        play_sound_js(effect.as_str());
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::play_sound;

#[cfg(not(target_arch = "wasm32"))]
pub fn play_sound(_effect: SoundEffect) {
    // No-op on non-wasm targets
}
