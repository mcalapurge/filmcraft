//! Platform Overlay: a mock-up of a short-video app's interface (TikTok, YouTube Shorts,
//! Instagram Reels) drawn over a monitor's picture, so titles and faces can be kept clear of
//! the buttons, captions and navigation bar the app puts on top of the video.
//!
//! The layout is a list of [`Mark`]s in unit coordinates of the phone screen (x across its
//! width, y down its height, a 9:16 portrait screen); [`paint`] fits that screen into the
//! picture (a landscape picture shows the part a portrait crop keeps, the rest darkened). The
//! glyphs are our own generic drawings (hearts, bubbles, arrows), never the apps' artwork or
//! logos, and the measurements are approximate: the apps change their layouts often.
//!
//! State is [`MonitorView::platform_overlay`](crate::state::MonitorView) and `overlay_platform`,
//! changed by `view.platformOverlay` (toggle) and `view.platformOverlay.<tiktok|shorts|reels>`.

use egui::epaint::PathShape;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::state::OverlayPlatform;

/// Width over height of the phone screen the overlay is laid out on.
pub const SCREEN_ASPECT: f32 = 9.0 / 16.0;

/// A generic interface glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glyph {
    /// A profile picture, with a follow badge under it.
    Avatar {
        follow: bool,
    },
    Heart,
    Comment,
    Bookmark,
    Share,
    /// A paper plane (send to a friend).
    Send,
    Dots,
    /// A spinning record (the clip's sound).
    Disc,
    ThumbUp,
    ThumbDown,
    Remix,
    /// A square thumbnail of the clip's sound.
    AudioSquare,
    Search,
    Camera,
}

/// One element of a platform's interface, in unit screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    /// A glyph centred at (`x`, `y`); `r` is half its size as a fraction of the screen width.
    Glyph { glyph: Glyph, x: f32, y: f32, r: f32 },
    /// A placeholder line of text from `x`, centred on `y`, `w` wide and `h` high; `strong`
    /// lines (user names, the selected tab) are brighter.
    Line { x: f32, y: f32, w: f32, h: f32, strong: bool },
    /// A Follow / Subscribe button: outlined, or `filled` white.
    Pill { x: f32, y: f32, w: f32, h: f32, filled: bool },
    /// The navigation bar from `y` to the bottom: `items` buttons, the middle one a create
    /// button when `create`.
    NavBar { y: f32, items: u8, create: bool },
}

impl Mark {
    /// Bounding box (min x, min y, max x, max y) in unit screen coordinates.
    pub fn bounds(&self) -> [f32; 4] {
        match *self {
            Mark::Glyph { glyph, x, y, r } => {
                let ry = r * SCREEN_ASPECT;
                // the follow badge hangs below the avatar
                let below = if matches!(glyph, Glyph::Avatar { follow: true }) { ry * 1.4 } else { ry };
                [x - r, y - ry, x + r, y + below]
            }
            Mark::Line { x, y, w, h, .. } | Mark::Pill { x, y, w, h, .. } => [x, y - h / 2.0, x + w, y + h / 2.0],
            Mark::NavBar { y, .. } => [0.0, y, 1.0, 1.0],
        }
    }
}

/// The area of the screen the app's interface leaves clear (min x, min y, max x, max y).
pub fn safe_zone(p: OverlayPlatform) -> [f32; 4] {
    // fractions of a 1080 x 1920 screen
    let px = |l: f32, t: f32, r: f32, b: f32| [l / 1080.0, t / 1920.0, r / 1080.0, b / 1920.0];
    match p {
        OverlayPlatform::Tiktok => px(60.0, 150.0, 940.0, 1440.0),
        OverlayPlatform::Shorts => px(48.0, 200.0, 900.0, 1440.0),
        OverlayPlatform::Reels => px(40.0, 220.0, 940.0, 1480.0),
    }
}

/// A right-hand column of glyphs, each with a count under it unless it is the last two kinds
/// (avatars and sound thumbnails carry none).
fn column(out: &mut Vec<Mark>, x: f32, r: f32, items: &[(Glyph, f32)]) {
    for &(glyph, y) in items {
        out.push(Mark::Glyph { glyph, x, y, r });
        if !matches!(glyph, Glyph::Avatar { .. } | Glyph::Disc | Glyph::AudioSquare | Glyph::Dots) {
            out.push(Mark::Line { x: x - 0.035, y: y + r * SCREEN_ASPECT + 0.016, w: 0.07, h: 0.009, strong: false });
        }
    }
}

/// The interface of a platform's video feed.
pub fn marks(p: OverlayPlatform) -> Vec<Mark> {
    let mut m = Vec::new();
    match p {
        OverlayPlatform::Tiktok => {
            // top tabs: the selected one underlined, a search glyph on the right
            m.push(Mark::Line { x: 0.27, y: 0.045, w: 0.17, h: 0.012, strong: false });
            m.push(Mark::Line { x: 0.52, y: 0.045, w: 0.19, h: 0.013, strong: true });
            m.push(Mark::Line { x: 0.57, y: 0.062, w: 0.09, h: 0.003, strong: true });
            m.push(Mark::Glyph { glyph: Glyph::Search, x: 0.92, y: 0.045, r: 0.032 });
            column(
                &mut m,
                0.915,
                0.042,
                &[
                    (Glyph::Avatar { follow: true }, 0.47),
                    (Glyph::Heart, 0.56),
                    (Glyph::Comment, 0.64),
                    (Glyph::Bookmark, 0.72),
                    (Glyph::Share, 0.80),
                    (Glyph::Disc, 0.885),
                ],
            );
            m.push(Mark::Line { x: 0.04, y: 0.80, w: 0.28, h: 0.014, strong: true });
            m.push(Mark::Line { x: 0.04, y: 0.828, w: 0.66, h: 0.011, strong: false });
            m.push(Mark::Line { x: 0.04, y: 0.85, w: 0.48, h: 0.011, strong: false });
            m.push(Mark::Line { x: 0.04, y: 0.885, w: 0.42, h: 0.011, strong: false });
            m.push(Mark::NavBar { y: 0.93, items: 5, create: true });
        }
        OverlayPlatform::Shorts => {
            m.push(Mark::Glyph { glyph: Glyph::Search, x: 0.78, y: 0.045, r: 0.032 });
            m.push(Mark::Glyph { glyph: Glyph::Dots, x: 0.92, y: 0.045, r: 0.032 });
            column(
                &mut m,
                0.915,
                0.045,
                &[
                    (Glyph::ThumbUp, 0.50),
                    (Glyph::ThumbDown, 0.58),
                    (Glyph::Comment, 0.66),
                    (Glyph::Share, 0.74),
                    (Glyph::Remix, 0.82),
                    (Glyph::AudioSquare, 0.895),
                ],
            );
            m.push(Mark::Glyph { glyph: Glyph::Avatar { follow: false }, x: 0.075, y: 0.80, r: 0.035 });
            m.push(Mark::Line { x: 0.13, y: 0.80, w: 0.24, h: 0.013, strong: true });
            m.push(Mark::Pill { x: 0.40, y: 0.80, w: 0.20, h: 0.03, filled: true });
            m.push(Mark::Line { x: 0.04, y: 0.84, w: 0.70, h: 0.012, strong: false });
            m.push(Mark::Line { x: 0.04, y: 0.863, w: 0.52, h: 0.012, strong: false });
            m.push(Mark::Line { x: 0.04, y: 0.895, w: 0.40, h: 0.011, strong: false });
            m.push(Mark::NavBar { y: 0.93, items: 5, create: true });
        }
        OverlayPlatform::Reels => {
            m.push(Mark::Line { x: 0.05, y: 0.045, w: 0.17, h: 0.016, strong: true });
            m.push(Mark::Glyph { glyph: Glyph::Camera, x: 0.92, y: 0.045, r: 0.035 });
            column(&mut m, 0.92, 0.04, &[(Glyph::Heart, 0.56), (Glyph::Comment, 0.64), (Glyph::Send, 0.72), (Glyph::Dots, 0.80), (Glyph::AudioSquare, 0.87)]);
            m.push(Mark::Glyph { glyph: Glyph::Avatar { follow: false }, x: 0.075, y: 0.815, r: 0.035 });
            m.push(Mark::Line { x: 0.13, y: 0.815, w: 0.24, h: 0.013, strong: true });
            m.push(Mark::Pill { x: 0.40, y: 0.815, w: 0.16, h: 0.028, filled: false });
            m.push(Mark::Line { x: 0.04, y: 0.853, w: 0.66, h: 0.012, strong: false });
            m.push(Mark::Line { x: 0.04, y: 0.886, w: 0.38, h: 0.011, strong: false });
            m.push(Mark::NavBar { y: 0.935, items: 5, create: false });
        }
    }
    m
}

/// The phone screen fitted into a picture: all of a portrait picture, the centre column of a
/// wider one (what a 9:16 crop keeps).
pub fn screen_rect(pic: Rect) -> Rect {
    let (w, h) = (pic.width(), pic.height());
    if w <= 0.0 || h <= 0.0 {
        return pic;
    }
    let size = if w / h > SCREEN_ASPECT { vec2(h * SCREEN_ASPECT, h) } else { vec2(w, w / SCREEN_ASPECT) };
    Rect::from_center_size(pic.center(), size)
}

const FG: Color32 = Color32::from_rgba_premultiplied(235, 235, 235, 235);
const SHADOW: Color32 = Color32::from_black_alpha(110);
const DIM: Color32 = Color32::from_rgba_premultiplied(150, 150, 150, 150);
const BADGE: Color32 = Color32::from_rgb(235, 64, 90);

/// Draw a platform's interface over `pic`, clipped to `clip`.
pub fn paint(painter: &Painter, pic: Rect, clip: Rect, p: OverlayPlatform) {
    let painter = painter.with_clip_rect(clip.intersect(pic));
    let screen = screen_rect(pic);
    // a landscape picture: darken what the portrait crop drops
    for side in [Rect::from_min_max(pic.min, pos2(screen.min.x, pic.max.y)), Rect::from_min_max(pos2(screen.max.x, pic.min.y), pic.max)] {
        if side.width() > 0.5 {
            painter.rect_filled(side, 0.0, Color32::from_black_alpha(150));
        }
    }
    let at = |x: f32, y: f32| pos2(screen.min.x + x * screen.width(), screen.min.y + y * screen.height());
    // shade outside the safe zone, then outline it
    let [l, t, r, b] = safe_zone(p);
    let safe = Rect::from_min_max(at(l, t), at(r, b));
    let shade = Color32::from_black_alpha(45);
    for band in [
        Rect::from_min_max(screen.min, pos2(screen.max.x, safe.min.y)),
        Rect::from_min_max(pos2(screen.min.x, safe.max.y), screen.max),
        Rect::from_min_max(pos2(screen.min.x, safe.min.y), pos2(safe.min.x, safe.max.y)),
        Rect::from_min_max(pos2(safe.max.x, safe.min.y), pos2(screen.max.x, safe.max.y)),
    ] {
        painter.rect_filled(band, 0.0, shade);
    }
    let corners = [safe.left_top(), safe.right_top(), safe.right_bottom(), safe.left_bottom(), safe.left_top()];
    painter.extend(Shape::dashed_line(&corners, Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 220, 255, 200)), 5.0, 4.0));
    let sw = screen.width();
    for mark in marks(p) {
        match mark {
            Mark::Glyph { glyph: g, x, y, r } => glyph(&painter, at(x, y), r * sw, g),
            Mark::Line { x, y, w, h, strong } => {
                let rect = Rect::from_min_max(at(x, y - h / 2.0), at(x + w, y + h / 2.0));
                painter.rect_filled(rect.translate(vec2(0.0, 1.0)), rect.height() / 2.0, SHADOW);
                painter.rect_filled(rect, rect.height() / 2.0, if strong { FG } else { DIM });
            }
            Mark::Pill { x, y, w, h, filled } => {
                let rect = Rect::from_min_max(at(x, y - h / 2.0), at(x + w, y + h / 2.0));
                if filled {
                    painter.rect_filled(rect, rect.height() / 2.0, FG);
                } else {
                    painter.rect_stroke(rect, rect.height() * 0.3, Stroke::new(1.0, FG), StrokeKind::Inside);
                }
            }
            Mark::NavBar { y, items, create } => {
                let bar = Rect::from_min_max(at(0.0, y), screen.max);
                painter.rect_filled(bar, 0.0, Color32::from_black_alpha(215));
                let n = items.max(1);
                let s = (bar.height() * 0.32).min(sw * 0.05);
                for i in 0..n {
                    let c = pos2(bar.min.x + bar.width() * (f32::from(i) + 0.5) / f32::from(n), bar.center().y);
                    if create && i == n / 2 {
                        let button = Rect::from_center_size(c, vec2(s * 2.6, s * 1.6));
                        painter.rect_filled(button, s * 0.4, FG);
                        let k = Stroke::new((s * 0.25).max(1.0), Color32::BLACK);
                        painter.line_segment([c - vec2(s * 0.5, 0.0), c + vec2(s * 0.5, 0.0)], k);
                        painter.line_segment([c - vec2(0.0, s * 0.5), c + vec2(0.0, s * 0.5)], k);
                    } else {
                        painter.rect_stroke(Rect::from_center_size(c, vec2(s, s)), s * 0.25, Stroke::new(1.2, DIM), StrokeKind::Middle);
                    }
                }
            }
        }
    }
}

/// Draw a glyph centred on `c`, `r` points from its centre to its edge.
fn glyph(painter: &Painter, c: Pos2, r: f32, g: Glyph) {
    let p = |x: f32, y: f32| c + vec2(x * r, y * r);
    let thick = Stroke::new((r * 0.2).max(1.0), FG);
    // a soft shadow keeps white glyphs visible on bright pictures
    painter.circle_filled(c + vec2(0.0, r * 0.08), r * 1.05, Color32::from_black_alpha(35));
    match g {
        Glyph::Avatar { follow } => {
            painter.circle_filled(c, r, Color32::from_gray(150));
            painter.circle_filled(p(0.0, -0.25), r * 0.38, Color32::from_gray(210));
            painter.circle_stroke(c, r, Stroke::new((r * 0.08).max(1.0), FG));
            if follow {
                let b = p(0.0, 1.0);
                painter.circle_filled(b, r * 0.38, BADGE);
                let k = Stroke::new((r * 0.1).max(1.0), Color32::WHITE);
                painter.line_segment([b - vec2(r * 0.2, 0.0), b + vec2(r * 0.2, 0.0)], k);
                painter.line_segment([b - vec2(0.0, r * 0.2), b + vec2(0.0, r * 0.2)], k);
            }
        }
        Glyph::Heart => {
            painter.circle_filled(p(-0.42, -0.25), r * 0.48, FG);
            painter.circle_filled(p(0.42, -0.25), r * 0.48, FG);
            painter.add(Shape::convex_polygon(vec![p(-0.88, -0.05), p(0.88, -0.05), p(0.0, 0.85)], FG, Stroke::NONE));
        }
        Glyph::Comment => {
            painter.circle_filled(p(0.0, -0.08), r * 0.8, FG);
            painter.add(Shape::convex_polygon(vec![p(-0.55, 0.3), p(-0.1, 0.62), p(-0.75, 0.85)], FG, Stroke::NONE));
            for dx in [-0.35, 0.0, 0.35] {
                painter.circle_filled(p(dx, -0.08), r * 0.1, Color32::from_gray(60));
            }
        }
        Glyph::Bookmark => {
            let pts = vec![p(-0.55, -0.8), p(0.55, -0.8), p(0.55, 0.8), p(0.0, 0.4), p(-0.55, 0.8)];
            painter.add(PathShape::closed_line(pts, thick));
        }
        Glyph::Share => {
            painter.add(Shape::convex_polygon(vec![p(0.1, -0.75), p(0.9, -0.05), p(0.1, 0.65)], FG, Stroke::NONE));
            painter.add(PathShape::line(vec![p(0.2, -0.05), p(-0.35, 0.05), p(-0.75, 0.6)], thick));
        }
        Glyph::Send => {
            painter.add(PathShape::closed_line(vec![p(-0.85, -0.7), p(0.85, -0.85), p(-0.1, 0.85), p(-0.2, -0.1)], thick));
        }
        Glyph::Dots => {
            for dx in [-0.6, 0.0, 0.6] {
                painter.circle_filled(p(dx, 0.0), r * 0.17, FG);
            }
        }
        Glyph::Disc => {
            painter.circle_filled(c, r * 0.9, Color32::from_gray(35));
            painter.circle_stroke(c, r * 0.6, Stroke::new(1.0, Color32::from_gray(80)));
            painter.circle_filled(c, r * 0.32, Color32::from_gray(150));
        }
        Glyph::ThumbUp | Glyph::ThumbDown => {
            let s = if g == Glyph::ThumbUp { 1.0 } else { -1.0 };
            let q = |x: f32, y: f32| p(x, y * s);
            painter.rect_filled(Rect::from_two_pos(q(-0.85, -0.05), q(-0.5, 0.8)), r * 0.08, FG);
            painter.rect_filled(Rect::from_two_pos(q(-0.35, -0.05), q(0.8, 0.8)), r * 0.25, FG);
            painter.add(Shape::convex_polygon(vec![q(-0.35, 0.0), q(0.05, -0.85), q(0.35, -0.7), q(0.25, 0.0)], FG, Stroke::NONE));
        }
        Glyph::Remix => {
            painter.circle_stroke(c, r * 0.7, thick);
            painter.add(Shape::convex_polygon(vec![p(0.45, -0.95), p(0.95, -0.55), p(0.35, -0.35)], FG, Stroke::NONE));
        }
        Glyph::AudioSquare => {
            let rect = Rect::from_center_size(c, vec2(r * 1.7, r * 1.7));
            painter.rect_filled(rect, r * 0.3, Color32::from_gray(120));
            painter.rect_stroke(rect, r * 0.3, Stroke::new((r * 0.1).max(1.0), FG), StrokeKind::Inside);
        }
        Glyph::Search => {
            painter.circle_stroke(p(-0.15, -0.15), r * 0.55, thick);
            painter.line_segment([p(0.25, 0.25), p(0.8, 0.8)], thick);
        }
        Glyph::Camera => {
            painter.rect_stroke(Rect::from_two_pos(p(-0.85, -0.55), p(0.85, 0.7)), r * 0.25, thick, StrokeKind::Middle);
            painter.circle_stroke(p(0.0, 0.08), r * 0.32, thick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_lays_out_inside_the_screen() {
        for p in OverlayPlatform::ALL {
            let [l, t, r, b] = safe_zone(p);
            assert!(0.0 < l && l < r && r < 1.0 && 0.0 < t && t < b && b < 1.0, "{p:?} safe zone");
            for m in marks(p) {
                let [x0, y0, x1, y1] = m.bounds();
                assert!(x0 >= 0.0 && y0 >= 0.0 && x1 <= 1.0 && y1 <= 1.0, "{p:?} {m:?} leaves the screen");
            }
        }
    }

    #[test]
    fn the_right_hand_buttons_and_the_navigation_bar_are_outside_the_safe_zone() {
        for p in OverlayPlatform::ALL {
            let [_, _, r, b] = safe_zone(p);
            for m in marks(p) {
                let [x0, y0, ..] = m.bounds();
                match m {
                    Mark::Glyph { y, .. } if y > 0.3 && x0 > 0.5 => assert!(x0 >= r, "{p:?} {m:?} overlaps the safe zone"),
                    Mark::NavBar { .. } => assert!(y0 >= b, "{p:?} nav bar"),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn the_screen_is_portrait_and_centred_in_any_picture() {
        let landscape = Rect::from_min_size(pos2(10.0, 20.0), vec2(1600.0, 900.0));
        let s = screen_rect(landscape);
        assert!((s.width() / s.height() - SCREEN_ASPECT).abs() < 1e-4);
        assert_eq!(s.height(), 900.0);
        assert!((s.center() - landscape.center()).length() < 1e-3);
        let portrait = Rect::from_min_size(Pos2::ZERO, vec2(540.0, 960.0));
        assert_eq!(screen_rect(portrait), portrait);
        // a narrower-than-9:16 picture fits by width
        let tall = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1000.0));
        assert_eq!(screen_rect(tall).width(), 400.0);
        // degenerate pictures pass through instead of dividing by zero
        let empty = Rect::from_min_size(Pos2::ZERO, vec2(0.0, 100.0));
        assert_eq!(screen_rect(empty), empty);
    }

    #[test]
    fn painting_handles_degenerate_rects() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            for pic in [Rect::NOTHING, Rect::from_min_size(Pos2::ZERO, vec2(0.0, 0.0)), Rect::from_min_size(Pos2::ZERO, vec2(320.0, 180.0))] {
                for p in OverlayPlatform::ALL {
                    paint(painter, pic, Rect::EVERYTHING, p);
                }
            }
        });
        out.textures_delta.clear();
    }
}
