# Captions

FilmCraft keeps captions on **caption tracks** in the sequence, shown in their own area above the
video tracks (as Premiere does). Caption tracks are saved in the `.fcproj` (project format
version 2; version 1 projects open with no caption tracks).

## Model

- **Caption track** (`CaptionTrack`, `crates/project/src/caption.rs`): name, format (`Subtitle`,
  `CEA-608`, `CEA-708`, `Teletext`), language, output eye (shown in the Program monitor and
  burned in on export), lock, sync lock, and a **track style**:
  font size (pixels at 1080 lines, scaled to the frame), text colour, background box and its colour,
  outline width and colour, alignment (left/centre/right), position (top/middle/bottom) and
  margin (a fraction of the frame height, default 0.08; `captions.setStyle` clamps it to 0–0.45),
  line spacing.
- **Caption**: in/out in exact `Tick`s, text (lines separated by `\n`; `<i>`, `<b>`, `<u>` kept
  as written), optional speaker, and the WebVTT cue id and cue settings, kept for round trips.
  Captions on a track never overlap.

## Files

| Format | Import | Export | Notes |
|---|---|---|---|
| SubRip `.srt` | ✓ | ✓ | Forgiving reader (BOM, UTF-16, Windows-1252, CRLF, missing indexes, `,`/`.` ms, missing hours). |
| WebVTT `.vtt` | ✓ | ✓ | Keeps cue ids, cue settings, `<v Speaker>`, STYLE/REGION/NOTE blocks. `line:N%` and `align:` are honoured when drawing. |
| Scenarist SCC `.scc` | ✓ | ✓ | CEA-608 at 29.97 fps. Reads pop-on, roll-up and paint-on; writes pop-on with drop-frame (default) or non-drop timecode. |

`File ▸ Import` (or dropping a file) recognises caption files by extension and content and adds a
new caption track to the open sequence (a sequence is created when none is open). Cue times are
snapped to the sequence's frames. `File ▸ Export ▸ Captions…` writes the chosen caption track in
the format given by the file extension (`.srt`, `.vtt`, `.scc`).

Details of each format and the SCC encoder are in [`crates/captions/README.md`](../crates/captions/README.md).

## Editing

| Command | Menu / shortcut | Params |
|---|---|---|
| `captions.newTrack` | Sequence ▸ Captions ▸ Add New Caption Track… (⌥⌘A) | `format`, `name`, `language` |
| `captions.add` | Sequence ▸ Captions ▸ Add Caption at Playhead (⌥⌘C) | `track`, `text`, `time`, `durationSeconds` (3) |
| `captions.next` / `captions.previous` | Sequence ▸ Captions ▸ Go to Next/Previous Caption Segment (⌥⌘↓/↑) | |
| `captions.showAll` / `captions.hideAll` | Sequence ▸ Captions | |
| `captions.split` | Text panel ✂; Razor tool on a caption (⇧: every caption track) | `caption`, `time` (default: every caption under the playhead) |
| `captions.merge` | Text panel | `captions` (consecutive, one track) |
| `captions.setText` | Text panel text field | `caption`, `text`, `speaker` |
| `captions.setTimes` | Text panel in/out fields | `caption`, `start…`/`end…` (`Time` ticks, `Frame`, `Seconds`, `Timecode`) |
| `captions.trim` | drag a caption edge in the timeline | `caption`, `edge` (`in`/`out`), `delta` or `deltaFrames` |
| `captions.move` | drag a caption in the timeline | `captions`, `delta` or `deltaFrames` |
| `captions.duplicate` | ⌥-drag a caption in the timeline | `captions`, `delta` or `deltaFrames` — copies on the same tracks, overwriting what is there |
| `captions.delete` | Edit ▸ Clear / Ripple Delete with captions selected | `captions`, `ripple` |
| `edit.copy` / `edit.cut` / `edit.paste` / `edit.pasteInsert` | ⌘C / ⌘X / ⌘V / ⇧⌘V with only captions selected | captions paste at the playhead on the tracks they came from, overwriting (Paste Insert pushes later captions on); Paste places whichever of clips or captions was copied last |
| `captions.select`, `captions.goTo` | click / number button | `captions`, `add` / `caption` |
| `captions.setTrack` | caption track header (eye, lock) | `track`, `name`, `format`, `language`, `enabled`, `locked`, `syncLock` |
| `captions.setStyle` | Text panel style strip; Live Captions inspector | `track`, `size`, `color`, `background`, `backgroundColor`, `outline`, `outlineColor`, `align`, `anchor`, `margin`, `lineSpacing`, `fontStyle`, `uppercase`, `highlight`, `highlightColor`, `highlightBoxColor`, `highlightScale`, `preset` (applied first, so the other keys adjust it), `reset` |
| `captions.createLive` | Sequence ▸ Captions ▸ Create Live Captions; Live Captions panel | `maxWords` (1–8, 3), `maxChars` (22), `preset`, `source` (`transcript`/`track`), `track`, `name` — see [Live captions](#live-captions) |
| `captions.setWord` | Live Captions panel (double-click a word) | `caption`, `index`, `text` (empty removes the word), `start`/`end` (sequence ticks) |
| `captions.deleteTrack` | | `track` |
| `captions.import` / `captions.export` | File ▸ Export ▸ Captions… | `path`, `format`, `track`, `dropFrame` |
| `captions.list` | (query) | `track` — tracks, styles and captions with timecodes (a `track` that names no caption track is an error) |

Tracks are addressed by id or `"C1"`, `"C2"`… (top first). Edits are limited by neighbouring
captions and a one-frame minimum. Sync-locked caption tracks follow insert and extract edits on
the media tracks (a caption across an insert point is split). `Add Edit to All Tracks` (⇧⌘K)
also splits captions.

For live captions, trims and overwrites remove words outside the surviving caption and update
its plain text, including caption sidecars. An insert splits the word list and moves the right
half with the inserted gap. Changing the sequence timebase snaps caption edges while preserving
the remaining words' sequence times.

## Timeline and Text panel

- Caption rows: header with `C1…` badge, lock, output eye, name and format; caption blocks in
  violet showing their text (Arabic and Hebrew in reading order from the block's right edge). Click selects (⌘/⇧ adds), drag moves (the whole selection; ⌥-drag
  copies), edge drag trims, the Razor tool cuts, double-click
  opens the Text panel. Automation ids: `timeline.caption.<id>`,
  `timeline.captionTrack.C1[.locked|.enabled]`.
- **Text panel ▸ Captions**: search, track picker (also creates tracks of each format), add /
  split / merge / delete / export buttons, the list of segments with editable in/out timecodes
  and text (committed when the field loses focus; Arabic and Hebrew show in reading order and are
  edited in logical order while the field has focus), and the track style strip (size, colour, box,
  alignment, position). The list keeps the caption under the playhead in view (centred as the
  playhead reaches it, not while a field is being edited). Automation ids: `text.tab.Captions`, `text.captions.add|split|merge|delete|export|search|track`,
  `text.captions.<id>.in|out|text|goto|row`, `text.captions.style.*`.

## Live captions

Short captions of one to four words with the word being spoken highlighted, as social video
editors make them (project schema v16).

- **Model.** A word-timed caption carries `Caption::words`: each word's text and bounds as offsets
  from the caption's start, so moving a caption moves its words. Split puts each word on the side
  it starts on (each half's text becomes its words); merge joins word-timed captions on one line;
  trims, new in points and extract keep the words where they are in the sequence; a text edit with
  the same number of words keeps every timing, otherwise the old span is shared out by word
  length. Ordinary captions have no words and save exactly as before.
- **Style.** The track style says how the spoken word is drawn (`highlight`): `off`, `color` (its
  own colour), `box` (a rounded box behind it) or `reveal` (words appear as they are spoken, the
  line keeping its final layout); `highlightScale` makes it pop. `fontStyle` (SemiBold by default)
  and `uppercase` apply to every caption. With `short_side` on, size and outline are for 1080
  pixels of the frame's shorter side instead of its height, so a look is the same size on vertical
  (1080×1920), square and landscape video; older projects and the Subtitle look keep sizing by
  height.
- **Looks.** Eight original presets (`CAPTION_PRESETS`): Classic, Pop, Box, Karaoke, Neon, Banner,
  Minimal, Subtitle. Applying one (`captions.createLive {preset}` or `captions.setStyle {preset}`)
  places it for the sequence's frame: 28% up on vertical video, clear of a phone app's buttons, and
  12% up on landscape and square video.
- **Making them.** `captions.createLive` cuts the sequence transcript (`source: "transcript"`, a new
  track at the top) or re-cuts an existing track (`source: "track"`; captions without word timings
  get their words spread over their duration by length). A caption holds at most `maxWords` words
  and `maxChars` characters, and only as many as fit on one line of the frame, measured in the
  track's font at the sequence's size (`filmcraft_captions::burn::line_fill`, which also leaves
  room for a popped word or a highlight box): long words and vertical video get fewer words per
  caption. A new caption also starts at a speaker change, at a pause of 0.6 s, after `.` `?` `!`
  `…`, and after `,` `;` `:` once it has two words. A caption ends at its last word's end,
  snapped outward to the sequence frame grid, with no additional hold. Use one word per caption
  to show only the current word. Existing saved timings require re-transcription and caption
  regeneration to pick up alignment corrections.
- **Too long.** A single word wider than the frame is drawn smaller until it fits, instead of
  being cut off. A word-timed caption that no longer fits one line (after a text edit, a bigger
  size or another frame size) wraps; `captions.list` lists these per track in `overflowing`, and
  Re-cut fits them.
- **Live Captions panel** (Window ▸ Live Captions): toolbar with the caption track, words per
  caption (1–4), **Generate** (transcribes the sequence first when it has no transcript), **Re-cut**
  and a "too wide" count that goes to the first such caption; rows with each caption's in point and
  its words as chips (the word under the playhead lit in the highlight colour, the list following
  the playhead; click a word to go to it, double-click to edit it; T edits the whole caption); and
  an inspector with **Templates** (the looks as cards), **Text** (font, weight, size, colour,
  capitals, outline, background, position) and **Highlight** (mode, word colour, box colour, pop).
  Numbers are hot text and commit when a drag ends. Automation ids: `liveCaptions.track|generate|
  recut|overflow`, `liveCaptions.words.1–4`, `liveCaptions.tab.*`, `liveCaptions.<id>.goto|row|
  edit|delete|text|overflow`, `liveCaptions.<id>.word.<n>`, `liveCaptions.preset.<name>`,
  `liveCaptions.highlight.<mode>`, `liveCaptions.section.*` and one per inspector control. UI state
  (`ui.inspect` → `live_captions`): `tab`, `words`, `collapsed`.

Known limits: words are joined with spaces, which is wrong for Chinese, Japanese and Thai; words
of Arabic and Hebrew captions are placed left to right; a sequence transcript heard through two
audio tracks of the same speech repeats its words; re-cutting a CEA-608/708 or Teletext track
makes word-timed captions those formats cannot show; caption file export writes the captions but
not their word timings.

## Burn-in

The Program monitor shows the visible caption tracks. On export, `file.exportMedia`
`{"burnCaptions": true}` (Export mode: **Burn Captions Into Video**) draws them into every frame.
Captions are set in the track's font (Inter, OFL, bundled, when none is chosen) in the track's font
style (SemiBold by default) by the text engine (`crates/text`: shaping, kerning, bidi);
italics/bold markup is not yet drawn differently. Word-timed captions are drawn word by word with
the word spoken at that frame highlighted ([Live captions](#live-captions)). The captions of a nested sequence
are part of the nest's picture: they are drawn wherever the nest is shown, in the Program monitor
and in every export, whether or not the outer sequence shows or burns in its own.

### Saving live-caption presets

In **Live Captions → Templates**, enter a name and choose **Save as new** to save
all of the current track's font, outline, position, background and spoken-word
highlight settings. Select a preset, make edits in **Text** or **Highlight**, then
choose **Save changes** to update that preset. Applying a preset changes the whole
track and can be undone.

Saved looks live in your preferences and are available in other projects and after
restarting. **Save changes** on a built-in look stores your own override;
**Reset preset** removes that override and restores its factory style on the track.
**Delete preset** removes a custom look from the library while keeping the current
track's style. Creating a look under an existing name requires an explicit update.
These presets store styling, not caption text, word timings or words-per-caption.

Agent commands:

- `captions.listPresets {}` lists factory and saved styles with `builtIn` and
  `customized` flags.
- `captions.savePreset {"track":"C1","name":"My look"}` saves a track style;
  pass `"overwrite":true` to update an existing look.
- `captions.deletePreset {"name":"My look"}` deletes a custom look or removes
  a built-in override.
- `captions.setStyle {"track":"C1","preset":"My look"}` applies a saved look.
  `captions.createLive` also accepts saved preset names.
