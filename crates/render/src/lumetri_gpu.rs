//! Evaluated Lumetri tables and coefficients shared with the Metal/WGSL renderer.
//! No pixels are graded here: curves keep the CPU's 1024-sample tables and imported LUTs
//! retain their original shaper/cube and tetrahedral interpolation.
use crate::{
    Image,
    effects::{self, FxCtx, b, choice, color, curve_param, f, grade_space, on, text, wheel_rgb},
};
use filmcraft_color::{GradeSpace, Lut, WorkingSpace, linear_to_srgb, rgb_to_hsl};
use filmcraft_project::{EffectInstance, Project, ProjectLut};
use filmcraft_time::Tick;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    pub(crate) static EVALUATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Tables are bounded below the minimum storage-buffer binding limit of supported adapters.
pub const MAX_DATA_FLOATS: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq)]
pub struct LumetriGpu {
    // Storage header (float indices): 0/4 GradeSpace; 8–31 basic/creative/vignette;
    // 32–47 looks/wheels; 48–59 HSL key/correction; 64–93 six selective bands;
    // 96–104 curve offsets (1024 entries each); 108/109 input/look LUT descriptors.
    // A LUT descriptor is 16 floats: shaper offset/count/min/max, cube offset/size/min/max.
    // All offsets are exact f32 integers because the complete buffer is capped below 2^24.
    pub data: Arc<Vec<f32>>,
    pub hsl: bool,
    pub selective: bool,
    pub median_radius: u32,
    pub mask_blur: (Vec<u32>, Vec<u32>),
    pub sharpen: f32,
    pub sharpen_blur: (Vec<u32>, Vec<u32>),
    effect: EffectInstance,
    luts: Vec<ProjectLut>,
    time: Tick,
    scale: f32,
    working: WorkingSpace,
}
fn grade(gs: GradeSpace) -> [f32; 4] {
    match gs {
        GradeSpace::Sdr => [0.0, 0.0, 1.0, 1.0],
        GradeSpace::Pq { white, norm } => [1.0, white, norm, 1.0],
        GradeSpace::Hlg { white, gamma } => [2.0, white, 1.0, gamma],
    }
}
fn append_lut(data: &mut Vec<f32>, lut: &Lut) -> Option<usize> {
    let count = lut.shaper.as_ref().map_or(0, |s| s.data.len()).checked_add(lut.cube.as_ref().map_or(0, |s| s.data.len()))?;
    if data.len().checked_add(16)?.checked_add(count.checked_mul(3)?)? > MAX_DATA_FLOATS {
        return None;
    }
    let base = data.len();
    data.resize(base + 16, 0.0);
    if let Some(s) = &lut.shaper {
        if s.data.len() < 2 || s.domain_min.iter().zip(s.domain_max).any(|(lo, hi)| !lo.is_finite() || !hi.is_finite() || hi <= *lo) {
            return None;
        }
        data[base] = data.len() as f32;
        data[base + 1] = s.data.len() as f32;
        data[base + 2..base + 5].copy_from_slice(&s.domain_min);
        data[base + 5..base + 8].copy_from_slice(&s.domain_max);
        data.extend(s.data.iter().flatten());
    }
    if let Some(s) = &lut.cube {
        if s.domain_min.iter().zip(s.domain_max).any(|(lo, hi)| !lo.is_finite() || !hi.is_finite() || hi <= *lo)
            || s.size < 2
            || s.size.checked_pow(3)? != s.data.len()
        {
            return None;
        }
        data[base + 8] = data.len() as f32;
        data[base + 9] = s.size as f32;
        data[base + 10..base + 13].copy_from_slice(&s.domain_min);
        data[base + 13..base + 16].copy_from_slice(&s.domain_max);
        data.extend(s.data.iter().flatten());
    }
    Some(base)
}
impl LumetriGpu {
    pub fn eval(e: &EffectInstance, cx: &FxCtx, w: usize, h: usize) -> Option<Self> {
        #[cfg(test)]
        EVALUATIONS.with(|count| count.set(count.get() + 1));
        if !cx.px_scale.is_finite() || !(0.0..=16.0).contains(&cx.px_scale) {
            return None;
        }
        // Reject malformed curves before the interpolation routine allocates temporary tables.
        for p in e.params.values() {
            if p.value.as_f64().is_some() && !p.f64_at(cx.t).is_finite() {
                return None;
            }
            if let Some(c) = p.value.as_curve()
                && (c.len() > 4096 || c.iter().flatten().any(|v| !v.is_finite()))
            {
                return None;
            }
        }
        let (basic, creative, curves, wheels) = (on(e, "basic_on"), on(e, "creative_on"), on(e, "curves_on"), on(e, "wheels_on"));
        let bf = |id| if basic { f(e, id, cx) } else { 0.0 };
        let cf = |id| if creative { f(e, id, cx) } else { 0.0 };
        let mut d = vec![0.0; 112];
        d[0..4].copy_from_slice(&grade(grade_space(e, cx, "hdr_white")));
        d[4..8].copy_from_slice(&grade(grade_space(e, cx, "curves_hdr_range")));
        let exposure = 2f32.powf(bf("exposure"));
        d[8..12].copy_from_slice(&[
            (1.0 + 0.35 * bf("temperature") / 100.0) * exposure,
            (1.0 - 0.3 * bf("tint") / 100.0) * exposure,
            (1.0 - 0.35 * bf("temperature") / 100.0) * exposure,
            bf("hdr_specular") / 100.0,
        ]);
        d[12..16].copy_from_slice(&[
            -bf("blacks") / 100.0 * 0.15,
            1.0 - bf("whites") / 100.0 * 0.15,
            bf("shadows") / 100.0 * 0.35,
            bf("highlights") / 100.0 * 0.35,
        ]);
        d[16..20].copy_from_slice(&[
            bf("contrast") / 100.0,
            cf("faded_film") / 100.0,
            if basic { f(e, "saturation", cx) / 100.0 } else { 1.0 } * if creative { f(e, "creative_sat", cx) / 100.0 } else { 1.0 },
            cf("vibrance") / 100.0,
        ]);
        let st = color(e, "shadow_tint", cx);
        let ht = color(e, "highlight_tint", cx);
        d[20..24].copy_from_slice(&[st[0], st[1], st[2], creative as u8 as f32]);
        d[24..28].copy_from_slice(&[ht[0], ht[1], ht[2], if on(e, "vignette_on") { f(e, "vignette_amount", cx) } else { 0.0 }]);
        d[28..32].copy_from_slice(&[
            f(e, "vignette_midpoint", cx) / 100.0,
            crate::gpufx::lumetri_vignette_aspect(w as f32 / h.max(1) as f32, f(e, "vignette_roundness", cx) / 100.0),
            f(e, "vignette_feather", cx) / 100.0,
            0.0,
        ]);
        d[32..36].copy_from_slice(&[
            if creative { choice(e, "look") as f32 } else { 0.0 },
            f(e, "look_intensity", cx) / 100.0,
            wheels as u8 as f32,
            b(e, "hsl_on") as u8 as f32,
        ]);
        for (offset, wheel, light) in
            [(36, "wheel_shadows", "wheel_shadows_l"), (40, "wheel_midtones", "wheel_midtones_l"), (44, "wheel_highlights", "wheel_highlights_l")]
        {
            let v = e.param(wheel).map(|p| p.vec2_at(cx.t)).unwrap_or_default();
            if !v.x.is_finite() || !v.y.is_finite() {
                return None;
            }
            d[offset..offset + 3].copy_from_slice(&wheel_rgb(v));
            d[offset + 3] = f(e, light, cx) / 100.0;
        }
        d[48..52].copy_from_slice(&[
            f(e, "hsl_hue", cx) / 360.0,
            (f(e, "hsl_hue_range", cx) / 360.0).max(1e-3),
            f(e, "hsl_sat_min", cx) / 100.0,
            f(e, "hsl_luma_min", cx) / 100.0,
        ]);
        d[52..56].copy_from_slice(&[
            f(e, "hsl_luma_max", cx) / 100.0,
            (f(e, "hsl_soft", cx) / 100.0 * 0.3).max(0.01),
            choice(e, "hsl_show_mask") as f32,
            f(e, "hsl_temp", cx) / 100.0,
        ]);
        d[56..60].copy_from_slice(&[
            f(e, "hsl_tint", cx) / 100.0,
            f(e, "hsl_sat", cx) / 100.0,
            f(e, "hsl_hue_shift", cx) / 360.0,
            f(e, "hsl_denoise", cx).clamp(0.0, 100.0) / 100.0,
        ]);
        for (i, band) in ["red", "yellow", "green", "cyan", "blue", "magenta"].iter().enumerate() {
            let id = format!("selective_{band}_color");
            let target =
                e.param(&id).and_then(|p| p.value_at(cx.t).as_color()).or_else(|| e.def().and_then(|def| def.param(&id)).and_then(|p| p.default.as_color()))?;
            let rgb = [target[0], target[1], target[2]].map(|v| linear_to_srgb(v.clamp(0.0, 1.0)));
            let value = |field| f(e, &format!("selective_{band}_{field}"), cx);
            d[64 + i * 5..69 + i * 5].copy_from_slice(&[
                rgb_to_hsl(rgb[0], rgb[1], rgb[2])[0],
                value("range").clamp(0.1, 3.0) / 6.0,
                value("hue").clamp(-180.0, 180.0) / 360.0,
                value("sat").clamp(-100.0, 100.0) / 100.0,
                value("luma").clamp(-100.0, 100.0) / 100.0,
            ]);
        }
        for (i, id) in
            ["curve_luma", "curve_red", "curve_green", "curve_blue", "hue_vs_sat", "hue_vs_hue", "hue_vs_luma", "luma_vs_sat", "sat_vs_sat"].iter().enumerate()
        {
            if !curves {
                break;
            }
            if let Some(points) = curve_param(e, id) {
                let table = if i < 4 {
                    if effects::is_identity_curve(&points) { None } else { Some(effects::curve_lut(&points, 1024)) }
                } else {
                    effects::hue_lut(&points, 1024)
                };
                if let Some(table) = table {
                    d[96 + i] = d.len() as f32;
                    d.extend(table);
                }
            }
        }
        for (slot, enabled, id) in [(108, basic, "input_lut"), (109, creative, "look_lut")] {
            if enabled && let Some(lut) = crate::luts::resolve(cx.project, text(e, id)) {
                d[slot] = append_lut(&mut d, &lut)? as f32;
                if slot == 109 {
                    d[32] = 0.0;
                }
            }
        }
        if d.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let denoise = d[59];
        let blur = f(e, "hsl_blur", cx).clamp(0.0, 100.0) / 100.0 * 20.0 * cx.px_scale;
        let sharpen = cf("sharpen") / 100.0;
        if !blur.is_finite() || !sharpen.is_finite() {
            return None;
        }
        if denoise > 0.0 && ((1.0 + 2.0 * denoise) * cx.px_scale).round().max(1.0) > 8.0 {
            return None;
        }
        let luts = cx
            .project
            .map(|p| p.luts.iter().filter(|l| [text(e, "input_lut"), text(e, "look_lut")].contains(&format!("lib:{}", l.id).as_str())).cloned().collect())
            .unwrap_or_default();
        Some(Self {
            data: Arc::new(d),
            hsl: b(e, "hsl_on"),
            selective: effects::selective_active(e, cx),
            median_radius: if denoise > 0.0 { ((1.0 + 2.0 * denoise) * cx.px_scale).round().max(1.0) as u32 } else { 0 },
            mask_blur: effects::gaussian_boxes(w, h, blur, blur),
            sharpen,
            sharpen_blur: effects::gaussian_boxes(w, h, 1.2 * cx.px_scale.max(0.35), 1.2 * cx.px_scale.max(0.35)),
            effect: e.clone(),
            luts,
            time: cx.t,
            scale: cx.px_scale,
            working: cx.working,
        })
    }
    pub fn apply(&self, img: &mut Image) {
        let project = Project { luts: self.luts.clone(), ..Default::default() };
        let cx = FxCtx {
            t: self.time,
            px_scale: self.scale,
            seconds: 0.0,
            timecode: "",
            clip_name: "",
            project: Some(&project),
            env: None,
            frame_rate: 24.0,
            working: self.working,
        };
        effects::lumetri(img, &self.effect, &cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{ParamValue, find_effect};
    fn context() -> FxCtx<'static> {
        FxCtx {
            t: Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 24.0,
            working: WorkingSpace::Rec709,
        }
    }
    #[test]
    fn hostile_gpu_tables_and_parameters_are_bounded() {
        let mut effect = find_effect("lumetri").unwrap().instance();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e30] {
            effect.params.get_mut("exposure").unwrap().value = ParamValue::Float(value);
            assert!(std::panic::catch_unwind(|| LumetriGpu::eval(&effect, &context(), 32, 24)).unwrap().is_none());
        }
        effect.params.get_mut("exposure").unwrap().value = ParamValue::Float(0.0);
        effect.params.get_mut("curve_luma").unwrap().value = ParamValue::Curve(vec![[0.0, 0.0]; 4097]);
        assert!(LumetriGpu::eval(&effect, &context(), 32, 24).is_none());
        let mut data = vec![0.0; 112];
        let lut = Lut {
            title: String::new(),
            shaper: Some(filmcraft_color::Lut1d { domain_min: [0.0; 3], domain_max: [0.0; 3], data: vec![[0.0; 3]; 2] }),
            cube: None,
        };
        assert!(append_lut(&mut data, &lut).is_none());
    }
}
