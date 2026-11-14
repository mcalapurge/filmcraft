//! Frame plans for the GPU compositor.
//!
//! [`plan_frame`] resolves what is visible at a time into a list of layers the GPU can draw
//! directly: a decoded source frame (YUV planes or RGBA), the matrix from source pixels to output
//! pixels, an opacity and a blend mode (all 27 of [`Blend`] are composited by the GPU). A media
//! clip whose enabled standard effects all have a GPU implementation ([`crate::gpufx`]) carries
//! them as a [`LayerFx`]: their parameters evaluated at the frame's time, run by the GPU on the
//! clip's working image before Motion places it, in the CPU's order (effects → Motion → Opacity /
<<<<<<< HEAD
//! blend). A nested sequence contributes its own layers when it is only shown, and is a
//! [`PlanStep::Nest`] when its effects all have a GPU implementation (its layers composited into a
//! picture of its own, the effects run on that, the picture placed). Anything the shaders don't
//! cover yet — other standard effects, masks, adjustment layers, other nested sequences,
//! transitions other than dissolves, dips and Push — is rendered on the CPU for that layer (or the
//! whole frame) and handed over as a pre-composited image, so the GPU path is always exact with
//! respect to the CPU reference.
=======
//! blend). Anything the shaders don't cover yet — other standard effects, masks, adjustment
//! layers other than [`PlanStep::Adjust`]'s, nested sequences, transitions other than dissolves,
//! dips and Push — is rendered on the CPU for that layer (or the whole frame) and handed over as a
//! pre-composited image, so the GPU path is always exact with respect to the CPU reference.
>>>>>>> upstream/main
//!
//! A Push transition is planned as ordered [`PlanStep`] operations on isolated clip inputs: each
//! side is drawn into its own image, the two are pushed together, and the result is composited (Normal) over
//! the canvas of the lower tracks, which is preserved for the blends above it. An adjustment layer
//! is a [`PlanStep::Adjust`] on the canvas of the tracks below it. [`execute_cpu`] runs the same
//! steps on the CPU and is the reference.

use std::sync::Arc;

use filmcraft_frame::VideoFrame;
use filmcraft_geom::Affine;
use filmcraft_media::FrameRequest;
use filmcraft_project::{ItemId, ItemKind, Project, Sequence, TrackItem};
use filmcraft_time::Tick;

use crate::fallback::{Fallback, Scope};
use crate::gpufx::FxOp;
use crate::{Blend, RenderOptions, SourceProvider, motion_matrix, output_size};

/// A GPU frame plan retains every resolved source frame. Nested sequences may share the same
/// child sequence many times, so recursion depth alone does not bound this expansion.
const MAX_PLAN_LAYERS: usize = 4096;

/// One layer for the GPU, bottom to top.
#[derive(Clone)]
pub struct PlanLayer {
    pub frame: Arc<VideoFrame>,
    /// Maps frame pixels (0..w, 0..h) to output pixels — or, with [`fx`](Self::fx), working-image
    /// pixels (0..fx.size).
    pub matrix: Affine,
    pub opacity: f32,
    /// How the layer combines with what is under it ([`crate::blend::composite`]).
    pub blend: Blend,
    /// Standard effects to run on the frame before it is placed (None: draw the frame directly).
    pub fx: Option<Arc<LayerFx>>,
    /// Colour management of a Y'CbCr frame that needs more than the default decode (HDR, BT.2020,
    /// P3 media on a Rec. 709 sequence; [`crate::colorman::gpu_input`]): run on the GPU as on the
    /// CPU. None: the default decode.
    pub input: Option<Arc<filmcraft_color::InputTransform>>,
}

/// The GPU effect stage of a layer: the frame is decoded into a working image of `size` (the
/// frame box-decimated by `decimation`, as the CPU decodes it, then turned clockwise by `turns`
/// quarter turns), `ops` run on it in order, and the result is placed with the layer's matrix,
/// opacity and blend mode.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerFx {
    pub size: (u32, u32),
    pub decimation: u32,
    /// The container's display rotation, left to this stage (see [`MediaSource::video_frame_unturned`]).
    ///
    /// [`MediaSource::video_frame_unturned`]: filmcraft_media::MediaSource::video_frame_unturned
    pub turns: u8,
    pub ops: Vec<FxOp>,
}

impl PlanLayer {
    /// A layer drawing `frame` directly.
    pub fn new(frame: Arc<VideoFrame>, matrix: Affine, opacity: f32, blend: Blend) -> Self {
        Self { frame, matrix, opacity, blend, fx: None, input: None }
    }

    /// Size of the picture the matrix places: the working image with effects, else the frame.
    pub fn size(&self) -> (u32, u32) {
        match &self.fx {
            Some(fx) => fx.size,
            None => (self.frame.width, self.frame.height),
        }
    }
}

#[derive(Clone)]
pub enum FramePlan {
    /// Draw these layers over black.
    Layers { width: usize, height: usize, layers: Vec<PlanLayer> },
    /// Ordered layer runs and two-input operations; each step composites onto the same canvas.
    Composite { width: usize, height: usize, steps: Vec<PlanStep> },
    /// The CPU produced the final image (fallback).
    Image(crate::Image),
}

#[derive(Clone)]
pub enum PlanStep {
    /// Whole-canvas GPU effects (adjustment layers and working-to-display conversion).
    Effects(Vec<FxOp>),
    Layers(Vec<PlanLayer>),
    /// Inputs are isolated clip canvases; transition result composites Normal over lower tracks.
    Transition {
        inputs: [Vec<PlanLayer>; 2],
        effect: filmcraft_project::EffectInstance,
        progress: f32,
        scale: f32,
    },
<<<<<<< HEAD
    /// A nested sequence with standard effects: its `layers` are composited into a picture of
    /// its own of `size` (transparent black first), `ops` run on that picture, and it is placed
    /// over the canvas with `matrix` (picture pixels to output pixels), `opacity` and `blend`, as
    /// [`crate::item_layer`] renders a nest, runs its effects and places it on the CPU.
    Nest {
        layers: Vec<PlanLayer>,
        size: (u32, u32),
        ops: Vec<FxOp>,
        matrix: Affine,
        opacity: f32,
        blend: Blend,
    },
=======
    /// An adjustment layer: `ops` run on the whole canvas of the tracks below, and the result
    /// replaces it (full frame, opacity 100 %, Normal, no masks: anything else plans the frame on
    /// the CPU).
    Adjust {
        ops: Vec<FxOp>,
    },
}

/// What the device that composites a plan can run. [`plan_frame`] plans a frame with a step the
/// device can't run as one CPU image, on the thread that plans it, instead of leaving the
/// compositor to fall back to the CPU while it draws (the UI thread for a monitor, a renderer slot
/// for an export). The monitor and the export can composite on different GPUs, so each passes its
/// own; a cache of plans keys them by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceCaps {
    /// Largest output side whose adjustment layers ([`PlanStep::Adjust`]) the device runs: its
    /// texture limit with an effect stage, 0 without one.
    pub adjust_side: u32,
}

impl DeviceCaps {
    /// A device without an effect stage: a frame with an adjustment layer plans on the CPU.
    pub const NONE: Self = Self { adjust_side: 0 };
    /// A device that runs every step ([`execute_cpu`] does).
    pub const ALL: Self = Self { adjust_side: u32::MAX };

    /// Whether the device runs the adjustment layers of a `w`×`h` frame.
    pub fn adjusts(self, w: usize, h: usize) -> bool {
        w.max(h) <= self.adjust_side as usize
    }
>>>>>>> upstream/main
}

#[derive(Default)]
struct PlanStack {
    steps: Vec<PlanStep>,
    exhausted: bool,
    /// Something at this time only the CPU renderer draws (an adjustment layer the GPU can't run):
    /// the whole frame goes to it, as when `exhausted`.
    cpu_only: bool,
    /// Layers held in `steps`, kept as they are pushed so the cap check stays O(1).
    layers: usize,
}

impl PlanStack {
    fn layer_count(&self) -> usize {
        self.layers
    }

    fn push(&mut self, layer: PlanLayer) {
        if self.exhausted {
            return;
        }
        if self.layer_count() >= MAX_PLAN_LAYERS {
            self.exhausted = true;
            return;
        }
        if let Some(PlanStep::Layers(layers)) = self.steps.last_mut() {
            layers.push(layer);
        } else {
            self.steps.push(PlanStep::Layers(vec![layer]));
        }
        self.layers = self.layers.saturating_add(1);
    }

    fn push_transition(&mut self, inputs: [Vec<PlanLayer>; 2], effect: filmcraft_project::EffectInstance, progress: f32, scale: f32) {
        let added: usize = inputs.iter().map(Vec::len).sum();
        if self.exhausted || self.layer_count().saturating_add(added) > MAX_PLAN_LAYERS {
            self.exhausted = true;
            return;
        }
        self.layers = self.layers.saturating_add(added);
        self.steps.push(PlanStep::Transition { inputs, effect, progress, scale });
    }

<<<<<<< HEAD
    /// Push a [`PlanStep::Nest`].
    fn push_nest(&mut self, step: PlanStep) {
        let added = match &step {
            PlanStep::Nest { layers, .. } => layers.len(),
            _ => 0,
        };
        if self.exhausted || self.layer_count().saturating_add(added) > MAX_PLAN_LAYERS {
            self.exhausted = true;
            return;
        }
        self.layers = self.layers.saturating_add(added);
        self.steps.push(step);
=======
    fn push_adjust(&mut self, ops: Vec<FxOp>) {
        if self.exhausted || ops.is_empty() {
            return;
        }
        self.steps.push(PlanStep::Adjust { ops });
>>>>>>> upstream/main
    }

    fn into_layers(self, width: usize, height: usize) -> Option<Vec<PlanLayer>> {
        if self.exhausted || self.cpu_only {
            return None;
        }
        match self.finish(width, height) {
            FramePlan::Layers { layers, .. } => Some(layers),
            plan => Some(vec![PlanLayer::new(cpu_frame(execute_cpu(&plan)), Affine::IDENTITY, 1.0, Blend::Normal)]),
        }
    }

    fn finish(mut self, width: usize, height: usize) -> FramePlan {
        if self.steps.len() <= 1 && self.steps.first().is_none_or(|s| matches!(s, PlanStep::Layers(_))) {
            let layers = match self.steps.pop() {
                Some(PlanStep::Layers(layers)) => layers,
                _ => Vec::new(),
            };
            FramePlan::Layers { width, height, layers }
        } else {
            FramePlan::Composite { width, height, steps: self.steps }
        }
    }
}

impl FramePlan {
    /// Source storage retained by the plan, before GPU preparation.
    pub fn source_bytes(&self) -> usize {
        let layers = |ls: &[PlanLayer]| ls.iter().map(|l| l.frame.byte_size()).sum::<usize>();
        match self {
            Self::Image(img) => img.px.len() * size_of::<f32>(),
            Self::Layers { layers: ls, .. } => layers(ls),
            Self::Composite { steps, .. } => steps
                .iter()
                .map(|s| match s {
                    PlanStep::Effects(_) => 0,
                    PlanStep::Layers(ls) | PlanStep::Nest { layers: ls, .. } => layers(ls),
                    PlanStep::Transition { inputs, .. } => inputs.iter().map(|ls| layers(ls)).sum(),
                    PlanStep::Adjust { .. } => 0,
                })
                .sum(),
        }
    }

    /// Largest output/source texture side required by the plan.
    pub fn max_side(&self) -> usize {
        let layer_side = |ls: &[PlanLayer]| ls.iter().map(|l| l.frame.width.max(l.frame.height) as usize).max().unwrap_or(0);
        match self {
            Self::Image(img) => img.w.max(img.h),
            Self::Layers { width, height, layers } => (*width).max(*height).max(layer_side(layers)),
            Self::Composite { width, height, steps } => steps
                .iter()
                .map(|s| match s {
                    PlanStep::Effects(_) => 0,
                    PlanStep::Layers(ls) => layer_side(ls),
                    PlanStep::Transition { inputs, .. } => inputs.iter().map(|ls| layer_side(ls)).max().unwrap_or(0),
<<<<<<< HEAD
                    PlanStep::Nest { layers, size, .. } => layer_side(layers).max(size.0.max(size.1) as usize),
=======
                    PlanStep::Adjust { .. } => 0,
>>>>>>> upstream/main
                })
                .fold((*width).max(*height), usize::max),
        }
    }
}

fn cpu_frame(img: crate::Image) -> Arc<VideoFrame> {
    Arc::new(VideoFrame::rgba_f32(img.w as u32, img.h as u32, img.px))
}

fn simple_transition(id: &str) -> bool {
    matches!(id, "cross_dissolve" | "dip_to_black" | "dip_to_white")
}

/// The standard effects of a media clip if the GPU can draw it (any blend mode, no opacity masks,
/// every enabled standard effect unmasked and with a GPU implementation; empty without effects or
/// with effects off). None: the clip is rendered on the CPU.
fn gpu_chain<'a>(project: &Project, item: &'a TrackItem, opts: RenderOptions) -> Option<Vec<&'a filmcraft_project::EffectInstance>> {
    let is_media = project.item(item.item).is_some_and(|p| matches!(p.kind, ItemKind::Media(_) | ItemKind::Subclip { .. }));
    if !is_media || item.has_opacity_masks() {
        return None;
    }
    gpu_effects(item, opts)
}

/// The enabled standard effects of `item` if every one is unmasked and has a GPU implementation
/// (empty with effects off). None: the GPU cannot run them.
fn gpu_effects(item: &TrackItem, opts: RenderOptions) -> Option<Vec<&filmcraft_project::EffectInstance>> {
    if !opts.effects {
        return Some(Vec::new());
    }
    let mut chain = Vec::new();
    for e in item.effects.iter().filter(|e| e.def().is_some_and(|d| !d.intrinsic) && !filmcraft_project::graphic::is_layer(e)) {
        if !e.enabled {
            continue;
        }
        let masked = e.masks.iter().any(|m| m.mode != filmcraft_project::MaskMode::None);
        if masked || !crate::gpufx::GPU_EFFECTS.contains(&e.effect.as_str()) {
            return None;
        }
        chain.push(e);
    }
    Some(chain)
}

/// Plan the frame at timeline `t` for a device that runs `caps`.
pub fn plan_frame(project: &Project, seq_id: ItemId, t: Tick, opts: RenderOptions, caps: DeviceCaps, sources: &dyn SourceProvider) -> FramePlan {
    let Some(seq) = project.sequence(seq_id) else { return FramePlan::Image(crate::Image::new(1, 1)) };
    let (w, h) = output_size(seq, opts.scale);
<<<<<<< HEAD
    if let Some(why) = frame_cause(project, seq, t) {
        crate::fallback::note(Scope::Frame, why);
        return FramePlan::Image(crate::render_sequence(project, seq_id, t, opts, sources));
    }
    let mut layers = PlanStack::default();
    push_tracks(project, seq, t, opts, sources, 0, &mut layers);
    if layers.exhausted {
        crate::fallback::note(Scope::Frame, Fallback::LayerLimit);
=======
    if whole_frame_on_cpu(project, seq, t, opts.scale, caps) {
        return FramePlan::Image(crate::render_sequence(project, seq_id, t, opts, sources));
    }
    let mut layers = PlanStack::default();
    push_tracks(Scene { project, seq, t, opts, caps, sources, nest: 0 }, &mut layers);
    if layers.exhausted || layers.cpu_only {
>>>>>>> upstream/main
        return FramePlan::Image(crate::render_sequence(project, seq_id, t, opts, sources));
    }
    if opts.depth == 0 && !opts.working_output && !seq.settings.color.is_plain() {
        layers.steps.push(PlanStep::Effects(vec![FxOp::OutputColor(crate::colorman::display_transform(&seq.settings.color).gpu_params())]));
    }
    if opts.captions {
        for o in crate::caption_overlays(seq, t, w, h) {
            layers.push(PlanLayer::new(
                Arc::new(VideoFrame::rgba_f32(o.w as u32, o.h as u32, o.px)),
                Affine::translate(o.x as f64, o.y as f64),
                1.0,
                Blend::Normal,
            ));
        }
    }
    if layers.exhausted {
        crate::fallback::note(Scope::Frame, Fallback::LayerLimit);
        return FramePlan::Image(crate::render_sequence(project, seq_id, t, opts, sources));
    }
    layers.finish(w, h)
}

/// Whether [`plan_frame`] hands the frame at `t` back as one CPU image ([`FramePlan::Image`]) for
/// a device that runs `caps`. A GPU renderer gains nothing on such a frame, so a caller with only a
/// few of them (the export's pool) can render it on the CPU without taking one.
pub fn is_cpu_frame(project: &Project, seq_id: ItemId, t: Tick, opts: RenderOptions, caps: DeviceCaps) -> bool {
    project.sequence(seq_id).is_none_or(|seq| whole_frame_on_cpu(project, seq, t, opts.scale, caps))
}

<<<<<<< HEAD
/// Why [`plan_frame`] hands the frame at `t` back as one CPU image, if it does (None also for a
/// missing sequence, which renders nothing). Callers that render such a frame on the CPU without
/// planning it (the export's pool) record it with [`crate::fallback::note`].
pub fn cpu_frame_cause(project: &Project, seq_id: ItemId, t: Tick) -> Option<Fallback> {
    project.sequence(seq_id).and_then(|seq| frame_cause(project, seq, t))
}

/// Frames with unsupported adjustment geometry/masks or complex transitions retain the
/// whole-frame CPU fallback. HDR/wide-gamut output is a planned canvas colour transform.
fn whole_frame_on_cpu(project: &Project, seq: &Sequence, t: Tick) -> bool {
    !layered_at(project, seq, t)
}

/// Whether the frame of `seq` at `t` can be planned as layers: no adjustment layer and no
/// transition the compositor cannot mix itself is showing (a transition into or out of an
/// adjustment layer counts, also on the side of the cut where the other clip is).
pub(crate) fn layered_at(project: &Project, seq: &Sequence, t: Tick) -> bool {
    frame_cause(project, seq, t).is_none()
}

/// What keeps the frame of `seq` at `t` from planning as layers ([`layered_at`]).
fn frame_cause(project: &Project, seq: &Sequence, t: Tick) -> Option<Fallback> {
    frame_cause_with(project, seq, t, |it| adjustment_ops(project, seq, it, t, 1.0).is_some())
}

/// Render-bar hint only: never builds shader payloads or reads LUT tables.
pub(crate) fn estimate_layered_at(project: &Project, seq: &Sequence, t: Tick) -> bool {
    frame_cause_with(project, seq, t, |it| adjustment_is_plain(seq, it, it.source_time_at(t)) && crate::preview::gpu_effect_cost(it).is_some()).is_none()
}

fn frame_cause_with(project: &Project, seq: &Sequence, t: Tick, adjustment_supported: impl Fn(&TrackItem) -> bool) -> Option<Fallback> {
    let adjustment = |it: &TrackItem| project.item(it.item).is_some_and(|p| matches!(p.kind, ItemKind::AdjustmentLayer { .. }));
=======
/// The frames [`plan_frame`] does not plan: an HDR / wide-gamut sequence composites and converts
/// on the CPU, and so does a frame with an adjustment layer the device (`caps`) can't run or a
/// complex transition anywhere at `t` (whole-frame fallback).
fn whole_frame_on_cpu(project: &Project, seq: &Sequence, t: Tick, scale: f32, caps: DeviceCaps) -> bool {
    !seq.settings.color.is_plain() || !layered_at(project, seq, t, scale, caps)
}

fn is_adjustment(project: &Project, it: &TrackItem) -> bool {
    project.item(it.item).is_some_and(|p| matches!(p.kind, ItemKind::AdjustmentLayer { .. }))
}

/// The ops of an adjustment layer that can be a [`PlanStep::Adjust`] at `t` and render `scale` on
/// a device that runs `caps`: the device adjusts a frame of this size, and the layer is opaque,
/// Normal, unmasked, covering the whole frame, with every enabled standard effect unmasked and
/// evaluating to an op the GPU runs (at the frame's size, as `adjustment_layer` applies them on the
/// CPU). None sends the frame to the CPU. [`layered_at`] and [`push_adjustment`] both ask this, so
/// [`is_cpu_frame`] knows which frames plan as one CPU image.
fn adjustment_ops(project: &Project, seq: &Sequence, it: &TrackItem, t: Tick, scale: f32, caps: DeviceCaps) -> Option<Vec<FxOp>> {
    let mt = it.effect_time_at(t);
    let (op, bl) = crate::opacity_blend(it, mt);
    let (w, h) = output_size(seq, scale);
    if !caps.adjusts(w, h)
        || op < 1.0 - 1e-6
        || bl != Blend::Normal
        || it.has_opacity_masks()
        || !crate::adjustment_covers_output(seq, it, project, mt, scale, w, h)
    {
        return None;
    }
    let cx = crate::effects::FxCtx {
        t: mt,
        px_scale: scale,
        mask_scale: scale,
        seconds: (t - it.start).seconds(),
        timecode: "",
        clip_name: &it.name,
        project: Some(project),
        env: None,
        frame_rate: seq.settings.frame_rate.as_f64(),
        working: seq.settings.color.working,
    };
    it.effects
        .iter()
        .filter(|e| e.enabled && e.def().is_some_and(|d| !d.intrinsic))
        .map(|e| {
            let gpu = e.masks.iter().all(|m| m.mode == filmcraft_project::MaskMode::None) && crate::gpufx::GPU_EFFECTS.contains(&e.effect.as_str());
            if gpu { FxOp::eval(e, &cx, w, h).filter(FxOp::gpu_ok) } else { None }
        })
        .collect()
}

/// Whether the frame of `seq` at `t` and render `scale` can be planned as layers for a device that
/// runs `caps`: no adjustment layer it can't run ([`adjustment_ops`]) and no transition the
/// compositor cannot mix itself is showing (a transition into or out of an adjustment layer counts,
/// also on the side of the cut where the other clip is).
pub(crate) fn layered_at(project: &Project, seq: &Sequence, t: Tick, scale: f32, caps: DeviceCaps) -> bool {
    let adjustment = |it: &TrackItem| is_adjustment(project, it);
>>>>>>> upstream/main
    for tr in &seq.video_tracks {
        if !tr.enabled {
            continue;
        }
        if let Some(trn) = tr.transitions.iter().find(|x| x.range().contains(t)) {
            if !simple_transition(&trn.effect.effect) && trn.effect.effect != "push" {
                return Some(Fallback::transition(&trn.effect));
            }
            if [trn.from, trn.to].into_iter().flatten().filter_map(|c| tr.item(c)).any(adjustment) {
                return Some(Fallback::AdjustmentTransition);
            }
        }
        if let Some(it) = tr.item_at(t)
            && it.enabled
            && adjustment(it)
<<<<<<< HEAD
            && !adjustment_supported(it)
=======
            && adjustment_ops(project, seq, it, t, scale, caps).is_none()
>>>>>>> upstream/main
        {
            return Some(adjustment_cause(seq, it, t));
        }
    }
    None
}

/// Why [`adjustment_ops`] declined an adjustment layer: its own masks, opacity, blend or Motion,
/// else the first effect the GPU can't run.
fn adjustment_cause(seq: &Sequence, item: &TrackItem, t: Tick) -> Fallback {
    if !adjustment_is_plain(seq, item, item.source_time_at(t)) {
        return Fallback::AdjustmentLayer;
    }
    let mut effects = item.effects.iter().filter(|e| e.enabled && e.def().is_some_and(|d| !d.intrinsic));
    unsupported_effect(effects.clone()).or_else(|| effects.next().map(Fallback::effect_values)).unwrap_or(Fallback::AdjustmentLayer)
}

/// The first of `effects` the GPU can't run at all: one with a mask, or one without a GPU
/// implementation.
fn unsupported_effect<'a>(effects: impl IntoIterator<Item = &'a filmcraft_project::EffectInstance>) -> Option<Fallback> {
    effects.into_iter().find_map(|e| {
        if e.masks.iter().any(|m| m.mode != filmcraft_project::MaskMode::None) {
            Some(Fallback::MaskedEffect)
        } else if !crate::gpufx::GPU_EFFECTS.contains(&e.effect.as_str()) {
            Some(Fallback::effect(e))
        } else {
            None
        }
    })
}

/// Why [`push_item`] renders `item` as a CPU layer, checked in the order its GPU paths are tried.
fn layer_cause(project: &Project, seq: &Sequence, item: &TrackItem, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Fallback {
    let mut effects =
        item.effects.iter().filter(|e| e.enabled && e.def().is_some_and(|d| !d.intrinsic) && !filmcraft_project::graphic::is_layer(e)).filter(|_| opts.effects);
    if let Some(why) = unsupported_effect(effects.clone()) {
        return why;
    }
    if item.has_opacity_masks() {
        return Fallback::OpacityMask;
    }
    match project.item(item.item).map(|p| &p.kind) {
        Some(ItemKind::Sequence(_)) => return Fallback::Nest,
        Some(ItemKind::Media(_) | ItemKind::Subclip { .. }) => {}
        _ => return Fallback::NotMedia,
    }
    if sources.source(item.item).is_some_and(|src| crate::media_interpolation(item, t, &src, seq.settings.frame_rate).is_some()) {
        return Fallback::FrameBlending;
    }
    effects.next().map_or(Fallback::ColorManagement, Fallback::effect_values)
}

/// Whether an adjustment layer applies its effects to the whole frame below as it is: no masks,
/// fully opaque, Normal, untransformed (at its source time `mt`).
fn adjustment_is_plain(seq: &Sequence, item: &TrackItem, mt: Tick) -> bool {
    let (opacity, blend) = crate::opacity_blend(item, mt);
    !item.has_opacity_masks()
        && (opacity - 1.0).abs() <= 1e-6
        && blend == Blend::Normal
        && near_identity(&motion_matrix(seq, item, (seq.settings.width, seq.settings.height), None, mt))
}

fn adjustment_ops(project: &Project, seq: &Sequence, item: &TrackItem, t: Tick, scale: f32) -> Option<Vec<FxOp>> {
    let mt = item.source_time_at(t);
    if !adjustment_is_plain(seq, item, mt) {
        return None;
    }
    let (w, h) = output_size(seq, scale);
    let cx = crate::effects::FxCtx {
        t: mt,
        px_scale: scale,
        seconds: 0.0,
        timecode: "",
        clip_name: &item.name,
        project: Some(project),
        env: None,
        frame_rate: seq.settings.frame_rate.as_f64(),
        working: seq.settings.color.working,
    };
    item.effects
        .iter()
        .filter(|e| e.enabled && e.def().is_some_and(|d| !d.intrinsic))
        .map(|e| {
            if e.masks.iter().any(|m| m.mode != filmcraft_project::MaskMode::None) {
                return None;
            }
            FxOp::eval(e, &cx, w, h).filter(FxOp::gpu_ok)
        })
        .collect()
}

/// Plan the adjustment layer `item` of `seq` at `t` as a [`PlanStep::Adjust`]. One
/// [`adjustment_ops`] declines sends the frame to the CPU ([`layered_at`] has declined the frame
/// already then).
fn push_adjustment(sc: Scene, item: &TrackItem, out: &mut PlanStack) {
    match adjustment_ops(sc.project, sc.seq, item, sc.t, sc.opts.scale, sc.caps) {
        Some(ops) => out.push_adjust(ops),
        None => out.cpu_only = true,
    }
}

/// Push the layers of every video track of the scene's sequence at its time, bottom track first
/// (the sequence must be [`layered_at`] that time).
fn push_tracks(sc: Scene, layers: &mut PlanStack) {
    let Scene { project, seq, t, opts, .. } = sc;
    let (w, h) = output_size(seq, opts.scale);
    for tr in &seq.video_tracks {
        if layers.exhausted {
            return;
        }
        if !tr.enabled {
            continue;
        }
        if let Some(trn) = tr.transitions.iter().find(|x| x.range().contains(t)) {
            // These dissolves are symmetric: Reverse (play B→A backwards) renders the same frames.
            let p = trn.progress(t) as f32;
            let a = trn.from.and_then(|id| tr.item(id)).filter(|i| i.enabled);
            let b = trn.to.and_then(|id| tr.item(id)).filter(|i| i.enabled);
            match trn.effect.effect.as_str() {
                "push" => {
                    let mut inputs = [Vec::new(), Vec::new()];
                    for (slot, item) in inputs.iter_mut().zip([a, b]) {
                        if let Some(item) = item {
                            let mut input = PlanStack::default();
                            push_item(sc, item, 1.0, Some(Blend::Normal), &mut input);
                            let Some(input_layers) = input.into_layers(w, h) else {
                                layers.exhausted = true;
                                return;
                            };
                            *slot = input_layers;
                        }
                    }
                    let progress = if trn.reverse {
                        inputs.swap(0, 1);
                        1.0 - p
                    } else {
                        p
                    };
                    layers.push_transition(inputs, trn.effect.clone(), progress, opts.scale);
                }
                "dip_to_black" | "dip_to_white" => {
                    let col = if trn.effect.effect == "dip_to_black" { [0.0, 0.0, 0.0, 1.0] } else { [1.0, 1.0, 1.0, 1.0] };
                    layers.push(PlanLayer::new(Arc::new(VideoFrame::rgba_f32(1, 1, col.to_vec())), Affine::scale(w as f64, h as f64), 1.0, Blend::Normal));
                    let (it, k) = if p < 0.5 { (a, 1.0 - p * 2.0) } else { (b, (p - 0.5) * 2.0) };
                    if let Some(it) = it {
                        push_item(sc, it, k, Some(Blend::Normal), layers);
                    }
                }
                _ => {
                    // cross dissolve: A at full, B over it at p (premultiplied over == linear mix when A is opaque)
                    if let Some(it) = a {
                        push_item(sc, it, 1.0 - if b.is_none() { p } else { 0.0 }, Some(Blend::Normal), layers);
                    }
                    if let Some(it) = b {
                        push_item(sc, it, p, Some(Blend::Normal), layers);
                    }
                }
            }
            continue;
        }
        let Some(item) = tr.item_at(t) else { continue };
        if !item.enabled {
            continue;
        }
<<<<<<< HEAD
        if project.item(item.item).is_some_and(|p| matches!(p.kind, ItemKind::AdjustmentLayer { .. })) {
            if opts.effects
                && let Some(ops) = adjustment_ops(project, seq, item, t, opts.scale)
            {
                layers.steps.push(PlanStep::Effects(ops));
=======
        if is_adjustment(project, item) {
            if opts.effects {
                push_adjustment(sc, item, layers);
>>>>>>> upstream/main
            }
            continue;
        }
        push_item(sc, item, 1.0, None, layers);
    }
}

/// Whether a nested sequence can be drawn by pushing its own layers into the plan of the sequence
/// it is in, instead of being rendered to an image on the CPU first. Compositing its layers one
/// by one over what is below gives the same picture as compositing them together first only when
/// they all blend Normal, and the frame must be one that plans as layers in the same colour
<<<<<<< HEAD
/// pipeline. (Its frame size may differ: [`push_item`] checks that every layer stays inside it.)
fn nest_is_plain(project: &Project, seq: &Sequence, nested: &Sequence, ft: Tick) -> bool {
    nested.settings.color == seq.settings.color
        && layered_at(project, nested, ft)
=======
/// pipeline. An adjustment layer inside it would adjust the outer tracks below too, so it keeps
/// the nested sequence on the CPU.
fn nest_is_plain(project: &Project, seq: &Sequence, nested: &Sequence, ft: Tick, scale: f32, caps: DeviceCaps) -> bool {
    nested.settings.color == seq.settings.color
        && (nested.settings.width, nested.settings.height) == (seq.settings.width, seq.settings.height)
        && layered_at(project, nested, ft, scale, caps)
        && !nested.video_tracks.iter().any(|tr| tr.enabled && tr.item_at(ft).is_some_and(|i| i.enabled && is_adjustment(project, i)))
>>>>>>> upstream/main
        && !nested.video_tracks.iter().any(|tr| tr.enabled && tr.transitions.iter().any(|t| t.range().contains(ft) && !simple_transition(&t.effect.effect)))
        && nested.video_tracks.iter().filter(|tr| tr.enabled).all(|tr| {
            tr.transitions.iter().any(|x| x.range().contains(ft))
                || tr.item_at(ft).filter(|i| i.enabled).is_none_or(|i| crate::opacity_blend(i, i.effect_time_at(ft)).1 == Blend::Normal)
        })
}

/// What a layer is pushed against: the sequence at time `t`, how it is rendered, what the
/// compositing device runs and how many nests were followed to reach it.
#[derive(Clone, Copy)]
struct Scene<'a> {
    project: &'a Project,
    seq: &'a Sequence,
    t: Tick,
    opts: RenderOptions,
    caps: DeviceCaps,
    sources: &'a dyn SourceProvider,
    nest: u32,
}

/// Push the layer(s) of `item`. `blend` overrides the item's own blend mode: inside a transition
/// the CPU reference mixes the clips and composites the result Normal, ignoring their modes.
/// `nest` counts the multi-camera clips already followed to reach `item` (see
/// [`crate::MAX_NEST_DEPTH`]).
fn push_item(sc: Scene, item: &TrackItem, extra_opacity: f32, blend: Option<Blend>, out: &mut PlanStack) {
    let Scene { project, seq, t, opts, caps, sources, nest } = sc;
    if out.exhausted {
        return;
    }
    // frame time (`ft`) vs. effect time (`mt`): they differ inside a frame hold without Hold Filters
    let ft = item.source_time_at(t);
    let ft = match project.item(item.item).map(|p| &p.kind) {
        Some(ItemKind::Sequence(nested)) => crate::nest_time(seq, nested, ft),
        _ => ft,
    };
    let mt = item.effect_time_at(t);
    let (op, own) = crate::opacity_blend(item, mt);
    let bl = blend.unwrap_or(own);
    // A multi-camera clip that only shows its angle (no effects, untransformed, same frame size)
    // draws the angle's clip directly: no CPU pass over the nested sequence.
    if let Some(ItemKind::Sequence(nested)) = project.item(item.item).map(|p| &p.kind)
        && nest < crate::MAX_NEST_DEPTH
        && let Some(angle) = item.multicam_angle(nested)
        && !(opts.effects && item.has_standard_effects())
        && (nested.settings.width, nested.settings.height) == (seq.settings.width, seq.settings.height)
        && near_identity(&motion_matrix(seq, item, (nested.settings.width, nested.settings.height), Some(nested.settings.par), mt))
        && let Some(tr) = nested.angle_video_track_index(angle).and_then(|i| nested.video_tracks.get(i))
        && !tr.transitions.iter().any(|x| x.range().contains(ft))
        && tr.item_at(ft).is_none_or(|i| crate::opacity_blend(i, i.effect_time_at(ft)).1 != Blend::Dissolve)
    {
        // Inside the nested sequence the angle's clip is composited onto an empty canvas, where
        // every mode but Dissolve is Normal; the multicam clip's own mode then applies to it.
        if let Some(inner) = tr.item_at(ft).filter(|i| i.enabled) {
            push_item(Scene { seq: nested, t: ft, nest: nest + 1, ..sc }, inner, extra_opacity * op, Some(bl), out);
        }
        return;
    }
    // A nested sequence that is only shown (no effects, fully opaque, blending Normal; Motion may
    // move, scale or rotate it) contributes its own layers: the compositor draws them like the
    // clips of this sequence, placed by the nest's Motion, with no CPU pass over the nested
    // sequence. Its captions are part of its picture.
    if let Some(ItemKind::Sequence(nested)) = project.item(item.item).map(|p| &p.kind)
        && nest < crate::MAX_NEST_DEPTH
        && item.multicam_angle(nested).is_none()
        && !(opts.effects && item.has_standard_effects())
        && !item.has_opacity_masks()
        && bl == Blend::Normal
        && extra_opacity * op >= 1.0 - 1e-6
<<<<<<< HEAD
        && nest_is_plain(project, seq, nested, ft)
        && let Some(steps) = nest_layers(project, seq, item, nested, ft, mt, opts, sources, nest)
    {
        for step in steps {
            match step {
                PlanStep::Layers(layers) => layers.into_iter().for_each(|l| out.push(l)),
                nested_step => out.push_nest(nested_step),
            }
=======
        && near_identity(&motion_matrix(seq, item, (nested.settings.width, nested.settings.height), Some(nested.settings.par), mt))
        && nest_is_plain(project, seq, nested, ft, opts.scale, caps)
    {
        push_tracks(Scene { seq: nested, t: ft, nest: nest + 1, ..sc }, out);
        if out.exhausted {
            return;
        }
        let (w, h) = output_size(nested, opts.scale);
        for o in crate::caption_overlays(nested, ft, w, h) {
            out.push(PlanLayer::new(
                Arc::new(VideoFrame::rgba_f32(o.w as u32, o.h as u32, o.px)),
                Affine::translate(o.x as f64, o.y as f64),
                1.0,
                Blend::Normal,
            ));
>>>>>>> upstream/main
        }
        return;
    }
    // A nested sequence with standard effects the GPU runs (Lumetri Color, …): its layers are
    // composited into a picture of its own, the effects run on it and Motion places it, all on the
    // GPU, instead of the CPU rendering the nested sequence and its effects every frame.
    if let Some(ItemKind::Sequence(nested)) = project.item(item.item).map(|p| &p.kind)
        && nest < crate::MAX_NEST_DEPTH
        && item.multicam_angle(nested).is_none()
        && opts.effects
        && item.has_standard_effects()
        && !item.has_opacity_masks()
        && !bl.reads_destination()
        && nested.settings.color == seq.settings.color
        && layered_at(project, nested, ft)
        && let Some(step) = nest_step(project, seq, item, nested, t, ft, mt, opts, sources, nest, op * extra_opacity, bl)
    {
        out.push_nest(step);
        return;
    }
    // Graphic clips without standard effects: the layers are rasterised (cached) into one tight
    // image the GPU places as a layer.
    if !(opts.effects && item.has_standard_effects() || item.has_opacity_masks())
        && project.item(item.item).is_some_and(|p| matches!(p.kind, ItemKind::Graphic { .. }))
    {
        let Some(size) = crate::source_size(project, item.item) else { return };
        let (w, h) = output_size(seq, opts.scale);
        let m = Affine::scale(opts.scale as f64, opts.scale as f64).then_apply(&motion_matrix(seq, item, size, None, mt));
        if let Some((img, x, y)) = crate::graphic_clip::render_graphic_tight(item, mt, size, &m, w, h) {
            out.push(PlanLayer::new(cpu_frame(img), Affine::translate(x as f64, y as f64), op * extra_opacity, bl));
        }
        return;
    }
    if let Some(chain) = gpu_chain(project, item, opts) {
        let Some(src) = sources.source(item.item) else { return };
        let Some(size) = crate::source_size(project, item.item) else { return };
        let motion = motion_matrix(seq, item, size, crate::source_par(project, item.item), mt);
        let lin = ((motion.a * motion.a + motion.b * motion.b).sqrt()).max((motion.c * motion.c + motion.d * motion.d).sqrt());
        let want = (lin * opts.scale as f64).clamp(1.0 / 64.0, 1.0) as f32;
        let time = crate::video_source_time(item, t, src.info().frame_rate());
        // The GPU turns the frame as decoded, the container's display rotation folded into the
        // layer's matrix or its effect stage: turning a 4K frame on the CPU is too slow to play (#3467).
        let Ok((frame, turns)) = src.video_frame_unturned(FrameRequest { time, scale: want }) else { return };
        let upright = |f: &VideoFrame| if turns % 2 == 1 { (f.height, f.width) } else { (f.width, f.height) };
        let cs = crate::colorman::source_space(project, item.item, &frame);
        // HDR and wide-gamut media on a Rec. 709 sequence is converted on the GPU too (#965); log
        // media and other pipelines are converted on the CPU (below), and so are blended
        // in-between frames (Frame Blending / Optical Flow on speed-changed or rate-conformed clips)
        let pipe = &seq.settings.color;
        let input = if crate::colorman::needs_management(pipe, cs, &frame) { crate::colorman::gpu_input(project, item.item, pipe, cs, &frame) } else { None };
        let plain = (input.is_some() || !crate::colorman::needs_management(pipe, cs, &frame))
            && crate::media_interpolation(item, t, &src, seq.settings.frame_rate).is_none();
        // a display-rotated frame falls through to the CPU path: this one has no turn to apply
        if turns == 0 && !plain && !chain.is_empty() && crate::media_interpolation(item, t, &src, seq.settings.frame_rate).is_none() {
            let n = crate::decimation(frame.width as f32, size.0 as f32 * want);
            let image = crate::colorman::decode(project, item.item, &frame, n, pipe);
            let (lw, lh) = (image.w, image.h);
            let px_scale = lw as f32 / size.0.max(1) as f32;
            let cx = crate::effects::FxCtx {
                t: mt,
                px_scale,
                seconds: 0.0,
                timecode: "",
                clip_name: &item.name,
                project: Some(project),
                env: None,
                frame_rate: seq.settings.frame_rate.as_f64(),
                working: seq.settings.color.working,
            };
            let ops: Option<Vec<_>> = chain.iter().map(|e| FxOp::eval(e, &cx, lw, lh).filter(FxOp::gpu_ok)).collect();
            if let Some(ops) = ops {
                let matrix = Affine::scale(opts.scale as f64, opts.scale as f64)
                    .then_apply(&motion)
                    .then_apply(&Affine::scale(1.0 / px_scale as f64, 1.0 / px_scale as f64));
                out.push(PlanLayer {
                    frame: cpu_frame(image),
                    matrix,
                    opacity: op * extra_opacity,
                    blend: bl,
                    fx: Some(Arc::new(LayerFx { size: (lw as u32, lh as u32), decimation: 1, turns: 0, ops })),
                    input: None,
                });
                return;
            }
        }
        if plain && !chain.is_empty() {
            // GPU effect stage: the working image the CPU would decode (`base_layer`), the
            // effects evaluated for it, placed as `item_layer` places it
            let (fw, fh) = upright(&frame);
            let n = crate::decimation(fw as f32, size.0 as f32 * want);
            let (lw, lh) = ((fw as usize / n).max(1), (fh as usize / n).max(1));
            let px_scale = lw as f32 / size.0.max(1) as f32;
            let tc = "";
            let cx = crate::effects::FxCtx {
                t: mt,
                // as the CPU path: effect sizes are capped, mask outlines use the true ratio
                px_scale: px_scale.min(crate::MAX_EFFECT_PX_SCALE),
                mask_scale: px_scale,
                seconds: (t - item.start).seconds(),
                timecode: tc,
                clip_name: &item.name,
                project: Some(project),
                env: None,
                frame_rate: seq.settings.frame_rate.as_f64(),
                working: seq.settings.color.working,
            };
            let ops: Option<Vec<FxOp>> = chain.iter().map(|e| FxOp::eval(e, &cx, lw, lh).filter(FxOp::gpu_ok)).collect();
            if let Some(ops) = ops {
                let m = Affine::scale(opts.scale as f64, opts.scale as f64)
                    .then_apply(&motion)
                    .then_apply(&Affine::scale(1.0 / px_scale as f64, 1.0 / px_scale as f64));
                let fx = LayerFx { size: (lw as u32, lh as u32), decimation: n as u32, turns, ops };
                out.push(PlanLayer { frame, matrix: m, opacity: op * extra_opacity, blend: bl, fx: Some(Arc::new(fx)), input });
                return;
            }
        } else if plain {
            // Draft playback at reduced resolution: hand over planes box-filtered to the size
            // drawn instead of the full picture (a quarter at 1/2, a sixteenth at 1/4 of the
            // upload and sampling). The mean is taken over Y'CbCr codes, not linear light as
            // the shader's supersampling does, so this stays limited to the opt-in draft mode.
            let n = if filmcraft_media::cancel::draft() { crate::decimation(upright(&frame).0 as f32, size.0 as f32 * want) } else { 1 };
            let frame = match frame.box_decimated(n) {
                Some(small) => Arc::new(small),
                None => frame,
            };
            let px_scale = upright(&frame).0 as f64 / size.0.max(1) as f64;
            let m = Affine::scale(opts.scale as f64, opts.scale as f64)
                .then_apply(&motion)
                .then_apply(&Affine::scale(1.0 / px_scale, 1.0 / px_scale))
                .then_apply(&quarter_turns(turns, frame.width as f64, frame.height as f64));
            out.push(PlanLayer { input, ..PlanLayer::new(frame, m, op * extra_opacity, bl) });
            return;
        }
    }
    // CPU-rendered layer (standard effects): drawn by the GPU as a pre-rendered canvas image.
    crate::fallback::note(Scope::Layer, layer_cause(project, seq, item, t, opts, sources));
    let tc = filmcraft_time::format_time(t, seq.settings.frame_rate, seq.settings.drop_frame, filmcraft_time::TimeDisplay::Timecode, 48_000);
    if let Some((img, op2, _)) = crate::item_layer(project, seq, item, t, opts, sources, &tc) {
        out.push(PlanLayer::new(cpu_frame(img), Affine::IDENTITY, op2 * extra_opacity, bl));
    }
}

/// The layers of a plain nested sequence ([`nest_is_plain`]) at its time `ft`, with its captions,
/// placed in the output of `seq` by the nest's Motion: [`PlanStep::Layers`] and the
/// [`PlanStep::Nest`]s of the nests with effects inside it. None when they cannot stand in for the
/// nest's picture: the plan of the nest needs other steps, or a layer reaches outside the nested
/// frame (rendered as a picture, the nest crops it there).
#[allow(clippy::too_many_arguments)]
fn nest_layers(
    project: &Project,
    seq: &Sequence,
    item: &TrackItem,
    nested: &Sequence,
    ft: Tick,
    mt: Tick,
    opts: RenderOptions,
    sources: &dyn SourceProvider,
    nest: u32,
) -> Option<Vec<PlanStep>> {
    let (w, h) = output_size(nested, opts.scale);
    // nested output pixels (at the render scale) → this sequence's output pixels
    let motion = motion_matrix(seq, item, (nested.settings.width, nested.settings.height), Some(nested.settings.par), mt);
    let captions = crate::caption_overlays(nested, ft, w, h);
    // Captions of a moved, scaled or rotated nest are set at the size it is shown (as the
    // picture renders them), not set full size and resampled: render the nest as a picture.
    if !captions.is_empty() && !near_identity(&motion) {
        return None;
    }
    let mut inner = PlanStack::default();
    push_tracks(project, nested, ft, opts, sources, nest + 1, &mut inner);
    for o in captions {
        inner.push(PlanLayer::new(Arc::new(VideoFrame::rgba_f32(o.w as u32, o.h as u32, o.px)), Affine::translate(o.x as f64, o.y as f64), 1.0, Blend::Normal));
    }
    if inner.exhausted {
        return None;
    }
    let steps = match inner.finish(w, h) {
        FramePlan::Layers { layers, .. } => vec![PlanStep::Layers(layers)],
        FramePlan::Composite { steps, .. } if steps.iter().all(|s| matches!(s, PlanStep::Layers(_) | PlanStep::Nest { .. })) => steps,
        _ => return None,
    };
    // Half a pixel of slack: a layer that exactly fills the nest lands on its edges.
    let inside = |(lw, lh): (u32, u32), m: &Affine| {
        [(0.0, 0.0), (lw as f64, 0.0), (0.0, lh as f64), (lw as f64, lh as f64)].into_iter().all(|(x, y)| {
            let p = m.apply(filmcraft_geom::Vec2::new(x, y));
            p.x >= -0.5 && p.y >= -0.5 && p.x <= w as f64 + 0.5 && p.y <= h as f64 + 0.5
        })
    };
    let all_inside = steps.iter().all(|s| match s {
        PlanStep::Layers(layers) => layers.iter().all(|l| inside(l.size(), &l.matrix)),
        PlanStep::Nest { size, matrix, .. } => inside(*size, matrix),
        _ => false,
    });
    if !all_inside {
        return None;
    }
    if near_identity(&motion) {
        return Some(steps);
    }
    let s = f64::from(opts.scale);
    if s <= 0.0 {
        return None;
    }
    let place = Affine::scale(s, s).then_apply(&motion).then_apply(&Affine::scale(1.0 / s, 1.0 / s));
    Some(
        steps
            .into_iter()
            .map(|step| match step {
                PlanStep::Layers(layers) => PlanStep::Layers(layers.into_iter().map(|l| PlanLayer { matrix: place.then_apply(&l.matrix), ..l }).collect()),
                PlanStep::Nest { layers, size, ops, matrix, opacity, blend } => {
                    PlanStep::Nest { layers, size, ops, matrix: place.then_apply(&matrix), opacity, blend }
                }
                other => other,
            })
            .collect(),
    )
}

/// The [`PlanStep::Nest`] of a nested sequence clip with standard effects at timeline `t` (its
/// time `ft` in the nested sequence, effect time `mt`), drawn with `opacity` and `blend`. It
/// mirrors [`crate::item_layer`]: the nest's picture at the size Motion needs, its effects
/// evaluated for that picture, the same placement. None when an effect has no GPU
/// implementation or is masked.
#[allow(clippy::too_many_arguments)]
fn nest_step(
    project: &Project,
    seq: &Sequence,
    item: &TrackItem,
    nested: &Sequence,
    t: Tick,
    ft: Tick,
    mt: Tick,
    opts: RenderOptions,
    sources: &dyn SourceProvider,
    nest: u32,
    opacity: f32,
    blend: Blend,
) -> Option<PlanStep> {
    let chain = gpu_effects(item, opts)?;
    let size = crate::source_size(project, item.item)?;
    let motion = motion_matrix(seq, item, size, crate::source_par(project, item.item), mt);
    let lin = ((motion.a * motion.a + motion.b * motion.b).sqrt()).max((motion.c * motion.c + motion.d * motion.d).sqrt());
    let want = (lin * opts.scale as f64).clamp(1.0 / 64.0, 1.0) as f32;
    let (nw, nh) = output_size(nested, want);
    let mut inner = PlanStack::default();
    push_tracks(project, nested, ft, RenderOptions { scale: want, ..opts }, sources, nest + 1, &mut inner);
    // a nested sequence's captions are part of its picture (see `crate::base_layer`)
    for o in crate::caption_overlays(nested, ft, nw, nh) {
        inner.push(PlanLayer::new(Arc::new(VideoFrame::rgba_f32(o.w as u32, o.h as u32, o.px)), Affine::translate(o.x as f64, o.y as f64), 1.0, Blend::Normal));
    }
    let layers = inner.into_layers(nw, nh)?;
    let px_scale = nw as f32 / size.0.max(1) as f32;
    let cx = crate::effects::FxCtx {
        t: mt,
        px_scale,
        seconds: (t - item.start).seconds(),
        timecode: "",
        clip_name: &item.name,
        project: Some(project),
        env: None,
        frame_rate: seq.settings.frame_rate.as_f64(),
        working: seq.settings.color.working,
    };
    let ops = chain.iter().map(|e| FxOp::eval(e, &cx, nw, nh).filter(FxOp::gpu_ok)).collect::<Option<Vec<_>>>()?;
    let matrix =
        Affine::scale(opts.scale as f64, opts.scale as f64).then_apply(&motion).then_apply(&Affine::scale(1.0 / px_scale as f64, 1.0 / px_scale as f64));
    Some(PlanStep::Nest { layers, size: (nw as u32, nh as u32), ops, matrix, opacity, blend })
}

/// Maps a `w`×`h` frame's pixels to the frame turned clockwise by `q` quarter turns, as
/// [`VideoFrame::rotated`] turns it.
pub fn quarter_turns(q: u8, w: f64, h: f64) -> Affine {
    match q % 4 {
        1 => Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, e: h, f: 0.0 },
        2 => Affine { a: -1.0, b: 0.0, c: 0.0, d: -1.0, e: w, f: h },
        3 => Affine { a: 0.0, b: -1.0, c: 1.0, d: 0.0, e: 0.0, f: w },
        _ => Affine::IDENTITY,
    }
}

fn near_identity(m: &Affine) -> bool {
    (m.a - 1.0).abs() < 1e-9 && (m.d - 1.0).abs() < 1e-9 && m.b.abs() < 1e-9 && m.c.abs() < 1e-9 && m.e.abs() < 1e-6 && m.f.abs() < 1e-6
}

/// A layer's working image with its effects applied, on the CPU (the reference for the GPU
/// effect stage).
pub fn effect_image(frame: &VideoFrame, fx: &LayerFx) -> crate::Image {
    effect_image_with(frame, fx, None)
}

/// [`effect_image`] of a colour-managed layer ([`PlanLayer::input`]).
pub fn effect_image_with(frame: &VideoFrame, fx: &LayerFx, input: Option<&filmcraft_color::InputTransform>) -> crate::Image {
    let mut img = turned(linear_image(frame, fx.decimation.max(1) as usize, input), fx.turns);
    for op in &fx.ops {
        op.apply(&mut img);
    }
    img
}

/// Execute a plan on the CPU (reference for the GPU compositor).
pub fn execute_cpu(plan: &FramePlan) -> crate::Image {
    match plan {
        FramePlan::Image(img) => img.clone(),
        FramePlan::Composite { width, height, steps } => {
            let mut canvas = crate::Image::new(*width, *height);
            for step in steps {
                match step {
                    PlanStep::Effects(ops) => {
                        for op in ops {
                            op.apply(&mut canvas);
                        }
                    }
                    PlanStep::Layers(layers) => execute_layers(&mut canvas, layers),
                    PlanStep::Transition { inputs, effect, progress, scale } => {
                        let [a, b] = inputs.each_ref().map(|layers| {
                            let mut input = crate::Image::new(*width, *height);
                            execute_layers(&mut input, layers);
                            input
                        });
                        let mixed = crate::transitions::apply_scaled(effect, &a, &b, *progress, *scale);
                        crate::blend::composite(&mut canvas, &mixed, 1.0, Blend::Normal);
                    }
<<<<<<< HEAD
                    PlanStep::Nest { layers, size, ops, matrix, opacity, blend } => {
                        let mut picture = crate::Image::new(size.0 as usize, size.1 as usize);
                        execute_layers(&mut picture, layers);
                        for op in ops {
                            op.apply(&mut picture);
                        }
                        let placed = picture.transformed(*width, *height, matrix);
                        crate::blend::composite(&mut canvas, &placed, *opacity, *blend);
=======
                    PlanStep::Adjust { ops } => {
                        for op in ops {
                            op.apply(&mut canvas);
                        }
>>>>>>> upstream/main
                    }
                }
            }
            canvas
        }
        FramePlan::Layers { width, height, layers } => {
            let mut canvas = crate::Image::new(*width, *height);
            execute_layers(&mut canvas, layers);
            canvas
        }
    }
}

/// A layer's frame (box-decimated by `n`) as a linear working image: the default decode, or
/// `input`'s table and pixel stage as `colorman::decode` runs them.
fn linear_image(frame: &VideoFrame, n: usize, input: Option<&filmcraft_color::InputTransform>) -> crate::Image {
    let (w, h, px) = frame.to_linear_f32_decimated_with(n, input.map(|t| &t.table));
    let mut img = crate::Image { w, h, px };
    if let Some(t) = input.filter(|t| t.has_pixel_stage()) {
        img.map_rgb(|c, _, _| t.apply(c));
    }
    img
}

/// `img` turned clockwise by `q` quarter turns, as [`VideoFrame::rotated`] turns a frame.
fn turned(img: crate::Image, q: u8) -> crate::Image {
    if q.is_multiple_of(4) {
        return img;
    }
    let (w, h) = (img.w, img.h);
    let (ow, oh) = if q % 2 == 1 { (h, w) } else { (w, h) };
    let mut px = vec![0.0; img.px.len()];
    for (i, p) in img.px.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        let (ox, oy) = match q % 4 {
            1 => (h - 1 - y, x),
            2 => (w - 1 - x, h - 1 - y),
            _ => (y, w - 1 - x),
        };
        px[(oy * ow + ox) * 4..][..4].copy_from_slice(p);
    }
    crate::Image { w: ow, h: oh, px }
}

fn execute_layers(canvas: &mut crate::Image, layers: &[PlanLayer]) {
    let (width, height) = (canvas.w, canvas.h);
    for l in layers {
        let src = match &l.fx {
            Some(fx) => effect_image_with(&l.frame, fx, l.input.as_deref()),
            None => linear_image(&l.frame, 1, l.input.as_deref()),
        };
        let placed = if l.matrix == Affine::scale(width as f64, height as f64) && src.w == 1 && src.h == 1 {
            crate::Image::filled(width, height, src.get(0, 0))
        } else {
            src.transformed(width, height, &l.matrix)
        };
        crate::blend::composite(canvas, &placed, l.opacity, l.blend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turns_place_a_frame_as_rotated_turns_it() {
        // 3×2 frame, every pixel distinct
        let px: Vec<f32> = (0..6).flat_map(|i| [i as f32 / 8.0, 0.0, 0.0, 1.0]).collect();
        let frame = VideoFrame::rgba_f32(3, 2, px);
        for q in 0..4u8 {
            let upright = Arc::new(frame.rotated(q));
            let (w, h) = (upright.width as usize, upright.height as usize);
            let want = execute_cpu(&FramePlan::Layers { width: w, height: h, layers: vec![PlanLayer::new(upright, Affine::IDENTITY, 1.0, Blend::Normal)] });
            let m = quarter_turns(q, 3.0, 2.0);
            let got = execute_cpu(&FramePlan::Layers { width: w, height: h, layers: vec![PlanLayer::new(Arc::new(frame.clone()), m, 1.0, Blend::Normal)] });
            assert_eq!(got.px, want.px, "q = {q}");
            // the effect stage's turn of a working image too
            let img = turned(crate::Image { w: 3, h: 2, px: (0..24).map(|i| i as f32).collect() }, q);
            let frame24 = VideoFrame::rgba_f32(3, 2, (0..24).map(|i| i as f32).collect()).rotated(q);
            let filmcraft_frame::PixelData::RgbaF32(px) = &frame24.data else { panic!("rgba f32") };
            assert_eq!((img.w as u32, img.h as u32, &img.px[..]), (frame24.width, frame24.height, &px[..]), "q = {q}");
        }
    }

    #[test]
    fn flattening_an_operation_preserves_its_picture() {
        let frame = Arc::new(VideoFrame::rgba_f32(1, 1, vec![0.25, 0.0, 0.0, 0.5]));
        let steps = vec![PlanStep::Transition {
            inputs: [vec![PlanLayer::new(frame, Affine::IDENTITY, 1.0, Blend::Normal)], vec![]],
            effect: filmcraft_project::find_effect("push").unwrap().instance(),
            progress: 0.0,
            scale: 1.0,
        }];
        let expected = execute_cpu(&FramePlan::Composite { width: 1, height: 1, steps: steps.clone() });
        let layers = PlanStack { steps, exhausted: false, cpu_only: false, layers: 1 }.into_layers(1, 1).unwrap();
        let got = execute_cpu(&FramePlan::Layers { width: 1, height: 1, layers });
        assert_eq!(got.px, expected.px);
        assert!(got.px.iter().any(|v| *v != 0.0));
    }

    #[test]
    fn layer_plan_has_a_global_budget() {
        let frame = Arc::new(VideoFrame::rgba_f32(1, 1, vec![0.0; 4]));
        let mut stack = PlanStack::default();
        for _ in 0..=MAX_PLAN_LAYERS {
            stack.push(PlanLayer::new(frame.clone(), Affine::IDENTITY, 1.0, Blend::Normal));
        }
        assert!(stack.exhausted);
        assert_eq!(stack.layer_count(), MAX_PLAN_LAYERS);
    }
}
