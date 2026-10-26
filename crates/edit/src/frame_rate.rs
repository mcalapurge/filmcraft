//! Sequence Settings ▸ Timebase: conform a sequence's edits to a new frame rate.
//!
//! Edits are stored in exact ticks, so a sequence cut at 23.976 fps keeps its edit points after a
//! switch to 30 fps, and those points then fall between frames of the new timebase (a cut shows a
//! fraction of a frame, a nest runs a fraction past its end and flashes black). [`conform`] moves
//! every edit onto the new frame grid.
//!
//! Rounding rules:
//! - Every distinct clip edge time in the sequence (on every track, locked or not) is one *cut
//!   point*, snapped once: clips that butt stay butted, and linked audio that shares its video's
//!   edges moves with it.
//! - A cut point goes to the nearest frame of the new rate, unless that would make a clip touching
//!   it show media it does not have (a start before the media's first frame, an end past its last
//!   frame, for a nested sequence past [`Sequence::duration`]): then it goes to the frame on the
//!   other side, which shrinks the clip instead. If neither frame suits every clip at the point,
//!   clips ending there go to the frame before it and clips starting there to the frame after it,
//!   leaving a gap of under one frame rather than growing a clip past its media.
//! - `source_in` moves with the clip's start (or, for a reversed clip, its end) so every frame
//!   that stays on the timeline shows the same picture as before.
//! - A clip that would shrink to nothing (it was shorter than one new frame) becomes one frame
//!   long, growing right into free space, else left; with no room either way the conform fails and
//!   the sequence is left as it was.
//! - Transitions shift with the cut or clip edge they sit on, then both ends snap to the nearest
//!   frame, kept inside their clips and at least one frame long.
//! - Sequence markers, In/Out (and split In/Out) points, the work area and captions snap to the
//!   nearest frame; anything that had a duration keeps at least one frame.
//!
//! Keyframes and clip markers are not moved: they are relative to the clip's media, which this
//! keeps in place.

use std::collections::{BTreeMap, HashMap};

use filmcraft_project::{ClipId, ItemId, Sequence, TrackItem};
use filmcraft_time::{FrameRate, Tick};

use crate::{EditError, Result};

/// How far each edge of a clip may move outwards (timeline ticks) before it runs out of media.
#[derive(Clone, Copy)]
struct Reach {
    /// Earliest allowed start.
    start_lo: Tick,
    /// Latest allowed end.
    end_hi: Tick,
}

/// Where a cut point lands: clips ending there end at `end`, clips starting there start at
/// `start` (equal unless the point had to split; see the module docs).
#[derive(Clone, Copy)]
struct Landing {
    end: Tick,
    start: Tick,
}

/// Conform `seq` to its current frame rate (call after setting `seq.settings.frame_rate`). See the
/// module docs for the rounding. `media_start` / `media_duration` give the media extent of a
/// project item, as for [`crate::EditCtx`]. Returns how many clips moved; on error `seq` is left
/// untouched.
pub fn conform(seq: &mut Sequence, media_start: &dyn Fn(ItemId) -> Tick, media_duration: &dyn Fn(ItemId) -> Option<Tick>) -> Result<usize> {
    let rate = seq.settings.frame_rate;
    let fd = rate.frame_duration();
    if fd.0 <= 0 {
        return Ok(0);
    }
    let mut work = seq.clone();

    // 1. how far each clip may grow, and the bounds that puts on each cut point
    let mut reach: HashMap<ClipId, Reach> = HashMap::new();
    let mut points: BTreeMap<Tick, (Tick, Tick)> = BTreeMap::new();
    for item in work.all_tracks().flat_map(|t| t.items.iter()) {
        let r = clip_reach(item, media_start, media_duration);
        reach.insert(item.id, r);
        let start = points.entry(item.start).or_insert((Tick::MIN, Tick::MAX));
        start.0 = start.0.max(r.start_lo);
        let end = points.entry(item.end()).or_insert((Tick::MIN, Tick::MAX));
        end.1 = end.1.min(r.end_hi);
    }

    // 2. snap each cut point once
    let landings: BTreeMap<Tick, Landing> = points.into_iter().map(|(p, (lo, hi))| (p, land(rate, p, lo, hi))).collect();

    // 3. move the clips
    let mut moved_clips: HashMap<ClipId, (Tick, Tick, Tick, Tick)> = HashMap::new();
    let mut moved = 0;
    for track in work.all_tracks_mut() {
        let mut spans: Vec<(Tick, Tick)> = Vec::with_capacity(track.items.len());
        for item in &track.items {
            let start = landings.get(&item.start).map_or(item.start, |l| l.start);
            let end = landings.get(&item.end()).map_or(item.end(), |l| l.end);
            spans.push((start, end));
        }
        // a clip shorter than one new frame: one frame long, right into free space, else left
        for i in 0..spans.len() {
            let Some(&(start, end)) = spans.get(i) else { continue };
            if end > start {
                continue;
            }
            let Some(item) = track.items.get(i) else { continue };
            let r = reach.get(&item.id).copied().unwrap_or(Reach { start_lo: Tick::MIN, end_hi: Tick::MAX });
            let next_start = spans.get(i + 1).map_or(Tick::MAX, |s| s.0);
            let prev_end = i.checked_sub(1).and_then(|j| spans.get(j)).map_or(Tick::MIN, |s| s.1);
            let right = (start, start + fd);
            let left = (end - fd, end);
            let fits = |(s, e): (Tick, Tick)| s >= prev_end && e <= next_start && s >= r.start_lo && e <= r.end_hi;
            let span = if fits(right) {
                right
            } else if fits(left) {
                left
            } else {
                return Err(EditError::Other(format!(
                    "`{}` on {} is shorter than one frame at {rate} fps and has no room to grow; lengthen it first",
                    item.name, track.name
                )));
            };
            if let Some(s) = spans.get_mut(i) {
                *s = span;
            }
        }
        for (item, &(start, end)) in track.items.iter_mut().zip(&spans) {
            if (start, end) == (item.start, item.end()) {
                continue;
            }
            moved_clips.insert(item.id, (item.start, item.end(), start, end));
            if item.frame_hold.is_none() {
                // keep the picture in place: the media shown at a timeline time does not change
                let shift = if item.reverse { item.end() - end } else { start - item.start };
                item.source_in = (item.source_in + media_ticks(shift, item.speed)).max(Tick::ZERO);
            }
            item.start = start;
            item.duration = end - start;
            moved += 1;
        }
    }

    // 4. transitions follow the cut or edge they sit on
    let new_range = |c: Option<ClipId>, track: &filmcraft_project::Track| -> Option<(Tick, Tick, Tick, Tick)> {
        let c = c?;
        moved_clips.get(&c).copied().or_else(|| track.item(c).map(|i| (i.start, i.end(), i.start, i.end())))
    };
    for track in work.all_tracks_mut() {
        let mut fixed = Vec::with_capacity(track.transitions.len());
        for tr in &track.transitions {
            let from = new_range(tr.from, track);
            let to = new_range(tr.to, track);
            let delta = match (from, to) {
                (Some((_, old_end, _, new_end)), _) => new_end - old_end,
                (None, Some((old_start, _, new_start, _))) => new_start - old_start,
                (None, None) => Tick::ZERO,
            };
            let lo = [from, to].into_iter().flatten().map(|r| r.2).min().unwrap_or(Tick::MIN);
            let hi = [from, to].into_iter().flatten().map(|r| r.3).max().unwrap_or(Tick::MAX);
            let mut s = rate.snap_nearest(tr.start + delta).max(lo);
            let mut e = rate.snap_nearest(tr.end() + delta).min(hi);
            if e - s < fd {
                e = (s + fd).min(hi);
                s = (e - fd).max(lo);
            }
            fixed.push(if e > s { (s, e - s) } else { (tr.start, tr.duration) });
        }
        for (tr, (start, duration)) in track.transitions.iter_mut().zip(fixed) {
            tr.start = start;
            tr.duration = duration;
        }
    }

    // 5. markers, In/Out, work area, captions
    for m in &mut work.markers {
        let (start, duration) = snap_span(rate, m.start, m.duration);
        m.start = start;
        m.duration = duration;
    }
    match (work.mark_in, work.mark_out) {
        (Some(i), Some(o)) => {
            let (i2, d) = snap_span(rate, i, o - i);
            work.mark_in = Some(i2);
            work.mark_out = Some(i2 + d);
        }
        (i, o) => {
            work.mark_in = i.map(|t| rate.snap_nearest(t));
            work.mark_out = o.map(|t| rate.snap_nearest(t));
        }
    }
    for t in [&mut work.split.video_in, &mut work.split.video_out, &mut work.split.audio_in, &mut work.split.audio_out].into_iter().flatten() {
        *t = rate.snap_nearest(*t);
    }
    if let Some(w) = &mut work.work_area {
        let (start, duration) = snap_span(rate, w.start, w.duration);
        w.start = start;
        w.duration = duration;
    }
    for track in &mut work.caption_tracks {
        let mut spans: Vec<(Tick, Tick)> = track.captions.iter().map(|c| (rate.snap_nearest(c.start), rate.snap_nearest(c.end()))).collect();
        for i in 0..spans.len() {
            let Some(&(start, end)) = spans.get(i) else { continue };
            if end > start {
                continue;
            }
            let next_start = spans.get(i + 1).map_or(Tick::MAX, |s| s.0);
            let prev_end = i.checked_sub(1).and_then(|j| spans.get(j)).map_or(Tick::MIN, |s| s.1);
            let span = if start + fd <= next_start {
                (start, start + fd)
            } else if end - fd >= prev_end {
                (end - fd, end)
            } else {
                return Err(EditError::Other(format!("a caption on {} is shorter than one frame at {rate} fps and has no room to grow", track.name)));
            };
            if let Some(s) = spans.get_mut(i) {
                *s = span;
            }
        }
        for (c, (start, end)) in track.captions.iter_mut().zip(spans) {
            let shift = start - c.start;
            c.start = start;
            c.duration = end - start;
            c.shift_words(shift);
        }
    }

    work.check().map_err(EditError::Other)?;
    *seq = work;
    Ok(moved)
}

/// Where cut point `p` lands given the earliest (`lo`) and latest (`hi`) times the clips touching
/// it allow.
fn land(rate: FrameRate, p: Tick, lo: Tick, hi: Tick) -> Landing {
    let floor = rate.snap(p);
    let ceil = if floor == p { p } else { rate.tick_of(rate.frame_at(p).saturating_add(1)) };
    let near = rate.snap_nearest(p);
    let other = if near == floor { ceil } else { floor };
    match [near, other].into_iter().find(|c| (lo..=hi).contains(c)) {
        Some(c) => Landing { end: c, start: c },
        None => {
            let start = p.max(lo);
            let start_floor = rate.snap(start);
            let start = if start_floor == start { start } else { rate.tick_of(rate.frame_at(start).saturating_add(1)) };
            Landing { end: rate.snap(p.min(hi)), start }
        }
    }
}

/// The outermost start and end `item` may take before it shows media outside its item's extent.
fn clip_reach(item: &TrackItem, media_start: &dyn Fn(ItemId) -> Tick, media_duration: &dyn Fn(ItemId) -> Option<Tick>) -> Reach {
    if item.frame_hold.is_some() {
        return Reach { start_lo: Tick::MIN, end_hi: Tick::MAX };
    }
    // media before source_in and after the source out point
    let head = Some((item.source_in - media_start(item.item)).max(Tick::ZERO));
    let tail = media_duration(item.item).map(|m| (m - item.source_out()).max(Tick::ZERO));
    // a reversed clip plays its media backwards: its start uses the media after the out point
    let (before, after) = if item.reverse { (tail, head) } else { (head, tail) };
    Reach {
        start_lo: before.map_or(Tick::MIN, |m| item.start - timeline_ticks(m, item.speed)),
        end_hi: after.map_or(Tick::MAX, |m| item.end() + timeline_ticks(m, item.speed)),
    }
}

/// Media ticks a timeline offset `d` spans at `speed` (signed, rounded as the clip model does).
fn media_ticks(d: Tick, speed: f64) -> Tick {
    let speed = speed.abs();
    let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
    Tick(((d.0 as f64) * speed).round() as i64)
}

/// Timeline ticks `m` media ticks last at `speed`, rounded down (so a clip never reaches past
/// its media).
fn timeline_ticks(m: Tick, speed: f64) -> Tick {
    let speed = speed.abs();
    let speed = if speed.is_finite() && speed > 1e-9 { speed } else { 1.0 };
    Tick(((m.0 as f64) / speed).floor() as i64).min(Tick::MAX)
}

/// Snap a `(start, duration)` span to the nearest frames; one with a duration keeps at least one
/// frame, an instant (duration 0) stays one.
fn snap_span(rate: FrameRate, start: Tick, duration: Tick) -> (Tick, Tick) {
    let s = rate.snap_nearest(start);
    if duration.0 <= 0 {
        return (s, Tick::ZERO);
    }
    let e = rate.snap_nearest(start + duration);
    (s, (e - s).max(rate.frame_duration()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{Label, Project, SequenceSettings};

    #[test]
    fn conform_keeps_live_words_at_their_original_sequence_times() {
        let mut project = Project::new("captions");
        let id = project.new_sequence("s", SequenceSettings { frame_rate: FrameRate::FPS_30, ..Default::default() }, 1, 1, None);
        let mut seq = project.sequence(id).unwrap().clone();
        let start = FrameRate::FPS_23_976.tick_of(1);
        let word_start = Tick::from_seconds_f64(0.02);
        let mut track = filmcraft_project::CaptionTrack::new(filmcraft_project::TrackId(900), "Live".into(), filmcraft_project::CaptionFormat::Subtitle);
        track.captions.push(filmcraft_project::Caption {
            id: ClipId(901),
            start,
            duration: Tick::from_seconds_f64(1.0),
            text: "hello".into(),
            speaker: None,
            cue_id: None,
            settings: String::new(),
            words: vec![filmcraft_project::CaptionWord { text: "hello".into(), start: word_start, end: word_start + Tick::from_seconds_f64(0.3) }],
        });
        seq.caption_tracks.push(track);
        conform(&mut seq, &|_| Tick::ZERO, &|_| None).unwrap();
        let caption = &seq.caption_tracks[0].captions[0];
        assert_eq!(caption.start + caption.words[0].start, start + word_start, "the new caption edge must not shift spoken words");
    }

    fn clip(id: u64, start: Tick, duration: Tick, source_in: Tick) -> TrackItem {
        TrackItem {
            id: ClipId(id),
            item: ItemId(1),
            name: format!("c{id}"),
            label: Label::Iris,
            start,
            duration,
            source_in,
            speed: 1.0,
            reverse: false,
            enabled: true,
            link: None,
            group: None,
            effects: vec![],
            markers: vec![],
            gain_db: 0.0,
            frame_hold: None,
            scale_to_frame: false,
            essential: None,
            multicam: None,
            time_interpolation: Default::default(),
            hold_filters: false,
            field_options: None,
            source_channels: Vec::new(),
            audio_stream: 0,
            graphic: None,
        }
    }

    fn seq_with(items: Vec<TrackItem>, rate: FrameRate) -> Sequence {
        let mut p = Project::new("t");
        let id = p.new_sequence("s", SequenceSettings { frame_rate: rate, ..Default::default() }, 1, 1, None);
        let mut s = p.sequence(id).unwrap().clone();
        s.video_tracks[0].items = items;
        s
    }

    #[test]
    fn a_clip_ending_at_its_media_end_shrinks_instead_of_growing() {
        let old = FrameRate::FPS_23_976;
        let fd = old.frame_duration();
        // ends at its last media frame, and that end rounds up at 30 fps
        let media = Tick(fd.0 * 122);
        assert!(FrameRate::FPS_30.snap_nearest(media) > media);
        let mut s = seq_with(vec![clip(1, Tick::ZERO, media, Tick::ZERO)], old);
        s.settings.frame_rate = FrameRate::FPS_30;
        conform(&mut s, &|_| Tick::ZERO, &|_| Some(media)).unwrap();
        let it = &s.video_tracks[0].items[0];
        assert!(it.source_out() <= media);
        assert_eq!(it.end(), FrameRate::FPS_30.snap(media));
    }

    #[test]
    fn a_reversed_clip_keeps_its_picture() {
        let old = FrameRate::FPS_23_976;
        let fd = old.frame_duration();
        let mut c = clip(1, Tick(fd.0 * 7), Tick(fd.0 * 50), Tick(fd.0 * 100));
        c.reverse = true;
        let before = c.clone();
        let mut s = seq_with(vec![c], old);
        s.settings.frame_rate = FrameRate::FPS_30;
        conform(&mut s, &|_| Tick::ZERO, &|_| None).unwrap();
        let it = &s.video_tracks[0].items[0];
        let t = it.start.max(before.start);
        assert_eq!(it.source_time_at(t), before.source_time_at(t));
    }
}
