//! Live captions: word-timed captions of one to a few words with the spoken word highlighted
//! (the captions of social video editors).
//!
//! `captions.createLive` cuts the sequence transcript (or an existing caption track) into short
//! word-timed captions on a caption track styled with a look from [`CAPTION_PRESETS`]. Each caption
//! holds as many words (up to `maxWords`) as fit on one line, measured in the track's font at the
//! sequence's frame size, so long words mean fewer words on screen and vertical video gets
//! shorter captions than landscape;
//! `captions.setWord` edits one word's text or timing. Looks and highlight settings are track
//! style (`captions.setStyle` with `preset`, `highlight`, `highlightColor`…).

use serde_json::{Value, json};

use filmcraft_edit::transcript as tx;
use filmcraft_project::{CAPTION_PRESETS, CaptionFormat, CaptionStyle, CaptionTrack, ClipId, TrackId};
use filmcraft_time::Tick;

use crate::captions::track_param;
use crate::commands::{CommandSpec, bad, has_seq, str_p, u64_p};
use crate::{EngineError, Result, Session};

/// Most words a live caption may hold.
pub const MAX_WORDS: usize = 8;

fn has_word_captions(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if s.active_sequence().is_some_and(|q| q.caption_tracks.iter().any(|t| t.captions.iter().any(|c| !c.words.is_empty()))) {
        Ok(())
    } else {
        Err("there are no word-timed captions".into())
    }
}

fn rules(p: &Value) -> tx::LiveCaptionRules {
    let d = tx::LiveCaptionRules::default();
    tx::LiveCaptionRules {
        max_words: u64_p(p, "maxWords").map(|n| n.clamp(1, MAX_WORDS as u64) as usize).unwrap_or(d.max_words),
        max_chars: u64_p(p, "maxChars").map(|n| n.clamp(1, 200) as usize).unwrap_or(d.max_chars),
        ..d
    }
}

/// The active sequence's frame size (1920×1080 when there is none).
pub fn frame_size(s: &Session) -> (u32, u32) {
    s.active_sequence().map(|q| (q.settings.width, q.settings.height)).unwrap_or((1920, 1080))
}

/// A look from [`CAPTION_PRESETS`], placed for the active sequence's frame.
pub fn preset_style(s: &Session, name: &str, cmd: &str) -> Result<CaptionStyle> {
    let (w, h) = frame_size(s);
    CaptionStyle::preset_for(name, w, h).ok_or_else(|| bad(cmd, format!("unknown preset `{name}` (one of {})", CAPTION_PRESETS.join(", "))))
}

fn preset_p(s: &Session, p: &Value, cmd: &str) -> Result<Option<CaptionStyle>> {
    str_p(p, "preset").map(|name| preset_style(s, name, cmd)).transpose()
}

/// Whether words fit on one line of a caption in `style` on the active sequence's frame.
fn fitter(s: &Session, style: CaptionStyle) -> impl Fn(&[&str]) -> bool + use<> {
    let (w, h) = frame_size(s);
    move |ws: &[&str]| filmcraft_captions::burn::line_fill(&style, w as usize, h as usize, ws) <= 1.0
}

/// Captions of `t` whose words no longer fit on one line (after an edit, a bigger size or a
/// different frame): Re-cut fixes them.
pub fn overflowing(t: &CaptionTrack, w: u32, h: u32) -> Vec<ClipId> {
    t.captions
        .iter()
        .filter(|c| c.words.len() > 1)
        .filter(|c| filmcraft_captions::burn::line_fill(&t.style, w as usize, h as usize, &c.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>()) > 1.0)
        .map(|c| c.id)
        .collect()
}

fn create_live(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "captions.createLive";
    let rules = rules(p);
    let rate = s.sequence_rate();
    let preset = preset_p(s, p, CMD)?;
    let words = crate::transcript::sequence_words(s);
    let source = match str_p(p, "source") {
        Some(v @ ("transcript" | "track")) => v,
        Some(o) => return Err(bad(CMD, format!("unknown source `{o}` (transcript or track)"))),
        None if !words.is_empty() => "transcript",
        None => "track",
    };
    if source == "transcript" {
        if words.is_empty() {
            return Err(bad(CMD, "the sequence has no transcript; transcribe it first (transcript.generate)"));
        }
        let style = match preset {
            Some(st) => st,
            None => preset_style(s, "Classic", CMD)?,
        };
        let blocks = tx::live_caption_blocks(&words, &rules, rate, &fitter(s, style.clone()));
        let n = blocks.len();
        let name = str_p(p, "name").unwrap_or("Live Captions").to_string();
        let tid = s.edit_sequence("Create Live Captions", |q, ctx, st| {
            let tid = TrackId(ctx.alloc());
            let mut t = CaptionTrack::new(tid, name, CaptionFormat::Subtitle);
            t.style = style;
            t.captions = tx::live_blocks_to_captions(&blocks, &words, ctx);
            q.caption_tracks.insert(0, t);
            st.caption_selection.clear();
            Ok(tid)
        })?;
        return Ok(json!({"track": tid.0, "captions": n}));
    }
    let tid = track_param(s, p).ok_or_else(|| bad(CMD, "no caption track to re-cut, and no transcript"))?;
    let style = match preset.clone() {
        Some(st) => st,
        None => s.active_sequence().and_then(|q| q.caption_track(tid)).map(|t| t.style.clone()).unwrap_or_default(),
    };
    let fits = fitter(s, style);
    let n = s.edit_sequence("Create Live Captions", |q, ctx, st| {
        let t = q.caption_track(tid).ok_or(EngineError::NoSequence)?;
        if t.locked {
            return Err(bad(CMD, "the caption track is locked"));
        }
        if t.captions.is_empty() {
            return Err(bad(CMD, "the caption track is empty"));
        }
        let caps = tx::rechunk_captions(&t.captions, &rules, rate, &fits, ctx);
        let n = caps.len();
        let t = q.caption_track_mut(tid).ok_or(EngineError::NoSequence)?;
        t.captions = caps;
        if let Some(style) = preset {
            t.style = style;
        }
        st.caption_selection.clear();
        Ok(n)
    })?;
    Ok(json!({"track": tid.0, "captions": n}))
}

fn set_word(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "captions.setWord";
    let id = p.get("caption").and_then(Value::as_u64).map(ClipId).ok_or_else(|| bad(CMD, "need `caption`"))?;
    let index = u64_p(p, "index").and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(CMD, "need a supported word `index`"))?;
    let text = str_p(p, "text").map(str::to_string);
    let tick = |k: &str| p.get(k).and_then(Value::as_i64).map(Tick);
    let (start, end) = (tick("start"), tick("end"));
    if text.is_none() && start.is_none() && end.is_none() {
        return Err(bad(CMD, "need `text`, `start` or `end`"));
    }
    s.edit_sequence("Edit Caption Word", |q, _, _| {
        let (tid, _) = q.find_caption(id).ok_or_else(|| bad(CMD, "no such caption"))?;
        let t = q.caption_track_mut(tid).ok_or(EngineError::NoSequence)?;
        if t.locked {
            return Err(bad(CMD, "the caption track is locked"));
        }
        let c = t.caption_mut(id).ok_or_else(|| bad(CMD, "no such caption"))?;
        let at = c.start;
        let w = c.words.get_mut(index).ok_or_else(|| bad(CMD, "no such word"))?;
        if let Some(s) = start {
            w.start = s - at;
        }
        if let Some(e) = end {
            w.end = e - at;
        }
        if w.end < w.start {
            w.end = w.start;
        }
        match text.as_deref().map(str::trim) {
            Some("") => {
                c.words.remove(index);
            }
            Some(txt) => {
                // one word stays one word
                w.text = txt.split_whitespace().collect::<Vec<_>>().join("\u{a0}");
            }
            None => {}
        }
        c.clamp_words();
        c.text = c.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        Ok(())
    })?;
    Ok(Value::Null)
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "captions.createLive",
            label: "Create Live Captions",
            menu: &["Sequence", "Captions"],
            shortcut: None,
            params: r#"{"maxWords":1..8=3,"maxChars":int=22 (a caption also never holds more words than fit on one line of the frame),"preset":"Classic|Pop|Box|Karaoke|Neon|Banner|Minimal|Subtitle"?,"source":"transcript|track"?,"track":id|"C1"?,"name":str?}"#,
            enabled: has_seq,
            run: create_live,
            journal: true,
        },
        CommandSpec {
            id: "captions.setWord",
            label: "Edit Caption Word",
            menu: &[],
            shortcut: None,
            params: r#"{"caption":id,"index":int,"text":str?,"start":ticks?,"end":ticks?}"#,
            enabled: has_word_captions,
            run: set_word,
            journal: true,
        },
    ]
}
