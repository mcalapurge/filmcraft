//! Text-based editing commands (`transcript.*`): the Text panel ▸ Transcript tab.
//!
//! Transcripts belong to media items (`Project::transcripts`, media time); the sequence transcript
//! is derived from them ([`filmcraft_edit::transcript::sequence_words`]). Words of the sequence
//! transcript are addressed by index (`from`, `to`, inclusive), as `transcript.inspect` lists them.
//!
//! Speech recognition goes through a [`Transcriber`]: [`Session::transcriber`] when a host or a
//! test installed one, else the Whisper model named by `model` from `<data dir>/models` (needs the
//! engine feature `whisper`; without it `transcript.generate` fails with a clear error, and agents
//! can still bring their own transcript with `transcript.set`).
//!
//! A host without threads (the web build) installs a [`SpeechHost`] instead: `transcript.generate`
//! then returns `{"job": id}` at once, [`poll`] reads the audio a block at a time and hands it to
//! the host, and the transcripts are applied when the host finishes the job.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use filmcraft_edit as edit;
use filmcraft_edit::transcript::{self as tx, CaptionRules, SeqWord};
use filmcraft_project::{CaptionFormat, CaptionTrack, ItemId, ItemKind, TrackId, Transcript};
use filmcraft_speech::{Options, SpeechError, Transcriber};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};

use crate::commands::{CommandSpec, always, bad, bool_p, f64_p, has_seq, str_p, u64_p};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

fn spec(id: &'static str, label: &'static str, menu: &'static [&'static str], params: &'static str, enabled: Enabled, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu, shortcut: None, params, enabled, run, journal }
}

/// Where downloaded speech models live (`<data dir>/models`).
pub fn models_dir() -> Option<std::path::PathBuf> {
    crate::autosave::default_data_dir().map(|d| d.join("models"))
}

/// Whether this build can transcribe with Whisper (feature `whisper`).
pub fn speech_available() -> bool {
    filmcraft_speech::available()
}

/// The words of the active sequence's transcript.
pub fn sequence_words(s: &Session) -> Vec<SeqWord> {
    match s.active_sequence() {
        Some(q) => tx::sequence_words(q, &s.project.transcripts),
        None => Vec::new(),
    }
}

fn has_transcript(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if sequence_words(s).is_empty() { Err("the sequence has no transcript (Transcribe first)".into()) } else { Ok(()) }
}

fn has_transcripts(s: &Session) -> std::result::Result<(), String> {
    if s.project.transcripts.is_empty() { Err("there are no transcripts".into()) } else { Ok(()) }
}

/// Why speech-to-text can't run in this build (no installed transcriber, built without `whisper`).
pub(crate) const NO_SPEECH: &str =
    "speech-to-text is not available in this build (built without the `whisper` feature); import a transcript with transcript.set instead";

/// `transcript.generate` can run: a host installed a transcriber or a speech host, or the build
/// has speech-to-text (#97: it reported enabled and then always failed).
pub fn can_transcribe(s: &Session) -> std::result::Result<(), String> {
    if s.transcriber.is_some() || s.speech_host.is_some() || speech_available() { Ok(()) } else { Err(NO_SPEECH.into()) }
}

/// `transcript.downloadModel` can run: a speech host downloads models, or the build has
/// `speech-download` (#98).
fn can_download(s: &Session) -> std::result::Result<(), String> {
    if s.speech_host.is_some() || cfg!(feature = "speech-download") {
        Ok(())
    } else {
        Err("model downloads are not available in this build (built without the `speech-download` feature)".into())
    }
}

/// The media item behind a project item (subclips resolve to their parent).
fn media_item(s: &Session, item: ItemId) -> Option<ItemId> {
    s.project.resolve_media(item).map(|(root, _, _)| root)
}

fn ids_p(p: &Value, k: &str) -> Option<Vec<ItemId>> {
    p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(ItemId).collect())
}

/// Items to transcribe: `items` / `item`, else the Project panel selection, else the media of the
/// active sequence's enabled audio clips. Subclips resolve to their media; duplicates are removed.
fn targets(s: &Session, p: &Value) -> Vec<ItemId> {
    let mut raw = ids_p(p, "items").or_else(|| u64_p(p, "item").map(|i| vec![ItemId(i)])).unwrap_or_default();
    if raw.is_empty() {
        raw = s.state.project_selection.clone();
    }
    if raw.is_empty()
        && let Some(q) = s.active_sequence()
    {
        raw = q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|it| it.enabled).map(|it| it.item).collect();
    }
    let mut out = Vec::new();
    for i in raw {
        if let Some(m) = media_item(s, i)
            && !out.contains(&m)
        {
            out.push(m);
        }
    }
    out
}

/// Mono 16 kHz audio of a media item (None: no audio).
fn item_audio(s: &Session, item: ItemId) -> Option<Vec<f32>> {
    let dur = match &s.project.item(item)?.kind {
        ItemKind::Media(m) => m.duration(),
        _ => return None,
    };
    let src = s.source(item)?;
    if !src.info().has_audio() {
        return None;
    }
    let sr = filmcraft_speech::SAMPLE_RATE;
    let len = dur.to_units_floor(sr as i64).max(0) as usize;
    let buf = src.audio(0, len, sr).ok()?;
    Some(filmcraft_speech::downmix(&buf.channels))
}

fn speech_err(e: SpeechError) -> EngineError {
    EngineError::Other(e.to_string())
}

/// The transcriber to use: the installed one, else the named catalogue model.
fn transcriber(s: &Session, p: &Value) -> Result<Arc<dyn Transcriber>> {
    if let Some(t) = &s.transcriber {
        return Ok(t.clone());
    }
    // Settings ▸ Media Analysis & Transcription ▸ Speech model
    let model = str_p(p, "model").unwrap_or(&s.prefs.media_analysis.whisper_model);
    if filmcraft_speech::models::find(model).is_none() {
        return Err(speech_err(SpeechError::UnknownModel(model.into())));
    }
    if !filmcraft_speech::available() {
        return Err(EngineError::Other(NO_SPEECH.into()));
    }
    let dir = models_dir().ok_or_else(|| EngineError::Other("no data directory for speech models".into()))?;
    filmcraft_speech::load(&dir, model).map_err(speech_err)
}

fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    let items = targets(s, p);
    if items.is_empty() {
        return Err(bad("transcript.generate", "nothing to transcribe (pass `items`, select clips, or open a sequence with audio)"));
    }
    // Settings ▸ Media Analysis & Transcription: language (or auto-detect) and speaker labelling
    let ma = &s.prefs.media_analysis;
    let default_language = if ma.language_auto_detect { None } else { Some(ma.default_language.clone()) };
    let opts = Options {
        language: match str_p(p, "language") {
            Some(l) => Some(l).filter(|l| !l.is_empty() && *l != "auto").map(str::to_string),
            None => default_language,
        },
        diarize: bool_p(p, "diarize").unwrap_or(ma.speaker_labeling != "off"),
        max_speakers: u64_p(p, "maxSpeakers").map(|n| n.clamp(1, 32) as usize).unwrap_or(Options::default().max_speakers),
    };
    let then = then_p(p)?;
    if s.transcriber.is_none()
        && let Some(host) = s.speech_host.clone()
    {
        let model = model_p(s, p)?;
        let n = items.len();
        let label = format!("Transcribe ({n} clip{})", if n == 1 { "" } else { "s" });
        let id = start_host_job(s, host.as_ref(), label, model, items.clone(), opts, then);
        return Ok(json!({"job": id, "items": items.iter().map(|i| i.0).collect::<Vec<_>>(), "model": model.id, "poll": "jobs.list", "cancel": "jobs.cancel"}));
    }
    let t = transcriber(s, p)?;
    let mut done: Vec<(ItemId, Transcript)> = Vec::new();
    let mut skipped = Vec::new();
    for item in items {
        let Some(audio) = item_audio(s, item) else {
            skipped.push(item.0);
            continue;
        };
        let mut tr = t.transcribe(&audio, &opts, &mut |_, _| true).map_err(speech_err)?;
        tr.normalize();
        done.push((item, tr));
    }
    if done.is_empty() {
        return Err(EngineError::Other("none of the clips has audio to transcribe".into()));
    }
    let report: Vec<Value> = done
        .iter()
        .map(|(i, t)| json!({"item": i.0, "words": t.words.len(), "speakers": t.speakers.len(), "language": t.language, "source": t.source}))
        .collect();
    s.edit("Transcribe", move |pr, _| {
        for (i, t) in done {
            pr.transcripts.insert(i, Arc::new(t));
        }
        Ok(())
    })?;
    let then = run_then(s, then);
    Ok(json!({"items": report, "skipped": skipped, "then": then}))
}

/// `then`: commands to run once the transcripts are in (`[{"command": id, "params": {…}}]`), so a
/// caller that needs the transcript works the same whether the transcription ran in the command
/// or as a [`SpeechHost`] job (Live Captions: transcribe, then `captions.createLive`).
fn then_p(p: &Value) -> Result<Vec<(String, Value)>> {
    let Some(list) = p.get("then") else { return Ok(Vec::new()) };
    let list = list.as_array().ok_or_else(|| bad("transcript.generate", "`then` must be a list of {\"command\", \"params\"}"))?;
    let mut out = Vec::new();
    for c in list.iter().take(MAX_THEN) {
        let id = c["command"].as_str().ok_or_else(|| bad("transcript.generate", "each `then` entry needs a `command`"))?;
        if id == "transcript.generate" {
            return Err(bad("transcript.generate", "`then` can't run transcript.generate"));
        }
        out.push((id.to_string(), c.get("params").cloned().unwrap_or_else(|| json!({}))));
    }
    Ok(out)
}

/// More follow-up commands than any caller needs.
const MAX_THEN: usize = 16;

/// Run the `then` commands; each one's result or error, in order.
fn run_then(s: &mut Session, then: Vec<(String, Value)>) -> Vec<Value> {
    then.into_iter()
        .map(|(id, params)| match s.execute(&id, params) {
            Ok(r) => json!({"command": id, "result": r}),
            Err(e) => {
                s.error_toast(&id, e.to_string());
                json!({"command": id, "error": e.to_string()})
            }
        })
        .collect()
}

/// The catalogue model named by `model`, else Settings ▸ Media Analysis & Transcription ▸ Speech
/// model.
fn model_p(s: &Session, p: &Value) -> Result<&'static filmcraft_speech::models::ModelInfo> {
    let id = str_p(p, "model").unwrap_or(&s.prefs.media_analysis.whisper_model);
    filmcraft_speech::models::find(id).ok_or_else(|| speech_err(SpeechError::UnknownModel(id.into())))
}

// ---------------------------------------------------------------------------------------------
// Speech hosts: transcription as a background job (the web build's Web Worker)
// ---------------------------------------------------------------------------------------------

/// Where a job's outcome goes (as in [`crate::Job::result`]).
pub type JobResult = Arc<Mutex<Option<std::result::Result<filmcraft_export::Report, String>>>>;

/// A host that runs speech-to-text away from the UI thread and keeps its own models (the web
/// build: a Web Worker, models in OPFS). Installed in [`Session::speech_host`].
pub trait SpeechHost: Send + Sync {
    /// Whether catalogue model `id` is downloaded and verified.
    fn installed(&self, id: &str) -> bool;
    /// Run `job` in the background: download its model unless installed, then transcribe each
    /// buffer of `job.audio` in order. The host updates `job.progress`, stores one transcript per
    /// buffer in `job.transcripts` and the outcome in `job.result`, then sets
    /// `job.progress.finished`. A job with no audio only downloads.
    fn start(&self, job: SpeechJob);
    /// Stop job `id`, queued or running; the host finishes it as stopped.
    fn cancel(&self, id: u64);
}

/// A transcription (or model download) handed to a [`SpeechHost`].
pub struct SpeechJob {
    pub id: u64,
    pub model: &'static filmcraft_speech::models::ModelInfo,
    /// Mono 16 kHz audio, one buffer per clip.
    pub audio: Vec<Vec<f32>>,
    pub options: Options,
    pub progress: Arc<filmcraft_export::Progress>,
    pub result: JobResult,
    pub transcripts: Arc<Mutex<Option<Vec<Transcript>>>>,
}

/// Longest audio one host job takes: its samples are held in wasm memory twice (the UI thread
/// reads them, the worker transcribes a copy), and a wasm page has 4 GB.
pub const MAX_HOST_AUDIO_SECONDS: u64 = 2 * 3600;

/// Audio read per step while a host job gathers its clips' audio (media bytes may still be
/// loading on the web, so a block that can't be read yet is retried on the next [`poll`]).
const READ_BLOCK_SECONDS: usize = 30;

/// How long one [`poll`] spends reading audio.
const READ_BUDGET: std::time::Duration = std::time::Duration::from_millis(15);

/// A clip whose audio is being read for a host job.
struct AudioRead {
    item: ItemId,
    samples: Vec<f32>,
    len: usize,
}

/// A [`SpeechHost`] job: reading its audio, then running on the host; applied by [`poll`].
pub struct PendingTranscript {
    pub job: u64,
    model: &'static filmcraft_speech::models::ModelInfo,
    options: Options,
    /// Clips still being read, in order, then the ones handed to the host.
    reads: Vec<AudioRead>,
    skipped: Vec<ItemId>,
    started: bool,
    transcripts: Arc<Mutex<Option<Vec<Transcript>>>>,
    then: Vec<(String, Value)>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn finish(progress: &filmcraft_export::Progress, result: &JobResult, r: std::result::Result<filmcraft_export::Report, String>) {
    match &r {
        Ok(_) => {}
        Err(e) => {
            *lock(&progress.status) = e.clone();
            *lock(&progress.error) = Some(e.clone());
        }
    }
    *lock(result) = Some(r);
    progress.finished.store(true, Ordering::Relaxed);
}

/// Queue a host job for `items` (none: only download `model`); returns its job id.
fn start_host_job(
    s: &mut Session,
    host: &dyn SpeechHost,
    label: String,
    model: &'static filmcraft_speech::models::ModelInfo,
    items: Vec<ItemId>,
    options: Options,
    then: Vec<(String, Value)>,
) -> u64 {
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label, progress: Default::default(), result: Default::default() };
    job.progress.total.store(1000, Ordering::Relaxed);
    let reads: Vec<AudioRead> = items.iter().map(|&item| AudioRead { item, samples: Vec::new(), len: 0 }).collect();
    let pj = PendingTranscript { job: id, model, options, reads, skipped: Vec::new(), started: false, transcripts: Arc::default(), then };
    if pj.reads.is_empty() {
        *lock(&job.progress.status) = format!("Downloading {}", model.name);
    } else {
        *lock(&job.progress.status) = "Reading audio".into();
    }
    s.jobs.push(job);
    s.transcript_jobs.push(pj);
    if items.is_empty() {
        // a download needs no audio: hand it over now
        poll_one(s, host, s.transcript_jobs.len() - 1);
    }
    id
}

/// Read the audio of a pending job's clips for up to [`READ_BUDGET`]. Returns an error that ends
/// the job (too long, unreadable).
fn read_audio(s: &Session, pj: &mut PendingTranscript, progress: &filmcraft_export::Progress) -> std::result::Result<(), String> {
    let sr = filmcraft_speech::SAMPLE_RATE as usize;
    if pj.reads.iter().any(|r| r.len == 0) {
        // first call: every clip's length (clips without audio are skipped)
        let mut skipped = Vec::new();
        for r in pj.reads.iter_mut() {
            let dur = match s.project.item(r.item).map(|it| &it.kind) {
                Some(ItemKind::Media(m)) => m.duration(),
                _ => Tick(0),
            };
            let has_audio = s.source(r.item).is_some_and(|src| src.info().has_audio());
            r.len = if has_audio { usize::try_from(dur.to_units_floor(sr as i64).max(0)).unwrap_or(0) } else { 0 };
            if r.len == 0 {
                skipped.push(r.item);
            }
        }
        pj.reads.retain(|r| r.len > 0);
        pj.skipped.extend(skipped);
        let total: u64 = pj.reads.iter().map(|r| r.len as u64).sum();
        if total > MAX_HOST_AUDIO_SECONDS * sr as u64 {
            return Err(format!(
                "{:.1} hours of audio is more than this build transcribes at once ({} hours); transcribe fewer clips at a time",
                total as f64 / sr as f64 / 3600.0,
                MAX_HOST_AUDIO_SECONDS / 3600
            ));
        }
    }
    let t0 = web_time::Instant::now();
    'clips: for r in pj.reads.iter_mut() {
        while r.samples.len() < r.len {
            if t0.elapsed() >= READ_BUDGET {
                break 'clips;
            }
            let Some(src) = s.source(r.item) else { return Err(format!("item {} has no media", r.item.0)) };
            let start = r.samples.len();
            let n = (READ_BLOCK_SECONDS * sr).min(r.len - start);
            let _ = filmcraft_media::pending::take();
            let block = src.audio(start as i64, n, sr as u32);
            // the bytes are on their way: read this block again on the next poll
            if filmcraft_media::pending::take() {
                break 'clips;
            }
            let buf = block.map_err(|e| format!("can't read the audio of item {}: {e}", r.item.0))?;
            let mut mono = filmcraft_speech::downmix(&buf.channels);
            mono.resize(n, 0.0);
            r.samples.extend_from_slice(&mono);
        }
    }
    let total: usize = pj.reads.iter().map(|r| r.len).sum();
    let read: usize = pj.reads.iter().map(|r| r.samples.len()).sum();
    *lock(&progress.status) = format!("Reading audio {:.0}%", 100.0 * read as f64 / total.max(1) as f64);
    Ok(())
}

/// Advance pending job `i`: read its audio, hand it to the host, stop it when cancelled.
fn poll_one(s: &mut Session, host: &dyn SpeechHost, i: usize) {
    let Some(id) = s.transcript_jobs.get(i).map(|pj| pj.job) else { return };
    let Some(job) = s.jobs.iter().find(|j| j.id == id).cloned() else { return };
    if job.progress.finished.load(Ordering::Relaxed) {
        return;
    }
    let Some(mut pj) = (i < s.transcript_jobs.len()).then(|| s.transcript_jobs.remove(i)) else { return };
    if job.progress.cancel.load(Ordering::Relaxed) {
        if pj.started {
            host.cancel(id);
        } else {
            finish(&job.progress, &job.result, Err("stopped".into()));
        }
        s.transcript_jobs.insert(i, pj);
        return;
    }
    if !pj.started {
        if let Err(e) = read_audio(s, &mut pj, &job.progress) {
            finish(&job.progress, &job.result, Err(e));
        } else if pj.reads.iter().all(|r| r.samples.len() >= r.len) {
            if pj.reads.is_empty() && !pj.skipped.is_empty() {
                finish(&job.progress, &job.result, Err("none of the clips has audio to transcribe".into()));
            } else {
                pj.started = true;
                let audio = pj.reads.iter_mut().map(|r| std::mem::take(&mut r.samples)).collect();
                host.start(SpeechJob {
                    id,
                    model: pj.model,
                    audio,
                    options: pj.options.clone(),
                    progress: job.progress.clone(),
                    result: job.result.clone(),
                    transcripts: pj.transcripts.clone(),
                });
            }
        }
    }
    s.transcript_jobs.insert(i, pj);
}

/// Advance [`SpeechHost`] jobs and apply the finished ones (one undo step each), then run their
/// `then` commands. Called once per UI frame from [`Session::poll_persistence`].
pub fn poll(s: &mut Session) {
    let Some(host) = s.speech_host.clone() else { return };
    let mut i = 0;
    while i < s.transcript_jobs.len() {
        poll_one(s, host.as_ref(), i);
        let id = s.transcript_jobs[i].job;
        let job = s.jobs.iter().find(|j| j.id == id).cloned();
        if job.as_ref().is_some_and(|j| !j.progress.finished.load(Ordering::Relaxed)) {
            i += 1;
            continue;
        }
        let pj = s.transcript_jobs.remove(i);
        let Some(job) = job else { continue };
        let outcome = lock(&job.result).clone();
        let transcripts = lock(&pj.transcripts).take();
        match (outcome, transcripts) {
            (Some(Ok(_)), Some(ts)) if !pj.reads.is_empty() => {
                let done: Vec<(ItemId, Transcript)> = pj
                    .reads
                    .iter()
                    .zip(ts)
                    .map(|(r, mut t)| {
                        t.normalize();
                        (r.item, t)
                    })
                    .collect();
                let words: usize = done.iter().map(|(_, t)| t.words.len()).sum();
                let n = done.len();
                let r = s.edit("Transcribe", move |pr, _| {
                    for (i, t) in done {
                        pr.transcripts.insert(i, Arc::new(t));
                    }
                    Ok(())
                });
                match r {
                    Ok(_) => {
                        s.toast(format!("Transcribed {n} clip{} ({words} words)", if n == 1 { "" } else { "s" }));
                        run_then(s, pj.then);
                    }
                    Err(e) => s.error_toast("transcript.generate", format!("Transcribe: {e}")),
                }
            }
            (Some(Ok(_)), _) if pj.reads.is_empty() && pj.skipped.is_empty() => s.toast(format!("Downloaded {}", pj.model.name)),
            (Some(Err(e)), _) if e == "stopped" => {}
            (Some(Err(e)), _) => s.error_toast("transcript.generate", format!("Transcribe: {e}")),
            _ => {}
        }
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let item = u64_p(p, "item").map(ItemId).ok_or_else(|| bad("transcript.set", "`item` is required"))?;
    let item = media_item(s, item).ok_or_else(|| bad("transcript.set", "no such media item"))?;
    let v = p.get("transcript").cloned().ok_or_else(|| bad("transcript.set", "`transcript` is required"))?;
    let mut t: Transcript = serde_json::from_value(v).map_err(|e| bad("transcript.set", e.to_string()))?;
    if t.source.is_empty() {
        t.source = "imported".into();
    }
    t.normalize();
    t.check().map_err(|e| bad("transcript.set", e))?;
    let n = t.words.len();
    s.edit("Set Transcript", move |pr, _| {
        pr.transcripts.insert(item, Arc::new(t));
        Ok(())
    })?;
    Ok(json!({"item": item.0, "words": n}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let items: Vec<ItemId> = match ids_p(p, "items").or_else(|| u64_p(p, "item").map(|i| vec![ItemId(i)])) {
        Some(v) => v.into_iter().filter_map(|i| media_item(s, i)).collect(),
        None => s.project.transcripts.keys().copied().collect(),
    };
    let n = items.iter().filter(|i| s.project.transcripts.contains_key(i)).count();
    if n == 0 {
        return Err(EngineError::Other("no transcript to delete".into()));
    }
    s.edit("Delete Transcript", move |pr, _| {
        for i in items {
            pr.transcripts.remove(&i);
        }
        Ok(())
    })?;
    Ok(json!({"deleted": n}))
}

fn word_json(i: usize, w: &SeqWord) -> Value {
    json!({"i": i, "text": w.text, "start": w.start.0, "end": w.end.0, "speaker": w.speaker, "clip": w.clip.0, "item": w.item.0, "confidence": w.confidence})
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let words = sequence_words(s);
    let gap = Tick::from_seconds_f64(f64_p(p, "paragraphGapSeconds").unwrap_or(1.5));
    let paras: Vec<Value> = tx::paragraphs(&words, gap)
        .into_iter()
        .map(|r| {
            let text = words[r.clone()].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            json!({"from": r.start, "to": r.end - 1, "speaker": words[r.start].speaker, "start": words[r.start].start.0, "end": words[r.end - 1].end.0, "text": text})
        })
        .collect();
    let speakers: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        for w in &words {
            if let Some(n) = &w.speaker
                && !v.contains(n)
            {
                v.push(n.clone());
            }
        }
        v
    };
    let current = tx::word_at(&words, s.playhead());
    Ok(json!({
        "words": words.iter().enumerate().map(|(i, w)| word_json(i, w)).collect::<Vec<_>>(),
        "paragraphs": paras,
        "speakers": speakers,
        "current": current,
        "items": s.project.transcripts.iter().map(|(i, t)| json!({"item": i.0, "words": t.words.len(), "language": t.language, "source": t.source, "speakers": t.speakers.iter().map(|k| &k.name).collect::<Vec<_>>()})).collect::<Vec<_>>(),
    }))
}

fn search(s: &mut Session, p: &Value) -> Result<Value> {
    let q = str_p(p, "query").ok_or_else(|| bad("transcript.search", "`query` is required"))?;
    let words = sequence_words(s);
    let hits: Vec<Value> = tx::search(&words, q)
        .into_iter()
        .map(|r| json!({"from": r.start, "to": r.end - 1, "start": words[r.start].start.0, "end": words[r.end - 1].end.0}))
        .collect();
    Ok(json!({"matches": hits}))
}

/// Timeline range of the words `from..=to` (frame-snapped outward).
fn range_p(s: &Session, p: &Value, cmd: &str) -> Result<TimeRange> {
    let words = sequence_words(s);
    let from = u64_p(p, "from").ok_or_else(|| bad(cmd, "`from` (word index) is required"))? as usize;
    let to = u64_p(p, "to").map(|n| n as usize).unwrap_or(from);
    tx::word_range(&words, from, to, s.sequence_rate()).ok_or_else(|| bad(cmd, format!("word index out of range (the transcript has {} words)", words.len())))
}

fn range_json(r: TimeRange) -> Value {
    json!({"start": r.start.0, "end": r.end().0})
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let r = range_p(s, p, "transcript.select")?;
    let fd = s.sequence_rate().frame_duration();
    s.edit_sequence("Mark Transcript Selection", |q, _, _| {
        q.mark_in = Some(r.start);
        q.mark_out = Some(r.end() - fd);
        Ok(())
    })?;
    s.set_playhead(r.start);
    Ok(range_json(r))
}

fn extract_or_lift(s: &mut Session, p: &Value, extract: bool) -> Result<Value> {
    let cmd = if extract { "transcript.extract" } else { "transcript.lift" };
    let r = range_p(s, p, cmd)?;
    let tg = s.targeting().targeted;
    s.edit_sequence(if extract { "Extract Text" } else { "Lift Text" }, |q, ctx, _| {
        if extract {
            edit::extract(q, &tg, r, ctx);
        } else {
            edit::lift(q, &tg, r, ctx);
        }
        q.mark_in = None;
        q.mark_out = None;
        Ok(())
    })?;
    s.set_playhead(r.start);
    Ok(range_json(r))
}

fn rename_speaker(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_p(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("transcript.renameSpeaker", "`name` is required"))?.to_string();
    let item = u64_p(p, "item").map(ItemId).and_then(|i| media_item(s, i));
    // `speaker`: the current name (every transcript), or an index (needs `item`)
    let (old_name, index) = match p.get("speaker") {
        Some(Value::String(n)) => (Some(n.clone()), None),
        Some(v) if v.is_u64() => (None, v.as_u64().map(|n| n as usize)),
        _ => return Err(bad("transcript.renameSpeaker", "`speaker` (name, or index with `item`) is required")),
    };
    if index.is_some() && item.is_none() {
        return Err(bad("transcript.renameSpeaker", "a speaker index needs `item`"));
    }
    let mut n = 0;
    let mut next = s.project.transcripts.clone();
    for (i, t) in next.iter_mut() {
        if item.is_some_and(|x| x != *i) {
            continue;
        }
        let tt = Arc::make_mut(t);
        for (k, sp) in tt.speakers.iter_mut().enumerate() {
            if old_name.as_ref().is_some_and(|o| *o == sp.name) || index == Some(k) {
                sp.name = name.clone();
                n += 1;
            }
        }
    }
    if n == 0 {
        return Err(EngineError::Other("no such speaker".into()));
    }
    s.edit("Rename Speaker", move |pr, _| {
        pr.transcripts = next;
        Ok(())
    })?;
    Ok(json!({"renamed": n}))
}

fn remove_ranges(s: &mut Session, label: &str, ranges: Vec<TimeRange>) -> Result<Value> {
    let n = ranges.len();
    if n == 0 {
        return Ok(json!({"removed": 0, "ticks": 0}));
    }
    let total = s.edit_sequence(label, |q, ctx, _| Ok(tx::ripple_delete_ranges(q, ranges, ctx)))?;
    Ok(json!({"removed": n, "ticks": total.0, "seconds": total.0 as f64 / TICKS_PER_SECOND as f64}))
}

fn remove_pauses(s: &mut Session, p: &Value) -> Result<Value> {
    let words = sequence_words(s);
    let min = Tick::from_seconds_f64(f64_p(p, "minSeconds").unwrap_or(1.0));
    let keep = Tick::from_seconds_f64(f64_p(p, "keepSeconds").unwrap_or(0.15));
    let ranges = tx::find_pauses(&words, min, keep, s.sequence_rate());
    remove_ranges(s, "Remove Pauses", ranges)
}

fn remove_fillers(s: &mut Session, p: &Value) -> Result<Value> {
    let words = sequence_words(s);
    let fillers: Vec<String> = match p.get("fillers").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => tx::DEFAULT_FILLERS.iter().map(|f| f.to_string()).collect(),
    };
    let hits = tx::find_fillers(&words, &fillers);
    let ranges = tx::filler_ranges(&words, &hits, s.sequence_rate());
    remove_ranges(s, "Remove Filler Words", ranges)
}

fn create_captions(s: &mut Session, p: &Value) -> Result<Value> {
    let words = sequence_words(s);
    let d = CaptionRules::default();
    let rules = CaptionRules {
        max_chars: u64_p(p, "maxChars").map(|n| n as usize).unwrap_or(d.max_chars),
        lines: u64_p(p, "lines").map(|n| n as usize).unwrap_or(d.lines),
        min_duration: f64_p(p, "minSeconds").map(Tick::from_seconds_f64).unwrap_or(d.min_duration),
        max_duration: f64_p(p, "maxSeconds").map(Tick::from_seconds_f64).unwrap_or(d.max_duration),
        gap_frames: p.get("gapFrames").and_then(Value::as_i64).unwrap_or(d.gap_frames),
        break_pause: d.break_pause,
    };
    let blocks = tx::caption_blocks(&words, &rules, s.sequence_rate());
    let format = str_p(p, "format").and_then(CaptionFormat::from_name).unwrap_or_default();
    let name = str_p(p, "name").unwrap_or("Transcript").to_string();
    let n = blocks.len();
    let tid = s.edit_sequence("Create Captions", |q, ctx, st| {
        let tid = TrackId(ctx.alloc());
        let mut t = CaptionTrack::new(tid, name, format);
        t.captions = tx::blocks_to_captions(&blocks, ctx);
        q.caption_tracks.insert(0, t);
        st.caption_selection.clear();
        Ok(tid)
    })?;
    Ok(json!({"track": tid.0, "captions": n}))
}

fn models(s: &mut Session, _: &Value) -> Result<Value> {
    let host = s.speech_host.clone();
    // a speech host keeps its models itself (the web build: OPFS)
    let dir = if host.is_some() { None } else { models_dir() };
    Ok(json!({
        // whether the models can run here (an installed `Session::transcriber` doesn't use them)
        "available": speech_available() || host.is_some(),
        "default": filmcraft_speech::models::DEFAULT_MODEL,
        "dir": dir.as_ref().map(|d| d.to_string_lossy().to_string()),
        "models": filmcraft_speech::models::catalogue().iter().map(|m| json!({
            "id": m.id, "name": m.name, "multilingual": m.multilingual, "description": m.description,
            "license": m.license, "source": m.source, "size": m.size(),
            "installed": match &host {
                Some(h) => h.installed(m.id),
                None => dir.as_ref().is_some_and(|d| filmcraft_speech::models::installed(d, m)),
            },
        })).collect::<Vec<_>>(),
    }))
}

/// Download a catalogue model into `<data dir>/models` (feature `speech-download`), or as a
/// [`SpeechHost`] job (`{"job": id}`). Hosts show the size, source and licence
/// (`transcript.models`) and ask before running this.
fn download_model(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_p(p, "model").unwrap_or(filmcraft_speech::models::DEFAULT_MODEL);
    let m = filmcraft_speech::models::find(id).ok_or_else(|| speech_err(SpeechError::UnknownModel(id.into())))?;
    if let Some(host) = s.speech_host.clone() {
        if host.installed(m.id) {
            return Ok(json!({"model": m.id, "installed": true}));
        }
        let job = start_host_job(s, host.as_ref(), format!("Download {}", m.name), m, Vec::new(), Options::default(), Vec::new());
        return Ok(json!({"model": m.id, "job": job, "poll": "jobs.list", "cancel": "jobs.cancel"}));
    }
    let dir = models_dir().ok_or_else(|| EngineError::Other("no data directory for speech models".into()))?;
    #[cfg(feature = "speech-download")]
    {
        filmcraft_speech::models::download(&dir, m, &mut |_, _, _| true).map_err(speech_err)?;
        Ok(json!({"model": m.id, "dir": filmcraft_speech::models::model_dir(&dir, m).to_string_lossy()}))
    }
    #[cfg(not(feature = "speech-download"))]
    {
        let _ = (m, dir);
        Err(EngineError::Other("model downloads are not available in this build (built without the `speech-download` feature)".into()))
    }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        spec(
            "transcript.generate",
            "Transcribe…",
            &["Sequence", "Transcript"],
            r#"{"items":[id]?,"model":"whisper-base"?,"language":"en|auto"?,"diarize":bool?,"maxSpeakers":n?}"#,
            can_transcribe,
            generate,
            true,
        ),
        spec(
            "transcript.set",
            "Import Transcript",
            &[],
            r#"{"item":id,"transcript":{"language":str,"speakers":[{"name":str}],"words":[{"text":str,"start":tick,"end":tick,"speaker":n?}]}}"#,
            always,
            set,
            true,
        ),
        spec("transcript.delete", "Delete Transcript", &["Sequence", "Transcript"], r#"{"items":[id]?}"#, has_transcripts, delete, true),
        spec("transcript.inspect", "Inspect Transcript", &[], r#"{"paragraphGapSeconds":f?}"#, always, inspect, false),
        spec("transcript.search", "Search Transcript", &[], r#"{"query":str}"#, always, search, false),
        spec("transcript.models", "List Speech Models", &[], "{}", always, models, false),
        spec("transcript.downloadModel", "Download Speech Model", &[], r#"{"model":"whisper-base"?}"#, can_download, download_model, true),
        spec("transcript.select", "Mark Selected Text", &[], r#"{"from":word,"to":word?}"#, has_transcript, select, true),
        spec("transcript.extract", "Extract Selected Text", &[], r#"{"from":word,"to":word?}"#, has_transcript, |s, p| extract_or_lift(s, p, true), true),
        spec("transcript.lift", "Lift Selected Text", &[], r#"{"from":word,"to":word?}"#, has_transcript, |s, p| extract_or_lift(s, p, false), true),
        spec(
            "transcript.renameSpeaker",
            "Rename Speaker…",
            &[],
            r#"{"speaker":"Speaker 1"|index,"name":str,"item":id?}"#,
            has_transcripts,
            rename_speaker,
            true,
        ),
        spec(
            "transcript.removePauses",
            "Remove Pauses",
            &["Sequence", "Transcript"],
            r#"{"minSeconds":f?,"keepSeconds":f?}"#,
            has_transcript,
            remove_pauses,
            true,
        ),
        spec("transcript.removeFillers", "Remove Filler Words", &["Sequence", "Transcript"], r#"{"fillers":[str]?}"#, has_transcript, remove_fillers, true),
        spec(
            "transcript.createCaptions",
            "Create Captions from Transcript…",
            &["Sequence", "Transcript"],
            r#"{"maxChars":n?,"lines":1|2?,"minSeconds":f?,"maxSeconds":f?,"gapFrames":n?,"format":str?,"name":str?}"#,
            has_transcript,
            create_captions,
            true,
        ),
    ]
}
