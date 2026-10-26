//! Caption tracks.
//!
//! Premiere keeps captions in their own track area above the video tracks. A [`CaptionTrack`] has a
//! format (Subtitle, CEA-608, CEA-708, Teletext), a track style (font, size, colour, background box,
//! position) and a list of [`Caption`] blocks with exact in/out times in sequence ticks. Captions on
//! one track never overlap (like track items); [`CaptionTrack::check`] verifies it.
//!
//! Caption text is kept as written (lines separated by `\n`, inline `<i>`/`<b>`/`<u>` tags and
//! WebVTT cue settings preserved) so files round-trip; renderers strip markup for display.
//!
//! **Word-timed captions** (social "live" captions, one to four words at a time with the spoken
//! word highlighted) carry [`Caption::words`]: each word's bounds as offsets from the caption's
//! start, so moving a caption moves its words. The track style's [`WordHighlight`] says how the
//! word being spoken is drawn; [`CaptionStyle::preset`] gives ready-made looks.

use filmcraft_time::{Tick, TimeRange};
use serde::{Deserialize, Serialize};

use crate::{ClipId, TrackId};

/// Caption stream format of a track (Premiere's "Caption Format" choices).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionFormat {
    /// Open/sidecar subtitles (SRT, WebVTT).
    #[default]
    Subtitle,
    /// CEA-608 (line 21) closed captions.
    Cea608,
    /// CEA-708 (DTVCC) closed captions.
    Cea708,
    /// EBU Teletext subtitles.
    Teletext,
}

impl CaptionFormat {
    pub const ALL: [CaptionFormat; 4] = [CaptionFormat::Subtitle, CaptionFormat::Cea608, CaptionFormat::Cea708, CaptionFormat::Teletext];
    pub fn label(self) -> &'static str {
        match self {
            CaptionFormat::Subtitle => "Subtitle",
            CaptionFormat::Cea608 => "CEA-608",
            CaptionFormat::Cea708 => "CEA-708",
            CaptionFormat::Teletext => "Teletext",
        }
    }
    pub fn from_name(s: &str) -> Option<CaptionFormat> {
        let n = s.to_ascii_lowercase().replace(['-', ' ', '_'], "");
        Self::ALL.iter().copied().find(|f| f.label().to_ascii_lowercase().replace(['-', ' '], "") == n)
    }
    /// Characters per line limit used when wrapping (608 is fixed at 32 columns).
    pub fn max_columns(self) -> Option<usize> {
        match self {
            CaptionFormat::Cea608 => Some(32),
            CaptionFormat::Teletext => Some(40),
            CaptionFormat::Cea708 => Some(42),
            CaptionFormat::Subtitle => None,
        }
    }
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Vertical placement of the caption block in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionAnchor {
    Top,
    Middle,
    #[default]
    Bottom,
}

/// How the word being spoken is marked in a word-timed caption.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WordHighlight {
    /// Every word drawn alike.
    #[default]
    Off,
    /// The spoken word in [`CaptionStyle::highlight_color`].
    Color,
    /// A rounded box in [`CaptionStyle::highlight_box_color`] behind the spoken word.
    Box,
    /// Words appear as they are spoken; the line keeps its final layout so nothing shifts.
    Reveal,
}

impl WordHighlight {
    pub const ALL: [WordHighlight; 4] = [WordHighlight::Off, WordHighlight::Color, WordHighlight::Box, WordHighlight::Reveal];
    pub fn label(self) -> &'static str {
        match self {
            WordHighlight::Off => "Off",
            WordHighlight::Color => "Color",
            WordHighlight::Box => "Box",
            WordHighlight::Reveal => "Reveal",
        }
    }
    pub fn from_name(s: &str) -> Option<WordHighlight> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Some(WordHighlight::Off),
            "color" | "colour" => Some(WordHighlight::Color),
            "box" => Some(WordHighlight::Box),
            "reveal" | "karaoke" => Some(WordHighlight::Reveal),
            _ => None,
        }
    }
}

/// Track-level caption style (Premiere's caption track style / Essential Graphics text settings).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionStyle {
    /// Font family name (bundled, craft-fonts or system); unknown or empty names fall back to Inter.
    pub font: String,
    /// Font size in pixels for a 1080-line frame (scaled with the frame height, or with the
    /// shorter side when [`CaptionStyle::short_side`] is on).
    pub size: f32,
    /// Text colour, sRGB + alpha.
    pub color: [u8; 4],
    /// Draw a background box behind each line.
    pub background: bool,
    /// Background box colour, sRGB + alpha.
    pub background_color: [u8; 4],
    pub align: CaptionAlign,
    pub anchor: CaptionAnchor,
    /// Distance from the anchored frame edge, as a fraction of the frame height.
    pub margin: f32,
    /// Line height as a multiple of the font size.
    pub line_spacing: f32,
    /// Text outline width in pixels at 1080 lines (0 = none).
    pub outline: f32,
    pub outline_color: [u8; 4],
    /// Font style within the family (`"SemiBold"`, `"Bold"`…); the nearest weight is used.
    pub font_style: String,
    /// Draw the text in capitals.
    pub uppercase: bool,
    /// How the spoken word of a word-timed caption is marked.
    pub highlight: WordHighlight,
    /// Text colour of the spoken word (every mode but `Off`).
    pub highlight_color: [u8; 4],
    /// Box behind the spoken word ([`WordHighlight::Box`]).
    pub highlight_box_color: [u8; 4],
    /// Size of the spoken word relative to the others (1 = same size; above 1 it "pops").
    pub highlight_scale: f32,
    /// Size and outline are for 1080 pixels of the frame's *shorter* side rather than its height,
    /// so a look reads the same on vertical (1080×1920), square and landscape video. Off (the
    /// subtitle convention, and what older projects have), they follow the frame height, which
    /// makes text on vertical video 1.78× larger.
    pub short_side: bool,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            font: "Inter".into(),
            size: 54.0,
            color: [255, 255, 255, 255],
            background: true,
            background_color: [0, 0, 0, 191],
            align: CaptionAlign::Center,
            anchor: CaptionAnchor::Bottom,
            margin: 0.08,
            line_spacing: 1.25,
            outline: 0.0,
            outline_color: [0, 0, 0, 255],
            font_style: "SemiBold".into(),
            uppercase: false,
            highlight: WordHighlight::Off,
            highlight_color: [255, 214, 10, 255],
            highlight_box_color: [124, 77, 255, 255],
            highlight_scale: 1.0,
            short_side: false,
        }
    }
}

/// The ready-made caption looks of the caption editor, by name ([`CaptionStyle::preset`]).
pub const CAPTION_PRESETS: [&str; 8] = ["Classic", "Pop", "Box", "Karaoke", "Neon", "Banner", "Minimal", "Subtitle"];

impl CaptionStyle {
    /// A ready-made look from [`CAPTION_PRESETS`] (case-insensitive); `None` for an unknown name.
    /// The social looks sit in the lower middle of the frame, clear of a phone app's buttons.
    pub fn preset(name: &str) -> Option<CaptionStyle> {
        let base = CaptionStyle {
            font_style: "Bold".into(),
            size: 84.0,
            background: false,
            outline: 6.0,
            outline_color: [0, 0, 0, 255],
            anchor: CaptionAnchor::Bottom,
            margin: 0.28,
            line_spacing: 1.15,
            uppercase: true,
            short_side: true,
            ..CaptionStyle::default()
        };
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "classic" => CaptionStyle { highlight: WordHighlight::Color, highlight_color: [255, 214, 10, 255], ..base },
            "pop" => CaptionStyle { highlight: WordHighlight::Color, highlight_color: [57, 255, 20, 255], highlight_scale: 1.18, ..base },
            "box" => CaptionStyle {
                highlight: WordHighlight::Box,
                highlight_color: [255, 255, 255, 255],
                highlight_box_color: [124, 77, 255, 255],
                outline: 0.0,
                ..base
            },
            "karaoke" => CaptionStyle { highlight: WordHighlight::Reveal, highlight_color: [255, 255, 255, 255], ..base },
            "neon" => {
                CaptionStyle { outline: 8.0, outline_color: [255, 0, 170, 255], highlight: WordHighlight::Color, highlight_color: [0, 240, 255, 255], ..base }
            }
            "banner" => CaptionStyle {
                highlight: WordHighlight::Box,
                highlight_color: [0, 0, 0, 255],
                highlight_box_color: [255, 214, 10, 255],
                outline: 0.0,
                background: true,
                background_color: [0, 0, 0, 160],
                ..base
            },
            "minimal" => CaptionStyle {
                size: 64.0,
                uppercase: false,
                font_style: "SemiBold".into(),
                outline: 3.0,
                outline_color: [0, 0, 0, 200],
                color: [255, 255, 255, 150],
                highlight: WordHighlight::Color,
                highlight_color: [255, 255, 255, 255],
                ..base
            },
            "subtitle" => CaptionStyle::default(),
            _ => return None,
        })
    }

    /// [`CaptionStyle::preset`] placed for a `w`×`h` frame: on vertical video the social looks
    /// sit 28% up, clear of a phone app's buttons and description; on landscape and square video
    /// that would be mid-frame, so they sit near the bottom (12%) instead.
    pub fn preset_for(name: &str, w: u32, h: u32) -> Option<CaptionStyle> {
        let mut st = CaptionStyle::preset(name)?;
        if st.short_side && h <= w {
            st.margin = 0.12;
        }
        Some(st)
    }
}

/// A word of a word-timed caption. Its bounds are offsets from the caption's start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub text: String,
    pub start: Tick,
    pub end: Tick,
}

/// One caption block on a caption track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub id: ClipId,
    /// Timeline start (sequence ticks).
    pub start: Tick,
    pub duration: Tick,
    /// Caption text; lines separated by `\n`.
    pub text: String,
    /// Speaker name (WebVTT `<v Name>` voice span).
    #[serde(default)]
    pub speaker: Option<String>,
    /// WebVTT cue identifier, kept for round trips.
    #[serde(default)]
    pub cue_id: Option<String>,
    /// WebVTT cue settings (`line:90% align:start`…), kept verbatim.
    #[serde(default)]
    pub settings: String,
    /// Word timings of a word-timed caption, in order (empty for an ordinary caption).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<CaptionWord>,
}

impl Caption {
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }
    /// The caption's text with markup removed and entities decoded (what is drawn).
    pub fn plain_lines(&self) -> Vec<String> {
        plain_text(&self.text).lines().map(str::to_string).collect()
    }

    /// The word being spoken at `offset` from the caption's start: the last word that has
    /// started, so a pause between words keeps the previous one lit. `None` before the first word
    /// and for an ordinary caption.
    pub fn word_at(&self, offset: Tick) -> Option<usize> {
        self.words.partition_point(|w| w.start <= offset).checked_sub(1)
    }

    /// The caption's start moved by `delta` (a trim or a new in point): shift the word offsets so
    /// the words stay where they are in the sequence, dropping words now outside the caption.
    pub fn shift_words(&mut self, delta: Tick) {
        if self.words.is_empty() {
            return;
        }
        for w in &mut self.words {
            w.start = Tick(w.start.0.saturating_sub(delta.0));
            w.end = Tick(w.end.0.saturating_sub(delta.0));
        }
        self.clamp_words();
    }

    /// Keep the words inside the caption, in order and not overlapping; a word wholly outside it
    /// is dropped. Dropping a word rewrites the plain text from the words left (empty when none
    /// is), so sidecars match what is drawn; otherwise the text, with its markup, is kept.
    pub fn clamp_words(&mut self) {
        if self.words.is_empty() {
            return;
        }
        let dur = self.duration.max(Tick::ZERO);
        let before = self.words.len();
        self.words.retain(|w| w.end > Tick::ZERO && w.start < dur && !w.text.trim().is_empty());
        let mut prev = Tick::ZERO;
        for w in &mut self.words {
            w.start = w.start.clamp(prev, dur);
            w.end = w.end.clamp(w.start, dur);
            prev = w.end;
        }
        if self.words.len() != before {
            self.text = self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        }
    }

    /// The text of a word-timed caption changed: time the words of the new text. The same number
    /// of words keeps every timing (a spelling fix); otherwise the span the old words covered is
    /// shared out in proportion to word length. An ordinary caption is left alone.
    pub fn resync_words(&mut self) {
        if self.words.is_empty() {
            return;
        }
        let texts: Vec<String> = plain_text(&self.text).split_whitespace().map(str::to_string).collect();
        if texts.len() == self.words.len() {
            for (w, t) in self.words.iter_mut().zip(texts) {
                w.text = t;
            }
            return;
        }
        let a = self.words.first().map(|w| w.start).unwrap_or(Tick::ZERO);
        let b = self.words.last().map(|w| w.end).unwrap_or(self.duration).max(a);
        self.words = spread_words(&texts, a, b);
    }

    /// Word-time an ordinary caption: its words spread over its whole duration in proportion to
    /// their length (imported or typed captions have no word timings).
    pub fn time_words_evenly(&mut self) {
        let texts: Vec<String> = plain_text(&self.text).split_whitespace().map(str::to_string).collect();
        self.words = spread_words(&texts, Tick::ZERO, self.duration.max(Tick::ZERO));
    }
}

/// Share `[a, b)` out among `texts` in proportion to their length (plus one for the space).
pub fn spread_words(texts: &[String], a: Tick, b: Tick) -> Vec<CaptionWord> {
    let weights: Vec<i128> = texts.iter().map(|t| t.chars().count() as i128 + 1).collect();
    let total: i128 = weights.iter().sum();
    if total <= 0 {
        return Vec::new();
    }
    let span = (b.0 as i128 - a.0 as i128).max(0);
    let at = |acc: i128| Tick((a.0 as i128 + span * acc / total).clamp(i64::MIN as i128, i64::MAX as i128) as i64);
    let mut acc = 0i128;
    let mut out = Vec::with_capacity(texts.len());
    for (t, w) in texts.iter().zip(&weights) {
        let start = at(acc);
        acc += w;
        out.push(CaptionWord { text: t.clone(), start, end: at(acc) });
    }
    out
}

/// A caption track (shown in the caption area above the video tracks).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub id: TrackId,
    pub name: String,
    pub format: CaptionFormat,
    /// BCP-47 language tag (`en`, `fr-CA`…).
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub style: CaptionStyle,
    /// Track output (eye): shown in the Program monitor and burned in on export.
    pub enabled: bool,
    pub locked: bool,
    /// Follows inserts / extracts on other tracks.
    #[serde(default = "yes")]
    pub sync_lock: bool,
    /// Captions sorted by start, non-overlapping.
    pub captions: Vec<Caption>,
    /// WebVTT header blocks (STYLE / REGION / NOTE) kept for round trips.
    #[serde(default)]
    pub vtt_blocks: Vec<String>,
}

fn yes() -> bool {
    true
}

impl CaptionTrack {
    pub fn new(id: TrackId, name: String, format: CaptionFormat) -> Self {
        Self {
            id,
            name,
            format,
            language: "en".into(),
            style: CaptionStyle::default(),
            enabled: true,
            locked: false,
            sync_lock: true,
            captions: Vec::new(),
            vtt_blocks: Vec::new(),
        }
    }
    pub fn end(&self) -> Tick {
        self.captions.iter().map(Caption::end).max().unwrap_or(Tick::ZERO)
    }
    /// Caption covering `t`.
    pub fn caption_at(&self, t: Tick) -> Option<&Caption> {
        let idx = self.captions.partition_point(|c| c.start <= t);
        idx.checked_sub(1).map(|i| &self.captions[i]).filter(|c| t < c.end())
    }
    pub fn caption(&self, id: ClipId) -> Option<&Caption> {
        self.captions.iter().find(|c| c.id == id)
    }
    pub fn caption_mut(&mut self, id: ClipId) -> Option<&mut Caption> {
        self.captions.iter_mut().find(|c| c.id == id)
    }
    pub fn sort(&mut self) {
        self.captions.sort_by_key(|c| c.start);
    }
    /// Invariant check: captions sorted, positive duration, no overlaps; word timings in order
    /// and inside their caption.
    pub fn check(&self) -> Result<(), String> {
        self.check_bounds()?;
        for c in &self.captions {
            if c.duration.0 <= 0 {
                return Err(format!("{}: caption {:?} has non-positive duration", self.name, c.id));
            }
            let mut previous_end = Tick::ZERO;
            for word in &c.words {
                if word.start < previous_end || word.end < word.start || word.end > c.duration {
                    return Err(format!("{}: caption {:?} has unordered or out-of-range word timings", self.name, c.id));
                }
                previous_end = word.end;
            }
        }
        for w in self.captions.windows(2) {
            if w[0].end() > w[1].start {
                return Err(format!("{}: captions {:?} and {:?} overlap", self.name, w[0].id, w[1].id));
            }
        }
        Ok(())
    }
    /// Only the time bounds that keep caption arithmetic from overflowing (see `Track::check_bounds`).
    pub fn check_bounds(&self) -> Result<(), String> {
        for c in &self.captions {
            if !crate::bounded_time_range(c.start, c.duration) {
                return Err(format!("{}: caption {:?} is outside supported time bounds", self.name, c.id));
            }
            // (order and range are editor invariants, repaired on load: see `check`)
            if c.words.iter().any(|w| !crate::bounded_time_range(w.start, Tick::ZERO) || !crate::bounded_time_range(w.end, Tick::ZERO)) {
                return Err(format!("{}: caption {:?} has word timings outside supported time bounds", self.name, c.id));
            }
        }
        Ok(())
    }
}

/// Strip `<…>` tags and decode the common HTML/WebVTT entities.
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // A tag runs to the next '>'.  Scan forward once; preserving an unterminated
                // tag as literal text must not rescan every later opening bracket.
                let mut literal = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '>' {
                        closed = true;
                        break;
                    }
                    literal.push(next);
                }
                if !closed {
                    out.push('<');
                    out.push_str(&literal);
                }
            }
            '{' if chars.peek() == Some(&'\\') => {
                // SSA-style override blocks some SRT files carry: {\an8}
                let mut literal = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    literal.push(next);
                }
                if !closed {
                    out.push('{');
                    out.push_str(&literal);
                }
            }
            '&' => {
                let rest: String = chars.clone().take(8).collect();
                let ents = [
                    ("amp;", '&'),
                    ("lt;", '<'),
                    ("gt;", '>'),
                    ("nbsp;", '\u{a0}'),
                    ("quot;", '"'),
                    ("apos;", '\''),
                    ("lrm;", '\u{200e}'),
                    ("rlm;", '\u{200f}'),
                ];
                if let Some((name, ch)) = ents.iter().find(|(n, _)| rest.starts_with(n)) {
                    for _ in 0..name.len() {
                        chars.next();
                    }
                    out.push(*ch);
                } else {
                    out.push('&');
                }
            }
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain() {
        assert_eq!(plain_text("<i>Hello</i> &amp; <b>bye</b>"), "Hello & bye");
        assert_eq!(plain_text("{\\an8}Top"), "Top");
        assert_eq!(plain_text("a < b"), "a < b");
        assert_eq!(plain_text("<v Bob>Hi</v>"), "Hi");
    }

    #[test]
    fn malformed_caption_markup_is_preserved_without_repeated_suffix_scans() {
        let angle = "<".repeat(10_000);
        let ssa = format!("{{\\{}", "{".repeat(10_000));
        assert_eq!(plain_text(&angle), angle);
        assert_eq!(plain_text(&ssa), ssa);
    }

    #[test]
    fn caption_at() {
        let mut t = CaptionTrack::new(TrackId(1), "Subtitle".into(), CaptionFormat::Subtitle);
        for (i, s) in [0i64, 100, 300].iter().enumerate() {
            t.captions.push(Caption {
                id: ClipId(i as u64),
                start: Tick(*s),
                duration: Tick(50),
                text: format!("c{i}"),
                speaker: None,
                cue_id: None,
                settings: String::new(),
                words: Vec::new(),
            });
        }
        assert_eq!(t.caption_at(Tick(120)).unwrap().text, "c1");
        assert!(t.caption_at(Tick(160)).is_none());
        assert!(t.check().is_ok());
        assert_eq!(CaptionFormat::from_name("cea608"), Some(CaptionFormat::Cea608));
        assert_eq!(CaptionFormat::from_name("CEA-708"), Some(CaptionFormat::Cea708));
    }

    #[test]
    fn corrupt_caption_times_do_not_overflow_the_overlap_check() {
        let mut track = CaptionTrack::new(TrackId(1), "Subtitles".into(), CaptionFormat::Subtitle);
        for (id, start) in [(1, i64::MAX), (2, 10)] {
            track.captions.push(Caption {
                id: ClipId(id),
                start: Tick(start),
                duration: Tick(50),
                text: String::new(),
                speaker: None,
                cue_id: None,
                settings: String::new(),
                words: Vec::new(),
            });
        }
        let result = std::panic::catch_unwind(|| track.check());
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }

    fn timed(text: &str, dur: i64, words: &[(&str, i64, i64)]) -> Caption {
        Caption {
            id: ClipId(1),
            start: Tick(1000),
            duration: Tick(dur),
            text: text.into(),
            speaker: None,
            cue_id: None,
            settings: String::new(),
            words: words.iter().map(|(t, a, b)| CaptionWord { text: t.to_string(), start: Tick(*a), end: Tick(*b) }).collect(),
        }
    }

    #[test]
    fn spoken_word_holds_through_pauses() {
        let c = timed("a b c", 100, &[("a", 0, 10), ("b", 20, 30), ("c", 50, 90)]);
        assert_eq!(c.word_at(Tick(-1)), None);
        assert_eq!(c.word_at(Tick(0)), Some(0));
        assert_eq!(c.word_at(Tick(15)), Some(0), "the pause after a keeps it lit");
        assert_eq!(c.word_at(Tick(20)), Some(1));
        assert_eq!(c.word_at(Tick(99)), Some(2));
        assert_eq!(timed("x", 10, &[]).word_at(Tick(5)), None);
    }

    #[test]
    fn words_follow_text_edits_and_trims() {
        let mut c = timed("one two", 100, &[("one", 10, 40), ("two", 50, 90)]);
        c.text = "One, two!".into();
        c.resync_words();
        assert_eq!(c.words.iter().map(|w| (w.text.as_str(), w.start.0)).collect::<Vec<_>>(), [("One,", 10), ("two!", 50)]);
        c.text = "a b c".into();
        c.resync_words();
        assert_eq!(c.words.len(), 3);
        assert_eq!((c.words[0].start, c.words[2].end), (Tick(10), Tick(90)), "spread over the old span");
        assert!(c.words.windows(2).all(|w| w[0].end <= w[1].start));
        // a trim of 30 at the head: words keep their sequence time, the first is cut to fit
        c.start += Tick(30);
        c.duration -= Tick(30);
        c.shift_words(Tick(30));
        assert!(c.words.iter().all(|w| w.start >= Tick::ZERO && w.end <= c.duration));
        // an ordinary caption is not made word-timed by a text edit
        let mut plain = timed("hi", 10, &[]);
        plain.text = "hello there".into();
        plain.resync_words();
        assert!(plain.words.is_empty());
        plain.time_words_evenly();
        assert_eq!(plain.words.len(), 2);
        assert_eq!(plain.words[1].end, Tick(10));
    }

    #[test]
    fn hostile_word_times_are_clamped() {
        let mut c = timed("a b c", 100, &[("a", i64::MIN, i64::MAX), ("b", 500, 400), ("c", -50, -10)]);
        c.clamp_words();
        assert!(c.words.iter().all(|w| w.start >= Tick::ZERO && w.start <= w.end && w.end <= c.duration));
        c.shift_words(Tick(i64::MIN));
        c.shift_words(Tick(i64::MAX));
        assert!(spread_words(&["x".into()], Tick(i64::MAX), Tick(i64::MIN)).iter().all(|w| w.start == w.end));
        assert!(spread_words(&[], Tick(0), Tick(10)).is_empty());
    }

    /// Review fix: clamping words that all still fit keeps the caption's own text (its markup and
    /// line breaks go to sidecar exports); only dropping a word rewrites it.
    #[test]
    fn clamping_words_that_still_fit_keeps_the_written_text() {
        let mut c = timed("<i>one</i>\ntwo", 100, &[("one", 0, 40), ("two", 50, 90)]);
        c.clamp_words();
        assert_eq!(c.text, "<i>one</i>\ntwo");
        c.duration = Tick(45);
        c.clamp_words();
        assert_eq!(c.text, "one", "a dropped word leaves the text");
    }

    /// Review fix: word order and range are editor invariants (`check`), not load-time bounds, so a
    /// project an earlier build wrote with a slightly late word still opens.
    #[test]
    fn word_order_is_an_editor_invariant_not_a_load_bound() {
        let mut track = CaptionTrack::new(TrackId(1), "Live".into(), CaptionFormat::Subtitle);
        track.captions.push(timed("x y", 100, &[("x", 0, 60), ("y", 50, 101)]));
        assert!(track.check_bounds().is_ok());
        assert!(track.check().is_err());
        track.captions[0].words[0].start = Tick(i64::MIN);
        assert!(track.check_bounds().is_err(), "an offset past the time bounds is still refused");
    }

    #[test]
    fn caption_tracks_reject_corrupt_word_offsets_without_panicking() {
        for words in [vec![("x", i64::MIN, 5)], vec![("x", 0, i64::MAX)], vec![("x", 5, 1)], vec![("x", 0, 20), ("y", 10, 30)]] {
            let mut track = CaptionTrack::new(TrackId(1), "Live".into(), CaptionFormat::Subtitle);
            track.captions.push(timed("x", 100, &words));
            let result = std::panic::catch_unwind(|| track.check());
            assert!(result.is_ok());
            assert!(result.unwrap().is_err());
        }
    }

    #[test]
    fn presets_and_back_compat() {
        for name in CAPTION_PRESETS {
            assert!(CaptionStyle::preset(name).is_some(), "{name}");
            assert!(CaptionStyle::preset(&name.to_uppercase()).is_some(), "{name}");
        }
        assert!(CaptionStyle::preset("nope").is_none());
        assert_eq!(CaptionStyle::preset("Subtitle"), Some(CaptionStyle::default()));
        // projects saved before word timings and highlight load unchanged
        let old: CaptionStyle = serde_json::from_str(r#"{"font":"Inter","size":40.0}"#).unwrap();
        assert_eq!(old.highlight, WordHighlight::Off);
        assert_eq!(old.font_style, "SemiBold");
        let c: Caption = serde_json::from_str(r#"{"id":1,"start":0,"duration":10,"text":"hi"}"#).unwrap();
        assert!(c.words.is_empty());
        assert!(!serde_json::to_string(&c).unwrap().contains("words"), "ordinary captions save as before");
        for (name, h) in [("karaoke", WordHighlight::Reveal), ("colour", WordHighlight::Color), ("none", WordHighlight::Off), ("BOX", WordHighlight::Box)] {
            assert_eq!(WordHighlight::from_name(name), Some(h));
        }
    }

    #[test]
    fn looks_are_placed_for_the_frame_shape() {
        let v = CaptionStyle::preset_for("Classic", 1080, 1920).unwrap();
        let l = CaptionStyle::preset_for("Classic", 1920, 1080).unwrap();
        let sq = CaptionStyle::preset_for("Pop", 1080, 1080).unwrap();
        assert!((v.margin - 0.28).abs() < 1e-6, "vertical: clear of the phone app's buttons");
        assert!((l.margin - 0.12).abs() < 1e-6 && (sq.margin - 0.12).abs() < 1e-6, "landscape and square: near the bottom");
        assert_eq!(CaptionStyle::preset_for("Subtitle", 1920, 1080), Some(CaptionStyle::default()), "subtitles keep their placement");
        assert!(CaptionStyle::preset_for("nope", 1, 1).is_none());
        // older projects have no short_side and keep sizing by height
        let old: CaptionStyle = serde_json::from_str(r#"{"size":40.0}"#).unwrap();
        assert!(!old.short_side);
    }
}
