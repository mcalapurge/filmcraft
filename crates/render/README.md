# filmcraft-render

CPU reference rendering, GPU frame planning and timeline preview estimation.

Render-bar GPU costs are lightweight hints based on effect support and masks. Whole-frame
eligibility is checked once per segment; estimates never build Lumetri shader tables or copy
LUT payloads. HSL effects use a conservative multipass cost. Evaluated parameter validity and
adapter/resource limits remain the actual renderer's responsibility, so a hint can be optimistic
when an effect falls back at render time. This shared path applies to native and web builds.

`preview_tests::gpu_cost_estimation_never_builds_lumetri_payloads` covers graded media and
adjustment layers, and masked adjustment fallback, with a thread-local payload-build counter.
