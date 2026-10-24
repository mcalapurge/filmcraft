//! Caption layout and burn-in.
//!
//! [`overlay`] lays out the caption showing on a track at a time with the track's
//! [`CaptionStyle`] (size relative to a 1080-line frame, colour, background box, outline,
//! alignment, anchor and margin; WebVTT `line:N%` and `align:` cue settings override the style)
//! and rasterises it into a small premultiplied linear-light RGBA [`Overlay`] positioned in the
//! frame. Renderers composite overlays over the finished picture.
//!
//! A word-timed caption ([`Caption::words`]) is laid out word by word, and the word spoken at the
//! time drawn is marked as the style's [`WordHighlight`] says: its own colour, a rounded box
//! behind it, or (Reveal) only the words spoken so far drawn; `highlight_scale` makes it pop.

use filmcraft_color::srgb_to_linear;
use filmcraft_project::{Caption, CaptionAlign, CaptionAnchor, CaptionStyle, CaptionTrack, Sequence, WordHighlight};
use filmcraft_time::Tick;

use filmcraft_text::{ParagraphStyle, TextStyle, layout, render};

/// Caption overlays are transient render resources, not source images. Keep one cue from
/// materialising an unbounded mask or RGBA buffer.
const MAX_OVERLAY_PIXELS: usize = 16 << 20;
const MAX_CAPTION_SIZE_1080: f32 = 256.0;
const MAX_OUTLINE_RADIUS: f32 = 64.0;

/// A rendered caption: premultiplied linear-light RGBA f32 pixels at `(x, y)` in the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Overlay {
    /// Composite over a premultiplied RGBA f32 canvas of `cw`×`ch` pixels.
    pub fn composite_onto(&self, canvas: &mut [f32], cw: usize, ch: usize) {
        for y in 0..self.h {
            let cy = self.y + y as i32;
            if cy < 0 || cy >= ch as i32 {
                continue;
            }
            for x in 0..self.w {
                let cx = self.x + x as i32;
                if cx < 0 || cx >= cw as i32 {
                    continue;
                }
                let s = &self.px[(y * self.w + x) * 4..(y * self.w + x) * 4 + 4];
                if s[3] <= 0.0 {
                    continue;
                }
                let i = (cy as usize * cw + cx as usize) * 4;
                let k = 1.0 - s[3];
                for c in 0..4 {
                    canvas[i + c] = s[c] + canvas[i + c] * k;
                }
            }
        }
    }
}

/// The caption face: the track's font family (Inter when empty or not installed) in its style
/// (SemiBold when empty) at `px`.
fn caption_style(font: &str, font_style: &str, px: f32) -> TextStyle {
    let family = if font.trim().is_empty() { filmcraft_text::fonts::DEFAULT_FAMILY } else { font.trim() };
    let style = if font_style.trim().is_empty() { "SemiBold" } else { font_style.trim() };
    TextStyle { family: family.into(), style: style.into(), size: px, ..Default::default() }
}

/// `v`, or `default` when a project file or command handed us NaN or an infinity.
fn finite(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

fn lin(c: [u8; 4]) -> [f32; 4] {
    let a = c[3] as f32 / 255.0;
    [srgb_to_linear(c[0] as f32 / 255.0) * a, srgb_to_linear(c[1] as f32 / 255.0) * a, srgb_to_linear(c[2] as f32 / 255.0) * a, a]
}

/// Word-wrap one line to `max_w` pixels.
fn wrap(line: &str, ts: &TextStyle, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split(' ') {
        let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && filmcraft_text::measure(&cand, ts) > max_w {
            out.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = cand;
        }
    }
    out.push(cur);
    out
}

/// Placement overrides from WebVTT cue settings.
fn vtt_overrides(settings: &str) -> (Option<f32>, Option<CaptionAlign>) {
    let mut line = None;
    let mut align = None;
    for kv in settings.split_whitespace() {
        let Some((k, v)) = kv.split_once(':') else { continue };
        match k {
            "line" => {
                let v = v.split(',').next().unwrap_or("");
                if let Some(p) = v.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
                    line = Some((p / 100.0).clamp(0.0, 1.0));
                }
            }
            "align" => {
                align = match v {
                    "start" | "left" => Some(CaptionAlign::Left),
                    "end" | "right" => Some(CaptionAlign::Right),
                    "center" | "middle" => Some(CaptionAlign::Center),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    (line, align)
}

/// A piece of a caption line: a whole line of an ordinary caption, or one word of a word-timed
/// caption (`word` is its index in [`Caption::words`]).
struct Token {
    text: String,
    word: Option<usize>,
    /// Left edge and width in frame pixels.
    x: f32,
    w: f32,
}

/// Word-wrap word-timed caption words to `max_w` pixels: lines of `(text, word index, width)`.
fn wrap_words(words: &[String], ts: &TextStyle, space: f32, max_w: f32) -> Vec<Vec<(String, usize, f32)>> {
    let mut out: Vec<Vec<(String, usize, f32)>> = Vec::new();
    let mut cur: Vec<(String, usize, f32)> = Vec::new();
    let mut cur_w = 0.0;
    for (i, word) in words.iter().enumerate() {
        let ww = filmcraft_text::measure(word, ts);
        if !cur.is_empty() && cur_w + space + ww > max_w {
            out.push(std::mem::take(&mut cur));
            cur_w = 0.0;
        }
        cur_w += if cur.is_empty() { ww } else { space + ww };
        cur.push((word.clone(), i, ww));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Anti-aliased coverage of a rounded rectangle, max-ed into `m` (an `ow`×`oh` mask at `(ox, oy)`).
fn fill_round_rect(m: &mut [f32], (ox, oy, ow, oh): (i32, i32, usize, usize), (x0, y0, x1, y1): (f32, f32, f32, f32), radius: f32) {
    let r = radius.clamp(0.0, ((x1 - x0).min(y1 - y0) / 2.0).max(0.0));
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    for y in (y0.floor() as i32).max(oy)..(y1.ceil() as i32).min(oy + oh as i32) {
        for x in (x0.floor() as i32).max(ox)..(x1.ceil() as i32).min(ox + ow as i32) {
            let (dx, dy) = (((x as f32 + 0.5) - cx).abs() - hx, ((y as f32 + 0.5) - cy).abs() - hy);
            let d = (dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)) - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            if let Some(p) = m.get_mut((y - oy) as usize * ow + (x - ox) as usize) {
                *p = p.max(a);
            }
        }
    }
}

/// What the style's sizes (font size, outline) are relative to, for a `w`×`h` frame: 1080 lines of
/// its height, or of its shorter side ([`CaptionStyle::short_side`]).
fn style_scale(style: &CaptionStyle, w: usize, h: usize) -> f32 {
    let basis = if style.short_side { w.min(h) } else { h };
    basis as f32 / 1080.0
}

/// The caption font size in frame pixels.
fn caption_px(style: &CaptionStyle, w: usize, h: usize) -> f32 {
    // (a frame under 4 px high used to panic here: `clamp` with max < min)
    (finite(style.size, 54.0).clamp(4.0, MAX_CAPTION_SIZE_1080) * style_scale(style, w, h)).clamp(4.0, (h as f32).max(4.0))
}

/// The gap between the words of a word-timed caption: a space, plus room for a highlight box.
fn word_gap(style: &CaptionStyle, ts: &TextStyle) -> f32 {
    let space = filmcraft_text::measure("a a", ts) - filmcraft_text::measure("aa", ts);
    // a highlight box reaches past its word; keep it off the neighbours (the same for every
    // word, so the line does not shift as the box moves)
    if style.highlight == WordHighlight::Box { space + (ts.size * 0.3).round() * 0.6 } else { space }
}

/// How wide `words` draw on one line as a word-timed caption in a `w`×`h` frame, as a fraction of
/// the width a caption may use (90% of the frame): up to 1.0 they fit on one line. Room is kept for
/// the widest word to pop ([`CaptionStyle::highlight_scale`]). Used to decide how many words a
/// live caption holds, and to flag captions that no longer fit.
pub fn line_fill(style: &CaptionStyle, w: usize, h: usize, words: &[&str]) -> f32 {
    if w == 0 || h == 0 || words.is_empty() {
        return 0.0;
    }
    let ts = caption_style(&style.font, &style.font_style, caption_px(style, w, h));
    let caps = |s: &str| if style.uppercase { s.to_uppercase() } else { s.to_string() };
    let widths: Vec<f32> = words.iter().map(|wd| filmcraft_text::measure(&caps(wd.trim()), &ts)).collect();
    let widest = widths.iter().copied().fold(0.0, f32::max);
    let pop = if style.highlight == WordHighlight::Off { 1.0 } else { finite(style.highlight_scale, 1.0).clamp(0.5, 2.0) };
    let total = widths.iter().sum::<f32>() + word_gap(style, &ts) * (widths.len() - 1) as f32 + widest * (pop - 1.0).max(0.0);
    total / (w as f32 * 0.9)
}

/// Lay out and rasterise one caption for a `w`×`h` frame, as it shows with no word spoken (an
/// ordinary caption always looks like this).
pub fn render_caption(c: &Caption, style: &CaptionStyle, w: usize, h: usize) -> Option<Overlay> {
    render_caption_at(c, style, w, h, None)
}

/// Lay out and rasterise one caption for a `w`×`h` frame at `offset` from the caption's start.
/// For a word-timed caption the word spoken at `offset` is drawn as the style's
/// [`WordHighlight`] says; `None` draws it with no word spoken.
pub fn render_caption_at(c: &Caption, style: &CaptionStyle, w: usize, h: usize, offset: Option<Tick>) -> Option<Overlay> {
    if w == 0 || h == 0 {
        return None;
    }
    let scale = style_scale(style, w, h);
    let max_w = w as f32 * 0.9;
    let caps = |s: &str| if style.uppercase { s.to_uppercase() } else { s.to_string() };
    let timed = !c.words.is_empty();
    let active = if timed { offset.and_then(|o| c.word_at(o)) } else { None };
    let words: Vec<String> = c.words.iter().map(|wd| caps(wd.text.trim())).collect();
    let mut px = caption_px(style, w, h);
    if timed {
        // a word too wide for the frame shrinks the caption until it fits, instead of being cut
        // off at the frame edge (an ordinary subtitle keeps its size and wraps)
        let probe = caption_style(&style.font, &style.font_style, px);
        let widest = words.iter().map(|wd| filmcraft_text::measure(wd, &probe)).fold(0.0, f32::max);
        if widest > max_w {
            px = (px * max_w / widest).max(4.0);
        }
    }
    let ts = caption_style(&style.font, &style.font_style, px);
    // (text, word index, width) per line
    let rows: Vec<Vec<(String, Option<usize>, f32)>> = if timed {
        wrap_words(&words, &ts, word_gap(style, &ts), max_w).into_iter().map(|l| l.into_iter().map(|(t, i, lw)| (t, Some(i), lw)).collect()).collect()
    } else {
        let mut lines: Vec<String> = Vec::new();
        // (speaker names are metadata; like Premiere, they are not burned in)
        for l in c.plain_lines() {
            lines.extend(wrap(caps(l.trim()).as_str(), &ts, max_w));
        }
        lines.into_iter().filter(|l| !l.is_empty()).map(|l| vec![(l.clone(), None, filmcraft_text::measure(&l, &ts))]).collect()
    };
    if rows.is_empty() {
        return None;
    }
    let vm = filmcraft_text::fonts::face(filmcraft_text::resolve(&ts.family, &ts.style).face).metrics(px);
    let (asc, desc) = (vm.ascent, vm.descent);
    let lh = px * finite(style.line_spacing, 1.25).clamp(0.8, 4.0);
    let pad_x = (px * 0.3).round();
    let block_h = lh * rows.len() as f32;
    let (line_override, align_override) = vtt_overrides(&c.settings);
    let align = align_override.unwrap_or(style.align);
    let margin = finite(style.margin, 0.08).clamp(0.0, 0.45) * h as f32;
    let top = match (line_override, style.anchor) {
        (Some(f), _) => (f * h as f32).min(h as f32 - block_h),
        (None, CaptionAnchor::Top) => margin,
        (None, CaptionAnchor::Middle) => (h as f32 - block_h) / 2.0,
        (None, CaptionAnchor::Bottom) => h as f32 - margin - block_h,
    }
    .max(0.0)
    .round();
    let side = w as f32 * 0.05;
    let space = if timed { word_gap(style, &ts) } else { 0.0 };
    let mut widths: Vec<f32> = Vec::with_capacity(rows.len());
    let mut lines: Vec<Vec<Token>> = Vec::with_capacity(rows.len());
    for row in rows {
        let lw = row.iter().map(|(_, _, tw)| *tw).sum::<f32>() + space * row.len().saturating_sub(1) as f32;
        let mut x = match align {
            CaptionAlign::Left => side + pad_x,
            CaptionAlign::Right => w as f32 - side - pad_x - lw,
            CaptionAlign::Center => (w as f32 - lw) / 2.0,
        };
        let mut toks = Vec::with_capacity(row.len());
        for (text, word, tw) in row {
            toks.push(Token { text, word, x, w: tw });
            x += tw + space;
        }
        widths.push(lw);
        lines.push(toks);
    }
    let xs: Vec<f32> = lines.iter().map(|l| l.first().map(|t| t.x).unwrap_or(0.0)).collect();
    let pop = if active.is_some() && style.highlight != WordHighlight::Off { finite(style.highlight_scale, 1.0).clamp(0.5, 2.0) } else { 1.0 };
    let outline = (finite(style.outline, 0.0) * scale).clamp(0.0, MAX_OUTLINE_RADIUS);
    let grow = outline.ceil() + 1.0 + (px * (pop - 1.0).max(0.0)).ceil() + if style.highlight == WordHighlight::Box { pad_x } else { 0.0 };
    let bx0 = xs.iter().zip(&widths).map(|(x, _)| x - pad_x - grow).fold(f32::MAX, f32::min).floor();
    let bx1 = xs.iter().zip(&widths).map(|(x, lw)| x + lw + pad_x + grow).fold(f32::MIN, f32::max).ceil();
    let by0 = (top - grow).floor();
    let by1 = (top + block_h + grow).ceil();
    // Text can be wider than the frame (an unbroken imported WebVTT word, for example). Raster
    // only the visible intersection; `draw` receives the original coordinates and clips into it.
    let (left, top_px) = (bx0.clamp(0.0, w as f32), by0.clamp(0.0, h as f32));
    let (right, bottom) = (bx1.clamp(0.0, w as f32), by1.clamp(0.0, h as f32));
    if right <= left || bottom <= top_px {
        return None;
    }
    let (ox, oy) = (left as i32, top_px as i32);
    let (ow, oh) = ((right - left) as usize, (bottom - top_px) as usize);
    if ow.checked_mul(oh).is_none_or(|pixels| pixels > MAX_OVERLAY_PIXELS) {
        return None;
    }
    let mut cover = vec![0.0f32; ow * oh];
    let mut cover_hi = vec![0.0f32; ow * oh];
    let mut bg = vec![0.0f32; ow * oh];
    let mut hbox = vec![0.0f32; ow * oh];
    let big = TextStyle { size: px * pop, ..ts.clone() };
    for (i, line) in lines.iter().enumerate() {
        let lt = top + lh * i as f32;
        if style.background && style.background_color[3] > 0 {
            let (x0, x1) = (xs[i] - pad_x, xs[i] + widths[i] + pad_x);
            let (y0, y1) = (lt, lt + lh);
            for y in (y0.floor() as i32).max(oy)..(y1.ceil() as i32).min(oy + oh as i32) {
                let fy = ((y as f32 + 1.0).min(y1) - (y as f32).max(y0)).clamp(0.0, 1.0);
                for x in (x0.floor() as i32).max(ox)..(x1.ceil() as i32).min(ox + ow as i32) {
                    let fx = ((x as f32 + 1.0).min(x1) - (x as f32).max(x0)).clamp(0.0, 1.0);
                    let j = (y - oy) as usize * ow + (x - ox) as usize;
                    bg[j] = (bg[j] + fx * fy).min(1.0);
                }
            }
        }
        let baseline = lt + (lh - (asc + desc)) / 2.0 + asc;
        for tok in line {
            let is_active = tok.word.is_some() && tok.word == active;
            // Reveal: words not spoken yet are not drawn (their place is kept)
            if style.highlight == WordHighlight::Reveal && timed && tok.word.is_some_and(|wi| active.is_none_or(|a| wi > a)) {
                continue;
            }
            let hi = is_active && style.highlight != WordHighlight::Off;
            if is_active && style.highlight == WordHighlight::Box && style.highlight_box_color[3] > 0 {
                let (bw, bh) = (tok.w * pop + pad_x * 1.2, lh * 0.92 * pop.max(1.0));
                let cx = tok.x + tok.w / 2.0;
                let cy = lt + lh / 2.0;
                fill_round_rect(&mut hbox, (ox, oy, ow, oh), (cx - bw / 2.0, cy - bh / 2.0, cx + bw / 2.0, cy + bh / 2.0), px * 0.22);
            }
            let (tstyle, tx) = if is_active && pop != 1.0 {
                let bw = filmcraft_text::measure(&tok.text, &big);
                (&big, tok.x + (tok.w - bw) / 2.0)
            } else {
                (&ts, tok.x)
            };
            let l = layout(&tok.text, tstyle, &ParagraphStyle::default());
            let target = if hi { &mut cover_hi } else { &mut cover };
            let mut m = filmcraft_text::Mask { w: ow, h: oh, a: std::mem::take(target) };
            render::draw(&l, &render::at(tx, baseline.round()), &mut m, (ox, oy));
            *target = m.a;
        }
    }
    let stroke = if outline > 0.0 {
        let all: Vec<f32> = cover.iter().zip(&cover_hi).map(|(a, b)| a.max(*b)).collect();
        dilate(&all, ow, oh, outline)
    } else {
        Vec::new()
    };
    let (tc, bc, oc) = (lin(style.color), lin(style.background_color), lin(style.outline_color));
    let (hc, hbc) = (lin(style.highlight_color), lin(style.highlight_box_color));
    let mut out = vec![0.0f32; ow * oh * 4];
    for j in 0..ow * oh {
        let mut p = [0.0f32; 4];
        let over = |p: &mut [f32; 4], c: [f32; 4], a: f32| {
            if a > 0.0 {
                for k in 0..4 {
                    p[k] = c[k] * a + p[k] * (1.0 - c[3] * a);
                }
            }
        };
        over(&mut p, bc, bg[j]);
        over(&mut p, hbc, hbox[j]);
        if !stroke.is_empty() {
            over(&mut p, oc, stroke[j]);
        }
        over(&mut p, tc, cover[j]);
        over(&mut p, hc, cover_hi[j]);
        out[j * 4..j * 4 + 4].copy_from_slice(&p);
    }
    Some(Overlay { x: ox, y: oy, w: ow, h: oh, px: out })
}

/// Max-filter a coverage mask with a disc of radius `r`.
fn dilate(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let ri = r.ceil() as i32;
    let mut offs = Vec::new();
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d <= r + 0.5 {
                offs.push((dx, dy, (r + 0.5 - d).clamp(0.0, 1.0)));
            }
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut v = 0.0f32;
            for &(dx, dy, wgt) in &offs {
                let (sx, sy) = (x + dx, y + dy);
                if sx >= 0 && sy >= 0 && sx < w as i32 && sy < h as i32 {
                    v = v.max(m[sy as usize * w + sx as usize] * wgt);
                }
            }
            out[y as usize * w + x as usize] = v;
        }
    }
    out
}

/// The overlay for a caption track at timeline time `t` (None when nothing shows).
pub fn track_overlay(track: &CaptionTrack, t: Tick, w: usize, h: usize) -> Option<Overlay> {
    if !track.enabled {
        return None;
    }
    let c = track.caption_at(t)?;
    render_caption_at(c, &track.style, w, h, Some(t - c.start))
}

/// Overlays of every visible caption track of a sequence at `t`, bottom track first.
pub fn sequence_overlays(seq: &Sequence, t: Tick, w: usize, h: usize) -> Vec<Overlay> {
    seq.caption_tracks.iter().rev().filter_map(|tr| track_overlay(tr, t, w, h)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{CaptionFormat, ClipId, TrackId};

    fn cap(text: &str) -> Caption {
        Caption {
            id: ClipId(1),
            start: Tick(0),
            duration: Tick(100),
            text: text.into(),
            speaker: None,
            cue_id: None,
            settings: String::new(),
            words: Vec::new(),
        }
    }

    #[test]
    fn bottom_centre_with_box() {
        let st = CaptionStyle::default();
        let o = render_caption(&cap("Hello world"), &st, 1920, 1080).unwrap();
        // centred horizontally, in the lower part of the frame
        let cx = o.x as f32 + o.w as f32 / 2.0;
        assert!((cx - 960.0).abs() < 4.0, "centre {cx}");
        assert!(o.y > 800 && (o.y as usize + o.h) < 1080, "y {} h {}", o.y, o.h);
        // white text pixels exist and the box is dark translucent
        let mut white = 0;
        let mut boxy = 0;
        for p in o.px.chunks(4) {
            if p[3] > 0.99 && p[0] > 0.9 {
                white += 1;
            } else if p[3] > 0.7 && p[0] < 0.01 {
                boxy += 1;
            }
        }
        assert!(white > 500, "{white}");
        assert!(boxy > 1000, "{boxy}");
    }

    #[test]
    fn top_anchor_and_vtt_line() {
        let st = CaptionStyle { anchor: CaptionAnchor::Top, ..Default::default() };
        let o = render_caption(&cap("Top"), &st, 1280, 720).unwrap();
        assert!(o.y < 100);
        let mut c = cap("Line");
        c.settings = "line:50% align:start".into();
        let o = render_caption(&c, &CaptionStyle::default(), 1280, 720).unwrap();
        assert!((o.y - 360).abs() < 4, "{}", o.y);
        assert!(o.x < 120, "{}", o.x);
    }

    #[test]
    fn margin_is_a_fraction_of_frame_height() {
        // #1542: on a portrait sequence, a bottom-anchored caption with margin 0.31 sits with
        // its bottom edge 0.31 of the frame height above the bottom; the top anchor mirrors it.
        let st = CaptionStyle { anchor: CaptionAnchor::Bottom, margin: 0.31, size: 78.0, ..Default::default() };
        let o = render_caption(&cap("Hello world"), &st, 1080, 1920).unwrap();
        // the overlay pads the content by the outline grow (the default outline is 0, so by 1 px)
        let bottom = (o.y as usize + o.h) as f32;
        let expected = 1920.0 - 0.31 * 1920.0;
        assert!((bottom - expected).abs() <= 2.0, "bottom edge {bottom}, expected {expected}");
        let st = CaptionStyle { anchor: CaptionAnchor::Top, margin: 0.31, size: 78.0, ..Default::default() };
        let o = render_caption(&cap("Hello world"), &st, 1080, 1920).unwrap();
        assert!((o.y as f32 - 0.31 * 1920.0).abs() <= 2.0, "top edge {}, expected {}", o.y, 0.31 * 1920.0);
        // margin 0 and the 0.45 clamp stay inside the frame
        for margin in [0.0, 0.31, 0.45, 9.0] {
            let st = CaptionStyle { anchor: CaptionAnchor::Bottom, margin, size: 78.0, ..Default::default() };
            let o = render_caption(&cap("Hello world"), &st, 1080, 1920).unwrap();
            // the outline-grow padding plus ceil rounding may reach 2 px past the edge; the
            // composite clips it
            assert!(o.y >= 0 && o.y as usize + o.h <= 1922, "margin {margin}: y {} h {}", o.y, o.h);
        }
    }

    #[test]
    fn wraps_and_composites() {
        let long = "word ".repeat(60);
        let o = render_caption(&cap(&long), &CaptionStyle::default(), 640, 360).unwrap();
        assert!(o.w <= 640 && o.h > 40, "{}x{}", o.w, o.h);
        let mut canvas = vec![0.0f32; 640 * 360 * 4];
        o.composite_onto(&mut canvas, 640, 360);
        assert!(canvas.chunks(4).any(|p| p[0] > 0.9));
    }

    #[test]
    fn overlay_is_clipped_to_the_frame_for_an_unbroken_word() {
        let o = render_caption(&cap(&"W".repeat(10_000)), &CaptionStyle::default(), 640, 360).unwrap();
        assert!(o.x >= 0 && o.y >= 0);
        assert!(o.w <= 640 && o.h <= 360, "{}x{}", o.w, o.h);
        assert!(o.w.checked_mul(o.h).unwrap() <= MAX_OVERLAY_PIXELS);
    }

    #[test]
    fn hostile_outline_is_bounded() {
        let st = CaptionStyle { outline: 1_000_000.0, ..Default::default() };
        let o = render_caption(&cap("Safe"), &st, 640, 360).unwrap();
        assert!(o.w <= 640 && o.h <= 360, "{}x{}", o.w, o.h);
    }

    #[test]
    fn outline_draws_around_text() {
        let st = CaptionStyle { background: false, outline: 4.0, outline_color: [255, 0, 0, 255], ..Default::default() };
        let o = render_caption(&cap("O"), &st, 1920, 1080).unwrap();
        assert!(o.px.chunks(4).any(|p| p[0] > 0.9 && p[1] < 0.05 && p[3] > 0.9), "red outline pixels");
    }

    #[test]
    fn track_font_is_used() {
        // the style's font used to be ignored (always Inter)
        let inter = render_caption(&cap("Hello world"), &CaptionStyle::default(), 1920, 1080).unwrap();
        let serif = CaptionStyle { font: "Noto Serif".into(), ..Default::default() };
        let serif = render_caption(&cap("Hello world"), &serif, 1920, 1080).unwrap();
        assert_ne!((inter.w, &inter.px), (serif.w, &serif.px), "Noto Serif renders like Inter");
        let mono = CaptionStyle { font: "jetbrains mono".into(), ..Default::default() };
        let mono = render_caption(&cap("Hello world"), &mono, 1920, 1080).unwrap();
        assert_ne!((inter.w, &inter.px), (mono.w, &mono.px), "JetBrains Mono renders like Inter");
    }

    #[test]
    fn unknown_or_empty_font_falls_back_to_inter() {
        let inter = render_caption(&cap("Hello"), &CaptionStyle::default(), 1280, 720).unwrap();
        for font in ["", "  ", "No Such Font 123"] {
            let st = CaptionStyle { font: font.into(), ..Default::default() };
            assert_eq!(render_caption(&cap("Hello"), &st, 1280, 720), Some(inter.clone()), "font {font:?}");
        }
    }

    #[test]
    fn hidden_track_draws_nothing() {
        let mut t = CaptionTrack::new(TrackId(1), "S".into(), CaptionFormat::Subtitle);
        t.captions.push(cap("x"));
        assert!(track_overlay(&t, Tick(5), 100, 100).is_some());
        t.enabled = false;
        assert!(track_overlay(&t, Tick(5), 100, 100).is_none());
    }

    fn live(style: CaptionStyle) -> (Caption, CaptionStyle) {
        let mut c = cap("one two three");
        c.duration = Tick(300);
        c.time_words_evenly();
        (c, style)
    }

    fn count(o: &Overlay, f: impl Fn(&[f32]) -> bool) -> usize {
        o.px.chunks(4).filter(|p| f(p)).count()
    }

    #[test]
    fn spoken_word_is_drawn_in_the_highlight_colour() {
        let st = CaptionStyle { background: false, highlight: WordHighlight::Color, highlight_color: [255, 0, 0, 255], ..Default::default() };
        let (c, st) = live(st);
        let red = |p: &[f32]| p[3] > 0.9 && p[0] > 0.9 && p[1] < 0.05;
        let none = render_caption_at(&c, &st, 1920, 1080, None).unwrap();
        assert_eq!(count(&none, red), 0, "no word spoken, nothing lit");
        let first = render_caption_at(&c, &st, 1920, 1080, Some(c.words[0].start)).unwrap();
        let last = render_caption_at(&c, &st, 1920, 1080, Some(c.words[2].start)).unwrap();
        assert!(count(&first, red) > 100 && count(&last, red) > 100);
        // the lit word moves from left to right
        let cx = |o: &Overlay| {
            let (mut sx, mut n) = (0usize, 0usize);
            for (i, p) in o.px.chunks(4).enumerate() {
                if red(p) {
                    sx += i % o.w;
                    n += 1;
                }
            }
            o.x as f32 + sx as f32 / n.max(1) as f32
        };
        assert!(cx(&first) + 100.0 < cx(&last), "{} {}", cx(&first), cx(&last));
        // with highlighting off the timing changes nothing
        let off = CaptionStyle { highlight: WordHighlight::Off, ..st.clone() };
        assert_eq!(render_caption_at(&c, &off, 640, 360, Some(Tick(0))), render_caption_at(&c, &off, 640, 360, Some(Tick(250))));
    }

    #[test]
    fn box_and_reveal_and_pop() {
        let st = CaptionStyle { background: false, highlight: WordHighlight::Box, highlight_box_color: [0, 255, 0, 255], ..Default::default() };
        let (c, st) = live(st);
        let green = |p: &[f32]| p[3] > 0.9 && p[1] > 0.9 && p[0] < 0.05;
        let o = render_caption_at(&c, &st, 1920, 1080, Some(c.words[1].start)).unwrap();
        assert!(count(&o, green) > 1000, "a box behind the spoken word");
        let reveal = CaptionStyle { highlight: WordHighlight::Reveal, ..st.clone() };
        let ink = |o: &Overlay| count(o, |p| p[3] > 0.5);
        let one = render_caption_at(&c, &reveal, 1920, 1080, Some(c.words[0].start)).unwrap();
        let all = render_caption_at(&c, &reveal, 1920, 1080, Some(c.words[2].start)).unwrap();
        assert!(ink(&one) * 2 < ink(&all), "words appear as they are spoken: {} {}", ink(&one), ink(&all));
        assert_eq!((one.x, one.w), (all.x, all.w), "the line does not move as words appear");
        let pop = CaptionStyle { highlight: WordHighlight::Color, highlight_scale: 1.5, ..st.clone() };
        let flat = CaptionStyle { highlight: WordHighlight::Color, ..st };
        let at = Some(c.words[1].start);
        assert!(ink(&render_caption_at(&c, &pop, 1920, 1080, at).unwrap()) > ink(&render_caption_at(&c, &flat, 1920, 1080, at).unwrap()));
    }

    #[test]
    fn track_overlay_animates_with_time_and_hostile_styles_are_safe() {
        let mut t = CaptionTrack::new(TrackId(1), "S".into(), CaptionFormat::Subtitle);
        t.style = CaptionStyle::preset("Pop").unwrap();
        let (c, _) = live(CaptionStyle::default());
        t.captions.push(c.clone());
        assert_ne!(track_overlay(&t, Tick(1), 640, 360), track_overlay(&t, Tick(250), 640, 360));
        for scale in [f32::NAN, f32::INFINITY, -5.0, 1e9] {
            let st = CaptionStyle { highlight: WordHighlight::Box, highlight_scale: scale, outline: f32::NAN, ..Default::default() };
            let _ = render_caption_at(&c, &st, 640, 360, Some(Tick(150)));
        }
        let mut bad = c.clone();
        bad.words[0].start = Tick(i64::MAX);
        bad.words[1].end = Tick(i64::MIN);
        let _ = render_caption_at(&bad, &t.style, 640, 360, Some(Tick(i64::MIN)));
        let _ = render_caption_at(&bad, &t.style, 1, 1, Some(Tick(i64::MAX)));
    }

    fn live_text(text: &str) -> Caption {
        let mut c = cap(text);
        c.duration = Tick(1000);
        c.time_words_evenly();
        c
    }

    /// Columns of a rendered overlay that hold text ink (alpha over half), in frame x.
    fn ink_span(o: &Overlay) -> (i32, i32) {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for (i, p) in o.px.chunks(4).enumerate() {
            if p[3] > 0.5 {
                let x = o.x + (i % o.w) as i32;
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        (lo, hi)
    }

    #[test]
    fn a_word_too_long_for_the_frame_shrinks_to_fit_instead_of_being_cut_off() {
        // vertical video, the Classic look: this word is far wider than 1080 px at full size
        let st = CaptionStyle::preset("Classic").unwrap();
        let word = "Pneumonoultramicroscopicsilicovolcanoconiosis";
        let c = live_text(word);
        let o = render_caption_at(&c, &st, 1080, 1920, Some(Tick(0))).unwrap();
        let (lo, hi) = ink_span(&o);
        assert!(lo > 0 && hi < 1079, "the whole word is inside the frame: ink {lo}..{hi}");
        assert!(hi - lo > 700, "and still large: {}", hi - lo);
        assert!(line_fill(&st, 1080, 1920, &[word]) > 1.0, "it would not have fitted at full size");
        // a short word keeps the look's size: it is not shrunk
        let short = render_caption_at(&live_text("HI"), &st, 1080, 1920, Some(Tick(0))).unwrap();
        let ink_h =
            |o: &Overlay| o.px.chunks(4).enumerate().filter(|(_, p)| p[3] > 0.5).map(|(i, _)| i / o.w).fold((usize::MAX, 0), |(a, b), y| (a.min(y), b.max(y)));
        let (a, b) = ink_h(&short);
        let (c0, c1) = ink_h(&o);
        assert!(b - a > c1 - c0, "the long word is drawn smaller than a short one");
        // a hostile run of one character never panics and stays in the frame
        let o = render_caption_at(&live_text(&"W".repeat(5000)), &st, 1080, 1920, Some(Tick(0))).unwrap();
        assert!(o.x >= 0 && o.x as usize + o.w <= 1080);
    }

    #[test]
    fn social_looks_are_the_same_size_on_vertical_and_landscape_video() {
        let text_h = |st: &CaptionStyle, w: usize, h: usize| {
            let o = render_caption_at(&live_text("HELLO"), st, w, h, Some(Tick(0))).unwrap();
            let rows: Vec<usize> = o.px.chunks(4).enumerate().filter(|(_, p)| p[3] > 0.5).map(|(i, _)| i / o.w).collect();
            rows.iter().max().unwrap() - rows.iter().min().unwrap()
        };
        let social = CaptionStyle::preset("Classic").unwrap();
        assert!(social.short_side);
        let (v, l) = (text_h(&social, 1080, 1920), text_h(&social, 1920, 1080));
        assert!((v as i64 - l as i64).abs() <= 2, "vertical {v} vs landscape {l}");
        // subtitles keep the height convention (older projects render as before): 1.78× on vertical
        let sub = CaptionStyle { short_side: false, ..social };
        let (v, l) = (text_h(&sub, 1080, 1920), text_h(&sub, 1920, 1080));
        assert!(v as f32 > l as f32 * 1.6, "vertical {v} vs landscape {l}");
    }

    #[test]
    fn line_fill_measures_against_the_frame() {
        let st = CaptionStyle::preset("Classic").unwrap();
        let words = ["CAPTIONS", "THAT", "POP", "EVERYWHERE"];
        let v = line_fill(&st, 1080, 1920, &words);
        let l = line_fill(&st, 1920, 1080, &words);
        assert!(v > l * 1.5, "a vertical line is narrower: {v} vs {l}");
        assert!(line_fill(&st, 1080, 1920, &["SO", "THIS"]) < 1.0);
        assert!(v > 1.0, "four long words do not fit a vertical line: {v}");
        // a pop look keeps room for the grown word; a box look for the box
        let pop = CaptionStyle { highlight_scale: 1.5, ..st.clone() };
        assert!(line_fill(&pop, 1080, 1920, &words) > v);
        assert_eq!(line_fill(&st, 0, 0, &words), 0.0);
        assert_eq!(line_fill(&st, 1080, 1920, &[]), 0.0);
    }
}
