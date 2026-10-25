//! Sequence Settings ▸ Timebase conforms the edits to the new frame rate
//! (`filmcraft_edit::frame_rate::conform`): a sequence cut at 23.976 fps and switched to 30 fps
//! has every edit on a 30 fps frame, nothing slips, and no clip shows media it does not have.

use super::*;
use filmcraft_project::TrackItem;
use serde_json::json;

const OLD: FrameRate = FrameRate::FPS_23_976;
const NEW: FrameRate = FrameRate::FPS_30;

/// The demo project with a new 23.976 fps sequence holding its first footage clip at 0, cut at
/// 23.976 fps frames 37 and 81 into three butted clips.
fn three_clips() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let footage = s.active_sequence().unwrap().video_tracks[0].items[0].item;
    s.execute("file.newSequence", json!({"name": "Cut", "fps": 23.976})).unwrap();
    assert_eq!(s.sequence_rate(), OLD);
    s.execute("timeline.place", json!({"item": footage.0, "time": 0})).unwrap();
    for f in [37, 81] {
        s.execute("timeline.razor", json!({"time": OLD.tick_of(f).0})).unwrap();
    }
    assert_eq!(s.active_sequence().unwrap().video_tracks[0].items.len(), 3);
    s
}

fn items(s: &Session) -> Vec<TrackItem> {
    s.active_sequence().unwrap().all_tracks().flat_map(|t| t.items.iter().cloned()).collect()
}

fn on_grid(r: FrameRate, t: Tick) -> bool {
    r.snap(t) == t
}

#[test]
fn butted_clips_cut_at_23_976_land_on_30_fps_frames_without_gaps() {
    let mut s = three_clips();
    let before = items(&s);
    // 37 and 81 frames at 23.976 fall between 30 fps frames
    assert!(before.iter().any(|i| !on_grid(NEW, i.start) || !on_grid(NEW, i.end())));
    let r = s.execute("sequence.settings", json!({"fps": 30})).unwrap();
    assert!(r["conformedClips"].as_u64().unwrap() > 0);
    assert_eq!(s.sequence_rate(), NEW);
    let seq = s.active_sequence().unwrap();
    seq.check().unwrap();
    for t in seq.all_tracks() {
        for it in &t.items {
            assert!(on_grid(NEW, it.start) && on_grid(NEW, it.end()), "{} {:?}..{:?} is off the 30 fps grid", t.name, it.start, it.end());
        }
        // still butted, and the picture continues across each cut as it did before
        for w in t.items.windows(2) {
            assert_eq!(w[0].end(), w[1].start, "{}: gap or overlap at a cut", t.name);
            assert_eq!(w[0].source_out(), w[1].source_in, "{}: the picture slipped at a cut", t.name);
        }
    }
    // the picture at each new cut is the frame it was at the same time before
    let v1 = &seq.video_tracks[0].items;
    let old_v1: Vec<&TrackItem> = before.iter().filter(|b| v1.iter().any(|i| i.id == b.id)).collect();
    for (old, new) in old_v1.iter().zip(v1) {
        assert_eq!(old.moving_source_time_at(new.start), new.source_in);
    }
    // linked audio follows its video
    let audio: Vec<&TrackItem> = seq.audio_tracks.iter().flat_map(|t| t.items.iter()).collect();
    for a in audio.iter().filter(|a| a.link.is_some()) {
        let v = v1.iter().find(|v| v.link == a.link).unwrap();
        assert_eq!((a.start, a.end()), (v.start, v.end()));
    }
}

#[test]
fn undo_restores_the_old_timebase_and_positions() {
    let mut s = three_clips();
    let before = items(&s);
    s.execute("sequence.settings", json!({"fps": 30})).unwrap();
    assert_ne!(items(&s), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.sequence_rate(), OLD);
    assert_eq!(items(&s), before);
}

#[test]
fn an_unchanged_frame_rate_moves_nothing() {
    let mut s = three_clips();
    let before = items(&s);
    let r = s.execute("sequence.settings", json!({"fps": 23.976, "name": "Renamed"})).unwrap();
    assert_eq!(r["conformedClips"], 0);
    assert_eq!(items(&s), before);
}

#[test]
fn a_nest_never_runs_past_its_sequence() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let footage = s.active_sequence().unwrap().video_tracks[0].items[0].item;
    // a 23.976 fps nested sequence 122 frames long: at 30 fps that is 152.65 frames, which would
    // round up past its end
    let inner = ItemId(s.execute("file.newSequence", json!({"name": "Inner", "fps": 23.976})).unwrap()["sequence"].as_u64().unwrap());
    s.execute("timeline.place", json!({"item": footage.0, "time": 0})).unwrap();
    let c = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
    let len = OLD.tick_of(122);
    s.execute("timeline.trim", json!({"clip": c.id.0, "edge": "out", "mode": "regular", "delta": (len - c.duration).0})).unwrap();
    for t in s.active_sequence().unwrap().audio_tracks.iter().flat_map(|t| t.items.iter()).map(|i| (i.id, i.duration)).collect::<Vec<_>>() {
        if t.1 != len {
            s.execute("timeline.trim", json!({"clip": t.0.0, "edge": "out", "mode": "regular", "delta": (len - t.1).0})).unwrap();
        }
    }
    let inner_len = s.project.sequence(inner).unwrap().duration();
    assert_eq!(inner_len, len);
    assert!(NEW.snap_nearest(len) > len);

    s.execute("file.newSequence", json!({"name": "Main", "fps": 23.976})).unwrap();
    s.execute("timeline.place", json!({"item": inner.0, "time": 0})).unwrap();
    // a second nest butted after the first
    s.execute("timeline.place", json!({"item": inner.0, "time": len.0})).unwrap();
    s.execute("sequence.settings", json!({"fps": 30})).unwrap();

    let seq = s.active_sequence().unwrap();
    seq.check().unwrap();
    let nests: Vec<&TrackItem> = seq.all_tracks().flat_map(|t| t.items.iter()).filter(|i| i.item == inner).collect();
    assert!(nests.len() >= 2);
    for n in nests {
        assert!(n.source_in >= Tick::ZERO && n.source_out() <= inner_len, "{:?}..{:?} runs past the nest's {inner_len:?}", n.source_in, n.source_out());
        assert!(on_grid(NEW, n.start) && on_grid(NEW, n.end()));
    }
    // the first nest shrank to the frame before its end instead of growing past it
    let first = &seq.video_tracks[0].items[0];
    assert_eq!(first.end(), NEW.snap(len));
}
