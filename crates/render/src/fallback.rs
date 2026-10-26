//! Why GPU frame plans hand work to the CPU (`perf.stats` `render.cpuFallbacks`).
//!
//! [`crate::plan::plan_frame`] records a [`Fallback`] each time it sends a whole frame
//! ([`Scope::Frame`]) or one layer ([`Scope::Layer`]) to the CPU reference renderer, and the GPU
//! compositor records the few cases it declines itself. Counts are per planned frame or layer,
//! process-wide, and only grow: diff two readings to measure an interval, like the other
//! counters. They tell which missing GPU implementation costs the most on a real project.
//!
//! Effect and transition ids come from the catalogue ([`filmcraft_project::EffectDef`]'s
//! `&'static str`), never from the project file, so a hostile project cannot grow the table: an id
//! the catalogue doesn't know is recorded as [`Fallback::UnknownEffect`].

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use filmcraft_project::EffectInstance;

/// Whether a fallback cost the whole frame or one layer of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Frame,
    Layer,
}

/// What sent a frame or a layer to the CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fallback {
    /// A standard effect without a GPU implementation.
    Effect(&'static str),
    /// A GPU effect whose values the shader doesn't reproduce (non-finite numbers, the CPU's mip
    /// path, …).
    EffectValues(&'static str),
    /// An effect id the catalogue doesn't know.
    UnknownEffect,
    /// An effect with a mask.
    MaskedEffect,
    /// An opacity mask on the clip.
    OpacityMask,
    /// A transition the compositor doesn't draw.
    Transition(&'static str),
    /// A transition into or out of an adjustment layer.
    AdjustmentTransition,
    /// An adjustment layer with masks, opacity below 100 %, a blend mode or Motion.
    AdjustmentLayer,
    /// A nested sequence the plan can't draw as layers or as its own picture.
    Nest,
    /// A clip that isn't media (a graphic with standard effects, a generator item, …).
    NotMedia,
    /// Frame blending or optical flow on a speed-changed clip.
    FrameBlending,
    /// Colour management the GPU input transform doesn't cover (log media, other pipelines).
    ColorManagement,
    /// More layers than a plan holds.
    LayerLimit,
    /// A working image or nested picture larger than the GPU's largest texture.
    TextureSize,
    /// The GPU device has no effect stage.
    NoEffectStage,
    /// Plan steps the compositor declined (transitions in an export, a step it can't draw).
    CompositorSteps,
}

impl Fallback {
    /// The key in `perf.stats`, e.g. `effect:ultra_key`, `opacityMask`.
    pub fn key(self) -> String {
        match self {
            Fallback::Effect(id) => format!("effect:{id}"),
            Fallback::EffectValues(id) => format!("effectValues:{id}"),
            Fallback::Transition(id) => format!("transition:{id}"),
            Fallback::UnknownEffect => "unknownEffect".into(),
            Fallback::MaskedEffect => "maskedEffect".into(),
            Fallback::OpacityMask => "opacityMask".into(),
            Fallback::AdjustmentTransition => "adjustmentTransition".into(),
            Fallback::AdjustmentLayer => "adjustmentLayer".into(),
            Fallback::Nest => "nest".into(),
            Fallback::NotMedia => "notMedia".into(),
            Fallback::FrameBlending => "frameBlending".into(),
            Fallback::ColorManagement => "colorManagement".into(),
            Fallback::LayerLimit => "layerLimit".into(),
            Fallback::TextureSize => "textureSize".into(),
            Fallback::NoEffectStage => "noEffectStage".into(),
            Fallback::CompositorSteps => "compositorSteps".into(),
        }
    }

    /// [`Fallback::Effect`] for `e`, by its catalogue id.
    pub fn effect(e: &EffectInstance) -> Self {
        e.def().map_or(Fallback::UnknownEffect, |d| Fallback::Effect(d.id))
    }

    /// [`Fallback::EffectValues`] for `e`, by its catalogue id.
    pub fn effect_values(e: &EffectInstance) -> Self {
        e.def().map_or(Fallback::UnknownEffect, |d| Fallback::EffectValues(d.id))
    }

    /// [`Fallback::Transition`] for a transition's effect, by its catalogue id.
    pub fn transition(e: &EffectInstance) -> Self {
        e.def().map_or(Fallback::UnknownEffect, |d| Fallback::Transition(d.id))
    }
}

fn table() -> &'static Mutex<HashMap<(Scope, Fallback), u64>> {
    static T: OnceLock<Mutex<HashMap<(Scope, Fallback), u64>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Count one frame or layer sent to the CPU for `why`.
pub fn note(scope: Scope, why: Fallback) {
    let mut t = table().lock().unwrap_or_else(PoisonError::into_inner);
    let n = t.entry((scope, why)).or_insert(0);
    *n = n.saturating_add(1);
}

/// Every count so far, highest first (ties by key).
pub fn counts() -> Vec<(Scope, Fallback, u64)> {
    let t = table().lock().unwrap_or_else(PoisonError::into_inner);
    let mut v: Vec<_> = t.iter().map(|((s, f), n)| (*s, *f, *n)).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.key().cmp(&b.1.key())));
    v
}

/// Total frames and layers sent to the CPU so far.
pub fn totals() -> (u64, u64) {
    counts().iter().fold((0u64, 0u64), |(f, l), (s, _, n)| match s {
        Scope::Frame => (f.saturating_add(*n), l),
        Scope::Layer => (f, l.saturating_add(*n)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_name_the_catalogue_id() {
        assert_eq!(Fallback::Effect("ultra_key").key(), "effect:ultra_key");
        assert_eq!(Fallback::Transition("clock_wipe").key(), "transition:clock_wipe");
        assert_eq!(Fallback::OpacityMask.key(), "opacityMask");
        let mut e = filmcraft_project::find_effect("ultra_key").unwrap().instance();
        assert_eq!(Fallback::effect(&e), Fallback::Effect("ultra_key"));
        e.effect = "not_an_effect_id_from_anywhere".into();
        assert_eq!(Fallback::effect(&e), Fallback::UnknownEffect, "ids outside the catalogue don't grow the table");
    }

    #[test]
    fn notes_accumulate_per_scope() {
        let (f0, l0) = totals();
        note(Scope::Layer, Fallback::TextureSize);
        note(Scope::Layer, Fallback::TextureSize);
        note(Scope::Frame, Fallback::LayerLimit);
        let (f1, l1) = totals();
        assert!(f1 > f0 && l1 >= l0 + 2, "{f0}/{l0} → {f1}/{l1}");
        assert!(counts().iter().any(|(s, f, n)| *s == Scope::Layer && *f == Fallback::TextureSize && *n >= 2));
    }
}
