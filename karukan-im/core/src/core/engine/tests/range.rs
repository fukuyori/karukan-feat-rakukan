//! Range selection tests: Shift+Right/Left select the reading head,
//! Space converts only the selection, Enter commits it partially and the
//! remainder keeps composing.

use super::*;
use crate::core::preedit::AttributeType;

fn shift_key(keysym: Keysym) -> KeyEvent {
    KeyEvent::new(
        keysym,
        KeyModifiers {
            shift_key: true,
            ..Default::default()
        },
        true,
    )
}

fn type_waseda_daigaku(engine: &mut InputMethodEngine) {
    for ch in "wasedadaigaku".chars() {
        engine.process_key(&press(ch));
    }
    assert_eq!(engine.input_buf.reading(), "わせだだいがく");
}

#[test]
fn shift_right_selects_the_reading_head() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    type_waseda_daigaku(&mut engine);

    for _ in 0..3 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }

    assert_eq!(engine.range_select, Some(3));
    let preedit = engine.preedit().unwrap();
    assert_eq!(preedit.text(), "わせだだいがく");
    assert_eq!(preedit.caret(), 3);
    let attrs = preedit.attributes();
    assert_eq!(attrs[0].attr_type, AttributeType::UnderlineDouble);
    assert_eq!((attrs[0].start, attrs[0].end), (0, 3));
    assert_eq!(attrs[1].attr_type, AttributeType::Underline);
    assert_eq!((attrs[1].start, attrs[1].end), (3, 7));
}

#[test]
fn selection_is_clamped_to_the_reading() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    for ch in "ai".chars() {
        engine.process_key(&press(ch));
    }
    for _ in 0..5 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }
    assert_eq!(engine.range_select, Some(2));
}

#[test]
fn shift_left_shrinks_and_exits_at_zero() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    type_waseda_daigaku(&mut engine);

    engine.process_key(&shift_key(Keysym::RIGHT));
    engine.process_key(&shift_key(Keysym::RIGHT));
    assert_eq!(engine.range_select, Some(2));

    engine.process_key(&shift_key(Keysym::LEFT));
    assert_eq!(engine.range_select, Some(1));
    engine.process_key(&shift_key(Keysym::LEFT));
    assert_eq!(engine.range_select, None);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
}

#[test]
fn escape_cancels_the_range_and_keeps_the_composition() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    type_waseda_daigaku(&mut engine);

    engine.process_key(&shift_key(Keysym::RIGHT));
    engine.process_key(&press_key(Keysym::ESCAPE));

    assert_eq!(engine.range_select, None);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.input_buf.reading(), "わせだだいがく");
}

#[test]
fn typing_drops_the_range_and_edits_normally() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    for ch in "waseda".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&shift_key(Keysym::RIGHT));
    assert!(engine.range_select.is_some());

    engine.process_key(&press('d'));
    engine.process_key(&press('a'));
    assert_eq!(engine.range_select, None);
    assert_eq!(engine.input_buf.reading(), "わせだだ");
}

#[test]
fn space_converts_only_the_selection() {
    let mut engine = engine_with_learned("わせだ", "早稲田");
    type_waseda_daigaku(&mut engine);

    for _ in 0..3 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }
    engine.process_key(&press_key(Keysym::SPACE));

    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert_eq!(engine.conversion_span, Some(3));
    let texts: Vec<String> = engine
        .state()
        .candidates()
        .unwrap()
        .candidates()
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(texts.contains(&"早稲田".to_string()), "got {texts:?}");

    // The preedit shows the selected candidate plus the unconverted
    // remainder.
    let learned_selected = {
        let mut e = engine;
        while selected_text(&e) != "早稲田" {
            e.process_key(&press_key(Keysym::SPACE));
        }
        let preedit = e.preedit().unwrap().clone();
        assert_eq!(preedit.text(), "早稲田だいがく");
        assert_eq!(preedit.caret(), 3);
        e
    };
    drop(learned_selected);
}

#[test]
fn predictive_candidates_stay_out_of_a_range_conversion() {
    // A predictive candidate's reading extends past the selection, so
    // committing it would consume more than the selected range.
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    engine.dicts.user = Some(dict_from_json(
        r#"[
            {"reading":"わせだ","candidates":[{"surface":"早稲田","score":100.0}]},
            {"reading":"わせだだいがく","candidates":[{"surface":"早稲田大学","score":100.0}]}
        ]"#,
    ));
    type_waseda_daigaku(&mut engine);

    for _ in 0..3 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }
    engine.process_key(&press_key(Keysym::SPACE));

    let texts: Vec<String> = engine
        .state()
        .candidates()
        .unwrap()
        .candidates()
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(texts.contains(&"早稲田".to_string()), "got {texts:?}");
    assert!(
        !texts.contains(&"早稲田大学".to_string()),
        "predictive completion must stay out, got {texts:?}"
    );
}

#[test]
fn enter_commits_the_head_and_keeps_composing_the_rest() {
    let mut engine = engine_with_learned("わせだ", "早稲田");
    type_waseda_daigaku(&mut engine);

    for _ in 0..3 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }
    engine.process_key(&press_key(Keysym::SPACE));
    while selected_text(&engine) != "早稲田" {
        engine.process_key(&press_key(Keysym::SPACE));
    }
    let result = engine.process_key(&press_key(Keysym::RETURN));

    let committed = result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(committed.as_deref(), Some("早稲田"));
    // The remainder keeps composing, raw keystrokes intact (F10 still works).
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.input_buf.reading(), "だいがく");
    assert_eq!(engine.input_buf.raw_text(), "daigaku");
    assert_eq!(engine.conversion_span, None);
    // The same result also carries the refreshed composing preedit, so
    // frontends apply commit-then-preedit in one event.
    assert!(
        result
            .actions
            .iter()
            .any(|a| matches!(a, EngineAction::UpdatePreedit(_))),
        "partial commit must carry the refreshed preedit"
    );
}

#[test]
fn committing_a_whole_reading_range_ends_the_composition() {
    let mut engine = engine_with_learned("あい", "藍");
    for ch in "ai".chars() {
        engine.process_key(&press(ch));
    }
    engine.process_key(&shift_key(Keysym::RIGHT));
    engine.process_key(&shift_key(Keysym::RIGHT));
    engine.process_key(&press_key(Keysym::SPACE));
    while selected_text(&engine) != "藍" {
        engine.process_key(&press_key(Keysym::SPACE));
    }
    let result = engine.process_key(&press_key(Keysym::RETURN));

    assert!(matches!(engine.state(), InputState::Empty));
    let committed = result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(committed.as_deref(), Some("藍"));
}

#[test]
fn escape_from_a_range_conversion_restores_the_full_reading() {
    let mut engine = engine_with_learned("わせだ", "早稲田");
    type_waseda_daigaku(&mut engine);

    for _ in 0..3 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }
    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_key(Keysym::ESCAPE));

    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.conversion_span, None);
    assert_eq!(engine.input_buf.reading(), "わせだだいがく");
}

#[test]
fn focus_out_during_range_conversion_commits_everything() {
    let mut engine = engine_with_learned("わせだ", "早稲田");
    type_waseda_daigaku(&mut engine);

    for _ in 0..3 {
        engine.process_key(&shift_key(Keysym::RIGHT));
    }
    engine.process_key(&press_key(Keysym::SPACE));
    while selected_text(&engine) != "早稲田" {
        engine.process_key(&press_key(Keysym::SPACE));
    }

    let text = engine.commit();
    assert_eq!(text, "早稲田だいがく");
    assert!(matches!(engine.state(), InputState::Empty));
}

fn selected_text(engine: &InputMethodEngine) -> String {
    engine
        .state()
        .candidates()
        .unwrap()
        .selected_text()
        .unwrap()
        .to_string()
}
