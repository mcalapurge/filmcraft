//! Saved live-caption looks: create, apply, update and reset through the panel controls.
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::{FilmcraftApp, dock::PanelKind};
use serde_json::json;

fn step(h: &mut Harness<'_, FilmcraftApp>) {
    for _ in 0..3 {
        h.step();
    }
}

fn click(h: &mut Harness<'_, FilmcraftApp>, id: &str) {
    step(h);
    let rect = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}")).rect;
    let pos = egui::pos2(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    h.event(egui::Event::PointerMoved(pos));
    step(h);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        step(h);
    }
}

#[test]
fn save_apply_update_and_reset_live_caption_presets() {
    let data = std::env::temp_dir().join(format!("filmcraft-caption-preset-ui-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let mut session = Session::default();
    session.prefs_path = Some(data.join("preferences.json"));
    session.execute("file.newSequence", json!({"name": "Portrait", "width": 1080, "height": 1920})).unwrap();
    session.execute("captions.add", json!({"text": "Save my caption look", "durationSeconds": 3})).unwrap();
    let mut app = FilmcraftApp::new(session);
    app.show_panel(PanelKind::LiveCaptions);
    app.ui.keys.maximized = Some(PanelKind::LiveCaptions);
    let snapshots = std::env::var_os("FILMCRAFT_CAPTION_PRESET_SNAPSHOT_DIR");
    let mut builder = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_max_steps(10000);
    if snapshots.is_some() {
        builder = builder.wgpu();
    }
    let mut h = builder.build_eframe(move |_| app);
    click(&mut h, "liveCaptions.preset.Pop");
    h.state_mut().session.execute("captions.setStyle", json!({"track": "C1", "size": 83})).unwrap();
    h.state_mut().ui.live_captions.preset_name = "My saved look".into();
    click(&mut h, "liveCaptions.savePreset");
    assert_eq!(h.state().session.prefs.caption_presets[0].name, "My saved look");
    click(&mut h, "liveCaptions.preset.Classic");
    click(&mut h, "liveCaptions.preset.My saved look");
    assert_eq!(h.state().session.active_sequence().unwrap().caption_tracks[0].style.size, 83.0);
    h.state_mut().session.execute("captions.setStyle", json!({"track": "C1", "size": 91})).unwrap();
    click(&mut h, "liveCaptions.updatePreset");
    assert_eq!(h.state().session.prefs.caption_presets[0].style.size, 91.0);
    if let Some(dir) = snapshots {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join("caption-presets.png")).unwrap();
    }
    click(&mut h, "liveCaptions.deletePreset");
    assert!(h.state().session.prefs.caption_presets.is_empty());
    click(&mut h, "liveCaptions.preset.Pop");
    h.state_mut().session.execute("captions.setStyle", json!({"track": "C1", "size": 94})).unwrap();
    click(&mut h, "liveCaptions.updatePreset");
    assert_eq!(h.state().session.prefs.caption_presets[0].name, "Pop");
    click(&mut h, "liveCaptions.deletePreset");
    assert!(h.state().session.prefs.caption_presets.is_empty());
    assert_eq!(h.state().session.active_sequence().unwrap().caption_tracks[0].style, filmcraft_project::CaptionStyle::preset_for("Pop", 1080, 1920).unwrap());
    std::fs::remove_dir_all(data).unwrap();
}
