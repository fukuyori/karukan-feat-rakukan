//! Degenerate-candidate detection for model output.
//!
//! Small models occasionally produce output that is not a conversion of the
//! reading: truncated prose, runaway repetition, kana echoes of the input,
//! or ruby-style annotations. [`degenerate_reason`] classifies a candidate
//! against its reading as a pure function so the checks are unit-testable
//! without a model. The thresholds are deliberately conservative: a filter
//! that sometimes lets a bad candidate through is annoying, one that eats a
//! correct conversion is a bug.

use crate::kana::katakana_to_hiragana;

/// A candidate may be this many times longer than its reading before it is
/// considered runaway output (`reading_chars * MAX_LEN_RATIO + MAX_LEN_SLACK`).
/// Kanji output is almost always *shorter* than its kana reading, so 1.5x
/// plus slack only triggers on genuine runaways.
const MAX_LEN_RATIO: f32 = 1.5;
const MAX_LEN_SLACK: f32 = 2.0;

/// A candidate this many times shorter than the reading is considered
/// degenerate. Kanji compresses kana at roughly 2:1 and rarely beyond 3:1,
/// so 4:1 only triggers on output that dropped most of the reading.
const MIN_LEN_RATIO: usize = 4;

/// The short-candidate check only applies to readings at least this long:
/// short readings legitimately compress hard (こころざし → 志).
const MIN_LEN_READING_CHARS: usize = 8;

/// Minimum repeated-unit length for the repetition check. Unit 1 repeats
/// (会社社長) are everyday Japanese; unit 2+ adjacent repeats are either
/// reduplication the reading also contains (ますます) or model runaway.
const REPEAT_UNIT_MIN: usize = 2;

/// Why a candidate was classified as degenerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegenerateReason {
    /// Empty (or whitespace-only) output.
    Empty,
    /// Extremely short relative to the reading: most of it was dropped.
    TooShort,
    /// Extremely long relative to the reading: runaway generation.
    TooLong,
    /// An adjacent repeated run the reading does not contain.
    Repetition,
    /// A kana-only strict prefix of the reading: the input echoed back cut
    /// short, not a conversion.
    PrefixEcho,
    /// A parenthesized kana-only run, i.e. ruby-style output like 明日(あした).
    RubyEcho,
}

/// Classify `candidate` against `reading` (hiragana). `None` means the
/// candidate passes every check.
pub fn degenerate_reason(candidate: &str, reading: &str) -> Option<DegenerateReason> {
    if candidate.trim().is_empty() {
        return Some(DegenerateReason::Empty);
    }

    let cand_chars: Vec<char> = candidate.chars().collect();
    let read_chars: Vec<char> = reading.chars().collect();

    // The echo shapes are checked before the length ratios so a candidate
    // that is both (ruby output is longer than the reading, a cut echo is
    // shorter) reports the specific reason, not the generic one.
    if has_kana_only_parenthetical(&cand_chars) {
        return Some(DegenerateReason::RubyEcho);
    }

    if is_prefix_echo(candidate, reading) {
        return Some(DegenerateReason::PrefixEcho);
    }

    if cand_chars.len() as f32 > read_chars.len() as f32 * MAX_LEN_RATIO + MAX_LEN_SLACK {
        return Some(DegenerateReason::TooLong);
    }

    if read_chars.len() >= MIN_LEN_READING_CHARS
        && cand_chars.len() * MIN_LEN_RATIO < read_chars.len()
    {
        return Some(DegenerateReason::TooShort);
    }

    // A reading that repeats itself (わかったわかった) legitimately converts
    // to repeated output, so the repetition check stands down entirely.
    if !has_adjacent_repeat(&read_chars, REPEAT_UNIT_MIN)
        && has_adjacent_repeat(&cand_chars, REPEAT_UNIT_MIN)
    {
        return Some(DegenerateReason::Repetition);
    }

    None
}

/// True when some unit of length >= `min_unit` occurs twice in a row.
fn has_adjacent_repeat(chars: &[char], min_unit: usize) -> bool {
    let n = chars.len();
    for unit in min_unit..=n / 2 {
        for i in 0..=n - 2 * unit {
            if chars[i..i + unit] == chars[i + unit..i + 2 * unit] {
                return true;
            }
        }
    }
    false
}

/// True when the candidate is kana-only and a strict prefix of the reading.
/// The full reading as a candidate is legitimate (a kana word converting to
/// itself); a *cut* echo is not a conversion of anything.
fn is_prefix_echo(candidate: &str, reading: &str) -> bool {
    if !candidate.chars().all(is_kana_char) {
        return false;
    }
    let normalized = katakana_to_hiragana(candidate);
    let reading_norm = katakana_to_hiragana(reading);
    normalized.chars().count() < reading_norm.chars().count()
        && reading_norm.starts_with(&normalized)
}

/// True when the text contains a parenthesized run that is entirely kana,
/// e.g. 明日(あした) or 明日（あした）— ruby-style output, not a surface.
fn has_kana_only_parenthetical(chars: &[char]) -> bool {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' || chars[i] == '（' {
            let close = if chars[i] == '(' { ')' } else { '）' };
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == close) {
                let inner = &chars[i + 1..i + 1 + end];
                if !inner.is_empty() && inner.iter().all(|&c| is_kana_char(c)) {
                    return true;
                }
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// Hiragana, katakana (full-width), or the prolonged sound mark.
fn is_kana_char(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{30A1}'..='\u{30FA}' | '\u{30FC}' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ')
}

/// The echo-context filter only applies when the reading has at least this
/// many kana: a short reading matches context sentences by coincidence far
/// too easily.
const ECHO_MIN_READING_KANA: usize = 5;

/// How many leading kana of the reading must appear in a context sentence
/// for it to count as an echo of the input.
const ECHO_HEAD_LEN: usize = 5;

/// Remove from `context` any sentence that is an unconverted (kanji-free)
/// echo of the reading's head, leaving every other sentence verbatim.
///
/// The composing buffer's own text can leak into the conversion context —
/// most directly when a chunk's model call comes back empty and the raw kana
/// reading is used as that chunk's converted text. A context that repeats
/// the input kana pulls the model toward echoing kana instead of converting.
/// Only the offending sentence is dropped, never the whole context, and the
/// caller's stored context is untouched — this filters what the model sees,
/// not what is remembered.
///
/// A sentence with any kanji is never dropped: it is converted text, i.e.
/// exactly the context the model should keep, even when its kana overlaps
/// the reading.
pub fn echo_free_context(context: &str, reading: &str) -> String {
    if context.is_empty() {
        return String::new();
    }
    let reading_kana: String = katakana_to_hiragana(reading)
        .chars()
        .filter(|&c| is_kana_char(c))
        .collect();
    if reading_kana.chars().count() < ECHO_MIN_READING_KANA {
        return context.to_string();
    }
    let echo_head: String = reading_kana.chars().take(ECHO_HEAD_LEN).collect();

    split_sentences(context)
        .into_iter()
        .filter(|s| !is_echo_sentence(s, &echo_head))
        .collect()
}

/// Split into segments, each keeping its trailing delimiter (。．！？!?
/// or newline). Text after the last delimiter forms the final segment.
fn split_sentences(context: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in context.char_indices() {
        if matches!(c, '。' | '．' | '！' | '？' | '!' | '?' | '\n') {
            let end = i + c.len_utf8();
            out.push(&context[start..end]);
            start = end;
        }
    }
    if start < context.len() {
        out.push(&context[start..]);
    }
    out
}

/// An echo sentence is kanji-free and contains the reading's head among its
/// kana. Kana are compared hiragana-normalized with everything else
/// (punctuation, ascii, digits) ignored, since the echo of interest is the
/// typed kana itself.
fn is_echo_sentence(sentence: &str, echo_head: &str) -> bool {
    if sentence
        .chars()
        .any(|c| matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々'))
    {
        return false;
    }
    let kana: String = katakana_to_hiragana(sentence)
        .chars()
        .filter(|&c| is_kana_char(c))
        .collect();
    kana.contains(echo_head)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(candidate: &str, reading: &str) -> Option<DegenerateReason> {
        degenerate_reason(candidate, reading)
    }

    #[test]
    fn accepts_normal_conversions() {
        assert_eq!(reason("漢字", "かんじ"), None);
        assert_eq!(
            reason("東京特許許可局", "とうきょうとっきょきょかきょく"),
            None
        );
        assert_eq!(reason("承る", "うけたまわる"), None);
        assert_eq!(
            reason(
                "今日は朝から雨が降っていて電車も遅れていた",
                "きょうはあさからあめがふっていてでんしゃもおくれていた"
            ),
            None
        );
        // A kana word converting to itself (equal to the reading) is real.
        assert_eq!(
            reason("すもももももももものうち", "すもももももももものうち"),
            None
        );
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(reason("", "かんじ"), Some(DegenerateReason::Empty));
        assert_eq!(reason("  ", "かんじ"), Some(DegenerateReason::Empty));
    }

    #[test]
    fn rejects_runaway_length() {
        assert_eq!(
            reason("天気天気だそうです知りませんでした", "てんき"),
            Some(DegenerateReason::TooLong)
        );
        // 1.5x + slack: a slightly longer candidate passes.
        assert_eq!(reason("お引っ越し", "ひっこし"), None);
    }

    #[test]
    fn rejects_dropped_output() {
        // 12-char reading collapsed to 2 chars: most of it is missing.
        assert_eq!(
            reason("挨拶", "あいさつをかわしましょう"),
            Some(DegenerateReason::TooShort)
        );
        // Short readings compress hard legitimately.
        assert_eq!(reason("志", "こころざし"), None);
    }

    #[test]
    fn rejects_repetition_the_reading_does_not_have() {
        assert_eq!(
            reason("今日は今日は良い天気", "きょうはよいてんき"),
            Some(DegenerateReason::Repetition)
        );
    }

    #[test]
    fn allows_repetition_the_reading_has() {
        assert_eq!(reason("わかったわかった", "わかったわかった"), None);
        assert_eq!(reason("ますます", "ますます"), None);
        assert_eq!(reason("まだまだこれからです", "まだまだこれからです"), None);
    }

    #[test]
    fn allows_single_char_repeats() {
        // Unit-1 repeats are everyday Japanese, not degeneration.
        assert_eq!(reason("会社社長", "かいしゃしゃちょう"), None);
    }

    #[test]
    fn rejects_prefix_echo() {
        assert_eq!(
            reason("あしたはあめ", "あしたはあめかもしれない"),
            Some(DegenerateReason::PrefixEcho)
        );
        // Katakana echo of a hiragana reading is still an echo.
        assert_eq!(
            reason("アシタ", "あしたはあめ"),
            Some(DegenerateReason::PrefixEcho)
        );
    }

    #[test]
    fn full_reading_is_not_an_echo() {
        assert_eq!(reason("あしたはあめ", "あしたはあめ"), None);
        // Katakana form of the full reading is a real candidate too.
        assert_eq!(reason("アシタハアメ", "あしたはあめ"), None);
    }

    #[test]
    fn rejects_ruby_output() {
        assert_eq!(
            reason("明日(あした)", "あした"),
            Some(DegenerateReason::RubyEcho)
        );
        assert_eq!(
            reason("明日（あした）", "あした"),
            Some(DegenerateReason::RubyEcho)
        );
    }

    #[test]
    fn allows_non_kana_parentheticals() {
        assert_eq!(reason("株式会社(仮)", "かぶしきがいしゃかり"), None);
    }

    #[test]
    fn echo_context_drops_identical_kana_sentence() {
        assert_eq!(
            echo_free_context("きょうはいいてんきですね", "きょうはいいてんきですね"),
            ""
        );
    }

    #[test]
    fn echo_context_drops_only_the_offending_sentence() {
        assert_eq!(
            echo_free_context(
                "今日は晴れ。あしたはあめかもしれない。また明日。",
                "あしたはあめ"
            ),
            "今日は晴れ。また明日。"
        );
    }

    #[test]
    fn echo_context_keeps_kanji_sentences() {
        // Converted text is exactly the context the model should keep,
        // even when its kana overlaps the reading.
        assert_eq!(
            echo_free_context("明日はあめかもしれない。", "あしたはあめ"),
            "明日はあめかもしれない。"
        );
        assert_eq!(
            echo_free_context("今日はいい", "てんきですね"),
            "今日はいい"
        );
    }

    #[test]
    fn echo_context_ignores_short_readings() {
        // A short reading matches by coincidence too easily; leave the
        // context alone.
        assert_eq!(echo_free_context("てんきですね", "てんき"), "てんきですね");
    }

    #[test]
    fn echo_context_matches_katakana_echoes() {
        assert_eq!(echo_free_context("アシタハアメダ", "あしたはあめ"), "");
    }

    #[test]
    fn echo_context_keeps_unrelated_kana_sentences() {
        assert_eq!(
            echo_free_context("そうですね。", "あしたはあめかな"),
            "そうですね。"
        );
    }
}
