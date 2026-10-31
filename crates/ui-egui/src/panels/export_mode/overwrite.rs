//! The question asked before Export, Send to Queue, Queue All or Quick Export would replace files that already
//! exist: Replace them, Keep Both (the new file gets a numbered name) or Cancel. Escape cancels.
//! The engine lists the files (`export.conflicts`) and does the numbering (`existing: keepBoth`).
//!
//! Automation ids: `export.overwrite` (the message), `export.overwrite.replace`,
//! `export.overwrite.keepBoth`, `export.overwrite.cancel`.

use egui::{Frame, Margin, RichText};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::FilmcraftApp;
use crate::panels::file_dialogs::{button, modal_frame};

/// Files listed by name in the question; the rest are counted.
const MAX_LISTED: usize = 8;

/// What the Export page was asked to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportAction {
    /// Export (`file.exportMedia`).
    #[default]
    Now,
    /// Send to Queue (`export.queue.add`).
    Queue,
    /// Queue All (`export.queue.add` with every sequence).
    QueueAll,
    /// The header's Quick Export (`export.quick`).
    Quick,
}

/// An export waiting for the answer to "replace the existing files?".
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OverwritePrompt {
    pub action: ExportAction,
    /// The command params, without `existing`.
    pub params: Value,
    /// The files it would replace.
    pub existing: Vec<String>,
}

/// Run `action` with `params`, first asking when it would replace existing files.
pub(super) fn request(app: &mut FilmcraftApp, action: ExportAction, params: Value) {
    let mut q = params.clone();
    match action {
        ExportAction::Now => q["now"] = json!(true),
        ExportAction::Quick => q["quick"] = json!(true),
        ExportAction::Queue | ExportAction::QueueAll => {}
    }
    let existing: Vec<String> = match app.session.execute("export.conflicts", q) {
        Ok(v) => v["existing"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        Err(e) => {
            app.ui.status = e.to_string();
            return;
        }
    };
    if existing.is_empty() {
        run(app, action, params);
    } else {
        app.ui.export.overwrite = Some(OverwritePrompt { action, params, existing });
    }
}

fn run(app: &mut FilmcraftApp, action: ExportAction, params: Value) {
    match action {
        ExportAction::Now => match app.session.execute("file.exportMedia", params) {
            Ok(v) => app.ui.status = tlf!("Exporting {path}", path = v["path"].as_str().unwrap_or("")),
            Err(e) => app.ui.status = e.to_string(),
        },
        ExportAction::Queue => match app.session.execute("export.queue.add", params) {
            Ok(_) => app.ui.status = tl!("Added to the export queue").into(),
            Err(e) => app.ui.status = e.to_string(),
        },
        ExportAction::QueueAll => match app.session.execute("export.queue.add", params) {
            Ok(v) => {
                let n = v["added"].as_array().map_or(0, Vec::len);
                app.ui.status = tlf!("Added {n} sequences to the export queue", n);
            }
            Err(e) => app.ui.status = e.to_string(),
        },
        ExportAction::Quick => match app.session.execute("export.quick", params) {
            Ok(v) => app.ui.status = tlf!("Quick Export: {path}", path = v["path"].as_str().unwrap_or("")),
            Err(e) => app.ui.status = e.to_string(),
        },
    }
}

/// The file name and its folder, for the message.
fn split(path: &str) -> (String, String) {
    let p = std::path::Path::new(path);
    let name = p.file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let dir = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    (name, dir)
}

/// Draw the question while one is pending (from the header, in every mode: Quick Export asks too).
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(prompt) = app.ui.export.overwrite.clone() else { return };
    let t = app.tokens;
    let n = prompt.existing.len();
    let mut answer: Option<Option<&str>> = None;
    let r = egui::Modal::new(egui::Id::unique("export-overwrite-modal")).frame(modal_frame(&t)).show(ctx, |ui| {
        ui.set_width(500.0);
        Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 20 }).show(ui, |ui| {
            let title = if n == 1 { tl!("Replace Existing File?") } else { tl!("Replace Existing Files?") };
            ui.label(RichText::new(title).size(18.0).strong().color(t.text));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            let only = prompt.existing.first().filter(|_| n == 1).map(|p| split(p));
            let message = match &only {
                Some((name, _)) => tlf!("“{name}” already exists. Replacing it overwrites its contents.", name),
                None => tlf!("{n} of the files to export already exist. Replacing them overwrites their contents.", n),
            };
            let m = ui.label(RichText::new(&message).size(13.5).color(t.text));
            app.auto.add("export.overwrite", m.rect, &message);
            if let Some((_, folder)) = &only {
                ui.label(RichText::new(folder).size(12.0).color(t.text_dim));
            } else {
                ui.add_space(4.0);
                for path in prompt.existing.iter().take(MAX_LISTED) {
                    ui.label(RichText::new(format!("• {}", split(path).0)).size(12.0).color(t.text_dim));
                }
                if n > MAX_LISTED {
                    ui.label(RichText::new(tlf!("…and {more} more", more = n - MAX_LISTED)).size(12.0).color(t.text_dim));
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Keep Both gives the new file a numbered name.")).size(11.5).color(t.text_dim));
            ui.add_space(18.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(app, ui, "export.overwrite.replace", tl!("Replace"), true) {
                    answer = Some(Some("replace"));
                }
                if button(app, ui, "export.overwrite.keepBoth", tl!("Keep Both"), false) {
                    answer = Some(Some("keepBoth"));
                }
                if button(app, ui, "export.overwrite.cancel", tl!("Cancel"), false) {
                    answer = Some(None);
                }
            });
        });
    });
    // the modal's own close fires on Escape and a click outside: not while a drag is being cancelled (#580)
    if answer.is_none() && r.should_close() && !ctx.input(|i| i.pointer.any_down()) {
        answer = Some(None);
    }
    let Some(choice) = answer else { return };
    app.ui.export.overwrite = None;
    if let Some(existing) = choice {
        let mut params = prompt.params;
        params["existing"] = json!(existing);
        run(app, prompt.action, params);
    }
}
