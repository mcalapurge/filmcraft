//! The Live Captions panel: short word-timed captions with the spoken word highlighted, edited the
//! way social video editors do it, drawn with the same parts as the Text and Essential Graphics
//! panels so it reads as one application in every theme.
//!
//! - **Toolbar:** the caption track, how many words a caption holds (1–4), **Generate**
//!   (transcribes the sequence when it has no transcript yet, then `captions.createLive`) and
//!   **Re-cut** (cuts the track again at the chosen word count, keeping the word timings).
//! - **Caption list:** rows like the Text panel's Captions tab, each caption's words as chips. The
//!   word under the playhead is lit in the track's highlight colour and the list follows the
//!   playhead; clicking a word goes to it, double-clicking edits it (`captions.setWord`), the T
//!   button edits the whole caption (`captions.setText`, which re-times the words), the bin deletes it.
//! - **Inspector:** Templates (the [`CAPTION_PRESETS`] looks as Essential Graphics–style cards),
//!   Text (font, weight, size, capitals, colour, outline, box, position) and Highlight (mode,
//!   colours, pop) in collapsible sections, all `captions.setStyle` on the track. Numbers are hot
//!   text and commit once when a drag ends, so a drag is one undo step.
//!
//! Colours come from the theme [`Tokens`]; the only fixed colours are the caption's own, drawn over
//! `monitor_bg` (the video stand-in, dark in every theme) so a preview looks like the result.
//! Every widget registers an automation id (`liveCaptions.*`); tab, word count and collapsed
//! sections are UI state.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use filmcraft_project::{CAPTION_PRESETS, CaptionAlign, CaptionAnchor, CaptionStyle, CaptionTrack, WordHighlight};
use filmcraft_time::{TimeDisplay, format_time};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;

/// The inspector tabs.
pub const TABS: [&str; 3] = ["Templates", "Text", "Highlight"];

/// Panel UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveCaptionsUi {
    /// Inspector tab (one of [`TABS`]).
    pub tab: String,
    /// Words per caption for Generate and Re-cut (1–4).
    pub words: u8,
    /// Inspector sections that are closed (`"Font"`, `"Outline"`, `"Position"`, `"Spoken word"`).
    pub collapsed: Vec<String>,
    pub selected_preset: String,
    pub preset_name: String,
}

impl Default for LiveCaptionsUi {
    fn default() -> Self {
        Self { tab: "Templates".into(), words: 3, collapsed: Vec::new(), selected_preset: String::new(), preset_name: String::new() }
    }
}

const TRACK_KEY: &str = "live-cap-track";

/// The word-timed captions of `tr` too wide for one line of a `w`×`h` frame, measured once per
/// project change (every edit makes a new project) rather than every frame.
fn overflowing(ui: &egui::Ui, project: &std::sync::Arc<filmcraft_project::Project>, tr: &CaptionTrack, w: u32, h: u32) -> std::sync::Arc<Vec<u64>> {
    type Cached = (usize, u64, std::sync::Arc<Vec<u64>>);
    let key = egui::Id::unique("live-cap-overflow");
    let at = std::sync::Arc::as_ptr(project) as usize;
    if let Some((p, t, ids)) = ui.data(|d| d.get_temp::<Cached>(key))
        && p == at
        && t == tr.id.0
    {
        return ids;
    }
    let ids = std::sync::Arc::new(filmcraft_engine::live_captions::overflowing(tr, w, h).iter().map(|c| c.0).collect::<Vec<_>>());
    ui.data_mut(|d| d.insert_temp::<Cached>(key, (at, tr.id.0, ids.clone())));
    ids
}

/// What each look is, one short line under its name on the card.
fn preset_blurb(name: &str) -> &'static str {
    match name {
        "Classic" => "Yellow word",
        "Pop" => "Green word grows",
        "Box" => "Purple word box",
        "Karaoke" => "Word by word",
        "Neon" => "Cyan on pink glow",
        "Banner" => "Yellow box on band",
        "Minimal" => "Soft white",
        _ => "Plain subtitles",
    }
}

fn c32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Black or white, whichever reads on `bg`.
fn on(bg: Color32) -> Color32 {
    let l = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if l > 150.0 { Color32::BLACK } else { Color32::WHITE }
}

/// The track the panel edits: the one picked, else the first with word-timed captions, else C1.
fn current_track(ui: &egui::Ui, tracks: &[CaptionTrack]) -> Option<usize> {
    let picked = ui.ctx().data(|d| d.get_temp::<u64>(egui::Id::unique(TRACK_KEY)));
    picked
        .and_then(|id| tracks.iter().position(|t| t.id.0 == id))
        .or_else(|| tracks.iter().position(|t| t.captions.iter().any(|c| !c.words.is_empty())))
        .or(if tracks.is_empty() { None } else { Some(0) })
}

/// A tab label with the accent underline when active (Essential Graphics' tabs).
fn tab_button(ui: &mut egui::Ui, r: Rect, label: &str, active: bool, t: &Tokens) -> egui::Response {
    let resp = ui.interact(r, egui::Id::unique(("live-cap-tab", label)), Sense::click());
    if active {
        ui.painter().line_segment([pos2(r.min.x + 6.0, r.max.y - 1.5), pos2(r.max.x - 6.0, r.max.y - 1.5)], Stroke::new(2.0, t.accent));
    }
    let color = if active || resp.hovered() { t.text } else { t.text_dim };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, crate::i18n::t(label), Tokens::semibold(12.0), color);
    resp
}

/// A toolbar icon button (the Text panel's).
fn tool_button(app: &mut FilmcraftApp, ui: &mut egui::Ui, r: Rect, icon: Icon, id: &str, label: &str) -> bool {
    let t = app.tokens;
    let resp = ui.interact(r, egui::Id::unique(id), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    icons::paint(ui.painter(), r.shrink(4.0), icon, t.icon);
    app.auto.add(id, r, label);
    resp.on_hover_text(label).clicked()
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    ui.painter().rect_filled(rect, 0.0, t.panel_bg);
    let project = app.session.project.clone();
    let Some(seq) = app.session.state.active_sequence.and_then(|id| project.sequence(id)) else {
        crate::dock::placeholder(ui, rect, &t, tl!("Open a sequence to make live captions"));
        return;
    };
    let mut actions: Vec<(String, Value)> = Vec::new();
    let has_transcript = !app.transcript_cache.content(&app.session).words.is_empty();
    let can_transcribe = filmcraft_engine::transcript::can_transcribe(&app.session).is_ok();
    let gen_enabled = has_transcript || can_transcribe;
    let track_idx = current_track(ui, &seq.caption_tracks);
    let track = track_idx.and_then(|i| seq.caption_tracks.get(i));
    let words = app.ui.live_captions.words.clamp(1, 4);
    let generate = move |actions: &mut Vec<(String, Value)>| {
        let create = json!({"maxWords": words, "source": "transcript"});
        if has_transcript {
            actions.push(("captions.createLive".into(), create));
        } else {
            // `then`: a transcription that runs as a background job (web) makes the captions when it ends
            actions.push(("transcript.generate".into(), json!({"then": [{"command": "captions.createLive", "params": create}]})));
        }
    };

    let Some((ti, tr)) = track_idx.zip(track) else {
        empty_state(app, ui, rect, gen_enabled, has_transcript, &mut actions, generate);
        run(app, ui, actions);
        return;
    };

    // ---- toolbar: track, words per caption, Generate, Re-cut
    let bar = Rect::from_min_size(rect.min + vec2(10.0, 4.0), vec2((rect.width() - 20.0).max(0.0), 26.0));
    let picker = Rect::from_min_size(pos2(bar.min.x, bar.min.y + 2.0), vec2(150.0f32.min(bar.width() * 0.35), 22.0));
    let presp = crate::widgets::dropdown_text(ui, picker, &format!("C{} · {}", ti + 1, tr.name), &t, egui::Id::unique("live-cap-track-picker"));
    app.auto.add("liveCaptions.track", picker, "Caption track");
    egui::Popup::menu(&presp).show(|ui| {
        for (i, other) in seq.caption_tracks.iter().enumerate() {
            if ui.selectable_label(i == ti, format!("C{} · {}", i + 1, other.name)).clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(TRACK_KEY), other.id.0));
            }
        }
    });
    let mut tb = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(picker.max.x + 12.0, bar.min.y), bar.max))
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt("live-cap-toolbar"),
    );
    tb.spacing_mut().item_spacing.x = 2.0;
    tb.label(egui::RichText::new(tl!("Words")).size(12.0).color(t.text_dim)).on_hover_text(tl!("Most words in one caption"));
    tb.add_space(4.0);
    for n in 1..=4u8 {
        let resp = tb.selectable_label(n == words, n.to_string());
        app.auto.add(&format!("liveCaptions.words.{n}"), resp.rect, &format!("{n} words per caption"));
        if resp.clicked() {
            app.ui.live_captions.words = n;
        }
    }
    tb.add_space(10.0);
    tb.spacing_mut().item_spacing.x = 6.0;
    let tip = if has_transcript {
        tl!("Make live captions from the sequence transcript")
    } else if can_transcribe {
        tl!("Transcribe the sequence, then make live captions")
    } else {
        tl!("This build has no speech-to-text; add a transcript first")
    };
    let resp = tb.add_enabled(gen_enabled, egui::Button::new(tl!("Generate"))).on_hover_text(tip).on_disabled_hover_text(tip);
    app.auto.add("liveCaptions.generate", resp.rect, "Generate");
    if resp.clicked() {
        generate(&mut actions);
    }
    let recut = !tr.captions.is_empty() && !tr.locked;
    let resp = tb
        .add_enabled(recut, egui::Button::new(tl!("Re-cut")))
        .on_hover_text(tl!("Cut this track again with the chosen number of words, keeping the word timings"));
    app.auto.add("liveCaptions.recut", resp.rect, "Re-cut");
    if resp.clicked() {
        actions.push(("captions.createLive".into(), json!({"source": "track", "track": tr.id.0, "maxWords": words})));
    }
    let over = overflowing(ui, &project, tr, seq.settings.width, seq.settings.height);
    if let Some(first) = over.first() {
        let label = tlf!("{count} too wide", count = over.len());
        let resp = tb
            .add(egui::Button::new(egui::RichText::new(label.as_str()).size(12.0).color(t.danger)).frame(false))
            .on_hover_text(tl!("Captions too wide for one line of the frame at this size. Re-cut fits them; click to go to the first."));
        app.auto.add("liveCaptions.overflow", resp.rect, &label);
        if resp.clicked() {
            actions.push(("captions.goTo".into(), json!({"caption": first})));
        }
    }

    // ---- body: caption list and inspector side by side, or stacked when narrow
    let body = Rect::from_min_max(pos2(rect.min.x, bar.max.y + 8.0), rect.max);
    let (list, insp) = if body.width() >= 560.0 {
        let iw = (body.width() * 0.42).clamp(260.0, 380.0);
        (
            Rect::from_min_max(body.min + vec2(6.0, 0.0), pos2(body.max.x - iw - 4.0, body.max.y - 4.0)),
            Rect::from_min_max(pos2(body.max.x - iw, body.min.y), body.max - vec2(6.0, 4.0)),
        )
    } else {
        let split = body.min.y + body.height() * 0.5;
        (
            Rect::from_min_max(body.min + vec2(6.0, 0.0), pos2(body.max.x - 6.0, split - 4.0)),
            Rect::from_min_max(pos2(body.min.x + 6.0, split + 4.0), body.max - vec2(6.0, 4.0)),
        )
    };
    caption_list(app, ui, list, seq, tr, &over, &mut actions);
    inspector(app, ui, insp, tr, (seq.settings.width, seq.settings.height), &mut actions);
    run(app, ui, actions);
}

/// No caption track yet: the Text panel's empty state, with Generate.
fn empty_state(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    rect: Rect,
    enabled: bool,
    has_transcript: bool,
    actions: &mut Vec<(String, Value)>,
    generate: impl Fn(&mut Vec<(String, Value)>),
) {
    let t = app.tokens;
    let c = rect.center();
    icons::paint(ui.painter(), Rect::from_center_size(c - vec2(0.0, 70.0), vec2(40.0, 40.0)), Icon::Captions, t.text_dim);
    ui.painter().text(c - vec2(0.0, 30.0), Align2::CENTER_CENTER, tl!("Live captions"), Tokens::semibold(16.0), t.text);
    ui.painter().text(c - vec2(0.0, 8.0), Align2::CENTER_CENTER, tl!("Short captions that light up each word as it is spoken."), Tokens::ui(12.0), t.text_dim);
    let note = if has_transcript {
        tl!("Made from the sequence transcript.")
    } else if enabled {
        tl!("The sequence is transcribed first.")
    } else {
        tl!("This build has no speech-to-text; add a transcript first.")
    };
    ui.painter().text(c + vec2(0.0, 10.0), Align2::CENTER_CENTER, note, Tokens::ui(11.5), t.text_faint);
    let r = Rect::from_center_size(c + vec2(0.0, 42.0), vec2(200.0, 26.0));
    let id = "liveCaptions.generate";
    let resp = ui.interact(r, egui::Id::unique(id), if enabled { Sense::click() } else { Sense::hover() });
    let fill = match (enabled, resp.hovered()) {
        (false, _) => t.field_bg,
        (true, true) => t.accent_hover,
        (true, false) => t.accent,
    };
    ui.painter().rect_filled(r, 13.0, fill);
    let label = tl!("Generate live captions");
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, Tokens::semibold(12.0), if enabled { Color32::WHITE } else { t.text_faint });
    app.auto.add(id, r, label);
    if resp.clicked() {
        generate(actions);
    }
}

/// The captions as rows (the Text panel's Captions list): number and in point, then the words.
fn caption_list(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    rect: Rect,
    seq: &filmcraft_project::Sequence,
    tr: &CaptionTrack,
    over: &[u64],
    actions: &mut Vec<(String, Value)>,
) {
    let t = app.tokens;
    ui.painter().rect_filled(rect, 3.0, t.app_bg);
    let (rate, df) = (seq.settings.frame_rate, seq.settings.drop_frame);
    let tc = |x: filmcraft_time::Tick| format_time(x, rate, df, TimeDisplay::Timecode, 48_000);
    let ph = app.session.playhead();
    let current = tr.caption_at(ph).map(|c| c.id);
    if current.is_none() {
        ui.data_mut(|d| d.remove::<u64>(egui::Id::unique("live-caption-follow")));
    }
    let sel = app.session.state.caption_selection.clone();
    let hi = c32(tr.style.highlight_color);
    let editing_key = egui::Id::unique("live-cap-editing");
    let editing: Option<(u64, Option<usize>)> = ui.ctx().data(|d| d.get_temp(editing_key));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(4.0)).id_salt("live-caption-list"));
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("live-caption-scroll").show(&mut child, |ui| {
        ui.set_width(rect.width() - 12.0);
        if tr.captions.is_empty() {
            ui.label(egui::RichText::new(tl!("No captions on this track. Generate live captions to fill it.")).color(t.text_faint));
        }
        for (n, c) in tr.captions.iter().enumerate() {
            let is_current = current == Some(c.id);
            let fill = if sel.contains(&c.id) {
                t.row_selected
            } else if n % 2 == 1 {
                t.row_alt
            } else {
                Color32::TRANSPARENT
            };
            let frame = egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(6, 4)).corner_radius(3.0);
            let fr = frame.show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    // number and in point; a click goes to the caption
                    ui.vertical(|ui| {
                        ui.set_width(84.0);
                        ui.label(egui::RichText::new(format!("{}", n + 1)).size(11.0).color(t.text_dim));
                        let goto = ui.add(egui::Button::new(egui::RichText::new(tc(c.start)).font(Tokens::mono(11.0)).color(t.hot_text)).frame(false));
                        app.auto.add(&format!("liveCaptions.{}.goto", c.id.0), goto.rect, "Go to caption");
                        if goto.clicked() {
                            actions.push(("captions.goTo".into(), json!({"caption": c.id.0})));
                        }
                        if over.contains(&c.id.0) {
                            let warn = ui.label(egui::RichText::new(tl!("Too wide")).size(11.0).color(t.danger));
                            app.auto.add(&format!("liveCaptions.{}.overflow", c.id.0), warn.rect, "Too wide");
                            warn.on_hover_text(tl!("Wider than one line of the frame at this size, so it wraps. Re-cut fits it."));
                        }
                    });
                    // tools at the right edge, then the words in what is left
                    let tools_w = 48.0;
                    let words_w = (ui.available_width() - tools_w).max(40.0);
                    ui.vertical(|ui| {
                        ui.set_width(words_w);
                        caption_words(app, ui, c, tr, is_current, ph, hi, editing, editing_key, &tc, actions);
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        for (icon, id, tip) in [(Icon::Trash, "delete", tl!("Delete caption")), (Icon::Type, "edit", tl!("Edit caption text"))] {
                            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            if tool_button(app, ui, r, icon, &format!("liveCaptions.{}.{id}", c.id.0), tip) {
                                if id == "delete" {
                                    actions.push(("captions.delete".into(), json!({"captions": [c.id.0]})));
                                } else {
                                    ui.ctx().data_mut(|d| d.insert_temp(editing_key, (c.id.0, None::<usize>)));
                                }
                            }
                        }
                    });
                });
            });
            let row = fr.response.rect;
            if is_current {
                ui.painter().rect_stroke(row, 3.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
                // the list follows the playhead: each caption it reaches is brought to the middle
                // once, but never while a field is being edited
                let follow = egui::Id::unique("live-caption-follow");
                if ui.data(|d| d.get_temp::<u64>(follow)) != Some(c.id.0) && ui.memory(|m| m.focused().is_none()) {
                    ui.scroll_to_rect(row, Some(egui::Align::Center));
                    ui.data_mut(|d| d.insert_temp(follow, c.id.0));
                }
            }
            app.auto.add(&format!("liveCaptions.{}.row", c.id.0), row, &c.text);
            ui.add_space(2.0);
        }
    });
}

/// A caption's words as chips (or the editor for the caption or one word).
#[allow(clippy::too_many_arguments)]
fn caption_words(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    c: &filmcraft_project::Caption,
    tr: &CaptionTrack,
    is_current: bool,
    ph: filmcraft_time::Tick,
    hi: Color32,
    editing: Option<(u64, Option<usize>)>,
    editing_key: egui::Id,
    tc: &dyn Fn(filmcraft_time::Tick) -> String,
    actions: &mut Vec<(String, Value)>,
) {
    let t = app.tokens;
    if editing == Some((c.id.0, None)) {
        let key = egui::Id::unique(("live-cap-text", c.id.0));
        let mut buf = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| c.text.clone());
        let resp = ui.add(egui::TextEdit::singleline(&mut buf).id(key.with("te")).desired_width(ui.available_width()).font(Tokens::ui(12.5)));
        app.auto.add(&format!("liveCaptions.{}.text", c.id.0), resp.rect, "Caption text");
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(key, buf.clone()));
        if resp.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(key));
            ui.ctx().data_mut(|d| d.remove::<(u64, Option<usize>)>(editing_key));
            if buf != c.text && !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                actions.push(("captions.setText".into(), json!({"caption": c.id.0, "text": buf})));
            }
        }
        return;
    }
    if c.words.is_empty() {
        ui.label(egui::RichText::new(c.plain_lines().join(" ")).size(12.5).color(t.text));
        ui.label(egui::RichText::new(tl!("No word timings; Re-cut the track to animate it.")).size(11.0).color(t.text_faint));
        return;
    }
    let spoken = if is_current { c.word_at(ph - c.start) } else { None };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        ui.spacing_mut().button_padding = vec2(5.0, 1.0);
        for (i, w) in c.words.iter().enumerate() {
            let auto_id = format!("liveCaptions.{}.word.{i}", c.id.0);
            if editing == Some((c.id.0, Some(i))) {
                let key = egui::Id::unique(("live-cap-word", c.id.0, i));
                let mut buf = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| w.text.clone());
                let width = (buf.chars().count() as f32 * 8.0 + 20.0).clamp(36.0, 200.0);
                let resp = ui.add(egui::TextEdit::singleline(&mut buf).id(key.with("te")).desired_width(width).font(Tokens::ui(12.5)));
                app.auto.add(&auto_id, resp.rect, &w.text);
                if !resp.has_focus() && !resp.lost_focus() {
                    resp.request_focus();
                }
                ui.data_mut(|d| d.insert_temp(key, buf.clone()));
                if resp.lost_focus() {
                    ui.data_mut(|d| d.remove::<String>(key));
                    ui.ctx().data_mut(|d| d.remove::<(u64, Option<usize>)>(editing_key));
                    if buf != w.text && !ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                        actions.push(("captions.setWord".into(), json!({"caption": c.id.0, "index": i, "text": buf})));
                    }
                }
                continue;
            }
            let lit = spoken == Some(i);
            let text = if tr.style.uppercase { w.text.to_uppercase() } else { w.text.clone() };
            let rich = if lit {
                egui::RichText::new(text).font(Tokens::semibold(12.5)).color(on(hi))
            } else {
                egui::RichText::new(text).font(Tokens::ui(12.5)).color(t.text)
            };
            // a chip never wraps inside itself; a word longer than the column ends in "…" and the
            // tooltip has it whole
            let chip = egui::Button::new(rich).fill(if lit { hi } else { t.field_bg }).stroke(Stroke::NONE).corner_radius(t.radius_sm);
            let resp = ui.add(chip.wrap_mode(egui::TextWrapMode::Truncate));
            app.auto.add(&auto_id, resp.rect, &w.text);
            let resp = resp.on_hover_text(format!("{} · {} – {}", w.text, tc(c.start + w.start), tc(c.start + w.end)));
            if resp.double_clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(editing_key, (c.id.0, Some(i))));
            } else if resp.clicked() {
                actions.push(("playhead.set".into(), json!({"time": (c.start + w.start).0})));
            }
        }
    });
}

/// A preview of a look on the video stand-in: three words with the middle one spoken.
fn caption_sample(ui: &egui::Ui, r: Rect, st: &CaptionStyle, t: &Tokens) {
    let p = ui.painter();
    p.rect_filled(r, 2.0, t.monitor_bg);
    let words = ["SAY", "IT", "LOUD"];
    let text = |w: &str| if st.uppercase { w.to_string() } else { w.to_lowercase() };
    let measure = |size: f32| {
        let font = FontId::new(size, Tokens::semibold(size).family);
        let galleys: Vec<_> = words.iter().map(|w| p.layout_no_wrap(text(w), font.clone(), Color32::WHITE)).collect();
        let gap = size * 0.35;
        let total: f32 = galleys.iter().map(|g| g.size().x).sum::<f32>() + gap * (galleys.len() as f32 - 1.0);
        (font, galleys, gap, total)
    };
    // the sample fits its frame with a margin, however narrow it is
    let size = (r.height() * 0.3).clamp(9.0, 15.0);
    let (_, _, _, wide) = measure(size);
    let room = (r.width() - 16.0).max(1.0);
    let size = if wide > room { (size * room / wide).max(6.0) } else { size };
    let (font, galleys, gap, total) = measure(size);
    let mut x = r.center().x - total / 2.0;
    let y = r.center().y;
    if st.background {
        p.rect_filled(Rect::from_center_size(pos2(r.center().x, y), vec2(total + 10.0, size * 1.5)), 2.0, c32(st.background_color));
    }
    for (i, g) in galleys.iter().enumerate() {
        let spoken = i == 1;
        let pos = pos2(x, y - g.size().y / 2.0);
        if spoken && st.highlight == WordHighlight::Box {
            p.rect_filled(Rect::from_min_size(pos, g.size()).expand2(vec2(3.0, 1.0)), 3.0, c32(st.highlight_box_color));
        }
        let hidden = st.highlight == WordHighlight::Reveal && i > 1;
        if !hidden {
            let color = if spoken && st.highlight != WordHighlight::Off { c32(st.highlight_color) } else { c32(st.color) };
            if st.outline > 0.0 {
                let o = c32(st.outline_color);
                for d in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
                    p.text(pos + d, Align2::LEFT_TOP, text(words[i]), font.clone(), o);
                }
            }
            p.text(pos, Align2::LEFT_TOP, text(words[i]), font.clone(), color);
        }
        x += g.size().x + gap;
    }
}

fn style_cmd(actions: &mut Vec<(String, Value)>, track: &CaptionTrack, key: &str, v: Value) {
    let mut p = json!({"track": track.id.0});
    p[key] = v;
    actions.push(("captions.setStyle".into(), p));
}

fn inspector(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect, tr: &CaptionTrack, frame: (u32, u32), actions: &mut Vec<(String, Value)>) {
    let t = app.tokens;
    // tabs over a separator, as in Essential Graphics
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 28.0));
    ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.separator));
    let mut x = head.min.x;
    for tab in TABS {
        let w = ui.painter().layout_no_wrap(crate::i18n::t(tab).to_string(), Tokens::semibold(12.0), t.text).size().x + 20.0;
        let r = Rect::from_min_size(pos2(x, head.min.y), vec2(w, head.height()));
        if tab_button(ui, r, tab, app.ui.live_captions.tab == tab, &t).clicked() {
            app.ui.live_captions.tab = tab.to_string();
        }
        app.auto.add(&format!("liveCaptions.tab.{tab}"), r, tab);
        x += w + 4.0;
    }
    let body = Rect::from_min_max(pos2(rect.min.x, head.max.y + 6.0), rect.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("live-cap-inspector"));
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("live-cap-inspector-scroll").show(&mut child, |ui| {
        ui.set_width(body.width() - 8.0);
        match app.ui.live_captions.tab.as_str() {
            "Text" => text_tab(app, ui, tr, actions),
            "Highlight" => highlight_tab(app, ui, tr, actions),
            _ => templates_tab(app, ui, tr, frame, actions),
        }
    });
}

/// The looks as Essential Graphics–style cards: a preview, the name and what it does.
fn templates_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui, tr: &CaptionTrack, _frame: (u32, u32), actions: &mut Vec<(String, Value)>) {
    let t = app.tokens;
    let names = filmcraft_engine::live_captions::preset_names(&app.session);
    if app.ui.live_captions.selected_preset.is_empty()
        && let Some(name) = names
            .iter()
            .find(|name| filmcraft_engine::live_captions::preset_style(&app.session, name, "captions.setStyle").is_ok_and(|style| style == tr.style))
    {
        app.ui.live_captions.selected_preset = name.clone();
    }
    ui.label(tl!("Save the current track style as a reusable look."));
    let entry = ui.add(
        egui::TextEdit::singleline(&mut app.ui.live_captions.preset_name).hint_text(tl!("New preset name")).char_limit(80).desired_width(ui.available_width()),
    );
    app.auto.add("liveCaptions.presetName", entry.rect, "New preset name");
    ui.horizontal_wrapped(|ui| {
        let save = ui.add_enabled(!app.ui.live_captions.preset_name.trim().is_empty(), egui::Button::new(tl!("Save as new")));
        app.auto.add("liveCaptions.savePreset", save.rect, "Save as new");
        if save.clicked() {
            let name = app.ui.live_captions.preset_name.trim().to_string();
            actions.push(("captions.savePreset".into(), json!({"track": tr.id.0, "name": name})));
            app.ui.live_captions.selected_preset = name;
        }
        let selected = app.ui.live_captions.selected_preset.clone();
        let exists = names.iter().any(|name| name.eq_ignore_ascii_case(&selected));
        let update = ui.add_enabled(exists, egui::Button::new(tl!("Save changes"))).on_hover_text(tlf!("Update preset: {name}", name = selected));
        app.auto.add("liveCaptions.updatePreset", update.rect, "Save changes");
        if update.clicked() {
            actions.push(("captions.savePreset".into(), json!({"track": tr.id.0, "name": selected, "overwrite": true})));
        }
        let built_in = CAPTION_PRESETS.iter().any(|name| name.eq_ignore_ascii_case(&selected));
        let saved = app.session.prefs.caption_presets.iter().any(|preset| preset.name.eq_ignore_ascii_case(&selected));
        let label = if built_in { tl!("Reset preset") } else { tl!("Delete preset") };
        let remove = ui.add_enabled(saved, egui::Button::new(label));
        app.auto.add("liveCaptions.deletePreset", remove.rect, label);
        if remove.clicked() {
            actions.push(("captions.deletePreset".into(), json!({"name": selected})));
            if built_in {
                style_cmd(actions, tr, "preset", json!(selected));
            } else {
                app.ui.live_captions.selected_preset.clear();
            }
        }
    });
    if !app.ui.live_captions.selected_preset.is_empty() {
        ui.label(tlf!("Selected preset: {name}", name = app.ui.live_captions.selected_preset));
    }
    ui.add_space(8.0);
    let gap = 8.0;
    let cols = ((ui.available_width() + gap) / (96.0 + gap)).floor().clamp(1.0, 3.0) as usize;
    let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    let card = vec2(cw, 52.0 + 36.0);
    let rows = names.len().div_ceil(cols);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), rows as f32 * (card.y + gap)), Sense::hover());
    for (i, name) in names.iter().enumerate() {
        // placed for this sequence's frame, as applying it places it
        let Ok(style) = filmcraft_engine::live_captions::preset_style(&app.session, name, "captions.setStyle") else { continue };
        let r = Rect::from_min_size(area.min + vec2((i % cols) as f32 * (cw + gap), (i / cols) as f32 * (card.y + gap)), card);
        let id = format!("liveCaptions.preset.{name}");
        let resp = ui.interact(r, egui::Id::unique(&id), Sense::click());
        let selected = tr.style == style;
        let fill = if selected {
            t.row_selected
        } else if resp.hovered() {
            t.hover
        } else {
            t.field_bg
        };
        ui.painter().rect_filled(r, 4.0, fill);
        let pr = Rect::from_min_size(r.min + vec2(5.0, 5.0), vec2(r.width() - 10.0, 46.0));
        caption_sample(ui, pr, &style, &t);
        ui.painter().text(pos2(r.min.x + 6.0, pr.max.y + 5.0), Align2::LEFT_TOP, crate::i18n::t(name), Tokens::ui(11.5), t.text);
        let customized = app.session.prefs.caption_presets.iter().any(|p| p.name.eq_ignore_ascii_case(name));
        let description = if customized { "Saved look" } else { preset_blurb(name) };
        let blurb = ui.painter().layout_no_wrap(crate::i18n::t(description).to_string(), Tokens::ui(10.0), t.text_faint);
        let clip = Rect::from_min_max(pos2(r.min.x, pr.max.y + 19.0), r.max - vec2(0.0, 2.0));
        ui.painter().with_clip_rect(clip).galley(pos2(r.min.x + 6.0, pr.max.y + 19.0), blurb, t.text_faint);
        if selected {
            ui.painter().rect_stroke(r, 4.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
        }
        app.auto.add(&id, r, name);
        if resp.on_hover_text(tl!("Apply this look to the track")).clicked() {
            app.ui.live_captions.selected_preset = name.clone();
            style_cmd(actions, tr, "preset", json!(name));
        }
    }
    ui.label(egui::RichText::new(tl!("A look sets the whole track style; fine-tune it in Text and Highlight.")).size(11.0).color(t.text_faint));
}

/// A collapsible section (Essential Sound's), open unless the user closed it.
fn section(app: &mut FilmcraftApp, ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut FilmcraftApp, &mut egui::Ui)) {
    let t = app.tokens;
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
    let open = !app.ui.live_captions.collapsed.iter().any(|s| s == title);
    let (resp, now) = crate::widgets::section_header(ui, egui::Id::unique(("live-cap-section", title)), crate::i18n::t(title), open, &t, true);
    app.auto.add(&format!("liveCaptions.section.{title}"), resp.rect, title);
    if now != open {
        if now {
            app.ui.live_captions.collapsed.retain(|s| s != title);
        } else {
            app.ui.live_captions.collapsed.push(title.to_string());
        }
    }
    if now {
        ui.add_space(2.0);
        add(app, ui);
        ui.add_space(6.0);
    }
}

/// A property row: the label in the left column, the controls after it (Essential Graphics' rows).
fn prop_row(ui: &mut egui::Ui, label: &str, t: &Tokens, add: impl FnOnce(&mut egui::Ui)) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 20.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(12.0), t.text_dim);
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(r.min.x + 100.0, r.min.y + 2.0), pos2(r.max.x - 4.0, r.max.y - 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    add(&mut row);
}

/// A hot-text number that commits once: dragging shows the value as it goes, and the release (or a
/// typed value) returns it, so a drag is one `captions.setStyle` and one undo step.
#[allow(clippy::too_many_arguments)]
fn hot(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: f64,
    speed: f64,
    range: (f64, f64),
    decimals: usize,
    suffix: &str,
) -> Option<f64> {
    let t = app.tokens;
    let key = egui::Id::unique(("live-cap-hot", id));
    let draft = key.with("draft");
    let shown = ui.data(|d| d.get_temp::<f64>(draft)).unwrap_or(value);
    let (resp, nv) = crate::widgets::hot_number(ui, label, key, shown, crate::widgets::HotNumberOptions::new(speed, range, decimals, suffix, &t));
    app.auto.add(id, resp.rect, label);
    if resp.dragged() {
        if let Some(v) = nv {
            ui.data_mut(|d| d.insert_temp(draft, v));
        }
        return None;
    }
    if resp.drag_stopped() {
        let v = ui.data_mut(|d| d.remove_temp::<f64>(draft));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return None;
        }
        return v.filter(|v| (v - value).abs() > f64::EPSILON);
    }
    nv
}

fn color_button(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: [u8; 4],
    tr: &CaptionTrack,
    key: &str,
    actions: &mut Vec<(String, Value)>,
) {
    let mut col = c32(value);
    let resp = egui::color_picker::color_edit_button_srgba(ui, &mut col, egui::color_picker::Alpha::OnlyBlend);
    app.auto.add(id, resp.rect, label);
    if resp.changed() {
        style_cmd(actions, tr, key, json!(col.to_srgba_unmultiplied()));
    }
}

fn text_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui, tr: &CaptionTrack, actions: &mut Vec<(String, Value)>) {
    let t = app.tokens;
    let st = tr.style.clone();
    section(app, ui, "Font", |app, ui| {
        prop_row(ui, tl!("Font"), &t, |ui| {
            let r = egui::ComboBox::from_id_salt(("live-cap-font", tr.id.0))
                .selected_text(&st.font)
                .width(ui.available_width().min(170.0))
                .height(400.0)
                .show_ui(ui, |ui| {
                    if !filmcraft_text::fonts::system_scanned() {
                        filmcraft_text::fonts::scan_system();
                    }
                    for (f, _) in filmcraft_text::families().into_iter().filter(|(f, _)| !f.starts_with('.')) {
                        let resp = ui.selectable_label(f.eq_ignore_ascii_case(&st.font), &f);
                        let visible = resp.rect.intersect(ui.clip_rect());
                        if visible.is_positive() {
                            app.auto.add(&format!("liveCaptions.font.option.{f}"), visible, &f);
                        }
                        if resp.clicked() {
                            style_cmd(actions, tr, "font", json!(f));
                        }
                    }
                });
            app.auto.add("liveCaptions.font", r.response.rect, "Caption font");
        });
        prop_row(ui, tl!("Weight"), &t, |ui| {
            let r = egui::ComboBox::from_id_salt(("live-cap-weight", tr.id.0)).selected_text(crate::i18n::t(&st.font_style)).width(110.0).show_ui(ui, |ui| {
                for w in ["Regular", "Medium", "SemiBold", "Bold"] {
                    let resp = ui.selectable_label(st.font_style.eq_ignore_ascii_case(w), crate::i18n::t(w));
                    app.auto.add(&format!("liveCaptions.weight.{w}"), resp.rect, w);
                    if resp.clicked() {
                        style_cmd(actions, tr, "fontStyle", json!(w));
                    }
                }
            });
            app.auto.add("liveCaptions.weight", r.response.rect, "Font weight");
        });
        prop_row(ui, tl!("Size"), &t, |ui| {
            if let Some(v) = hot(app, ui, "liveCaptions.size", "Caption size", st.size as f64, 0.5, (8.0, 200.0), 0, " px") {
                style_cmd(actions, tr, "size", json!(v));
            }
        });
        prop_row(ui, tl!("Color"), &t, |ui| color_button(app, ui, "liveCaptions.color", "Text colour", st.color, tr, "color", actions));
        prop_row(ui, tl!("Capitals"), &t, |ui| {
            let mut caps = st.uppercase;
            let resp = ui.checkbox(&mut caps, "");
            app.auto.add("liveCaptions.uppercase", resp.rect, "Capitals");
            if resp.changed() {
                style_cmd(actions, tr, "uppercase", json!(caps));
            }
        });
    });
    section(app, ui, "Outline", |app, ui| {
        prop_row(ui, tl!("Stroke"), &t, |ui| {
            if let Some(v) = hot(app, ui, "liveCaptions.outline", "Outline width", st.outline as f64, 0.2, (0.0, 40.0), 1, " px") {
                style_cmd(actions, tr, "outline", json!(v));
            }
            ui.add_space(8.0);
            color_button(app, ui, "liveCaptions.outlineColor", "Outline colour", st.outline_color, tr, "outlineColor", actions);
        });
        prop_row(ui, tl!("Background"), &t, |ui| {
            let mut bg = st.background;
            let resp = ui.checkbox(&mut bg, "");
            app.auto.add("liveCaptions.background", resp.rect, "Background box");
            if resp.changed() {
                style_cmd(actions, tr, "background", json!(bg));
            }
            color_button(app, ui, "liveCaptions.backgroundColor", "Box colour", st.background_color, tr, "backgroundColor", actions);
        });
    });
    section(app, ui, "Position", |app, ui| {
        prop_row(ui, tl!("Anchor"), &t, |ui| {
            for (a, name, label) in
                [(CaptionAnchor::Top, "top", tl!("Top")), (CaptionAnchor::Middle, "middle", tl!("Middle")), (CaptionAnchor::Bottom, "bottom", tl!("Bottom"))]
            {
                let resp = ui.selectable_label(st.anchor == a, label);
                app.auto.add(&format!("liveCaptions.anchor.{name}"), resp.rect, label);
                if resp.clicked() {
                    style_cmd(actions, tr, "anchor", json!(name));
                }
            }
        });
        prop_row(ui, tl!("Offset"), &t, |ui| {
            if let Some(v) = hot(app, ui, "liveCaptions.margin", "Distance from the edge", (st.margin * 100.0) as f64, 0.2, (0.0, 45.0), 0, "%") {
                style_cmd(actions, tr, "margin", json!(v / 100.0));
            }
        });
        prop_row(ui, tl!("Align"), &t, |ui| {
            for (a, name, label) in
                [(CaptionAlign::Left, "left", tl!("Left")), (CaptionAlign::Center, "center", tl!("Center")), (CaptionAlign::Right, "right", tl!("Right"))]
            {
                let resp = ui.selectable_label(st.align == a, label);
                app.auto.add(&format!("liveCaptions.align.{name}"), resp.rect, label);
                if resp.clicked() {
                    style_cmd(actions, tr, "align", json!(name));
                }
            }
        });
    });
}

fn highlight_tab(app: &mut FilmcraftApp, ui: &mut egui::Ui, tr: &CaptionTrack, actions: &mut Vec<(String, Value)>) {
    let t = app.tokens;
    let st = tr.style.clone();
    section(app, ui, "Spoken word", |app, ui| {
        // the four modes as small cards, each previewing itself in the track's colours
        let gap = 6.0;
        let w = ((ui.available_width() - 20.0 - gap * 3.0) / 4.0).max(40.0);
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
        for (i, h) in WordHighlight::ALL.into_iter().enumerate() {
            let r = Rect::from_min_size(area.min + vec2(20.0 + i as f32 * (w + gap), 0.0), vec2(w, 54.0));
            let id = format!("liveCaptions.highlight.{}", h.label().to_lowercase());
            let resp = ui.interact(r, egui::Id::unique(&id), Sense::click());
            let active = st.highlight == h;
            let fill = if active {
                t.row_selected
            } else if resp.hovered() {
                t.hover
            } else {
                t.field_bg
            };
            ui.painter().rect_filled(r, 4.0, fill);
            let pr = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(r.width() - 8.0, 28.0));
            ui.painter().rect_filled(pr, 2.0, t.monitor_bg);
            let s = pr.center();
            let f = Tokens::semibold(11.0);
            let (base, lit) = (c32(st.color), c32(st.highlight_color));
            match h {
                WordHighlight::Off => {
                    ui.painter().text(s, Align2::CENTER_CENTER, "Aa", f, base);
                }
                WordHighlight::Color => {
                    ui.painter().text(s, Align2::CENTER_CENTER, "Aa", f, lit);
                }
                WordHighlight::Box => {
                    ui.painter().rect_filled(Rect::from_center_size(s, vec2(22.0, 15.0)), 3.0, c32(st.highlight_box_color));
                    ui.painter().text(s, Align2::CENTER_CENTER, "Aa", f, lit);
                }
                WordHighlight::Reveal => {
                    ui.painter().text(s - vec2(5.0, 0.0), Align2::CENTER_CENTER, "A", f.clone(), lit);
                    ui.painter().text(s + vec2(5.0, 0.0), Align2::CENTER_CENTER, "a", f, base.gamma_multiply(0.25));
                }
            }
            let label = crate::i18n::t(h.label());
            ui.painter().text(pos2(r.center().x, r.max.y - 10.0), Align2::CENTER_CENTER, label, Tokens::ui(11.0), if active { t.text } else { t.text_dim });
            if active {
                ui.painter().rect_stroke(r, 4.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
            }
            app.auto.add(&id, r, label);
            if resp.clicked() {
                style_cmd(actions, tr, "highlight", json!(h.label().to_lowercase()));
            }
        }
        if st.highlight == WordHighlight::Off {
            prop_row(ui, "", &t, |ui| {
                ui.label(egui::RichText::new(tl!("Every word looks the same.")).size(11.0).color(t.text_faint));
            });
            return;
        }
        prop_row(ui, tl!("Word color"), &t, |ui| {
            color_button(app, ui, "liveCaptions.highlightColor", "Spoken word colour", st.highlight_color, tr, "highlightColor", actions)
        });
        if st.highlight == WordHighlight::Box {
            prop_row(ui, tl!("Box color"), &t, |ui| {
                color_button(app, ui, "liveCaptions.highlightBoxColor", "Box colour", st.highlight_box_color, tr, "highlightBoxColor", actions)
            });
        }
        prop_row(ui, tl!("Pop"), &t, |ui| {
            if let Some(v) = hot(app, ui, "liveCaptions.highlightScale", "Spoken word size", (st.highlight_scale * 100.0) as f64, 0.25, (100.0, 150.0), 0, "%")
            {
                style_cmd(actions, tr, "highlightScale", json!(v / 100.0));
            }
        });
    });
}

fn run(app: &mut FilmcraftApp, ui: &egui::Ui, actions: Vec<(String, Value)>) {
    let ctx = ui.ctx().clone();
    for (cmd, p) in actions {
        if let Err(e) = crate::menus::invoke(app, &ctx, &cmd, p) {
            app.ui.status = e;
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_text_on_highlight() {
        assert_eq!(on(Color32::from_rgb(255, 214, 10)), Color32::BLACK);
        assert_eq!(on(Color32::from_rgb(124, 77, 255)), Color32::WHITE);
    }

    #[test]
    fn ui_state_defaults_and_round_trips() {
        let d = LiveCaptionsUi::default();
        assert_eq!((d.tab.as_str(), d.words), ("Templates", 3));
        assert!(d.collapsed.is_empty());
        let back: LiveCaptionsUi = serde_json::from_str("{}").unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn every_look_has_a_description() {
        for name in CAPTION_PRESETS {
            assert!(!preset_blurb(name).is_empty());
        }
        assert_ne!(preset_blurb("Classic"), preset_blurb("Subtitle"));
    }
}
