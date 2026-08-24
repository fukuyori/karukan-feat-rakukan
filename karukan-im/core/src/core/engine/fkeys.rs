//! F6–F8 fixed transforms: convert the whole composition to hiragana /
//! full-width katakana / half-width katakana, mozc-style.
//!
//! The transform is entered as an ordinary conversion whose candidate list
//! holds the three transforms, with the pressed key's transform selected —
//! so a repeated or different F-key just moves the selection, Enter commits
//! through the normal path, and Esc returns to the editable composition.
//! No new state is introduced.

use super::*;

/// Candidate-list index for an F-key: F6 → ひらがな, F7 → 全角カタカナ,
/// F8 → 半角カタカナ. `None` for any other key.
pub(super) fn fkey_transform_index(keysym: Keysym) -> Option<usize> {
    match keysym {
        Keysym::F6 => Some(0),
        Keysym::F7 => Some(1),
        Keysym::F8 => Some(2),
        _ => None,
    }
}

impl InputMethodEngine {
    /// Enter (or re-enter) the fixed-transform conversion with the
    /// `index`-th transform selected. Works from Composing and from an
    /// existing Conversion; a no-op for emoji queries and empty readings.
    pub(super) fn fkey_conversion(&mut self, index: usize) -> EngineResult {
        if self.mode.current() == InputMode::Emoji {
            return EngineResult::consumed();
        }
        // From Conversion the reading is already settled in the state; from
        // Composing settle the romaji tail the way Space does, leaving the
        // buffer itself untouched so Esc returns to an editable composition.
        let reading = match &self.state {
            InputState::Conversion { reading, .. } => reading.clone(),
            _ => self.input_buf.settled_reading(&self.converters.romaji),
        };
        if reading.is_empty() {
            return EngineResult::consumed();
        }

        let katakana = karukan_engine::hiragana_to_katakana(&reading);
        let half = karukan_engine::kana::katakana_to_half_width(&katakana);
        let transforms = [
            (reading.clone(), "ひらがな"),
            (katakana, "[全]カタカナ"),
            (half, "[半]カタカナ"),
        ];
        let target = transforms[index].0.clone();

        // A reading with no kana produces identical transforms; dedup by
        // text (keeping the first description) and select the requested
        // transform wherever it landed.
        let mut candidates: Vec<Candidate> = Vec::with_capacity(transforms.len());
        for (text, description) in transforms {
            if candidates.iter().any(|c| c.text == text) {
                continue;
            }
            candidates.push(Candidate {
                text,
                reading: Some(reading.clone()),
                source: None,
                description: Some(description.to_string()),
            });
        }
        let cursor = candidates
            .iter()
            .position(|c| c.text == target)
            .unwrap_or(0);

        self.live.shown = false;
        let mut list = CandidateList::new(candidates);
        list.set_cursor(cursor);
        self.enter_conversion_state(&reading, list)
    }
}
