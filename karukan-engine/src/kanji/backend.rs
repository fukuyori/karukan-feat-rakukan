//! Backend interface for kanji conversion using llama.cpp

use super::error::KanjiError;
use super::hf_download::{get_tokenizer_path, get_variant_path};
use super::llamacpp::LlamaCppModel;
use super::model_config::{ModelFamily, VariantConfig, registry};
use super::{CONTEXT_TOKEN, INPUT_START_TOKEN, OUTPUT_START_TOKEN};
use crate::kana::{hiragana_to_katakana, normalize_nfkc};

type Result<T> = super::error::Result<T>;

/// Configuration for kanji conversion
#[derive(Debug, Clone)]
pub struct ConversionConfig {
    /// Maximum number of new tokens to generate
    pub max_new_tokens: usize,
}

impl Default for ConversionConfig {
    fn default() -> Self {
        Self { max_new_tokens: 50 }
    }
}

/// Hard ceiling for the generation budget, whatever the reading length.
/// Bounds inference time on abnormal inputs that never reach EOS.
const MAX_GENERATION_BUDGET: usize = 256;

/// Generation budget in tokens for a reading of `reading_chars` characters.
///
/// `configured_max` (`ConversionConfig::max_new_tokens`) acts as the floor:
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

/// Inference backend configuration (llama.cpp GGUF format with external tokenizer)
#[derive(Debug, Clone)]
pub struct Backend {
    gguf_path: String,
    tokenizer_json_path: String,
    /// Display name for the model (variant id for registry models, "custom" for GGUF paths)
    display_name: String,
}

impl Backend {
    /// Create a backend from a `(ModelFamily, VariantConfig)` pair.
    ///
    /// Downloads the GGUF and the external tokenizer from HuggingFace.
    pub fn from_variant(family: &ModelFamily, variant: &VariantConfig) -> Result<Self> {
        let path = get_variant_path(family, variant)?;
        let tokenizer_path = get_tokenizer_path(family)?;
        Ok(Backend {
            gguf_path: path.to_string_lossy().to_string(),
            tokenizer_json_path: tokenizer_path.to_string_lossy().to_string(),
            display_name: variant.id.clone(),
        })
    }

    /// Create a backend by looking up a variant id in the global registry.
    ///
    /// E.g. `Backend::from_variant_id("jinen-v1-xsmall-q5")`
    pub fn from_variant_id(variant_id: &str) -> Result<Self> {
        let (family, variant) = registry()
            .find_variant(variant_id)
            .ok_or_else(|| KanjiError::UnknownVariant(variant_id.to_string()))?;
        Self::from_variant(family, variant)
    }
}

/// Kanji converter using llama.cpp backend
pub struct KanaKanjiConverter {
    model: LlamaCppModel,
    config: ConversionConfig,
    display_name: String,
}

impl KanaKanjiConverter {
    /// Create a new converter with the specified backend
    pub fn new(backend: Backend) -> Result<Self> {
        Self::with_config(backend, ConversionConfig::default())
    }

    /// Create a new converter with the specified backend and configuration
    pub fn with_config(backend: Backend, config: ConversionConfig) -> Result<Self> {
        let model = LlamaCppModel::from_file(&backend.gguf_path, &backend.tokenizer_json_path)?;
        Ok(KanaKanjiConverter {
            model,
            config,
            display_name: backend.display_name,
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
        let budget = generation_budget(katakana.chars().count(), self.config.max_new_tokens);

        let mut candidates = Vec::with_capacity(num_candidates);

        // Degenerate output (echoes, runaway repetition, extreme lengths) is
        // dropped instead of surfacing as a candidate; when everything is
        // dropped the reading fallback below still applies.
        let mut push_checked =
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
        // Skipped rather than failing when the model isn't available offline.
        let Ok(backend) = Backend::from_variant_id("jinen-v2-small-q5") else {
            eprintln!("model unavailable, skipping");
            return;
        };
        let converter = KanaKanjiConverter::new(backend).expect("Failed to create converter");

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
        use super::super::hf_download::{get_path_by_id, get_tokenizer_path_by_id};
        use super::super::{CONTEXT_TOKEN, INPUT_START_TOKEN, OUTPUT_START_TOKEN};
        // Skipped rather than failing when the model isn't available offline.
        let Ok(path) = get_path_by_id("jinen-v1-xsmall-q5") else {
            eprintln!("model unavailable, skipping");
            return;
        };
        let Ok(tok_path) = get_tokenizer_path_by_id("jinen-v1-xsmall-q5") else {
            eprintln!("tokenizer unavailable, skipping");
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
        // Skipped rather than failing when the model isn't available offline.
        let Ok(backend) = Backend::from_variant_id("jinen-v1-xsmall-q5") else {
            eprintln!("model unavailable, skipping");
            return;
        };
        let converter = KanaKanjiConverter::new(backend).expect("Failed to create converter");

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
