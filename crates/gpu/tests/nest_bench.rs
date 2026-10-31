//! Playback cost of a scaled nested sequence through the GPU frame renderer: a 2160x3840 nest at
//! 50 % in a 1080x1920 sequence (a 4K vertical edit nested into an HD vertical one), and the same
//! nest unscaled for comparison. The sources hand out cached frames, as the decoded-frame cache
//! does in playback, so the time is the nest's: planning (where a nest rendered as a picture is
//! drawn on the CPU), upload, compositing and read-back. Timing only, so it is ignored by default:
//! `cargo test -p filmcraft-gpu --test nest_bench -- --ignored --nocapture`.

use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{FrameRequest, Generator, MediaInfo, MediaSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, ParamValue, Project, SequenceSettings, TrackKind};
use filmcraft_render::{RenderOptions, SourceMap};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

const RATE: FrameRate = FrameRate::FPS_24;
const FRAMES: i64 = 96;

/// A source that makes each frame size once (the generators here are still pictures).
struct Cached {
    inner: GeneratorSource,
    frames: Mutex<HashMap<u32, Arc<VideoFrame>>>,
}

impl MediaSource for Cached {
    fn info(&self) -> &MediaInfo {
        self.inner.info()
    }
    fn video_frame(&self, req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        let mut frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(f) = frames.get(&req.scale.to_bits()) {
            return Ok(f.clone());
        }
        let f = self.inner.video_frame(req)?;
        frames.insert(req.scale.to_bits(), f.clone());
        Ok(f)
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        self.inner.audio(start, frames, sample_rate)
    }
}

fn generator(p: &mut Project, m: &mut SourceMap, generator: Generator, w: u32, h: u32) -> ItemId {
    let g = GeneratorSource::new(generator, w, h, RATE, RATE.tick_of(FRAMES));
    let info = g.info().clone();
    let id = p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(Box::new(MediaClip {
            media: MediaRef::Generator(g.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        })),
        None,
    );
    m.0.insert(id, Arc::new(Cached { inner: g, frames: Mutex::new(HashMap::new()) }));
    id
}

/// A 2160x3840 sequence (bars under a translucent matte) nested at `scale` % in a 1080x1920 one.
fn project(scale: f64) -> (Project, ItemId, SourceMap) {
    let mut p = Project::new("bench");
    let mut m = SourceMap::default();
    let settings = |width, height| SequenceSettings { width, height, frame_rate: RATE, ..Default::default() };
    let inner = p.new_sequence("inner", settings(2160, 3840), 2, 0, None);
    let bars = generator(&mut p, &mut m, Generator::BarsAndTone, 2160, 3840);
    let matte = generator(&mut p, &mut m, Generator::ColorMatte { color: [0.9, 0.3, 0.1, 1.0] }, 1080, 1080);
    let range = TimeRange::new(Tick::ZERO, RATE.tick_of(FRAMES));
    for (track, item) in [(0, bars), (1, matte)] {
        let mut it = p.make_track_item(item, TrackKind::Video, Tick::ZERO, range, RATE).unwrap();
        if item == matte {
            it.effect_mut("opacity").unwrap().param_mut("opacity").unwrap().value = ParamValue::Float(60.0);
        }
        p.sequence_mut(inner).unwrap().video_tracks[track].items.push(it);
    }
    let outer = p.new_sequence("outer", settings(1080, 1920), 1, 0, None);
    let mut nest = p.make_track_item(inner, TrackKind::Video, Tick::ZERO, range, RATE).unwrap();
    nest.effect_mut("motion").unwrap().param_mut("scale").unwrap().value = ParamValue::Float(scale);
    p.sequence_mut(outer).unwrap().video_tracks[0].items.push(nest);
    (p, outer, m)
}

/// Milliseconds per frame over `FRAMES / 2` distinct frames, after as many to warm up.
fn per_frame(mut render: impl FnMut(Tick) -> bool) -> f64 {
    for f in 0..FRAMES / 2 {
        assert!(render(RATE.tick_of(f)), "frame {f} rendered");
    }
    let start = Instant::now();
    for f in FRAMES / 2..FRAMES {
        assert!(render(RATE.tick_of(f)), "frame {f} rendered");
    }
    start.elapsed().as_secs_f64() * 1000.0 / (FRAMES / 2) as f64
}

#[test]
#[ignore = "timing benchmark"]
fn scaled_nest_playback_cost() {
    let Some(mut gpu) = filmcraft_gpu::ExportRenderer::new() else {
        eprintln!("SKIPPED (nest bench): no adapter");
        return;
    };
    let opts = RenderOptions::default();
    for scale in [50.0, 100.0] {
        let (p, seq, m) = project(scale);
        let ms_gpu = per_frame(|t| gpu.render(&p, seq, t, opts, &m).is_some());
        let ms_cpu = per_frame(|t| filmcraft_render::render_sequence(&p, seq, t, opts, &m).w > 0);
        let ms_plan = per_frame(|t| {
            let _ = filmcraft_render::plan::plan_frame(&p, seq, t, opts, &m);
            true
        });
        let kind = match filmcraft_render::plan::plan_frame(&p, seq, Tick::ZERO, opts, &m) {
            filmcraft_render::plan::FramePlan::Image(_) => "one CPU image".to_string(),
            filmcraft_render::plan::FramePlan::Layers { layers, .. } => format!("layers {:?}", layers.iter().map(|l| l.size()).collect::<Vec<_>>()),
            filmcraft_render::plan::FramePlan::Composite { steps, .. } => format!("{} composite steps", steps.len()),
        };
        eprintln!("BENCH nest at {scale}%: GPU renderer {ms_gpu:.2} ms/frame (of which planning {ms_plan:.2}), CPU reference {ms_cpu:.2}; plan: {kind}");
    }
}
