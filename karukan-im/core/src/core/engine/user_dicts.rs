//! User-dictionary hot reload.
//!
//! Watches the `user_dicts/` directory by fingerprint (path + mtime + size)
//! instead of re-reading files on every check: a stat pass per file decides
//! whether anything changed, and only changed files are re-parsed. Each
//! file's parsed dictionary is cached, so a file that becomes unreadable or
//! corrupt keeps serving its last good contents until it changes again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use karukan_engine::Dictionary;
use tracing::{debug, warn};

/// A file's identity as far as cheap stat calls can tell: modification
/// time plus size. Content edits move at least one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    mtime: SystemTime,
    size: u64,
}

impl FileStamp {
    fn of(path: &Path) -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok()?;
        Some(FileStamp {
            mtime: meta.modified().ok()?,
            size: meta.len(),
        })
    }
}

/// One tracked file: the stamp at the last load *attempt* and the parsed
/// dictionary from the last *successful* load. A broken edit keeps the
/// previous dictionary (last-good) while the stamp advances, so the file
/// is retried only when it changes again — not on every check.
struct UserDictFile {
    stamp: FileStamp,
    dict: Option<Dictionary>,
}

/// Watches a user-dictionary directory and rebuilds the merged dictionary
/// when its files change. `BTreeMap` keeps the files in path order, which
/// preserves the alphabetical merge priority of the one-shot loader this
/// replaces.
pub(super) struct UserDictWatcher {
    dir: PathBuf,
    files: BTreeMap<PathBuf, UserDictFile>,
}

impl UserDictWatcher {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            files: BTreeMap::new(),
        }
    }

    /// Stat the directory and reload whatever changed.
    ///
    /// Returns `None` when nothing changed. Returns `Some(merged)` when
    /// files were added, edited, or removed — `merged` is `None` when no
    /// dictionary file currently loads (e.g. the last file was deleted).
    /// A directory that does not exist reads as empty, so creating it
    /// later is picked up like any other change.
    pub fn refresh(&mut self) -> Option<Option<Dictionary>> {
        let current = self.scan();
        let mut changed = false;

        self.files.retain(|path, _| {
            let keep = current.contains_key(path);
            if !keep {
                debug!("User dictionary removed: {:?}", path);
                changed = true;
            }
            keep
        });

        for (path, stamp) in current {
            use std::collections::btree_map::Entry;
            match self.files.entry(path) {
                Entry::Occupied(mut o) => {
                    if o.get().stamp == stamp {
                        continue;
                    }
                    changed = true;
                    match Dictionary::load_auto(o.key()) {
                        Ok(dict) => {
                            debug!("User dictionary reloaded: {:?}", o.key());
                            o.get_mut().dict = Some(dict);
                        }
                        Err(e) => {
                            // Keep serving the previous contents; the stamp
                            // still advances so this is not retried until
                            // the file changes again.
                            warn!(
                                "User dictionary {:?} failed to load, keeping previous: {}",
                                o.key(),
                                e
                            );
                        }
                    }
                    o.get_mut().stamp = stamp;
                }
                Entry::Vacant(v) => {
                    changed = true;
                    let dict = match Dictionary::load_auto(v.key()) {
                        Ok(dict) => {
                            debug!("User dictionary loaded: {:?}", v.key());
                            Some(dict)
                        }
                        Err(e) => {
                            warn!("User dictionary {:?} failed to load: {}", v.key(), e);
                            None
                        }
                    };
                    v.insert(UserDictFile { stamp, dict });
                }
            }
        }

        if !changed {
            return None;
        }
        let merged = Dictionary::merge(self.files.values().filter_map(|f| f.dict.as_ref()))
            .unwrap_or_else(|e| {
                warn!("Failed to merge user dictionaries: {}", e);
                None
            });
        Some(merged)
    }

    /// One stat pass over the directory: regular files with their stamps.
    fn scan(&self) -> BTreeMap<PathBuf, FileStamp> {
        let mut out = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(stamp) = FileStamp::of(&path) {
                out.insert(path, stamp);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tsv(dir: &Path, name: &str, rows: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let body: String = rows
            .iter()
            .map(|(reading, word)| format!("{reading}\t{word}\t名詞\t\n"))
            .collect();
        std::fs::write(&path, body).unwrap();
        path
    }

    fn has_word(dict: &Dictionary, reading: &str, word: &str) -> bool {
        dict.exact_match_search(reading)
            .is_some_and(|r| r.candidates.iter().any(|c| c.surface == word))
    }

    #[test]
    fn initial_refresh_loads_and_merges_all_files() {
        let dir = tempfile::tempdir().unwrap();
        write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);
        write_tsv(dir.path(), "b.tsv", &[("めいだい", "明大")]);

        let mut watcher = UserDictWatcher::new(dir.path().to_path_buf());
        let merged = watcher.refresh().expect("first refresh is a change");
        let dict = merged.expect("two files loaded");
        assert!(has_word(&dict, "わせだ", "早稲田"));
        assert!(has_word(&dict, "めいだい", "明大"));
    }

    #[test]
    fn unchanged_directory_reports_no_change() {
        let dir = tempfile::tempdir().unwrap();
        write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);

        let mut watcher = UserDictWatcher::new(dir.path().to_path_buf());
        assert!(watcher.refresh().is_some());
        assert!(watcher.refresh().is_none(), "no files changed");
    }

    #[test]
    fn edited_file_is_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);

        let mut watcher = UserDictWatcher::new(dir.path().to_path_buf());
        watcher.refresh().unwrap();

        // Different byte length, so the stamp changes even within the same
        // mtime second.
        write_tsv(
            dir.path(),
            "a.tsv",
            &[("わせだ", "早稲田"), ("とうだい", "東大")],
        );
        let dict = watcher.refresh().expect("edit detected").expect("loads");
        assert!(has_word(&dict, "とうだい", "東大"));
    }

    #[test]
    fn removed_file_drops_its_words() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);
        write_tsv(dir.path(), "b.tsv", &[("めいだい", "明大")]);

        let mut watcher = UserDictWatcher::new(dir.path().to_path_buf());
        watcher.refresh().unwrap();

        std::fs::remove_file(&a).unwrap();
        let dict = watcher
            .refresh()
            .expect("removal detected")
            .expect("b remains");
        assert!(!has_word(&dict, "わせだ", "早稲田"));
        assert!(has_word(&dict, "めいだい", "明大"));
    }

    #[test]
    fn removing_the_last_file_clears_the_dictionary() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);

        let mut watcher = UserDictWatcher::new(dir.path().to_path_buf());
        watcher.refresh().unwrap();

        std::fs::remove_file(&a).unwrap();
        let merged = watcher.refresh().expect("removal detected");
        assert!(merged.is_none(), "no files left to merge");
    }

    #[test]
    fn broken_edit_keeps_the_last_good_contents() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_tsv(dir.path(), "a.tsv", &[("わせだ", "早稲田")]);

        let mut watcher = UserDictWatcher::new(dir.path().to_path_buf());
        watcher.refresh().unwrap();

        // A KRKN magic with garbage fails the binary loader.
        std::fs::write(&a, b"KRKN\x00broken").unwrap();
        let dict = watcher
            .refresh()
            .expect("change detected")
            .expect("last good is kept");
        assert!(
            has_word(&dict, "わせだ", "早稲田"),
            "broken file must keep serving its last good contents"
        );

        // And it is not retried while unchanged.
        assert!(watcher.refresh().is_none());

        // Fixing the file is picked up again.
        write_tsv(
            dir.path(),
            "a.tsv",
            &[("わせだ", "早稲田"), ("こまば", "駒場")],
        );
        let dict = watcher.refresh().unwrap().unwrap();
        assert!(has_word(&dict, "こまば", "駒場"));
    }

    #[test]
    fn missing_directory_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("not-yet");

        let mut watcher = UserDictWatcher::new(sub.clone());
        assert!(watcher.refresh().is_none(), "nothing to load, no change");

        // Creating the directory later is picked up like any other change.
        std::fs::create_dir(&sub).unwrap();
        write_tsv(&sub, "a.tsv", &[("わせだ", "早稲田")]);
        let dict = watcher.refresh().expect("new files detected").unwrap();
        assert!(has_word(&dict, "わせだ", "早稲田"));
    }
}
