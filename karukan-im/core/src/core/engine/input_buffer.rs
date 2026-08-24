//! InputBuffer: a recorded element array plus a caret, with every view
//! derived by evaluation.
//!
//! **The record** is the single source of truth: one element per display
//! character plus `cursor`, the caret as an element index. Typing `kyo` records
//! `[Romaji(k), Romaji(y), Romaji(o)]`, which evaluation re-records as
//! `[Converted(き), Converted(ょ)]` — elements and displayed characters
//! always correspond one to one, so the record can never disagree with
//! what is shown, and the caret is simply an index into both.
//!
//! - [`Element::Romaji`]: one keystroke not yet consumed by a rule (`y`,
//!   `k`, a lone `n`). Shown verbatim; evaluation may later consume it.
//! - [`Element::Converted`]: one settled character — a fired rule's kana,
//!   a passthrough like `1`, or direct input (alphabet/emoji mode). Opaque
//!   to evaluation; it never reverts.
//!
//! **Evaluation** derives everything else: the display, the conversion
//! reading, and the aux romaji tail. After a romaji keystroke is recorded,
//! the Romaji run ending at the cursor is evaluated through the converter:
//! keystrokes a rule consumed are re-recorded as its output. Elements
//! right of the cursor are never touched, so nothing combines across the
//! caret, and the caret moves without settling anything — `[Romaji(k),
//! Romaji(y), Converted(K)]` plus `o` typed before the `K` evaluates to
//! 「きょK」.
//!
//! Every record edit ends with an evaluation. Typing evaluates the run
//! ending at the caret; backspace/delete remove exactly one element and
//! then evaluate the run the removal joined, so the result always equals
//! typing the remaining keystrokes fresh: removing こ from `ytko`
//! re-exposes the live elements (`o` → 「yと」, again 「よ」), and
//! removing the `1` from `yt1t` evaluates `ytt` → 「yっt」.

use karukan_engine::RomajiConverter;

/// One display character of the composition.
#[derive(Clone)]
enum Element {
    /// A keystroke not yet consumed by a conversion rule
    Romaji(char),
    /// A settled character: fired rule output (`ko` → こ), passthrough
    /// (`1`), or direct input — excluded from romaji evaluation
    Converted {
        ch: char,
        /// The keystrokes that produced this character (mozc's raw input,
        /// what F9/F10 convert back to). A rule firing into several display
        /// characters (`kya` → きゃ) stores the whole group's keystrokes on
        /// its first character and leaves the rest empty, so concatenating
        /// over the elements reproduces the typed keys. Direct input and
        /// passthrough store themselves.
        raw: String,
    },
}

impl Element {
    fn ch(&self) -> char {
        match self {
            Element::Romaji(ch) => *ch,
            Element::Converted { ch, .. } => *ch,
        }
    }

    fn is_romaji(&self) -> bool {
        matches!(self, Element::Romaji(_))
    }

    fn direct(ch: char) -> Element {
        Element::Converted {
            ch,
            raw: ch.to_string(),
        }
    }
}

/// The recorded composition: elements plus the caret index.
pub(super) struct InputBuffer {
    elements: Vec<Element>,
    /// Caret: a boundary index into `elements`, which — with one element
    /// per display character — is also the display position.
    ///
    /// ```text
    /// elements: [Romaji(k), Romaji(y), Converted(1), Converted(K)]
    /// boundary: 0         1          2             3             4
    ///                                ↑ cursor = 2 (between y and 1)
    /// ```
    cursor: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            cursor: 0,
        }
    }

    pub fn clear(&mut self) {
        self.elements.clear();
        self.cursor = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    // --- Record edits -----------------------------------------------------

    /// Record a kana-mode keystroke at the caret, then evaluate the active
    /// run it now ends.
    pub fn push_romaji(&mut self, ch: char, romaji: &RomajiConverter) {
        self.elements
            .insert(self.cursor, Element::Romaji(ch.to_ascii_lowercase()));
        self.cursor += 1;
        self.evaluate_active_run(romaji);
    }

    /// Record a direct-input keystroke (alphabet/emoji mode) at the caret,
    /// settled as-is.
    pub fn push_direct(&mut self, ch: char) {
        self.elements.insert(self.cursor, Element::direct(ch));
        self.cursor += 1;
    }

    /// Record settled text at the caret. Test setup only — production
    /// code always goes through the typed-key paths.
    #[cfg(test)]
    pub fn insert(&mut self, text: &str) {
        let count = text.chars().count();
        self.elements
            .splice(self.cursor..self.cursor, text.chars().map(Element::direct));
        self.cursor += count;
    }

    /// Remove the element before the caret, then evaluate the Romaji run
    /// the removal joined, so the result matches typing the remaining
    /// keystrokes fresh (`yt1t` minus the `1` → 「yっt」). Returns false
    /// when the caret is at the start.
    pub fn backspace(&mut self, romaji: &RomajiConverter) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.elements.remove(self.cursor);
        self.evaluate_joined_run(romaji);
        true
    }

    /// Remove the element at the caret (delete key), then evaluate the
    /// Romaji run the removal joined. Returns false when the caret is at
    /// the end.
    pub fn delete_at_cursor(&mut self, romaji: &RomajiConverter) -> bool {
        if self.cursor == self.elements.len() {
            return false;
        }
        self.elements.remove(self.cursor);
        self.evaluate_joined_run(romaji);
        true
    }

    /// Evaluate the active run (the Romaji run ending at the cursor),
    /// re-recording keystrokes a rule consumed as its output. Typing never
    /// combines across the caret, so this stops there.
    fn evaluate_active_run(&mut self, romaji: &RomajiConverter) {
        let range = self.active_run();
        let evaluated_len = self.evaluate_range(range.clone(), romaji);
        self.cursor = range.start + evaluated_len;
    }

    /// Evaluate the Romaji run containing the caret — both sides of a
    /// deletion point. The caret keeps its offset from the run start,
    /// clamped to the evaluated length.
    fn evaluate_joined_run(&mut self, romaji: &RomajiConverter) {
        let start = self.elements[..self.cursor]
            .iter()
            .rposition(|e| !e.is_romaji())
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = self.cursor
            + self.elements[self.cursor..]
                .iter()
                .position(|e| !e.is_romaji())
                .unwrap_or(self.elements.len() - self.cursor);
        let offset = self.cursor - start;
        let evaluated_len = self.evaluate_range(start..end, romaji);
        self.cursor = start + offset.min(evaluated_len);
    }

    /// Replace a Romaji range with its evaluation; returns the new length.
    fn evaluate_range(&mut self, range: std::ops::Range<usize>, romaji: &RomajiConverter) -> usize {
        if range.is_empty() {
            return 0;
        }
        let run: String = self.elements[range.clone()]
            .iter()
            .map(Element::ch)
            .collect();
        let evaluated = evaluate_run(&run, romaji);
        let len = evaluated.len();
        self.elements.splice(range, evaluated);
        len
    }

    /// The reading as it would settle: Romaji runs force-converted in
    /// place, everything else as displayed. The non-destructive
    /// counterpart of [`Self::settle_romaji`] — used when the composition
    /// must stay editable (starting a conversion that Escape can undo).
    pub fn settled_reading(&self, romaji: &RomajiConverter) -> String {
        let mut reading = String::new();
        let mut run = String::new();
        for element in &self.elements {
            match element {
                Element::Romaji(ch) => run.push(*ch),
                Element::Converted { ch, .. } => {
                    if !run.is_empty() {
                        reading.push_str(&romaji.convert_flush(&run));
                        run.clear();
                    }
                    reading.push(*ch);
                }
            }
        }
        if !run.is_empty() {
            reading.push_str(&romaji.convert_flush(&run));
        }
        reading
    }

    /// Settle all Romaji keystrokes in place (`ltu` → っ; unmatched
    /// consonants pass through literally). Called before conversion,
    /// commit, and katakana baking. The caret keeps its distance from the
    /// end, so an end-of-composition caret stays at the end.
    pub fn settle_romaji(&mut self, romaji: &RomajiConverter) {
        if !self.elements.iter().any(Element::is_romaji) {
            return;
        }
        let from_end = self.elements.len() - self.cursor;
        let mut settled: Vec<Element> = Vec::with_capacity(self.elements.len());
        let mut run = String::new();
        for element in self.elements.drain(..) {
            match element {
                Element::Romaji(ch) => run.push(ch),
                other => {
                    flush_run(&mut settled, &mut run, romaji);
                    settled.push(other);
                }
            }
        }
        flush_run(&mut settled, &mut run, romaji);
        self.elements = settled;
        self.cursor = self.elements.len().saturating_sub(from_end);
    }

    /// Convert every settled element to katakana permanently. Called when
    /// leaving katakana mode so the preedit doesn't revert. The raw
    /// keystrokes are untouched — the same keys produced the katakana.
    pub fn bake_katakana(&mut self) {
        for element in &mut self.elements {
            if let Element::Converted { ch, .. } = element {
                let katakana = karukan_engine::hiragana_to_katakana(&ch.to_string());
                *ch = katakana.chars().next().unwrap_or(*ch);
            }
        }
    }

    /// The keystrokes that produced the composition, in order — mozc's raw
    /// input, what F9/F10 convert back to. Fired rules contribute the keys
    /// that fired them (きゃ → `kya`), passthrough and direct input
    /// contribute themselves, live keystrokes ride as-is.
    ///
    /// Best-effort under partial edits: deleting some but not all display
    /// characters of one fired rule leaves the group's keys attached to its
    /// first character (or drops them with it), so the raw text can then
    /// diverge from what typing the remaining text fresh would give.
    pub fn raw_text(&self) -> String {
        let mut out = String::new();
        for element in &self.elements {
            match element {
                Element::Romaji(ch) => out.push(*ch),
                Element::Converted { raw, .. } => out.push_str(raw),
            }
        }
        out
    }

    /// Move the caret to a display position (also its element index).
    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos.min(self.elements.len());
    }

    /// Remove the first `n` display characters. The caret keeps its
    /// position relative to the remaining text. Used by the range commit,
    /// which commits the converted head and keeps composing the rest —
    /// call after `settle_romaji` so elements and reading characters
    /// correspond one to one.
    pub fn drain_prefix(&mut self, n: usize) {
        let n = n.min(self.elements.len());
        self.elements.drain(..n);
        self.cursor = self.cursor.saturating_sub(n);
    }

    // --- Evaluation: views derived from the record ------------------------

    /// Display caret position (== the element index of the caret).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Full composition display.
    pub fn display(&self) -> String {
        self.elements.iter().map(Element::ch).collect()
    }

    pub fn char_count(&self) -> usize {
        self.elements.len()
    }

    /// Element indices of the active run: the maximal Romaji run ending at
    /// the cursor — the keystrokes currently being typed. Empty when the
    /// element left of the cursor is settled (a stranded consonant elsewhere
    /// is NOT active; it stays part of the reading at its position).
    fn active_run(&self) -> std::ops::Range<usize> {
        let start = self.elements[..self.cursor]
            .iter()
            .rposition(|e| !e.is_romaji())
            .map(|i| i + 1)
            .unwrap_or(0);
        start..self.cursor
    }

    /// Keystrokes of the active run (shown as the aux romaji tail).
    pub fn pending(&self) -> String {
        self.elements[self.active_run()]
            .iter()
            .map(Element::ch)
            .collect()
    }

    /// Conversion reading: everything except the active run. A Romaji
    /// keystroke stranded away from the caret counts as a literal
    /// character at its position, so `y1` + `ka` reads 「y1か」.
    pub fn reading(&self) -> String {
        let active = self.active_run();
        self.elements
            .iter()
            .enumerate()
            .filter(|(i, _)| !active.contains(i))
            .map(|(_, e)| e.ch())
            .collect()
    }

    /// Caret position within [`Self::reading`]. The active run sits just
    /// before the cursor and is excluded from the reading, so this is the
    /// caret minus the active run's length.
    pub fn reading_cursor(&self) -> usize {
        self.cursor - self.active_run().len()
    }
}

/// Settle one Romaji run into `out` and clear it.
fn flush_run(out: &mut Vec<Element>, run: &mut String, romaji: &RomajiConverter) {
    if run.is_empty() {
        return;
    }
    out.extend(evaluate_keystrokes(run, romaji, true));
    run.clear();
}

/// Evaluate a run of romaji keystrokes: convert the run and record one
/// element per output character (see [`evaluate_keystrokes`]).
fn evaluate_run(run: &str, romaji: &RomajiConverter) -> Vec<Element> {
    evaluate_keystrokes(run, romaji, false)
}

/// Evaluate a run of romaji keystrokes, replaying them one at a time so
/// each fired output knows which keystrokes produced it (the raw input
/// F9/F10 convert back to).
///
/// Rule outputs never contain ASCII (see the converter's contract), so an
/// ASCII character in the output is a keystroke that passed through: it
/// stays live (`Romaji`) if it can still begin a rule (`ykt` → BS → `o`
/// → 「yこ」) and settles otherwise (`1`). Everything else is a fired
/// rule's output, settled for good. With `flush` false the trailing
/// pending stays `Romaji` per keystroke; with `flush` true it is forced
/// through the flush table and settles (`ltu` → っ, a stranded `k`
/// literally).
///
/// Settling is where the configured width applies, after the classification
/// above: a character settles at the width in force when it was typed, so
/// switching to alphabet input mid-word (`（` then Shift+A) leaves what is
/// already settled alone.
fn evaluate_keystrokes(run: &str, romaji: &RomajiConverter, flush: bool) -> Vec<Element> {
    let mut elements: Vec<Element> = Vec::new();
    // Keystrokes since the last output growth: when the converter emits new
    // characters, these are the keys that produced them.
    let mut group_raw = String::new();
    let mut prefix = String::new();
    let mut emitted = 0usize;
    let mut pending = String::new();
    for key in run.chars() {
        prefix.push(key);
        group_raw.push(key);
        let converted = romaji.convert(&prefix);
        let text: Vec<char> = converted.text.chars().collect();
        if text.len() > emitted {
            // The emission consumed group_raw minus the keystrokes still
            // pending afterwards — the pending is always a tail of the
            // accumulated keys, since the converter works left to right.
            let total = group_raw.chars().count();
            let keep = converted.pending.chars().count();
            let split = total.saturating_sub(keep);
            let mut consumed: String = group_raw.chars().take(split).collect();
            let rest: String = group_raw.chars().skip(split).collect();
            push_batch(
                &mut elements,
                &text[emitted..],
                &mut consumed,
                romaji,
                false,
            );
            group_raw = rest;
            emitted = text.len();
        }
        pending = converted.pending;
    }
    if flush {
        let flushed: Vec<char> = romaji.flush_pending(&pending).chars().collect();
        if !flushed.is_empty() {
            // group_raw holds exactly the pending keystrokes at this point.
            push_batch(&mut elements, &flushed, &mut group_raw, romaji, true);
        }
    } else {
        elements.extend(pending.chars().map(Element::Romaji));
    }
    elements
}

/// Record one batch of newly emitted characters, attributing `group_raw`
/// (the keystrokes since the last emission) to them.
///
/// ASCII characters in the output are keystrokes emitted verbatim, so they
/// align one-to-one with their identical characters in `group_raw`; each is
/// its own raw. A maximal non-ASCII run is a fired rule's output and
/// carries the keystrokes between the surrounding alignment points on its
/// first character, the rest empty. `settle_all` forces even rule-starting
/// ASCII to settle (the flush path).
fn push_batch(
    elements: &mut Vec<Element>,
    batch: &[char],
    group_raw: &mut String,
    romaji: &RomajiConverter,
    settle_all: bool,
) {
    let raw: Vec<char> = group_raw.chars().collect();
    let mut r = 0; // consumed prefix of `raw`
    let mut i = 0;
    while i < batch.len() {
        let c = batch[i];
        if c.is_ascii() {
            // A passthrough keystroke: aligned with itself in the raw.
            if r < raw.len() && raw[r] == c {
                r += 1;
            }
            if !settle_all && romaji.starts_rule(c) {
                elements.push(Element::Romaji(c));
            } else {
                elements.push(Element::Converted {
                    ch: romaji.width().apply(c),
                    raw: c.to_string(),
                });
            }
            i += 1;
            continue;
        }
        // Maximal fired (non-ASCII) run: consumes the keystrokes up to the
        // next passthrough's alignment point (or all remaining ones).
        let mut j = i;
        while j < batch.len() && !batch[j].is_ascii() {
            j += 1;
        }
        let end = if j < batch.len() {
            raw[r..]
                .iter()
                .position(|&k| k == batch[j])
                .map(|p| r + p)
                .unwrap_or(raw.len())
        } else {
            raw.len()
        };
        let run_raw: String = raw[r..end].iter().collect();
        r = end;
        for (offset, &k) in batch[i..j].iter().enumerate() {
            elements.push(Element::Converted {
                ch: romaji.width().apply(k),
                raw: if offset == 0 {
                    run_raw.clone()
                } else {
                    String::new()
                },
            });
        }
        i = j;
    }
    group_raw.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str) -> InputBuffer {
        let romaji = RomajiConverter::new();
        let mut buf = InputBuffer::new();
        for ch in keys.chars() {
            buf.push_romaji(ch, &romaji);
        }
        buf
    }

    #[test]
    fn raw_text_reproduces_typed_keys() {
        assert_eq!(typed("kya").raw_text(), "kya");
        assert_eq!(typed("wasedad").raw_text(), "wasedad");
        assert_eq!(typed("konnnitiha").raw_text(), "konnnitiha");
    }

    #[test]
    fn raw_text_keeps_passthrough_keystrokes() {
        // `y` stays live, `1` passes through, `ka` fires.
        let buf = typed("y1ka");
        assert_eq!(buf.display(), "y1か");
        assert_eq!(buf.raw_text(), "y1ka");
    }

    #[test]
    fn raw_text_keeps_symbol_rule_keystrokes() {
        // `,` fires the punctuation rule (、by default).
        let buf = typed("a,");
        assert_eq!(buf.raw_text(), "a,");
    }

    #[test]
    fn raw_text_covers_direct_input_verbatim() {
        let mut buf = InputBuffer::new();
        buf.push_direct('A');
        buf.push_direct('b');
        assert_eq!(buf.raw_text(), "Ab");
    }

    #[test]
    fn raw_text_survives_settle_and_bake() {
        let romaji = RomajiConverter::new();
        let mut buf = typed("kyoltu");
        buf.settle_romaji(&romaji);
        assert_eq!(buf.display(), "きょっ");
        assert_eq!(buf.raw_text(), "kyoltu");
        buf.bake_katakana();
        assert_eq!(buf.display(), "キョッ");
        assert_eq!(buf.raw_text(), "kyoltu");
    }

    #[test]
    fn drain_prefix_removes_head_and_keeps_caret_relative() {
        let romaji = RomajiConverter::new();
        let mut buf = typed("wasedadaigaku");
        buf.settle_romaji(&romaji);
        assert_eq!(buf.display(), "わせだだいがく");
        assert_eq!(buf.cursor(), 7);

        buf.drain_prefix(3); // commit わせだ, keep だいがく
        assert_eq!(buf.display(), "だいがく");
        assert_eq!(buf.cursor(), 4);
        // The remainder keeps its raw keystrokes.
        assert_eq!(buf.raw_text(), "daigaku");
    }

    #[test]
    fn drain_prefix_clamps_and_empties() {
        let romaji = RomajiConverter::new();
        let mut buf = typed("ai");
        buf.settle_romaji(&romaji);
        buf.drain_prefix(10);
        assert!(buf.is_empty());
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn raw_text_after_partial_group_deletion_is_best_effort() {
        // Deleting ゃ from きゃ leaves the group's keys on き: the raw
        // text then reads "kya" for a buffer displaying き. Documented
        // best-effort behavior, pinned here so a change is deliberate.
        let romaji = RomajiConverter::new();
        let mut buf = typed("kya");
        assert_eq!(buf.display(), "きゃ");
        buf.backspace(&romaji);
        assert_eq!(buf.display(), "き");
        assert_eq!(buf.raw_text(), "kya");
    }
}
