//! F6/F7/F8 fixed-transform tests: whole-composition conversion to
//! hiragana / full-width katakana / half-width katakana.

use super::*;

fn selected_text(engine: &InputMethodEngine) -> String {
    engine
        .state()
        .candidates()
        .unwrap()
        .selected_text()
        .unwrap()
        .to_string()
}

#[test]
fn f7_converts_composition_to_katakana() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    let result = engine.process_key(&press_key(Keysym::F7));

    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert_eq!(selected_text(&engine), "アイ");
    assert_eq!(engine.preedit().unwrap().text(), "アイ");
}

#[test]
fn f8_converts_composition_to_half_katakana() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press('g'));
    engine.process_key(&press('a'));
    let result = engine.process_key(&press_key(Keysym::F8));

    assert!(result.consumed);
    // Voiced kana expands to base + dakuten in half-width.
    assert_eq!(selected_text(&engine), "ｶﾞ");
}

#[test]
fn f6_returns_to_hiragana_after_f7() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::F7));
    assert_eq!(selected_text(&engine), "アイ");

    // A different F-key just moves the selection within the same list.
    engine.process_key(&press_key(Keysym::F8));
    assert_eq!(selected_text(&engine), "ｱｲ");
    engine.process_key(&press_key(Keysym::F6));
    assert_eq!(selected_text(&engine), "あい");
}

#[test]
fn enter_commits_the_selected_transform() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    engine.learning = Some(LearningCache::new(LearningConfig::default()));

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::F7));
    let result = engine.process_key(&press_key(Keysym::RETURN));

    let committed = result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(committed.as_deref(), Some("アイ"));
    assert!(matches!(engine.state(), InputState::Empty));
    // An F-key transform is an explicit choice: recorded like a kana commit.
    assert_eq!(
        engine.learning.as_ref().unwrap().lookup("あい")[0].0,
        "アイ"
    );
}

#[test]
fn escape_returns_to_editable_composition() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::F7));
    engine.process_key(&press_key(Keysym::ESCAPE));

    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "あい");

    // The buffer is still editable.
    engine.process_key(&press('u'));
    assert_eq!(engine.input_buf.reading(), "あいう");
}

#[test]
fn fkey_settles_the_romaji_tail_like_space() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press('k'));
    engine.process_key(&press_key(Keysym::F7));

    // The unresolved `k` rides along as-is, like Space conversion.
    assert_eq!(selected_text(&engine), "アイk");
}

#[test]
fn fkeys_from_conversion_state_switch_to_the_transform() {
    // Space first (normal conversion), then F7: the conversion is rebuilt
    // as the fixed-transform list.
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    engine.process_key(&press_key(Keysym::F7));
    assert_eq!(selected_text(&engine), "アイ");
}

#[test]
fn fkeys_pass_through_in_empty_state() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    let result = engine.process_key(&press_key(Keysym::F7));
    assert!(
        !result.consumed,
        "F7 with no composition belongs to the app"
    );
}

#[test]
fn f10_converts_typed_keys_to_half_alnum() {
    // わたし was typed as watasi: F10 gives back the keystrokes, not a
    // romanization of the kana.
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    for ch in "watasi".chars() {
        engine.process_key(&press(ch));
    }
    assert_eq!(engine.preedit().unwrap().text(), "わたし");

    engine.process_key(&press_key(Keysym::F10));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert_eq!(selected_text(&engine), "watasi");
}

#[test]
fn f9_converts_typed_keys_to_full_alnum() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    for ch in "spam".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&press_key(Keysym::F9));
    assert_eq!(selected_text(&engine), "ｓｐａｍ");
}

#[test]
fn f10_repeat_cycles_case() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    for ch in "spam".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(selected_text(&engine), "spam");
    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(selected_text(&engine), "SPAM");
    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(selected_text(&engine), "Spam");
    // The cycle wraps.
    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(selected_text(&engine), "spam");
}

#[test]
fn f9_after_f10_keeps_case_and_changes_width() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    for ch in "spam".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&press_key(Keysym::F10));
    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(selected_text(&engine), "SPAM");
    engine.process_key(&press_key(Keysym::F9));
    assert_eq!(selected_text(&engine), "ＳＰＡＭ");
}

#[test]
fn f10_covers_passthrough_and_symbol_keystrokes() {
    // よろしく、 typed as yorosiku, — F10 reproduces the keys including
    // the comma that fired the punctuation rule.
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    for ch in "yorosiku,".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(selected_text(&engine), "yorosiku,");
}

#[test]
fn f10_commit_records_learning_under_the_reading() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    engine.learning = Some(LearningCache::new(LearningConfig::default()));

    for ch in "spam".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&press_key(Keysym::F10));
    let result = engine.process_key(&press_key(Keysym::RETURN));

    let committed = result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(committed.as_deref(), Some("spam"));
    // The settled reading of the keys s,p,a,m is sぱm (a lone `s` never
    // becomes kana); the learning entry lands under that key.
    assert_eq!(
        engine.learning.as_ref().unwrap().lookup("sぱm")[0].0,
        "spam"
    );
}

#[test]
fn fkeys_are_inert_in_emoji_mode() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;

    engine.process_key(&press(':'));
    assert_eq!(engine.mode.current(), InputMode::Emoji);
    let result = engine.process_key(&press_key(Keysym::F7));
    assert!(
        result.consumed,
        "consumed as a no-op, not leaked to the app"
    );
    assert!(matches!(engine.state(), InputState::Composing { .. }));
}
