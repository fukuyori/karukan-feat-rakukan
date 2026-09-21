//! Model source resolution and the kana-kanji converter

use super::error::KanjiError;
use super::hf_download::download_gguf;
use super::llamacpp::LlamaCppModel;
use super::{CONTEXT_TOKEN, INPUT_START_TOKEN, OUTPUT_START_TOKEN};
use crate::kana::{hiragana_to_katakana, normalize_nfkc};
use std::path::PathBuf;

type Result<T> = super::error::Result<T>;

/// Where a conversion model's GGUF comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSource {
    /// A HuggingFace repo; `tokenizer.json` comes from the same repo.
    HuggingFace { repo: String, filename: String },
    /// A local GGUF file; `tokenizer.json` must sit in the same directory.
    Path(PathBuf),
}

impl ModelSource {
    /// Resolve to local `(gguf, tokenizer.json)` paths. HuggingFace files
    /// are served cache-first and downloaded on a cache miss.
    pub fn resolve(&self) -> Result<(PathBuf, PathBuf)> {
        match self {
            ModelSource::HuggingFace { repo, filename } => Ok((
                download_gguf(repo, filename)?,
                download_gguf(repo, "tokenizer.json")?,
            )),
            ModelSource::Path(path) => {
                if !path.is_file() {
                    return Err(KanjiError::ModelNotFound(path.clone()));
                }
                let tokenizer = path.with_file_name("tokenizer.json");
                if !tokenizer.is_file() {
                    return Err(KanjiError::TokenizerNotFound(tokenizer));
                }
                Ok((path.clone(), tokenizer))
            }
        }
    }
}

/// Cap on the tokens generated per conversion, and the floor of the
/// length-scaled budget below.
const MAX_NEW_TOKENS: usize = 50;

/// Hard ceiling for the generation budget, whatever the reading length.
/// Bounds inference time on abnormal inputs that never reach EOS.
const MAX_GENERATION_BUDGET: usize = 256;

/// Generation budget in tokens for a reading of `reading_chars` characters.
///
/// `configured_max` ([`MAX_NEW_TOKENS`]) acts as the floor:
/// short readings keep the configured budget, while long readings get
/// `reading_chars * 2 + 8` so the output is never truncated merely because
/// the reading was long. Kanji output is at most ~1 token per reading char
/// and byte-fallback runs cost up to 3 tokens per char, so 2x + slack covers
/// real conversions; [`MAX_GENERATION_BUDGET`] caps the pathological case.
pub fn generation_budget(reading_chars: usize, configured_max: usize) -> usize {
    (reading_chars.saturating_mul(2).saturating_add(8))
        .max(configured_max)
        .min(MAX_GENERATION_BUDGET)
}

/// Build a prompt in jinen format.
///
/// The prompt is NFKC-normalized: jinen models are trained on NFKC text and
/// full-width ASCII in the context degrades accuracy. The special tokens
/// (U+EE00–U+EE02) are unaffected by NFKC.
pub fn build_jinen_prompt(katakana: &str, context: &str) -> String {
    normalize_nfkc(&format!(
        "{}{}{}{}{}",
        CONTEXT_TOKEN, context, INPUT_START_TOKEN, katakana, OUTPUT_START_TOKEN
    ))
}

/// Clean model output by trimming whitespace.
///
/// Special tokens (BOS/EOS) are handled at the decode level via
/// `skip_special_tokens` rather than string replacement.
pub fn clean_model_output(text: &str) -> String {
    text.trim().to_string()
}

/// Kanji converter using llama.cpp backend
pub struct KanaKanjiConverter {
    model: LlamaCppModel,
    display_name: String,
}

impl KanaKanjiConverter {
    /// Load the model at `source`. `name` is what the UI shows for it: the
    /// `[models]` key, for a configured model.
    pub fn from_source(source: &ModelSource, name: &str) -> Result<Self> {
        let (gguf, tokenizer) = source.resolve()?;
        let model = LlamaCppModel::from_file(&gguf, &tokenizer)?;
        Ok(KanaKanjiConverter {
            model,
            display_name: name.to_string(),
        })
    }

    /// Set the number of threads for inference (0 = default).
    pub fn set_n_threads(&mut self, n: u32) {
        self.model.set_n_threads(n);
    }

    /// Convert hiragana to kanji candidates
    ///
    /// # Arguments
    /// * `reading` - Input reading in hiragana
    /// * `context` - Left context (previously converted text)
    /// * `num_candidates` - Number of candidates to generate
    ///
    /// # Returns
    /// Vector of conversion candidates
    pub fn convert(
        &self,
        reading: &str,
        context: &str,
        num_candidates: usize,
    ) -> Result<Vec<String>> {
        // Convert hiragana to katakana (model expects katakana input)
        let katakana = hiragana_to_katakana(reading);

        // A context sentence that echoes the input kana pulls the model
        // toward echoing instead of converting; filter what the model sees.
        // The caller's stored context (and any cache key built from it) is
        // untouched.
        let filtered_context = super::quality::echo_free_context(context, reading);
        if filtered_context != context {
            tracing::debug!("echo context filtered: {context:?} -> {filtered_context:?}");
        }

        // Build prompt in jinen format
        let prompt = build_jinen_prompt(&katakana, &filtered_context);

        // Tokenize
        let tokens = self.model.tokenize(&prompt)?;
        let eos = Some(self.model.eos_token_id().0);

        // Budget scales with the reading so long readings aren't truncated
        // mid-output by the fixed configured maximum.
        let budget = generation_budget(katakana.chars().count(), MAX_NEW_TOKENS);

        let mut candidates = Vec::with_capacity(num_candidates);

        // Degenerate output (echoes, runaway repetition, extreme lengths) is
        // dropped instead of surfacing as a candidate; when everything is
        // dropped the reading fallback below still applies.
        let push_checked =
            |candidates: &mut Vec<String>, clean: String| match super::quality::degenerate_reason(
                &clean, reading,
            ) {
                None => {
                    if !candidates.contains(&clean) {
                        candidates.push(clean);
                    }
                }
                Some(why) => {
                    tracing::debug!("dropped degenerate candidate ({why:?}): {clean:?}");
                }
            };

        if num_candidates == 1 {
            // Single candidate: use greedy decoding (faster)
            let output_tokens = self.model.generate(&tokens, budget, eos)?;
            let generated = &output_tokens[tokens.len()..];
            let text = self.model.decode(generated, true)?;
            let clean = clean_model_output(&text);

            push_checked(&mut candidates, clean);
        } else {
            // Multiple candidates: use beam search
            let results = self
                .model
                .generate_beam_search(&tokens, budget, eos, num_candidates)?;

            // Only beams that reached EOS become candidates: a budget-cut
            // beam is prose cut mid-output, not a conversion of the reading.
            // If every beam was cut, the reading fallback below still applies.
            let (complete, truncated): (Vec<_>, Vec<_>) =
                results.into_iter().partition(|c| c.finished);
            if !truncated.is_empty() {
                tracing::debug!(
                    "beam search: {}/{} beams hit the generation budget and were dropped",
                    truncated.len(),
                    truncated.len() + complete.len()
                );
            }
            for c in complete {
                let text = self.model.decode(&c.tokens, true)?;
                let clean = clean_model_output(&text);

                // Observation stage of the confidence filter (Phase 1-E):
                // log the length-normalized score only. A rejection rule is
                // added once real distributions have been collected.
                tracing::debug!(
                    "candidate {:?}: avg_logprob {:.3} over {} tokens",
                    clean,
                    c.score / c.tokens.len().max(1) as f32,
                    c.tokens.len()
                );

                push_checked(&mut candidates, clean);
            }
        }

        // If no candidates, return the original reading
        if candidates.is_empty() {
            candidates.push(reading.to_string());
        }

        Ok(candidates)
    }

    /// Get a human-readable model name for display
    pub fn model_display_name(&self) -> &str {
        &self.display_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hugging_face_source(repo: &str, filename: &str) -> ModelSource {
        ModelSource::HuggingFace {
            repo: repo.to_string(),
            filename: filename.to_string(),
        }
    }

    #[test]
    fn test_resolve_missing_gguf() {
        let source = ModelSource::Path(PathBuf::from("/nonexistent/model.gguf"));
        let err = source.resolve().unwrap_err();
        assert!(matches!(err, KanjiError::ModelNotFound(_)), "{err}");
    }

    #[test]
    fn test_resolve_missing_tokenizer() {
        let dir = tempfile::tempdir().unwrap();
        let gguf = dir.path().join("model.gguf");
        std::fs::write(&gguf, b"gguf").unwrap();

        let err = ModelSource::Path(gguf).resolve().unwrap_err();
        let expected = dir.path().join("tokenizer.json");
        match err {
            KanjiError::TokenizerNotFound(path) => assert_eq!(path, expected),
            other => panic!("expected TokenizerNotFound, got {other}"),
        }
    }

    #[test]
    fn test_resolve_local_path() {
        let dir = tempfile::tempdir().unwrap();
        let gguf = dir.path().join("my-model.gguf");
        std::fs::write(&gguf, b"gguf").unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), b"{}").unwrap();

        let (model, tokenizer) = ModelSource::Path(gguf.clone()).resolve().unwrap();
        assert_eq!(model, gguf);
        assert_eq!(tokenizer, dir.path().join("tokenizer.json"));
    }

    #[test]
    fn generation_budget_uses_configured_max_as_floor() {
        // Short readings keep the configured budget.
        assert_eq!(generation_budget(0, 50), 50);
        assert_eq!(generation_budget(10, 50), 50);
        // 21 chars is the break-even: 21 * 2 + 8 = 50.
        assert_eq!(generation_budget(21, 50), 50);
        assert_eq!(generation_budget(22, 50), 52);
    }

    #[test]
    fn generation_budget_scales_with_reading_length() {
        assert_eq!(generation_budget(30, 50), 68);
        assert_eq!(generation_budget(45, 50), 98);
        // A configured max above the formula wins.
        assert_eq!(generation_budget(30, 100), 100);
    }

    #[test]
    fn generation_budget_is_capped() {
        assert_eq!(generation_budget(1000, 50), MAX_GENERATION_BUDGET);
        assert_eq!(generation_budget(124, 50), MAX_GENERATION_BUDGET);
        // 123 chars: 123 * 2 + 8 = 254, just under the cap.
        assert_eq!(generation_budget(123, 50), 254);
        // The cap also bounds an oversized configured max.
        assert_eq!(generation_budget(10, 10_000), MAX_GENERATION_BUDGET);
    }

    #[test]

    fn test_default_model_conversion() {
        let source = hugging_face_source(
            "togatogah/jinen-v2-small.gguf",
            "jinen-v2-small-Q5_K_M.gguf",
        );
        // Skipped rather than failing when the model isn't available offline.
        let Ok(converter) = KanaKanjiConverter::from_source(&source, "small") else {
            eprintln!("model unavailable, skipping");
            return;
        };

        let result = converter.convert("かんじ", "", 1);
        assert!(result.is_ok(), "Conversion failed: {:?}", result.err());

        let candidates = result.unwrap();
        assert!(!candidates.is_empty(), "No candidates returned");

        let output = &candidates[0];
        assert!(
            !output.contains("ã"),
            "Output contains mojibake: '{}'",
            output
        );
    }

    #[test]

    fn test_xsmall_special_tokens() {
        use super::super::{CONTEXT_TOKEN, INPUT_START_TOKEN, OUTPUT_START_TOKEN};
        let source = hugging_face_source(
            "togatogah/jinen-v1-xsmall.gguf",
            "jinen-v1-xsmall-Q5_K_M.gguf",
        );
        // Skipped rather than failing when the model isn't available offline.
        let Ok((path, tok_path)) = source.resolve() else {
            eprintln!("model unavailable, skipping");
            return;
        };
        let model = LlamaCppModel::from_file(&path, &tok_path).expect("Failed to load model");

        let prompt = build_jinen_prompt("テスト", "");
        let tokens = model.tokenize(&prompt).expect("Failed to tokenize");

        let mut found_context = false;
        let mut found_input_start = false;
        let mut found_output_start = false;

        for token in &tokens {
            let display = model.decode_token_for_display(*token);
            if display.contains(CONTEXT_TOKEN) {
                found_context = true;
            }
            if display.contains(INPUT_START_TOKEN) {
                found_input_start = true;
            }
            if display.contains(OUTPUT_START_TOKEN) {
                found_output_start = true;
            }
        }

        assert!(found_context, "CONTEXT token (U+EE02) not found");
        assert!(found_input_start, "INPUT_START token (U+EE00) not found");
        assert!(found_output_start, "OUTPUT_START token (U+EE01) not found");
    }

    #[test]

    fn test_xsmall_conversion() {
        let source = hugging_face_source(
            "togatogah/jinen-v1-xsmall.gguf",
            "jinen-v1-xsmall-Q5_K_M.gguf",
        );
        // Skipped rather than failing when the model isn't available offline.
        let Ok(converter) = KanaKanjiConverter::from_source(&source, "xsmall") else {
            eprintln!("model unavailable, skipping");
            return;
        };

        let result = converter.convert("かんじ", "", 1);
        assert!(result.is_ok(), "Conversion failed: {:?}", result.err());

        let candidates = result.unwrap();
        assert!(!candidates.is_empty(), "No candidates returned");

        let output = &candidates[0];
        assert!(
            !output.contains("ã"),
            "Output contains mojibake (GPT-2 byte encoding leak): '{}'",
            output
        );
    }
}
