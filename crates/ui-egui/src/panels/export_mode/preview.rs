use super::settings::{combo, drag};
use super::*;

// ---------------------------------------------------------------------------------------------
// right: preview, range, summary, buttons
// ---------------------------------------------------------------------------------------------

pub(crate) const RANGES: [(&str, &str); 4] = [("entire", "Entire Source"), ("inOut", "Source In/Out"), ("workArea", "Work Area"), ("custom", "Custom")];

pub(super) fn preview_column(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect, seq_id: ItemId) {
    let t = app.tokens;
    let mut reg = Reg::default();
    let Some(q) = app.session.project.sequence(seq_id).cloned() else { return };
    let rate = q.settings.frame_rate;
    // preview (aspect of the output frame)
    let r = app.ui.export.settings.resolve(q.settings.width, q.settings.height, rate, q.settings.sample_rate);
    let pv_area = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(rect.width() - 28.0, (rect.height() * 0.42).max(160.0)));
    let pic = crate::panels::monitor::fit(pv_area, r.width as f32, r.height as f32);
    ui.painter().rect_filled(pic, 0.0, Color32::BLACK);
    if app.ui.export.settings.has_video() {
        // the sequence's own playhead (it need not be the active one)
        let frame = rate.frame_at(app.session.state.playheads.get(&seq_id).copied().unwrap_or_default());
        let key = FrameKey { target: Target::Sequence(seq_id), frame, size: 500, revision: app.session.revision, draft: false };
        let project = app.session.project.clone();
        app.frames.request(key, rate.tick_of(frame), 0.5, &project, 0);
        if let Some(img) = app.frames.get(&key) {
            let ctx = ui.ctx().clone();
            let tex = app.texture_for(&ctx, "export-preview", key, &img);
            let inner = match app.ui.export.settings.scaling {
                Scaling::StretchToFill => pic,
                Scaling::ScaleToFit => crate::panels::monitor::fit(pic, q.settings.width as f32, q.settings.height as f32),
                Scaling::ScaleToFill => {
                    let s = (pic.width() / q.settings.width as f32).max(pic.height() / q.settings.height as f32);
                    Rect::from_center_size(pic.center(), vec2(q.settings.width as f32 * s, q.settings.height as f32 * s))
                }
            };
            ui.painter().with_clip_rect(pic).image(tex, inner, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    } else {
        ui.painter().text(pic.center(), Align2::CENTER_CENTER, tl!("Audio only"), Tokens::ui(13.0), t.text_dim);
    }
    reg.add("export.preview", pic, "Preview");
    // range + scaling
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.min.x + 14.0, pv_area.max.y + 8.0), pos2(rect.max.x - 14.0, rect.max.y - 50.0)))
            .id_salt("export-right"),
    );
    let ex = &mut app.ui.export;
    let has_work_area = q.work_area.is_some();
    child.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Range")).color(t.text_dim));
        let cur = RANGES.iter().find(|(k, _)| *k == ex.range).map(|x| x.1).unwrap_or("Entire Source");
        let labels: Vec<(String, bool)> = RANGES.iter().map(|(k, l)| (l.to_string(), *k != "workArea" || has_work_area)).collect();
        if let Some(i) = combo(ui, &mut reg, "export.range", cur, &labels, 140.0) {
            ex.range = RANGES[i].0.into();
        }
        if ex.range == "custom" {
            drag(ui, &mut reg, "export.range.start", &mut ex.custom_start, 0.0..=86_400.0, 0.04, " s", 2);
            drag(ui, &mut reg, "export.range.end", &mut ex.custom_end, 0.0..=86_400.0, 0.04, " s", 2);
        }
    });
    if ex.settings.has_video() {
        child.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Scaling")).color(t.text_dim));
            let o =
                [(Scaling::ScaleToFit, tl!("Scale To Fit")), (Scaling::ScaleToFill, tl!("Scale To Fill")), (Scaling::StretchToFill, tl!("Stretch To Fill"))];
            let cur = o.iter().find(|(x, _)| *x == ex.settings.scaling).map_or(o[0].1, |x| x.1);
            let labels: Vec<(String, bool)> = o.iter().map(|(_, l)| (l.to_string(), true)).collect();
            if let Some(i) = combo(ui, &mut reg, "export.scaling", cur, &labels, 140.0) {
                ex.settings.scaling = o[i].0;
            }
        });
    }
    // summary
    let range = range_ticks(ex, &q);
    let sum = ex.settings.summary(q.settings.width, q.settings.height, rate, q.settings.sample_rate, range.1 - range.0);
    child.add_space(8.0);
    let resp = egui::Frame::NONE
        .fill(t.field_bg)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10))
        .show(&mut child, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(tl!("Summary")).strong());
            egui::Grid::new("export-summary-grid").num_columns(2).spacing(vec2(10.0, 3.0)).show(ui, |ui| {
                let line = |ui: &mut egui::Ui, k: &str, v: &str| {
                    ui.label(egui::RichText::new(crate::i18n::t(k)).color(t.text_dim).size(11.5));
                    ui.add(egui::Label::new(egui::RichText::new(v).size(11.5)).wrap_mode(egui::TextWrapMode::Wrap).halign(egui::Align::Min));
                    ui.end_row();
                };
                line(ui, "Output", &output_path(ex));
                line(ui, "", &sum.format);
                line(ui, "", &sum.video);
                line(ui, "", &sum.audio);
                line(
                    ui,
                    "Source",
                    &tlf!(
                        "Sequence, {w}x{h}, {fps} fps, {rate} Hz",
                        w = q.settings.width,
                        h = q.settings.height,
                        fps = rate.label(),
                        rate = q.settings.sample_rate
                    ),
                );
                line(
                    ui,
                    "Range",
                    &format!(
                        "{} ({:.2} s)",
                        crate::i18n::t(RANGES.iter().find(|(k, _)| *k == ex.range).map(|x| x.1).unwrap_or("")),
                        (range.1 - range.0).seconds()
                    ),
                );
            });
        })
        .response;
    reg.add("export.summary", resp.rect, sum.video.clone());
    child.add_space(4.0);
    let est = child.label(egui::RichText::new(tlf!("Estimated file size: {size}", size = format_bytes(sum.estimated_bytes))).color(t.text_dim));
    reg.add("export.estimate", est.rect, sum.estimated_size.clone());
    // buttons
    let go = Rect::from_min_size(pos2(rect.max.x - 120.0, rect.max.y - 44.0), vec2(104.0, 30.0));
    let queue = Rect::from_min_size(pos2(go.min.x - 132.0, go.min.y), vec2(122.0, 30.0));
    let qresp = ui.interact(queue, egui::Id::unique("export-queue-send"), Sense::click());
    ui.painter().rect_stroke(queue, 15.0, egui::Stroke::new(1.0, if qresp.hovered() { t.text } else { t.text_dim }), egui::StrokeKind::Inside);
    ui.painter().text(queue.center(), Align2::CENTER_CENTER, tl!("Send to Queue"), Tokens::semibold(12.5), t.text);
    reg.add("export.sendToQueue", queue, "Send to Queue");
    // Queue All: every sequence at once, when there is more than one
    let multi = app.session.project.sequences().nth(1).is_some();
    if !multi {
        app.ui.export.queue_all_info = false;
    }
    let all = multi.then(|| {
        let r = Rect::from_min_size(pos2(queue.min.x - 106.0, go.min.y), vec2(96.0, 30.0));
        let resp = ui.interact(r, egui::Id::unique("export-queue-all"), Sense::click());
        ui.painter().rect_stroke(r, 15.0, egui::Stroke::new(1.0, if resp.hovered() { t.text } else { t.text_dim }), egui::StrokeKind::Inside);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, tl!("Queue All"), Tokens::semibold(12.5), t.text);
        reg.add("export.queueAll", r, "Queue All");
        // a circled ? that opens what Queue All will use: left of the button, or above it when
        // the column is too narrow
        let mut info = Rect::from_center_size(pos2(r.min.x - 14.0, r.center().y), vec2(18.0, 18.0));
        if info.min.x < rect.min.x + 4.0 {
            info = Rect::from_center_size(pos2(r.min.x + 9.0, r.min.y - 14.0), vec2(18.0, 18.0));
        }
        let iresp = ui.interact(info, egui::Id::unique("export-queue-all-info"), Sense::click());
        let open = app.ui.export.queue_all_info;
        let col = if iresp.hovered() || open { t.text } else { t.text_dim };
        if open {
            ui.painter().circle_filled(info.center(), 8.0, t.field_bg);
        }
        ui.painter().circle_stroke(info.center(), 8.0, egui::Stroke::new(1.0, col));
        ui.painter().text(info.center(), Align2::CENTER_CENTER, "?", Tokens::semibold(11.5), col);
        reg.add("export.queueAll.info", info, "Queue All details");
        let toggled = iresp.clicked();
        if toggled {
            app.ui.export.queue_all_info = !open;
        }
        if app.ui.export.queue_all_info {
            queue_all_details(app, ui.ctx(), &mut reg, info, toggled);
        }
        resp
    });
    let resp = ui.interact(go, egui::Id::unique("export-go"), Sense::click());
    ui.painter().rect_filled(go, 15.0, if resp.hovered() { t.accent_hover } else { t.accent });
    ui.painter().text(go.center(), Align2::CENTER_CENTER, tl!("Export"), Tokens::semibold(13.0), Color32::WHITE);
    reg.add("export.button", go, "Export");
    reg.flush(app);
    if resp.clicked() {
        let r = app.session.execute("file.exportMedia", export_params(app));
        match r {
            Ok(v) => app.ui.status = tlf!("Exporting {path}", path = v["path"].as_str().unwrap_or("")),
            Err(e) => app.ui.status = e.to_string(),
        }
    }
    if qresp.clicked() {
        let mut p = export_params(app);
        if app.ui.export.preset != CUSTOM {
            // the queue shows which preset an item came from
            p["preset"] = json!(app.ui.export.preset);
        }
        match app.session.execute("export.queue.add", p) {
            Ok(_) => app.ui.status = tl!("Added to the export queue").into(),
            Err(e) => app.ui.status = e.to_string(),
        }
    }
    if all.is_some_and(|r| r.clicked()) {
        match app.session.execute("export.queue.add", queue_all_params(app)) {
            Ok(v) => {
                let n = v["added"].as_array().map_or(0, Vec::len);
                app.ui.status = tlf!("Added {n} sequences to the export queue", n);
            }
            Err(e) => app.ui.status = e.to_string(),
        }
    }
}

/// The Queue All details panel, above the ⓘ at `info`; Escape or a click elsewhere closes it.
fn queue_all_details(app: &mut FilmcraftApp, ctx: &egui::Context, reg: &mut Reg, info: Rect, toggled: bool) {
    let t = app.tokens;
    let help = queue_all_help(app);
    let area = egui::Area::new(egui::Id::unique("export-queue-all-details"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(info.min.x, info.min.y - 6.0))
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(340.0);
                ui.label(egui::RichText::new(tl!("Queue All")).strong().size(13.0));
                ui.add_space(2.0);
                for line in help.lines() {
                    ui.label(egui::RichText::new(line).size(11.5).color(t.text));
                }
            });
        });
    reg.add("export.queueAll.details", area.response.rect, help);
    if crate::widgets::escape_closes(ctx) || (area.response.clicked_elsewhere() && !toggled) {
        app.ui.export.queue_all_info = false;
    }
}

/// The export range in sequence ticks (for the summary).
fn range_ticks(ex: &ExportUi, q: &filmcraft_engine::project::Sequence) -> (Tick, Tick) {
    let fd = q.settings.frame_rate.frame_duration();
    let end = q.duration().max(fd);
    match ex.range.as_str() {
        "inOut" => {
            let a = q.mark_in.unwrap_or(Tick::ZERO);
            (a, q.mark_out.map(|o| o + fd).unwrap_or(end).max(a + fd))
        }
        "workArea" => q.work_area.map(|w| (w.start, w.end())).unwrap_or((Tick::ZERO, end)),
        "custom" => (Tick::from_seconds_f64(ex.custom_start.max(0.0)), Tick::from_seconds_f64(ex.custom_end.max(ex.custom_start + 0.001))),
        _ => (Tick::ZERO, end),
    }
}
