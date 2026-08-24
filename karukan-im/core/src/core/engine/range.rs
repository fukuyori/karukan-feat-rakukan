//! Range selection: convert and commit only the head of the reading.
//!
//! The range is deliberately the simplest thing that can work — a prefix
//! length (`range_select: Option<usize>`), never an arbitrary span. It is a
//! display concern of the Composing state: Shift+Right/Left grow and shrink
//! it, Space converts just that prefix (an ordinary conversion whose scope
//! is remembered in `conversion_span`), Enter commits the prefix and the
//! remaining reading keeps composing with live conversion resumed. Every
//! exit path funnels through `clear_composition`, which drops both fields,
//! so no stale range can survive a cancel, commit, or focus change.

use super::*;
use crate::core::preedit::{AttributeType, PreeditSegment};

impl InputMethodEngine {
    /// Shift+Right: enter the range mode, or grow the selection by one
    /// reading character. Entering settles the romaji tail (like Space)
    /// and drops the live display, so what is selected is what is shown.
    pub(super) fn range_extend(&mut self) -> EngineResult {
        if self.range_select.is_none() {
            self.settle_romaji();
            self.live.shown = false;
        }
        let total = self.input_buf.reading().chars().count();
        let n = (self.range_select.unwrap_or(0) + 1).min(total);
        self.range_select = Some(n);
        self.render_range()
    }

    /// Shift+Left: shrink the selection by one character; at zero the
    /// range mode ends and the ordinary composing display returns.
    pub(super) fn range_shrink(&mut self) -> EngineResult {
        let Some(n) = self.range_select else {
            return EngineResult::consumed();
        };
        if n <= 1 {
            self.range_select = None;
            return self.refresh_input_state();
        }
        self.range_select = Some(n - 1);
        self.render_range()
    }

    /// Leave the range mode without converting (Escape, or any editing
    /// key). The composition itself is untouched.
    pub(super) fn cancel_range(&mut self) -> EngineResult {
        self.range_select = None;
        self.refresh_input_state()
    }

    /// Space in range mode: convert only the selected head, as an ordinary
    /// conversion whose scope is remembered in `conversion_span`.
    pub(super) fn start_range_conversion(&mut self) -> EngineResult {
        let Some(n) = self.range_select else {
            return EngineResult::consumed();
        };
        let reading: String = self.input_buf.reading().chars().take(n).collect();
        if reading.is_empty() {
            return self.cancel_range();
        }
        self.live.shown = false;
        let mut candidates = self.build_conversion_candidates(
            &reading,
            &reading,
            "",
            self.config.num_candidates,
            LearningLookup::Use,
        );
        // A predictive candidate completes past the selection; committing
        // it would consume more than the selected range, so it is out of
        // scope here.
        candidates.retain(|c| c.reading.as_deref().is_none_or(|r| r == reading));
        if candidates.is_empty() {
            return self.render_range();
        }
        let list = self.to_conversion_candidate_list(candidates, &reading);
        self.range_select = None;
        self.conversion_span = Some(n);
        self.enter_conversion_state(&reading, list)
    }

    /// Commit the selected candidate of a range conversion: the head
    /// leaves the buffer and is committed, the remaining reading keeps
    /// composing with live conversion resumed. The one partial-commit
    /// primitive — frontends just apply Commit followed by the refreshed
    /// composing render.
    pub(super) fn commit_range(&mut self, n: usize) -> EngineResult {
        let Some((text, reading, source)) = self.selected_conversion_info() else {
            return EngineResult::not_consumed();
        };
        if text.is_empty() {
            return EngineResult::consumed();
        }
        let learn = source.is_none_or(|s| s.records_learning(true));
        if learn && let Some(reading) = &reading {
            self.record_learning(reading, &text);
        }

        self.conversion_span = None;
        // The buffer was settled when the range mode was entered, so
        // elements and reading characters correspond one to one.
        self.input_buf.drain_prefix(n);
        // Manual chunk breaks are reading positions: shift them left with
        // the removed head; a break inside the committed range disappears.
        self.chunk_breaks = self
            .chunk_breaks
            .iter()
            .filter_map(|b| b.checked_sub(n))
            .filter(|&b| b > 0)
            .collect();
        self.chunks.clear();

        if self.input_buf.is_empty() {
            self.end_composition();
            return EngineResult::consumed()
                .with_action(EngineAction::Commit(text))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::HideAuxText);
        }

        // Commit first, then the refreshed composing render — the
        // remainder keeps composing and live conversion resumes over it.
        let refresh = self.refresh_input_state();
        let mut result = EngineResult::consumed().with_action(EngineAction::Commit(text));
        result.actions.extend(refresh.actions);
        result
    }

    /// The range display: selected head double-underlined, the rest with
    /// the ordinary composing underline, caret at the selection edge.
    fn render_range(&mut self) -> EngineResult {
        let reading = self.input_buf.reading();
        let total = reading.chars().count();
        let n = self.range_select.unwrap_or(0).min(total);
        let head: String = reading.chars().take(n).collect();
        let rest: String = reading.chars().skip(n).collect();
        let mut segments = vec![PreeditSegment::new(head, AttributeType::UnderlineDouble)];
        if !rest.is_empty() {
            segments.push(PreeditSegment::new(rest, AttributeType::Underline));
        }
        let preedit = Preedit::from_segments(segments, n);
        self.state = InputState::Composing {
            preedit: preedit.clone(),
        };
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::UpdateAuxText(format!("範囲選択 {n}/{total}")))
    }
}
