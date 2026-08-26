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

/// Width for an alphanumeric F-key: F9 → full-width, F10 → half-width.
/// `None` for any other key.
pub(super) fn fkey_alnum_fullwidth(keysym: Keysym) -> Option<bool> {
    match keysym {
        Keysym::F9 => Some(true),
        Keysym::F10 => Some(false),
        _ => None,
    }
}

/// The three case variants of the raw keystrokes — lowercase, UPPERCASE,
/// Capitalized — at the requested width, paired with their descriptions.
fn alnum_variants(raw: &str, full: bool) -> Vec<(String, &'static str)> {
    let lower = raw.to_lowercase();
    let upper = raw.to_uppercase();
    let mut capitalized = String::with_capacity(lower.len());
    let mut chars = lower.chars();
    if let Some(first) = chars.next() {
        capitalized.extend(first.to_uppercase());
        capitalized.push_str(chars.as_str());
    }
    // width::to_full_width, not kana::ascii_to_fullwidth_char: the raw
    // keystrokes carry symbols too (`wa-do`, `w@ve`), and only the former
    // has the symbol width pairs (`-` → `－`).
    let widen = |s: String| -> String {
        if full {
            karukan_engine::width::to_full_width(&s)
        } else {
            s
        }
    };
    let descriptions = if full {
        ["[全]英小文字", "[全]英大文字", "[全]英先頭大文字"]
    } else {
        ["[半]英小文字", "[半]英大文字", "[半]英先頭大文字"]
    };
    [lower, upper, capitalized]
        .into_iter()
        .zip(descriptions)
        .map(|(text, description)| (widen(text), description))
        .collect()
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

    /// F9 (full-width) / F10 (half-width): convert the typed keystrokes to
    /// alphanumerics, cycling 小文字 → 大文字 → 先頭大文字 on repeated
    /// presses. Switching between F9 and F10 keeps the case and changes
    /// only the width. Entered as an ordinary conversion, like
    /// [`Self::fkey_conversion`].
    pub(super) fn fkey_alnum_conversion(&mut self, full: bool) -> EngineResult {
        if self.mode.current() == InputMode::Emoji {
            return EngineResult::consumed();
        }
        let reading = match &self.state {
            InputState::Conversion { reading, .. } => reading.clone(),
            _ => self.input_buf.settled_reading(&self.converters.romaji),
        };
        let raw = self.input_buf.raw_text();
        if reading.is_empty() || raw.is_empty() {
            return EngineResult::consumed();
        }

        let variants = alnum_variants(&raw, full);
        // Repeat detection: if the conversion already shows one of the
        // alnum variant lists, a repeated key advances the case cycle and
        // the other width's key keeps the case, changing only the width.
        let matches_list = |list: &CandidateList, expected: &[(String, &'static str)]| {
            list.len() == expected.len()
                && list
                    .candidates()
                    .iter()
                    .zip(expected)
                    .all(|(c, (text, _))| c.text == *text)
        };
        let cursor = match self.state.candidates() {
            Some(list) if matches_list(list, &variants) => (list.cursor() + 1) % variants.len(),
            Some(list) if matches_list(list, &alnum_variants(&raw, !full)) => list.cursor(),
            _ => 0,
        };

        let candidates: Vec<Candidate> = variants
            .into_iter()
            .map(|(text, description)| Candidate {
                text,
                reading: Some(reading.clone()),
                source: None,
                description: Some(description.to_string()),
            })
            .collect();

        self.live.shown = false;
        let mut list = CandidateList::new(candidates);
        list.set_cursor(cursor);
        self.enter_conversion_state(&reading, list)
    }
}
