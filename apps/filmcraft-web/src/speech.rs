//! Speech-to-text in the browser: Whisper in a Web Worker, models in OPFS.
//!
//! [`WebSpeech`] is the engine's [`SpeechHost`]: `transcript.generate` and
//! `transcript.downloadModel` queue jobs here, and they run one at a time.
//!
//! 1. **Download** (unless OPFS already has the model): each file pinned in the catalogue is
//!    fetched from Hugging Face, hashed (SHA-256) and written to `speech/<model>/<file>` as it
//!    streams in, so a model never sits in wasm memory whole. `speech/<model>/verified.json`
//!    marks a complete copy whose files all matched their size and checksum.
//! 2. **Transcribe**: the audio is transferred to `speech-worker.js`, a module worker that runs
//!    this same wasm module ([`speech_worker_load`], [`speech_worker_transcribe`]). It reads the
//!    model from OPFS itself and keeps it loaded for the next job. Whisper blocks the worker,
//!    never the UI thread.
//!
//! A running transcription can't be interrupted inside the worker, so cancelling terminates the
//! worker; the next job starts a fresh one (and loads the model again).

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::Ordering;

use filmcraft_engine::export::{Progress, Report};
use filmcraft_engine::project::Transcript;
use filmcraft_engine::transcript::{JobResult, SpeechHost, SpeechJob};
use filmcraft_speech::models::ModelInfo;
use filmcraft_speech::whisper::Whisper;
use filmcraft_speech::{Options, Transcriber};
use serde_json::{Value, json};
use sha2::Digest;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Progress (out of [`STEPS`]) a job reaches when its model is downloaded and transcription
/// starts; a download-only job uses the whole range.
const DOWNLOAD_SHARE: u64 = 300;
const STEPS: u64 = 1000;

/// The engine's speech host on the web (state lives in a thread-local: the UI thread owns it).
pub struct WebSpeech;

/// What the running job is doing.
enum Phase {
    Downloading,
    /// Posted to the worker; transcribes `clips` buffers.
    Transcribing {
        clips: usize,
        base: u64,
    },
}

struct Running {
    job: SpeechJob,
    phase: Phase,
}

#[derive(Default)]
struct State {
    /// Models with a verified copy in OPFS.
    installed: BTreeSet<String>,
    queue: VecDeque<SpeechJob>,
    running: Option<Running>,
    worker: Option<web_sys::Worker>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn finish(progress: &Progress, result: &JobResult, r: Result<Report, String>) {
    if let Err(e) = &r {
        *lock(&progress.status) = if e == "stopped" { "Stopped: nothing was changed".into() } else { e.clone() };
        *lock(&progress.error) = Some(e.clone());
    }
    *lock(result) = Some(r);
    progress.finished.store(true, Ordering::Relaxed);
    crate::repaint();
}

fn report(seconds: f64) -> Report {
    Report { path: String::new(), frames: 0, seconds, bytes: 0, render_fps: 0.0, extra_files: Vec::new() }
}

fn set_status(progress: &Progress, done: u64, status: String) {
    progress.done.store(done.min(STEPS), Ordering::Relaxed);
    *lock(&progress.status) = status;
    crate::repaint();
}

impl SpeechHost for WebSpeech {
    fn installed(&self, id: &str) -> bool {
        STATE.with(|s| s.borrow().installed.contains(id))
    }

    fn start(&self, job: SpeechJob) {
        STATE.with(|s| s.borrow_mut().queue.push_back(job));
        next();
    }

    fn cancel(&self, id: u64) {
        let stopped = STATE.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(i) = s.queue.iter().position(|j| j.id == id) {
                return s.queue.remove(i);
            }
            // a download notices `progress.cancel` between chunks; a transcription can only be
            // stopped by ending the worker
            if s.running.as_ref().is_some_and(|r| r.job.id == id && matches!(r.phase, Phase::Transcribing { .. })) {
                if let Some(w) = s.worker.take() {
                    w.terminate();
                }
                return s.running.take().map(|r| r.job);
            }
            None
        });
        if let Some(job) = stopped {
            finish(&job.progress, &job.result, Err("stopped".into()));
            next();
        }
    }
}

/// Start the next queued job unless one is running.
fn next() {
    let started = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.running.is_some() {
            return false;
        }
        let Some(job) = s.queue.pop_front() else { return false };
        s.running = Some(Running { job, phase: Phase::Downloading });
        true
    });
    if started {
        wasm_bindgen_futures::spawn_local(run());
    }
}

/// The running job's id, model and progress (None: nothing runs).
fn running() -> Option<(u64, &'static ModelInfo, std::sync::Arc<Progress>, JobResult, bool)> {
    STATE.with(|s| s.borrow().running.as_ref().map(|r| (r.job.id, r.job.model, r.job.progress.clone(), r.job.result.clone(), r.job.audio.is_empty())))
}

/// End the running job `id` with `r` and start the next one.
fn end(id: u64, r: Result<Report, String>) {
    let job = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.running.as_ref().is_some_and(|x| x.job.id == id) { s.running.take().map(|x| x.job) } else { None }
    });
    if let Some(job) = job {
        finish(&job.progress, &job.result, r);
    }
    next();
}

async fn run() {
    let Some((id, model, progress, _, download_only)) = running() else { return };
    let t0 = web_time::Instant::now();
    let downloaded = !verified(model).await;
    if downloaded {
        let share = if download_only { STEPS } else { DOWNLOAD_SHARE };
        if let Err(e) = download(model, &progress, share).await {
            end(id, Err(e));
            return;
        }
    }
    STATE.with(|s| s.borrow_mut().installed.insert(model.id.to_string()));
    if download_only {
        set_status(&progress, STEPS, format!("Downloaded {}", model.name));
        end(id, Ok(report(t0.elapsed().as_secs_f64())));
        return;
    }
    if let Err(e) = post_to_worker(id, if downloaded { DOWNLOAD_SHARE } else { 0 }) {
        end(id, Err(e));
    }
}

/// OPFS path of a model file.
fn model_path(model: &ModelInfo, file: &str) -> String {
    format!("speech/{}/{file}", model.id)
}

/// The marker of a verified copy: every file's name and checksum, as pinned in the catalogue.
fn marker(model: &ModelInfo) -> Vec<u8> {
    let files: Vec<Value> = model.files.iter().map(|f| json!({"name": f.name, "sha256": f.sha256, "size": f.size})).collect();
    json!({"model": model.id, "files": files}).to_string().into_bytes()
}

/// Whether OPFS has a verified copy of `model` (its marker matches the catalogue's pins, so a
/// model whose pins changed downloads again).
async fn verified(model: &ModelInfo) -> bool {
    crate::opfs::read(&model_path(model, "verified.json")).await.is_some_and(|m| m == marker(model))
}

/// Find the models already in OPFS (at start-up, so `transcript.models` reports them).
pub async fn refresh_installed() {
    for m in filmcraft_speech::models::catalogue() {
        if verified(m).await {
            STATE.with(|s| s.borrow_mut().installed.insert(m.id.to_string()));
        }
    }
}

/// Download `model` into OPFS, checking each file's size and SHA-256 as it streams in. Progress
/// goes from 0 to `share` of [`STEPS`].
async fn download(model: &ModelInfo, progress: &Progress, share: u64) -> Result<(), String> {
    let total = model.size().max(1);
    let mut done = 0u64;
    crate::opfs::remove(&model_path(model, "verified.json")).await;
    for f in model.files {
        let path = model_path(model, f.name);
        let r = download_file(f, &path, progress, |n| {
            done += n;
            let pct = done as f64 / total as f64;
            set_status(progress, (pct * share as f64) as u64, format!("Downloading {} ({:.0} of {:.0} MB)", model.name, done as f64 / 1e6, total as f64 / 1e6));
        })
        .await;
        if let Err(e) = r {
            crate::opfs::remove(&path).await;
            return Err(e);
        }
    }
    let w = crate::opfs::writable(&model_path(model, "verified.json")).await.map_err(js_err)?;
    JsFuture::from(w.write_with_u8_array(&marker(model)).map_err(js_err)?).await.map_err(js_err)?;
    JsFuture::from(w.close()).await.map_err(js_err)?;
    Ok(())
}

fn js_err(e: JsValue) -> String {
    e.as_string().or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_else(|| format!("{e:?}"))
}

/// Stream one file to OPFS; `on_bytes` gets each chunk's length.
async fn download_file(f: &filmcraft_speech::models::ModelFile, path: &str, progress: &Progress, mut on_bytes: impl FnMut(u64)) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response =
        JsFuture::from(window.fetch_with_str(f.url)).await.map_err(|e| format!("{}: {}", f.name, js_err(e)))?.dyn_into().map_err(js_err)?;
    if !resp.ok() {
        return Err(format!("{}: the server answered {}", f.name, resp.status()));
    }
    let body = resp.body().ok_or_else(|| format!("{}: empty response", f.name))?;
    let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().dyn_into().map_err(|o: js_sys::Object| js_err(o.into()))?;
    let w = crate::opfs::writable(path).await.map_err(js_err)?;
    let mut hasher = sha2::Sha256::new();
    let mut size = 0u64;
    let r: Result<(), String> = async {
        loop {
            if progress.cancel.load(Ordering::Relaxed) {
                return Err("stopped".to_string());
            }
            let chunk = JsFuture::from(reader.read()).await.map_err(js_err)?;
            if js_sys::Reflect::get(&chunk, &"done".into()).ok().and_then(|d| d.as_bool()).unwrap_or(true) {
                return Ok(());
            }
            let value: js_sys::Uint8Array = js_sys::Reflect::get(&chunk, &"value".into()).map_err(js_err)?.dyn_into().map_err(js_err)?;
            let bytes = value.to_vec();
            size = size.saturating_add(bytes.len() as u64);
            if size > f.size {
                return Err(format!("{}: larger than the pinned {} bytes", f.name, f.size));
            }
            hasher.update(&bytes);
            JsFuture::from(w.write_with_u8_array(&bytes).map_err(js_err)?).await.map_err(js_err)?;
            on_bytes(bytes.len() as u64);
        }
    }
    .await;
    if let Err(e) = r {
        let _ = JsFuture::from(reader.cancel()).await;
        let _ = JsFuture::from(w.abort()).await;
        return Err(e);
    }
    JsFuture::from(w.close()).await.map_err(js_err)?;
    let sha = format!("{:x}", hasher.finalize());
    if size != f.size || !sha.eq_ignore_ascii_case(f.sha256) {
        return Err(format!("{}: the download doesn't match its pinned size and checksum", f.name));
    }
    Ok(())
}

/// The worker (started on first use; a cancelled transcription or a worker error ends it).
fn worker() -> Result<web_sys::Worker, String> {
    if let Some(w) = STATE.with(|s| s.borrow().worker.clone()) {
        return Ok(w);
    }
    // the same `?v=<build>` as the page's glue and wasm (they are cached as immutable)
    let v = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"filmcraftLoad".into()).ok())
        .and_then(|l| js_sys::Reflect::get(&l, &"v".into()).ok())
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let opts = web_sys::WorkerOptions::new();
    opts.set_type(web_sys::WorkerType::Module);
    opts.set_name("filmcraft-speech");
    let w = web_sys::Worker::new_with_options(&format!("speech-worker.js{v}"), &opts).map_err(|e| format!("can't start the speech worker: {}", js_err(e)))?;
    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|e: web_sys::MessageEvent| on_message(e.data()));
    w.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
    let on_error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(|e: web_sys::ErrorEvent| {
        e.prevent_default();
        worker_failed(format!("the speech worker stopped: {}", e.message()));
    });
    w.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();
    STATE.with(|s| s.borrow_mut().worker = Some(w.clone()));
    Ok(w)
}

/// The worker died or reported an error: drop it (the next job starts a fresh one) and end the
/// running job.
fn worker_failed(msg: String) {
    let id = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(w) = s.worker.take() {
            w.terminate();
        }
        s.running.as_ref().filter(|r| matches!(r.phase, Phase::Transcribing { .. })).map(|r| r.job.id)
    });
    if let Some(id) = id {
        end(id, Err(msg));
    }
}

/// Hand the running job's audio to the worker; its progress continues from `base`.
fn post_to_worker(id: u64, base: u64) -> Result<(), String> {
    let w = worker()?;
    let msg = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let r = s.running.as_mut().filter(|r| r.job.id == id)?;
        let audio: Vec<Vec<f32>> = std::mem::take(&mut r.job.audio);
        r.phase = Phase::Transcribing { clips: audio.len(), base };
        Some((r.job.model, r.job.options.clone(), audio))
    });
    let Some((model, options, audio)) = msg else { return Ok(()) };
    let buffers = js_sys::Array::new();
    let transfer = js_sys::Array::new();
    for a in &audio {
        let f = js_sys::Float32Array::new_with_length(u32::try_from(a.len()).map_err(|_| "a clip is too long".to_string())?);
        f.copy_from(a);
        transfer.push(&f.buffer());
        buffers.push(&f);
    }
    drop(audio);
    let o = js_sys::Object::new();
    let set = |k: &str, v: &JsValue| js_sys::Reflect::set(&o, &k.into(), v).map_err(js_err);
    set("type", &"transcribe".into())?;
    set("job", &JsValue::from_f64(id as f64))?;
    set("model", &model.id.into())?;
    set("files", &crate::api::to_js(&json!(model.files.iter().map(|f| f.name).collect::<Vec<_>>())))?;
    set("options", &options_json(&options).into())?;
    set("audio", &buffers)?;
    w.post_message_with_transfer(&o, &transfer).map_err(|e| format!("can't reach the speech worker: {}", js_err(e)))?;
    if let Some((_, _, progress, _, _)) = running() {
        set_status(&progress, progress.done.load(Ordering::Relaxed), format!("Loading {}", model.name));
    }
    Ok(())
}

fn options_json(o: &Options) -> String {
    json!({"language": o.language, "diarize": o.diarize, "maxSpeakers": o.max_speakers}).to_string()
}

fn options_from_json(s: &str) -> Options {
    let v: Value = serde_json::from_str(s).unwrap_or_default();
    let d = Options::default();
    Options {
        language: v["language"].as_str().map(str::to_string),
        diarize: v["diarize"].as_bool().unwrap_or(d.diarize),
        max_speakers: v["maxSpeakers"].as_u64().map(|n| n.clamp(1, 32) as usize).unwrap_or(d.max_speakers),
    }
}

/// A message from the worker: `progress`, `done` (one transcript JSON per clip) or `error`.
fn on_message(data: JsValue) {
    let v = crate::api::from_js(&data);
    let Some(id) = v["job"].as_u64() else { return };
    let Some((rid, _, progress, _, _)) = running() else { return };
    if rid != id {
        return;
    }
    match v["type"].as_str() {
        Some("progress") => {
            let Some((clips, base)) = STATE.with(|s| match s.borrow().running.as_ref().map(|r| &r.phase) {
                Some(Phase::Transcribing { clips, base }) => Some((*clips, *base)),
                _ => None,
            }) else {
                return;
            };
            let index = v["index"].as_u64().unwrap_or(0) as f64;
            let f = v["fraction"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
            let overall = ((index + f) / clips.max(1) as f64).clamp(0.0, 1.0);
            let status = v["status"].as_str().unwrap_or("Transcribing");
            let label = if clips > 1 { format!("Clip {} of {clips}: {status}", index as usize + 1) } else { status.to_string() };
            set_status(&progress, base + (overall * (STEPS - base) as f64) as u64, label);
        }
        Some("done") => {
            let transcripts: Result<Vec<Transcript>, String> = v["transcripts"]
                .as_array()
                .map(|a| a.iter().map(|t| serde_json::from_str(t.as_str().unwrap_or_default()).map_err(|e| format!("transcript: {e}"))).collect())
                .unwrap_or_else(|| Err("the worker sent no transcripts".into()));
            let job = STATE.with(|s| s.borrow().running.as_ref().map(|r| (r.job.transcripts.clone(), r.job.result.clone())));
            match (transcripts, job) {
                (Ok(ts), Some((out, _))) => {
                    *lock(&out) = Some(ts);
                    set_status(&progress, STEPS, "Done".into());
                    end(id, Ok(report(v["seconds"].as_f64().unwrap_or(0.0))));
                }
                (Err(e), _) => end(id, Err(e)),
                (_, None) => {}
            }
        }
        Some("error") => {
            let msg = v["message"].as_str().unwrap_or("the speech worker failed").to_string();
            // a Rust panic leaves the worker's wasm instance unusable: start a fresh worker next time
            worker_failed(msg);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Inside the worker (`speech-worker.js` calls these on its own instance of this module)
// ---------------------------------------------------------------------------------------------

thread_local! {
    static MODEL: RefCell<Option<Whisper>> = const { RefCell::new(None) };
}

/// Load model `id` from the bytes of its four files (worker only).
#[wasm_bindgen]
pub fn speech_worker_load(id: &str, config: &[u8], generation_config: &[u8], tokenizer: &[u8], weights: Vec<u8>) -> Result<(), JsError> {
    MODEL.with(|m| *m.borrow_mut() = None);
    let w = Whisper::from_bytes(id, config, generation_config, tokenizer, weights).map_err(|e| JsError::new(&e.to_string()))?;
    MODEL.with(|m| *m.borrow_mut() = Some(w));
    Ok(())
}

/// Transcribe mono 16 kHz `audio` with the loaded model (worker only); `progress(fraction,
/// status)` is called as it goes. Returns the transcript as JSON.
#[wasm_bindgen]
pub fn speech_worker_transcribe(audio: &[f32], options: &str, progress: &js_sys::Function) -> Result<String, JsError> {
    let opts = options_from_json(options);
    MODEL.with(|m| {
        let m = m.borrow();
        let w = m.as_ref().ok_or_else(|| JsError::new("no speech model is loaded"))?;
        let t = w
            .transcribe(audio, &opts, &mut |f, status| {
                let _ = progress.call2(&JsValue::NULL, &JsValue::from_f64(f as f64), &status.into());
                true
            })
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_json::to_string(&t).map_err(|e| JsError::new(&e.to_string()))
    })
}
