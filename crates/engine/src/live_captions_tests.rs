use std::sync::Arc;

use serde_json::json;

use crate::Session;
use filmcraft_project::{CaptionStyle, Transcript, Word, WordHighlight};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::Tick;

/// A new empty sequence of `w`×`h` holding one ordinary caption of `text` (no word timings).
fn sequence_with_caption(w: u32, h: u32, text: &str) -> Session {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "S", "width": w, "height": h})).unwrap();
    s.execute("captions.add", json!({"text": text, "seconds": 0.0, "durationSeconds": 8.0})).unwrap();
    s
}

const LONG_TALK: &str = "honestly the internationalisation of our infrastructure was extraordinarily complicated but we got it done in a week";

#[test]
fn live_caption_commands_are_disabled_without_what_they_need_and_undo_and_redo() {
    // no sequence: neither command applies
    let s = Session::default();
    assert!(!s.is_enabled("captions.createLive"), "no sequence");
    assert!(!s.is_enabled("captions.setWord"), "no sequence");

    // ordinary captions only: Create Live Captions applies, Edit Caption Word does not
    let mut s = sequence_with_caption(1920, 1080, "one two three four");
    let first = |s: &Session| s.active_sequence().unwrap().caption_tracks[0].captions[0].id.0;
    assert!(s.is_enabled("captions.createLive"));
    assert!(!s.is_enabled("captions.setWord"), "no word-timed captions yet");
    assert!(s.execute("captions.setWord", json!({"caption": first(&s), "index": 0, "text": "x"})).is_err());

    // Create Live Captions: one undo step, and redo brings it back
    s.execute("captions.createLive", json!({"source": "track", "maxWords": 2})).unwrap();
    let live = texts(&s);
    assert_eq!(live, ["one two", "three four"]);
    assert!(s.is_enabled("captions.setWord"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(texts(&s), ["one two three four"]);
    assert!(!s.is_enabled("captions.setWord"));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(texts(&s), live);

    // Edit Caption Word: undo and redo
    let id = first(&s);
    s.execute("captions.setWord", json!({"caption": id, "index": 0, "text": "ONE"})).unwrap();
    assert_eq!(texts(&s)[0], "ONE two");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(texts(&s)[0], "one two");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(texts(&s)[0], "ONE two");

    // a locked track refuses both, and nothing changes
    s.execute("captions.setTrack", json!({"track": "C1", "locked": true})).unwrap();
    let before = s.active_sequence().unwrap().caption_tracks[0].clone();
    assert!(s.execute("captions.createLive", json!({"source": "track", "maxWords": 1})).is_err());
    assert!(s.execute("captions.setWord", json!({"caption": id, "index": 0, "text": "nope"})).is_err());
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0], before);
}

#[test]
fn vertical_video_gets_fewer_words_per_caption_and_every_caption_fits() {
    let mut caption_counts = Vec::new();
    for (w, h) in [(1080, 1920), (1920, 1080)] {
        let mut s = sequence_with_caption(w, h, LONG_TALK);
        s.execute("captions.createLive", json!({"source": "track", "maxWords": 4, "maxChars": 200, "preset": "Classic"})).unwrap();
        let q = s.active_sequence().unwrap();
        q.check().unwrap();
        let t = &q.caption_tracks[0];
        let words: usize = t.captions.iter().map(|c| c.words.len()).sum();
        assert_eq!(words, LONG_TALK.split_whitespace().count(), "no word lost on {w}x{h}");
        assert!(crate::live_captions::overflowing(t, w, h).is_empty(), "every caption fits one line of {w}x{h}");
        assert!(t.captions.iter().all(|c| (1..=4).contains(&c.words.len())));
        if h > w {
            // the long words stand alone on vertical video
            let alone: Vec<&str> = t.captions.iter().filter(|c| c.words.len() == 1).map(|c| c.text.as_str()).collect();
            assert!(alone.contains(&"internationalisation") && alone.contains(&"extraordinarily"), "{alone:?}");
            assert!((t.style.margin - 0.28).abs() < 1e-6, "placed clear of a phone app's buttons");
        } else {
            assert!((t.style.margin - 0.12).abs() < 1e-6, "placed near the bottom of landscape video");
        }
        caption_counts.push(t.captions.len());
    }
    assert!(caption_counts[0] > caption_counts[1], "vertical {} captions vs landscape {}", caption_counts[0], caption_counts[1]);
}

#[test]
fn captions_that_no_longer_fit_are_listed_and_recut_fixes_them() {
    let mut s = sequence_with_caption(1080, 1920, "make it big and bold for everyone");
    s.execute("captions.createLive", json!({"source": "track", "maxWords": 4, "preset": "Pop"})).unwrap();
    let r = s.execute("captions.list", json!({})).unwrap();
    assert_eq!(r["tracks"][0]["overflowing"], json!([]), "{r}");
    // a much bigger size: the multi-word captions no longer fit
    s.execute("captions.setStyle", json!({"track": "C1", "size": 190})).unwrap();
    let r = s.execute("captions.list", json!({})).unwrap();
    assert!(!r["tracks"][0]["overflowing"].as_array().unwrap().is_empty(), "{r}");
    // Re-cut at the new size: everything fits again, the size is kept
    s.execute("captions.createLive", json!({"source": "track", "maxWords": 4})).unwrap();
    let r = s.execute("captions.list", json!({})).unwrap();
    assert_eq!(r["tracks"][0]["overflowing"], json!([]), "{r}");
    assert_eq!(r["tracks"][0]["style"]["size"], 190.0);
}

/// The demo project with a fake transcriber whose transcript fits the first A1 clip's media:
/// "Hello there my friend, how are you today?" then, after a pause, "Fine."
fn session() -> Session {
    let (s, _) = session_item();
    s
}

/// [`session`] and the media item the transcript belongs to.
fn session_item() -> (Session, filmcraft_project::ItemId) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let a = &q.audio_tracks[0].items[0];
    let (item, sin) = (a.item, a.source_in);
    let sec = |x: f64| sin + Tick::from_seconds_f64(x);
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    let words = [
        ("Hello", 0.2, 0.5),
        ("there", 0.5, 0.8),
        ("my", 0.8, 0.95),
        ("friend,", 0.95, 1.4),
        ("how", 1.5, 1.7),
        ("are", 1.7, 1.85),
        ("you", 1.85, 2.0),
        ("today?", 2.0, 2.5),
        ("Fine.", 4.0, 4.4),
    ];
    for (text, a, b) in words {
        t.words.push(Word::new(text, sec(a), sec(b)));
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    (s, item)
}

/// Transcribe the one clip the fake transcript belongs to.
fn transcribe(s: &mut Session) {
    let item = s.active_sequence().unwrap().audio_tracks[0].items[0].item;
    s.execute("transcript.generate", json!({"items": [item.0]})).unwrap();
}

fn texts(s: &Session) -> Vec<String> {
    s.active_sequence().unwrap().caption_tracks[0].captions.iter().map(|c| c.text.clone()).collect()
}

#[test]
fn create_live_captions_from_the_transcript() {
    let mut s = session();
    // no transcript yet: a clear error, nothing changed
    let e = s.execute("captions.createLive", json!({"source": "transcript"})).unwrap_err();
    assert!(e.to_string().contains("transcribe"), "{e}");
    transcribe(&mut s);
    let r = s.execute("captions.createLive", json!({"maxWords": 3, "preset": "Pop"})).unwrap();
    assert_eq!(r["captions"], 4, "{r}");
    assert_eq!(texts(&s), ["Hello there my", "friend, how are", "you today?", "Fine."]);
    let q = s.active_sequence().unwrap();
    q.check().unwrap();
    let tr = &q.caption_tracks[0];
    assert_eq!(tr.name, "Live Captions");
    // the look, placed for the demo's frame
    assert_eq!(tr.style, CaptionStyle::preset_for("Pop", q.settings.width, q.settings.height).unwrap());
    for c in &tr.captions {
        assert!((1..=3).contains(&c.words.len()), "{:?}", c.text);
        assert_eq!(c.text, c.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "));
        assert!(c.words.iter().all(|w| w.start >= Tick::ZERO && w.end <= c.duration && w.start <= w.end));
    }
    // undoable as one step
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().caption_tracks.is_empty());
}

#[test]
fn words_per_caption_and_phrase_breaks() {
    let mut s = session();
    transcribe(&mut s);
    s.execute("captions.createLive", json!({"maxWords": 4})).unwrap();
    // the comma and the question mark end phrases; the pause before "Fine." too
    assert_eq!(texts(&s), ["Hello there my friend,", "how are you today?", "Fine."]);
    s.execute("captions.createLive", json!({"maxWords": 1})).unwrap();
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.len(), 2, "a new track each time from the transcript");
    assert!(q.caption_tracks[0].captions.iter().all(|c| c.words.len() == 1));
    assert_eq!(q.caption_tracks[0].captions.len(), 9);
    q.check().unwrap();
}

#[test]
fn recut_a_track_keeps_word_timings() {
    let mut s = session();
    transcribe(&mut s);
    s.execute("captions.createLive", json!({"maxWords": 4})).unwrap();
    let before: Vec<(String, Tick)> =
        s.active_sequence().unwrap().caption_tracks[0].captions.iter().flat_map(|c| c.words.iter().map(move |w| (w.text.clone(), c.start + w.start))).collect();
    let r = s.execute("captions.createLive", json!({"source": "track", "track": "C1", "maxWords": 2})).unwrap();
    assert_eq!(r["captions"], 5, "{r}");
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.len(), 1, "re-cut in place");
    let after: Vec<(String, Tick)> =
        q.caption_tracks[0].captions.iter().flat_map(|c| c.words.iter().map(move |w| (w.text.clone(), c.start + w.start))).collect();
    assert_eq!(before, after);
    q.check().unwrap();
}

#[test]
fn recut_an_imported_track_without_word_timings() {
    let mut s = session();
    s.execute("captions.add", json!({"text": "one two three four five six", "seconds": 0.0, "durationSeconds": 3.0})).unwrap();
    s.execute("captions.createLive", json!({"source": "track", "maxWords": 2, "preset": "Box"})).unwrap();
    assert_eq!(texts(&s), ["one two", "three four", "five six"]);
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks[0].style.highlight, WordHighlight::Box);
    q.check().unwrap();
}

#[test]
fn edit_words_style_and_list() {
    let mut s = session();
    transcribe(&mut s);
    s.execute("captions.createLive", json!({"maxWords": 3})).unwrap();
    let c = s.active_sequence().unwrap().caption_tracks[0].captions[0].clone();
    s.execute("captions.setWord", json!({"caption": c.id.0, "index": 1, "text": "THERE"})).unwrap();
    let c2 = s.active_sequence().unwrap().caption_tracks[0].captions[0].clone();
    assert_eq!(c2.text, "Hello THERE my");
    assert_eq!(c2.words[1].start, c.words[1].start, "a text edit keeps the timing");
    // retime a word in sequence time
    let at = c.start + c.words[2].start + Tick(1000);
    s.execute("captions.setWord", json!({"caption": c.id.0, "index": 2, "start": at.0})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions[0].words[2].start, at - c.start);
    // an empty text removes the word
    s.execute("captions.setWord", json!({"caption": c.id.0, "index": 0, "text": " "})).unwrap();
    assert_eq!(texts(&s)[0], "THERE my");
    assert!(s.execute("captions.setWord", json!({"caption": c.id.0, "index": 99, "text": "x"})).is_err());
    // a whole-text edit re-times the words
    s.execute("captions.setText", json!({"caption": c.id.0, "text": "brand new words here"})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions[0].words.len(), 4);

    // style: a preset, then a tweak
    s.execute("captions.setStyle", json!({"track": "C1", "preset": "Karaoke", "highlightScale": 1.3, "highlightColor": "#ff0000"})).unwrap();
    let st = s.active_sequence().unwrap().caption_tracks[0].style.clone();
    assert_eq!(st.highlight, WordHighlight::Reveal);
    assert_eq!(st.highlight_color, [255, 0, 0, 255]);
    assert!((st.highlight_scale - 1.3).abs() < 1e-6);
    assert!(s.execute("captions.setStyle", json!({"track": "C1", "preset": "Nope"})).is_err());
    assert!(s.execute("captions.setStyle", json!({"track": "C1", "highlight": "sparkle"})).is_err());
    s.execute("captions.setStyle", json!({"track": "C1", "highlight": "box", "highlightScale": 99})).unwrap();
    assert!((s.active_sequence().unwrap().caption_tracks[0].style.highlight_scale - 2.0).abs() < 1e-6, "clamped");

    let r = s.execute("captions.list", json!({})).unwrap();
    let track = &r["tracks"][0];
    assert_eq!(track["style"]["highlight"], "box", "{track}");
    let w = &track["captions"][0]["words"][0];
    assert_eq!(w["text"], "brand", "{w}");
    assert!(w["start"].as_i64().unwrap() >= track["captions"][0]["start"].as_i64().unwrap());
}

#[test]
fn hostile_params_do_not_panic() {
    let mut s = session();
    transcribe(&mut s);
    for p in [json!({"maxWords": 0}), json!({"maxWords": u64::MAX}), json!({"maxChars": 0}), json!({"source": "nowhere"}), json!({"preset": 5})] {
        let _ = s.execute("captions.createLive", p);
    }
    let c = s.active_sequence().unwrap().caption_tracks[0].captions[0].id;
    for p in [
        json!({"caption": c.0, "index": 0, "start": i64::MAX}),
        json!({"caption": c.0, "index": 0, "end": i64::MIN}),
        json!({"caption": c.0, "index": u64::MAX, "text": "x"}),
        json!({"caption": 0, "index": 0, "text": "x"}),
        json!({"caption": c.0, "index": 0}),
    ] {
        let _ = s.execute("captions.setWord", p);
    }
    s.active_sequence().unwrap().check().unwrap();
}

#[test]
fn caption_presets_save_update_apply_reset_and_survive_restart() {
    let dir = std::env::temp_dir().join(format!("filmcraft-caption-preset-restart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("preferences.json");
    let mut s = sequence_with_caption(1080, 1920, "Hello");
    s.prefs_path = Some(path.clone());
    s.execute("captions.setStyle", json!({"track": "C1", "preset": "Pop", "font": "Test Font", "size": 88, "margin": 0.2, "highlightScale": 1.4})).unwrap();
    let expected = s.active_sequence().unwrap().caption_tracks[0].style.clone();
    s.execute("captions.savePreset", json!({"track": "C1", "name": "My look"})).unwrap();
    assert!(s.execute("captions.savePreset", json!({"name": "my LOOK"})).is_err());
    s.execute("captions.setStyle", json!({"track": "C1", "reset": true})).unwrap();
    s.prefs = crate::autosave::Preferences::load(&path);
    s.execute("captions.setStyle", json!({"track": "C1", "preset": "My look"})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].style, expected);
    s.execute("edit.undo", json!({})).unwrap();
    assert_ne!(s.active_sequence().unwrap().caption_tracks[0].style, expected);
    s.execute("edit.redo", json!({})).unwrap();
    s.execute("captions.setStyle", json!({"track": "C1", "size": 99})).unwrap();
    s.execute("captions.savePreset", json!({"name": "MY LOOK", "overwrite": true})).unwrap();
    assert_eq!(s.prefs.caption_presets.len(), 1);
    assert_eq!(s.prefs.caption_presets[0].style.size, 99.0);
    s.execute("captions.savePreset", json!({"name": "Pop", "overwrite": true})).unwrap();
    assert_eq!(crate::live_captions::preset_style(&s, "pop", "test").unwrap().size, 99.0);
    s.execute("captions.deletePreset", json!({"name": "Pop"})).unwrap();
    assert_eq!(crate::live_captions::preset_style(&s, "Pop", "test").unwrap(), CaptionStyle::preset_for("Pop", 1080, 1920).unwrap());
    s.execute("captions.deletePreset", json!({"name": "My look"})).unwrap();
    assert!(crate::autosave::Preferences::load(&path).caption_presets.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn caption_preset_write_failure_keeps_library_and_track_unchanged() {
    let dir = std::env::temp_dir().join(format!("filmcraft-caption-preset-failure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = sequence_with_caption(1080, 1920, "Hello");
    // A directory cannot be replaced with the preferences file.
    s.prefs_path = Some(dir.clone());
    let before = s.prefs.clone();
    let project = s.project.clone();
    let error = s.execute("captions.savePreset", json!({"name": "Lost look"})).unwrap_err();
    assert!(error.to_string().contains("Could not save caption presets"));
    assert_eq!(s.prefs, before);
    assert_eq!(s.project, project);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_caption_presets_do_not_discard_preferences_or_other_looks() {
    let valid = json!({"name": "Good", "style": CaptionStyle::default()});
    let prefs = crate::autosave::Preferences::from_value(json!({
        "captionPresets": [null, {"name": "broken", "style": {"size": -1}}, valid, {"name": "bad", "style": {"margin": 1e100}}],
        "autoSave": {"enabled": false}
    }));
    assert!(!prefs.auto_save.enabled);
    assert_eq!(prefs.caption_presets.len(), 1);
    assert_eq!(prefs.caption_presets[0].name, "Good");
}

#[test]
fn caption_preset_commands_reject_hostile_parameters() {
    let mut s = sequence_with_caption(1080, 1920, "Hello");
    for params in [json!({}), json!({"name": " "}), json!({"name": "bad\nname"}), json!({"name": "x".repeat(81)}), json!({"name": "look", "track": u64::MAX})] {
        assert!(s.execute("captions.savePreset", params).is_err());
    }
    assert!(s.execute("captions.savePreset", json!({"name": "Classic"})).is_err());
    assert!(s.execute("captions.deletePreset", json!({"name": "Classic"})).is_err());
    assert!(s.execute("captions.deletePreset", json!({})).is_err());
    let list = s.execute("captions.listPresets", json!({})).unwrap();
    assert_eq!(list["presets"].as_array().unwrap().len(), 8);
}

#[test]
fn mutated_caption_preset_preferences_are_bounded_and_never_panic() {
    let source = serde_json::to_vec(&json!({"captionPresets": [{"name": "Look", "style": CaptionStyle::default()}]})).unwrap();
    for i in 0..source.len() {
        for bytes in [source[..i].to_vec(), {
            let mut changed = source.clone();
            changed[i] ^= 0xff;
            changed
        }] {
            assert!(
                std::panic::catch_unwind(|| {
                    if let Ok(value) = serde_json::from_slice(&bytes) {
                        let prefs = crate::autosave::Preferences::from_value(value);
                        assert!(prefs.caption_presets.len() <= 128);
                    }
                })
                .is_ok()
            );
        }
    }
    let entries: Vec<_> = (0..140).map(|i| json!({"name": format!("Look {i}"), "style": CaptionStyle::default()})).collect();
    let prefs = crate::autosave::Preferences::from_value(json!({"captionPresets": entries}));
    assert_eq!(prefs.caption_presets.len(), 128);
    let mut s = sequence_with_caption(1080, 1920, "Hello");
    s.prefs = prefs;
    assert!(s.execute("captions.savePreset", json!({"name": "One too many"})).is_err());
}
