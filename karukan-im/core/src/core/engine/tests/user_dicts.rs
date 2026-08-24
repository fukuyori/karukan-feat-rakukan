//! Engine-level tests for user-dictionary hot reload: the watcher is
//! polled at the top of `process_key`, so edits to `user_dicts/` reach the
//! merged dictionary without an IME restart.

use super::*;
use std::path::Path;

fn write_tsv(dir: &Path, name: &str, rows: &[(&str, &str)]) {
    let body: String = rows
        .iter()
        .map(|(reading, word)| format!("{reading}\t{word}\t名詞\t\n"))
        .collect();
    std::fs::write(dir.join(name), body).unwrap();
}

/// Engine with its user-dict watcher pointed at `dir`, mirroring what
/// `init_user_dictionaries` does with the real data directory.
fn engine_watching(dir: &Path) -> InputMethodEngine {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    let mut watcher = crate::core::engine::user_dicts::UserDictWatcher::new(dir.to_path_buf());
    if let Some(merged) = watcher.refresh() {
        engine.dicts.user = merged;
    }
    engine.user_dict_watcher = Some(watcher);
    engine
}

fn user_dict_has(engine: &InputMethodEngine, reading: &str, word: &str) -> bool {
    engine
        .dicts
        .user
        .as_ref()
        .and_then(|d| d.exact_match_search(reading))
        .is_some_and(|r| r.candidates.iter().any(|c| c.surface == word))
}

#[test]
fn added_file_is_picked_up_by_poll() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_watching(dir.path());
    assert!(engine.dicts.user.is_none());

    write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);
    engine.user_dicts_checked = None; // bypass the 2s throttle
    engine.poll_user_dicts();

    assert!(user_dict_has(&engine, "わせだ", "早稲田"));
}

#[test]
fn edited_file_is_picked_up_by_poll() {
    let dir = tempfile::tempdir().unwrap();
    write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);
    let mut engine = engine_watching(dir.path());
    assert!(user_dict_has(&engine, "わせだ", "早稲田"));

    write_tsv(
        dir.path(),
        "a.tsv",
        &[("わせだ", "早稲田"), ("とうだい", "東大")],
    );
    engine.user_dicts_checked = None;
    engine.poll_user_dicts();

    assert!(user_dict_has(&engine, "とうだい", "東大"));
}

#[test]
fn removed_file_is_picked_up_by_poll() {
    let dir = tempfile::tempdir().unwrap();
    write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);
    let mut engine = engine_watching(dir.path());
    assert!(user_dict_has(&engine, "わせだ", "早稲田"));

    std::fs::remove_file(dir.path().join("a.tsv")).unwrap();
    engine.user_dicts_checked = None;
    engine.poll_user_dicts();

    assert!(engine.dicts.user.is_none());
}

#[test]
fn poll_is_throttled() {
    // A check that just ran suppresses the next one, so per-keystroke cost
    // stays at a timestamp comparison.
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_watching(dir.path());
    engine.user_dicts_checked = Some(std::time::Instant::now());

    write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);
    engine.poll_user_dicts();

    assert!(
        engine.dicts.user.is_none(),
        "a just-checked watcher must not stat again immediately"
    );
}
