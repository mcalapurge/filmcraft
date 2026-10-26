# Formal review: vertical content enhancements

Review date: 2026-10-26. Reviewer: Codex. Fix-forward branch:
`codex/vertical-content-review-fixes`. Original branch remains unchanged.

## Progress report

- [x] Pin and compare original branch, local/fork main, upstream main and latest release.
- [x] Review contributor rules and map every added feature to acceptance evidence.
- [x] Install build/oracle/browser tooling and official pinned fonts.
- [x] Create a separate fix-forward branch, leaving the original branch intact.
- [x] Reproduce and repair review findings with regression coverage.
- [x] Build final desktop/CLI releases and the actual browser bundle.
- [x] Verify native UI screenshots, real Metal parity and native speech acceptance.
- [x] Complete final full gates without optional fonts (exit 0).
- [x] Pass formatting and all-target Clippy with official fonts.
- [x] Complete workspace tests/doctests and remaining official-font gates (exit 0).
- [x] Finish final Chrome speech/OPFS/undo smoke check (18 recognized words; actual WASM bundle).
- [x] Record final validation, commit fixes and verify contributor authorship (`3ae77200`).

Progress is marked only after the associated work succeeds. Both final font configurations and
browser/native checks passed. Concurrent native builds with different font inputs caused a
rustdoc dependency-artifact mismatch (`E0463`); the successful final official-font run was
serialized to avoid changing compiled dependencies while doctests use them.

## Baselines and scope

| Baseline | Pinned revision | Relationship to reviewed branch |
|---|---|---|
| `vertical-content-enhancements-split` | `57b498b5` | Original review head |
| Local main, fork main, upstream main | `8fda2f232198c4c79ff9cfe5b6a347bb1375fa9e` | Exact ancestor; 0 missing, 26 additional commits including merges |
| Upstream release branch / latest release v0.13.0 | `30536bc5d0ddf690f2181b7b97182db8658a6038` | Exact ancestor; 0 missing, 167 additional commits including merges |

Upstream refs and the latest release were read from GitHub and fetched locally. The feature diff
against main is 120 files, 8,979 insertions and 486 deletions. The release comparison includes
329 files, 25,203 insertions and 4,220 deletions because it also includes upstream changes between
v0.13.0 and main. Those inherited changes are integration context, rather than newly attributed
feature work. In particular, upstream replaced the docking model and text/IME routing after the
release; the workspace test below had missed that integration.

No feature PR or separate acceptance specification was found for this branch in the fork or
upstream. Acceptance was derived from its commit descriptions, public feature documentation,
command contracts and AGENTS.md / CLAUDE.md / contributor instructions.

## Findings and disposition

All findings listed here are addressed on the fix-forward branch. P1 means a merge blocker with
potential data loss, output corruption or application failure; P2 means a correctness, validation or integration defect.

| Priority | Finding | Fix and regression evidence |
|---|---|---|
| P1 | A completed browser transcription could modify a replacement project whose item ids were reused. | Bind pending transcription to `document_epoch`; discard stale results before project/history mutation. `finished_transcription_cannot_edit_a_replaced_document_with_reused_ids` failed before the fix. |
| P1 | A second export batch could reuse filenames reserved by earlier ready/encoding entries and overwrite their outputs. | Reserve paths across batches, case-fold on macOS/Windows, fail atomically when no unique name is available. `queue_batches_reserve_paths_already_waiting_for_export` failed before the fix. |
| P1 | The official font build panicked when the caption font picker reached its Devanagari sample; the font was embedded but omitted from UI fallback stacks. | Install remaining craft-fonts scripts after the ordered language fallbacks; cover the sample in every UI family/theme under `catch_unwind`. The official-font gate reproduced the panic before the fix. |
| P2 | Portrait workspace integration test referenced the removed `DockNode` API and prevented the entire all-targets lint gate from compiling. | Use the shared docking `Layout`/`Node` API and verify the Program panel is frontmost beside the timeline. Initial `cargo xtask ci` reproduced the compile failure. |
| P2 | Trims/extracts retained removed words in sidecar text; removing every timed word could restore the old sentence as a plain caption. Inserts and overwrites did not split/rebase timings. | Synchronize text with surviving words, split and rebase inserted/overwritten caption pieces. Four focused regressions failed before the fix. |
| P2 | Frame-rate conform snapped caption starts without compensating word offsets, moving the spoken words. | Rebase offsets by the snapped edge delta. `conform_keeps_live_words_at_their_original_sequence_times` failed before the fix. |
| P2 | Long captions fitted at resting size could clip when the active word popped, especially with edge alignment; popped words could also collide with their neighbors. | Reserve maximum pop/box/outline extents during fitting and alignment, plus stable inter-word spacing. Test covers Pop/Box, 2× enlargement and left/center/right alignment; it failed before the fix. A native screenshot exposed the spacing defect and a pixel regression reproduced it. |
| P2 | A transcription follow-up could generate captions in whichever sequence happened to be active when the job finished. | Save the starting sequence; preserve media transcripts but skip follow-ups with an actionable message if focus changed. Regression failed before the fix. |
| P1 | Transcript normalization expanded missing speaker labels up to an input-derived numeric index, allowing enormous allocations on the speech-result/import path. | Compact sparse extreme indices by identity, preserving display labels; allocation follows the actual word list. A synthetic `u32::MAX` regression runs under `catch_unwind`. |
| P2 | A speech host returning fewer transcripts than requested silently committed a partial edit and reported success. | Validate result count and surviving media before committing; missing results fail the job and preserve undo history. Regression failed before the fix. |
| P2 | Browser speech jobs had an unlimited backlog, and the audio cap applied separately to each job. | Limit pending speech jobs to eight and share the two-hour audio budget across pending jobs. Backlog regression failed before the fix. |
| P2 | Batch export accepted mixed invalid sequence ids and could expand sequences × ranges without an output bound. | Validate every id, cap a batch at 10,000 outputs, and check id arithmetic before mutation. Hostile-parameter regression failed before the fix. |
| P2 | Word offsets from project files were not validated against caption duration/order; 64-bit command indices could wrap on wasm32. | Validate ordered in-range offsets and use checked index conversion; clamp bounded word/character counts before narrowing. Synthetic corrupt-time tests run under `catch_unwind`. |
| P2 | Live-caption track picker options lacked automation ids, and its overflow cache identified snapshots by an unowned raw address that could be reused. | Register `liveCaptions.track.<id>` options and retain an `Arc` for snapshot identity. |

Documentation now describes caption edit/fitting behavior and speech-job limits; the speech README
no longer incorrectly states that Whisper is never enabled on wasm. The public command catalogue
also advertises `transcript.generate.then`, whose type and count are validated before job creation.

## Feature acceptance matrix

| Feature | Requirements reviewed | Verification |
|---|---|---|
| Live captions and original looks | Word timing, highlighting modes, fitting, locked tracks, undo/redo, ordinary-caption compatibility, portrait placement, edit operations, schema preservation | project/caption, edit/captions, edit/transcript, engine/live_captions tests; captions burn-in tests; v15 fixture migration and v16 round trip; UI preset and caption tests |
| Saved caption presets | Reusable preferences, case-insensitive names/overrides, limits, corrupt entries, persistence failure, cross-project reuse, automation | engine/live_captions_tests and UI `save_apply_update_and_reset_live_caption_presets`; visual inspection |
| Vertical workspace | Portrait preview beside editing area, workspace menu, persistence/restart, current shared docking model | `vertical_workspace_keeps_portrait_preview_beside_the_timeline`; native screenshots |
| Swap Orientation | Width/height and non-square pixel aspect swap, shape preservation, one undo step, unchanged-rate behavior | sequence settings unit/UI tests and engine/par_tests |
| Frame-rate conform | Frame-grid alignment, linked/butted clips, media handles, nests, transitions, captions, atomic failure and undo | edit/frame_rate tests; engine/frame_rate_conform_tests; new absolute word-time regression |
| Export any sequence and Queue All | Default selection/fallback, closed sequences, chosen ranges/settings, same-name outputs, non-destructive queuing, minimized/background continuation | engine/export_tests; UI/export_ui; new repeated-batch and hostile-size tests |
| Full Lumetri GPU path and selective colour | CPU oracle for SDR/PQ/HLG, curves/wheels/HSL/LUTs/sharpen, fallback for unsupported inputs, adjustment canvases and HDR display conversion | GPU fx parity, render HDR/adjustment tests and golden scenes; explicit Metal parity run |
| GPU nests | Scaling/cropping, colour settings, nested effects/captions, source-time overrun, fallbacks, bounded nesting/plans | render/nest_tests and GPU nest-step/CPU parity tests |
| Lumetri multi-selection | First-selected display, every selected video clip edited atomically, per-clip effect index/time, deduplication and undo/redo | engine/color_tests and UI/color_ui |
| Web speech | Actual WASM bundle, verified model download/OPFS, worker isolation, asynchronous progress, cancellation/errors, complete result application and follow-ups | speech host tests; Whisper unit/model acceptance; Chrome smoke plus optional real speech-worker test |
| Performance reporting | Cumulative bounded reasons and stage counters, read-only command behavior, CPU/GPU estimates | engine/perf tests, render fallback/preview tests and UI/perf tests |
| Upstream test fixes | Independent HEVC probe process and correct Matroska lacing timing assumptions | platform HEVC probe and Matroska oracle suites with FFmpeg available |

The optional macOS speech-fixture generator stalled in `say` during the expanded corpus run.
Its external synthesis/FFmpeg calls now time out after 30 seconds and silent output reports a
clear error. Native recognition/alignment acceptance passed on the two completed clean/noisy
fixtures (36 reference words); the expanded three-sentence corpus is not claimed as validated.

## Contributor requirements

- No new shipped image, font, sound, video or other non-code asset is introduced. Test media and
  model weights are generated/downloaded under ignored `target/`; no Adobe material or third-party
  decoder implementation was consulted during this review. Existing original presets and WGSL are code.
- New dependencies remain permissive; the feature uses the existing pinned model catalogue.
  No GPL/LGPL library is linked into the product. FFmpeg is an external test oracle only.
- Feature edits remain engine commands; project changes preserve transactional undo/redo.
  Invalid word timings and excessive job/batch input receive errors. No new unsafe or production
  panic shortcut is introduced.
- Schema v16 protects live-caption fields from older writers and retains v15 compatibility.
- The human explicitly chose public GitHub-name synchronization. Only
  `[people.mcalapurge] sync_github_name = true` was added; generated contributor statistics and
  other people's entries remain untouched. The entry was committed as the authenticated
  contributor, using their verified GitHub id/noreply identity.
- Feature and limitation documentation is updated. No readiness percentage was increased by
  this review. Existing upstream commit history was preserved.

## Validation record

| Check | Result | Local evidence (ignored `target/review/`) |
|---|---|---|
| Final `cargo xtask ci`, no optional fonts | PASS, exit 0; 3,936 test passes including the repeated translation report, 47 intentionally ignored, zero failures; layers clean, 69 assets attributed, 49 WASM crates | `ci-no-fonts-final.log` |
| Final `cargo xtask ci`, pinned official fonts | PASS, exit 0; 3,936 test passes, 47 intentionally ignored, zero failures; layers clean, 69 assets attributed, 49 WASM crates | `ci-fonts-serial.log` |
| Release desktop + CLI, `--features whisper`, required official fonts | PASS | `release-verified.log` |
| Actual web bundle, `cargo xtask web --dev` | PASS | `web-verified.log` |
| Chrome import/playback/export/recovery smoke and real Whisper worker | PASS; 18 words recognized, verified model retained in OPFS, live-caption follow-up, undo/redo and 64-bit hostile index rejection | `web-smoke-report.json` |
| Native desktop control/UI smoke | PASS; portrait caption render, Vertical layout, swapped draft fields 1920×1080 / 16:9, cancel preserves 1080×1920 project, Queue All produces two unique outputs | `native-verified.log`, `native-orientation-elements.json`, `vertical-live-captions-final.png`, `sequence-settings-final.png`, `export-sequences-final.png` |
| Explicit Metal Lumetri CPU/GPU parity | PASS, 51 cases across SDR/PQ/HLG; maximum normalized error 0.0001763 | `metal.log` |
| Optional native Whisper model acceptance | PASS, two tests; WER 2.8% over 36 reference words, 32 aligned words, mean start error +72 ms, end MAE 137 ms | `whisper-model.log` |
| Lockfile and syntax integrity | PASS; `cargo metadata --locked`, no additional Cargo.lock changes, `git diff --check`, JS and Python syntax checks | Workspace checks |

Tooling available: Rust/Cargo 1.99.1, rustfmt, clippy, wasm32 target, wasm-bindgen CLI 0.2.129
(matching Cargo.lock), FFmpeg/ffprobe 9.0.2, Node 26.12.1, GitHub CLI 2.103.2 and Chrome 156.
Official fonts use the CI pin `b076417b5880bd4154bd22f00695b1d28c6bea83` from craft-fonts.
The supported cargo-test fallback was used instead of cargo-nextest. Generated media, downloaded
models, font checkout and screenshots are not staged for contribution.

## Review conclusion

No outstanding actionable finding remains in the reviewed features after the fixes and final
local gates. Fixes are committed in `3ae77200`. The feature acceptance matrix and contributor checks support merge approval within
the tested macOS/Chrome scope, subject to normal remote CI and maintainer review. The original
branch is preserved; no merge, push, release or publication is performed by this review.

## Practical limits

Local execution is on Apple Silicon macOS and Chrome/WebGPU. WASM compilation checks the browser
and lower-layer targets. Windows, Linux, FreeBSD packaging, signing and platform-specific driver
behavior require their normal remote CI/hardware; a local macOS pass does not certify those runs.
Speech alignment is approximate and model quality remains a documented limitation. Existing
unused optional Cargo patch warnings and missing-translation reports are informational, not
suppressed or relabeled as gate failures.
