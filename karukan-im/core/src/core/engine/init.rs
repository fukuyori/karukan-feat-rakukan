//! Engine initialization (model loading, dictionary setup)

use std::sync::mpsc;

use anyhow::{Context, Result};
use karukan_engine::ModelSource;
use tracing::debug;

use crate::config::settings::StrategyMode;

use super::*;

/// Explicit configuration is authoritative; otherwise prefer the user's
/// dictionary over the read-only dictionary installed by the Linux package.
fn system_dictionary_path(
    configured: Option<&str>,
    data_dir: Option<std::path::PathBuf>,
    packaged: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if let Some(path) = configured {
        return Some(path.into());
    }
    let user = data_dir.map(|dir| dir.join("dict.bin"));
    if let Some(path) = &user
        && path.exists()
    {
        return user;
    }
    packaged.map(std::path::Path::to_path_buf).or(user)
}

/// Converters produced by the background model-loading thread, handed to the
/// engine through the `model_loading` channel.
pub(super) struct LoadedConverters {
    pub kanji: KanaKanjiConverter,
    pub light_kanji: Option<KanaKanjiConverter>,
}

/// A `[models]` key with the source it resolved to; the key is the name
/// the UI shows for the model.
type NamedSource = (String, ModelSource);

/// Load `source` as the converter shown as `name`, optionally setting the
/// thread count.
fn create_converter((name, source): &NamedSource, n_threads: u32) -> Result<KanaKanjiConverter> {
    let mut converter = KanaKanjiConverter::from_source(source, name)
        .with_context(|| format!("failed to load model '{name}'"))?;
    if n_threads > 0 {
        converter.set_n_threads(n_threads);
    }
    Ok(converter)
}

/// Load the conversion models. Runs on the background loading thread — it
/// may block on a model download, which must stay off the key-event thread.
/// A light-model failure is non-fatal (beam search is simply unavailable).
fn load_converters(
    main: NamedSource,
    light: Option<NamedSource>,
    n_threads: u32,
) -> Result<LoadedConverters> {
    let kanji = create_converter(&main, n_threads)?;
    tracing::info!("Main model loaded: {}", kanji.model_display_name());

    let light_kanji = light.and_then(|source| match create_converter(&source, n_threads) {
        Ok(converter) => {
            tracing::info!("Beam model loaded: {}", converter.model_display_name());
            Some(converter)
        }
        Err(e) => {
            tracing::warn!("Failed to initialize beam model: {e:#}");
            None
        }
    });
    Ok(LoadedConverters { kanji, light_kanji })
}

impl InputMethodEngine {
    /// Full engine initialization from user settings: system dictionary,
    /// user dictionaries, learning cache, and conversion models according
    /// to the configured strategy.
    ///
    /// Shared by the fcitx5 FFI (`karukan_engine_init`) and the stdio
    /// JSON-RPC server (`init` method). Dictionaries and the learning cache
    /// load synchronously (local files, fast); the models load on a
    /// background thread because resolving them can touch the network.
    /// Until they arrive (or if loading fails) the engine runs with what it
    /// has: romaji conversion, dictionaries, learning cache, rewriters.
    pub fn init_from_settings(&mut self, settings: &Settings) -> Result<()> {
        tracing::info!(
            "Karukan init: model={:?}, light_model={:?}, strategy={:?}",
            settings.conversion.model,
            settings.conversion.light_model,
            settings.conversion.strategy,
        );

        self.init_system_dictionary(settings.conversion.dict_path.as_deref());
        self.init_user_dictionaries();
        self.init_learning_cache(
            settings.learning.enabled,
            LearningConfig {
                max_entries: settings.learning.max_entries,
                max_surface_chars: settings.learning.max_surface_chars,
                stale_days: settings.learning.stale_days,
            },
        );

        self.spawn_model_loading(settings);
        Ok(())
    }

    /// Load the conversion models on a background thread; never blocks.
    ///
    /// The result arrives through the `model_loading` channel and is
    /// installed by `poll_loaded_models` on the next key event. A failure is
    /// logged on the loader thread and surfaces here only as a disconnected
    /// channel: the engine keeps running without a model.
    fn spawn_model_loading(&mut self, settings: &Settings) {
        if self.converters.kanji.is_some() || self.model_loading.is_some() {
            return;
        }

        let conv = &settings.conversion;
        // Light runs the light model alone in the main slot; only Adaptive
        // keeps a separate light model for beam search.
        let (main_key, light_key) = match conv.strategy {
            StrategyMode::Light => (&conv.light_model, None),
            StrategyMode::Main => (&conv.model, None),
            StrategyMode::Adaptive => (&conv.model, Some(&conv.light_model)),
        };
        let named = |key: &String| settings.model_source(key).map(|s| (key.clone(), s));
        let main = match named(main_key) {
            Ok(source) => source,
            Err(e) => {
                tracing::error!("invalid model settings, continuing without model: {e:#}");
                return;
            }
        };
        let light = light_key.and_then(|key| {
            named(key)
                .inspect_err(|e| {
                    tracing::warn!("invalid light_model settings, beam search unavailable: {e:#}")
                })
                .ok()
        });
        let n_threads = conv.n_threads;

        let (tx, rx) = mpsc::channel();
        self.model_loading = Some(rx);
        let spawned = std::thread::Builder::new()
            .name("karukan-model-load".to_string())
            .spawn(move || {
                match load_converters(main, light, n_threads) {
                    // A dead receiver just means the engine was dropped.
                    Ok(loaded) => drop(tx.send(loaded)),
                    Err(e) => {
                        tracing::error!("model loading failed, continuing without model: {e:#}");
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!("failed to spawn model loading thread: {e}");
            self.model_loading = None;
        }
    }

    /// Install converters the background loader has finished. Non-blocking;
    /// called at the top of `process_key`. A disconnected channel means the
    /// loader failed (already logged) — clear it so `model_name` stops
    /// reporting "loading".
    pub(super) fn poll_loaded_models(&mut self) {
        let Some(rx) = &self.model_loading else {
            return;
        };
        match rx.try_recv() {
            Ok(loaded) => {
                self.converters.kanji = Some(loaded.kanji);
                self.converters.light_kanji = loaded.light_kanji;
                self.model_loading = None;
                tracing::info!("Karukan init complete: {}", self.model_name());
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.model_loading = None;
            }
        }
    }

    /// Initialize the system dictionary for candidate lookup
    ///
    /// Uses `dict_path`, then `data_dir/dict.bin`, then the Linux package dictionary.
    /// If the file doesn't exist, the engine continues without a dictionary.
    pub fn init_system_dictionary(&mut self, dict_path: Option<&str>) {
        if self.dicts.system.is_some() {
            return;
        }

        let packaged = cfg!(target_os = "linux")
            .then_some(std::path::Path::new("/usr/share/karukan-im/dict.bin"));
        let Some(path) = system_dictionary_path(dict_path, Settings::data_dir(), packaged) else {
            tracing::warn!("Could not determine data directory for system dictionary");
            return;
        };

        if !path.exists() {
            tracing::warn!("System dictionary not found at {:?}, skipping", path);
            return;
        }

        match Dictionary::load(&path) {
            Ok(dict) => {
                tracing::info!("System dictionary loaded from {:?}", path);
                self.dicts.system = Some(dict);
            }
            Err(e) => {
                tracing::warn!("Failed to load system dictionary from {:?}: {}", path, e);
            }
        }
    }

    /// Initialize the learning cache from disk.
    ///
    /// Loads `~/.local/share/karukan-im/learning.tsv` if it exists.
    /// If the file doesn't exist, creates an empty in-memory cache.
    /// `config.max_surface_chars` caps the surface length `record` accepts;
    /// entries already on disk are loaded regardless (they can be removed
    /// with Ctrl+Delete or by eviction).
    pub fn init_learning_cache(&mut self, enabled: bool, config: LearningConfig) {
        if !enabled || self.learning.is_some() {
            return;
        }

        let cache = match Settings::learning_file() {
            Some(path) if path.exists() => match LearningCache::load(&path, config) {
                Ok(cache) => {
                    debug!(
                        "Learning cache loaded from {:?} ({} entries)",
                        path,
                        cache.entry_count()
                    );
                    cache
                }
                Err(e) => {
                    debug!("Failed to load learning cache from {:?}: {}", path, e);
                    LearningCache::new(config)
                }
            },
            Some(path) => {
                debug!("Learning cache not found at {:?}, starting empty", path);
                LearningCache::new(config)
            }
            None => {
                debug!("Could not determine learning cache path");
                LearningCache::new(config)
            }
        };
        self.learning = Some(cache);
    }

    /// Initialize user dictionaries by scanning the user dictionary directory.
    ///
    /// All files in the directory are loaded with `Dictionary::load_auto()`
    /// (auto-detects KRKN binary or Mozc TSV). Files are loaded in sorted
    /// order; earlier files have higher priority after merging.
    ///
    /// Default directory: `~/.local/share/karukan-im/user_dicts/`
    pub fn init_user_dictionaries(&mut self) {
        if self.user_dict_watcher.is_some() {
            return;
        }

        let Some(dir) = Settings::user_dict_dir() else {
            debug!("Could not determine user dictionary directory");
            return;
        };

        // The watcher owns loading from here on: the initial refresh is the
        // one-shot load this used to do, and later refreshes (throttled at
        // the top of process_key) pick up edits without an IME restart. A
        // directory that does not exist yet reads as empty, so creating it
        // later is detected like any other change.
        let mut watcher = super::user_dicts::UserDictWatcher::new(dir);
        if let Some(merged) = watcher.refresh() {
            self.dicts.user = merged;
        }
        self.user_dict_watcher = Some(watcher);
        self.user_dicts_checked = Some(std::time::Instant::now());
    }
}

#[cfg(test)]
mod dictionary_path_tests {
    use super::system_dictionary_path;
    use std::path::Path;

    #[test]
    fn packaged_dictionary_is_used_without_user_dictionary() {
        let dir = tempfile::tempdir().unwrap();
        let packaged = Path::new("/usr/share/karukan-im/dict.bin");
        assert_eq!(
            system_dictionary_path(None, Some(dir.path().into()), Some(packaged)),
            Some(packaged.into())
        );
        assert_eq!(
            system_dictionary_path(None, None, Some(packaged)),
            Some(packaged.into())
        );
    }

    #[test]
    fn user_dictionary_overrides_package() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("dict.bin");
        std::fs::write(&user, b"user dictionary").unwrap();
        assert_eq!(
            system_dictionary_path(
                None,
                Some(dir.path().into()),
                Some(Path::new("/package/dict.bin"))
            ),
            Some(user)
        );
    }

    #[test]
    fn explicit_path_is_authoritative_even_when_missing() {
        assert_eq!(
            system_dictionary_path(
                Some("/custom/missing.bin"),
                None,
                Some(Path::new("/package/dict.bin"))
            ),
            Some("/custom/missing.bin".into())
        );
    }

    #[test]
    fn platforms_without_package_keep_user_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            system_dictionary_path(None, Some(dir.path().into()), None),
            Some(dir.path().join("dict.bin"))
        );
        assert_eq!(system_dictionary_path(None, None, None), None);
    }
}
