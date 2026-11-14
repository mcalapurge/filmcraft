# filmcraft-captions

Caption files and caption burn-in for FilmCraft (layer L2: depends on `filmcraft-time`,
`filmcraft-color`, `filmcraft-project`, `filmcraft-text` and craft-libs' `craft-captions`).

The caption *model* (caption tracks in a sequence, captions, track style) lives in
`filmcraft-project` (`caption.rs`) because it is part of the `.fcproj` document. This crate
converts caption files to and from caption tracks, and draws captions.

## Formats

SubRip, WebVTT, Scenarist SCC, MacCaption MCC, EBU STL and TTML / DFXP are read and written by
[`craft-captions`](https://github.com/storytold/craft-libs/tree/main/crates/captions) in
craft-libs. This crate re-exports it. The formats, the SCC and MCC encoders, the TTML timing and
speaker rules, and their tests are documented there, and format fixes go there.

What stays here:

- `track_from_document` / `document_from_track`: a parsed document to and from a caption track
  (language, WebVTT blocks, speakers, cue ids and settings).
- `track_format`: the caption track format a file maps to (SCC: CEA-608, MCC: CEA-708, STL:
  Teletext, the rest: subtitle).
- `write_options`: write options that name FilmCraft as the creating program in MCC and EBU STL
  files.

## Burn-in

`burn::render_caption` lays out a caption with the track style (font size relative to a 1080-line
frame, colour, background box, outline, alignment, top/middle/bottom anchor and margin; WebVTT
`line:N%` and `align:` settings override them), wraps lines to 90% of the frame width and
rasterises it to a small premultiplied linear-light RGBA overlay. `filmcraft-render` composites the
overlays of visible caption tracks over the finished frame (Program monitor and export burn-in).

Word-timed (live) captions (`Caption::words`): `burn::render_caption_at` takes the time into the
caption and draws the word being spoken as the track style's `WordHighlight` says — its own colour,
a rounded box behind it, or only the words spoken so far (Reveal, the line keeping its final
layout) — optionally enlarged (`highlight_scale`). Words are laid out one by one with a fixed gap
(wider for a box, so the line never shifts as the highlight moves). A word wider than 90% of the
frame shrinks the caption until it fits instead of being clipped. With the style's `short_side`,
size and outline are relative to the frame's shorter side, so a look is the same size on vertical
and landscape video. `burn::line_fill` measures how much of the line a set of words takes (room
included for a popped word or a box); the engine uses it to decide how many words a live caption
holds and to find captions that no longer fit.

Fitting reserves the largest highlighted word, outline and box when wrapping lines (`line_fill`
uses the same reserve, so a group it passes draws on one line), and the overlay is sized for the
popped word, so a long word stays whole even at 2× pop. A popped word may reach into the side
margin; the caption moves, all lines together, only when one would leave the frame, so left- or
right-aligned lines share one edge. Caption edits that drop timed words rewrite the plain text used
by sidecar exports from the surviving words (empty when none is left, rather than restoring the old
sentence); otherwise the written text and its markup are kept. Inserts, overwrites and frame-rate
conforms preserve the remaining words' sequence times (words after an inserted gap move with it).

Text is set in the track's font (**Inter**, `assets/fonts/Inter-*.ttf`, SIL OFL 1.1, attributed in
`ATTRIBUTION.md`, when none is chosen) in the track's font style (SemiBold by default) by the
`filmcraft-text` engine: shaped with kerning and ligatures, bidi-ordered, and drawn from its
sub-pixel positioned glyph cache.

Specifications used: SMPTE ST 334-2 (CDP structure), CEA-608 / CEA-708 (public descriptions of
the caption commands), the MacCaption MCC format description (line syntax and abbreviations),
EBU Tech 3264-E (1991), ISO/IEC 6937, W3C TTML1 (2nd ed.), TTML2 and IMSC 1.1. No other
implementation's code was consulted.

## Tests

`cargo test -p filmcraft-captions`: the track mapping, the program name in written files, and
burn-in. The format tests are in craft-captions.

- Live captions (`burn.rs`): the spoken word's colour moves left to right with time; box, reveal
  and pop; a 45-letter word on vertical video shrinks inside the frame and a 5,000-character word
  never panics; social looks have the same text height on 1080×1920 and 1920×1080; `line_fill`
  against the frame; NaN / infinite style values and a 1-pixel frame never panic.
