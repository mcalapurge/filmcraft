//! Speech-to-text on a background thread: [`NativeSpeech`] is the [`SpeechHost`] the desktop app
//! installs, so `transcript.generate` and `transcript.downloadModel` run as jobs (progress in the
//! status bar, cancellable) instead of blocking the UI until the model has finished.
//!
//! Jobs run one at a time on one worker thread. The worker downloads a missing model, loads it
//! (on the GPU where [`filmcraft_speech::whisper`] finds one) and keeps it loaded for the next
//! job, then transcribes the job's clips in order. Each job runs under `catch_unwind`; a panic
//! ends that job with an error and drops the loaded model.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use filmcraft_export::{Progress, Report};
use filmcraft_speech::models::{self, ModelInfo};
use filmcraft_speech::{SpeechError, Transcriber};

use crate::transcript::{JobResult, SpeechHost, SpeechJob};

/// Progress steps of a job (`Progress::total` is set to this by the engine).
const STEPS: u64 = 1000;
/// Steps a download takes when the job then transcribes; a download-only job uses all of them.
const DOWNLOAD_SHARE: u64 = 300;
/// What a stopped job reports (the engine shows nothing for it).
const STOPPED: &str = "stopped";

/// The desktop [`SpeechHost`]: models in `models_dir`, one worker thread.
pub struct NativeSpeech {
    models_dir: PathBuf,
    worker: Mutex<Option<Sender<SpeechJob>>>,
}

impl NativeSpeech {
    pub fn new(models_dir: PathBuf) -> Self {
        Self { models_dir, worker: Mutex::new(None) }
    }

    /// The worker's queue, starting the worker when there is none (or it has gone).
    fn send(&self, job: SpeechJob) -> Result<(), String> {
        let mut worker = lock(&self.worker);
        let job = match worker.as_ref() {
            Some(tx) => match tx.send(job) {
                Ok(()) => return Ok(()),
                // the worker has gone: start another
                Err(e) => e.0,
            },
            None => job,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = self.models_dir.clone();
        if let Err(e) = std::thread::Builder::new().name("filmcraft-speech".into()).spawn(move || run(&dir, rx)) {
            return Err(format!("can't start speech-to-text: {e}"));
        }
        match tx.send(job) {
            Ok(()) => {
                *worker = Some(tx);
                Ok(())
            }
            Err(_) => Err("speech-to-text stopped unexpectedly".into()),
        }
    }
}

impl SpeechHost for NativeSpeech {
    fn installed(&self, id: &str) -> bool {
        models::find(id).is_some_and(|m| models::installed(&self.models_dir, m))
    }

    fn start(&self, job: SpeechJob) {
        let (progress, result) = (job.progress.clone(), job.result.clone());
        // the worker replaces this when it picks the job up
        *lock(&progress.status) = "Waiting for the transcription before it".into();
        if let Err(e) = self.send(job) {
            finish(&progress, &result, Err(e));
        }
    }

    /// Nothing to do: the engine has set the job's `progress.cancel`, which the worker checks
    /// between download blocks and transcription steps.
    fn cancel(&self, _id: u64) {}
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn finish(progress: &Progress, result: &JobResult, r: Result<Report, String>) {
    if let Err(e) = &r {
        *lock(&progress.status) = if e == STOPPED { "Stopped: nothing was changed".into() } else { e.clone() };
        *lock(&progress.error) = Some(e.clone());
    }
    *lock(result) = Some(r);
    progress.finished.store(true, Ordering::Relaxed);
}

fn set_status(progress: &Progress, done: u64, status: String) {
    progress.done.store(done.min(STEPS), Ordering::Relaxed);
    *lock(&progress.status) = status;
}

/// The worker: runs queued jobs until the host is dropped.
fn run(models_dir: &Path, rx: Receiver<SpeechJob>) {
    let mut loaded: Option<(&'static str, Arc<dyn Transcriber>)> = None;
    for job in rx {
        let started = web_time::Instant::now();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(models_dir, &mut loaded, &job)));
        let r = match r {
            Ok(r) => r,
            Err(_) => {
                loaded = None;
                Err("speech-to-text failed unexpectedly; nothing was changed".into())
            }
        };
        let r =
            r.map(|()| Report { path: String::new(), frames: 0, seconds: started.elapsed().as_secs_f64(), bytes: 0, render_fps: 0.0, extra_files: Vec::new() });
        finish(&job.progress, &job.result, r);
    }
}

fn run_job(models_dir: &Path, loaded: &mut Option<(&'static str, Arc<dyn Transcriber>)>, job: &SpeechJob) -> Result<(), String> {
    let p = &job.progress;
    let stopped = || p.cancel.load(Ordering::Relaxed);
    let m = job.model;
    if stopped() {
        return Err(STOPPED.into());
    }
    let mut base = 0;
    if !models::installed(models_dir, m) {
        let share = if job.audio.is_empty() { STEPS } else { DOWNLOAD_SHARE };
        download(models_dir, m, p, share, &stopped)?;
        base = share;
    }
    if job.audio.is_empty() {
        return Ok(());
    }
    if stopped() {
        return Err(STOPPED.into());
    }
    let t = match loaded.as_ref() {
        Some((id, t)) if *id == m.id => t.clone(),
        _ => {
            // free the previous model before loading the next (the large ones take gigabytes)
            *loaded = None;
            set_status(p, base, format!("Loading {}", m.name));
            let t = filmcraft_speech::load(models_dir, m.id).map_err(|e| e.to_string())?;
            *loaded = Some((m.id, t.clone()));
            t
        }
    };
    let n = job.audio.len();
    let mut out = Vec::with_capacity(n);
    for (i, audio) in job.audio.iter().enumerate() {
        let mut progress = |f: f32, _: &str| {
            let frac = ((i as f64 + f64::from(f.clamp(0.0, 1.0))) / n as f64).clamp(0.0, 1.0);
            let done = base + ((STEPS - base) as f64 * frac) as u64;
            let clip = if n > 1 { format!(" clip {} of {n}", i + 1) } else { String::new() };
            set_status(p, done, format!("Transcribing{clip} with {} {:.0}%", m.name, frac * 100.0));
            !stopped()
        };
        match t.transcribe(audio, &job.options, &mut progress) {
            Ok(tr) => out.push(tr),
            Err(SpeechError::Cancelled) => return Err(STOPPED.into()),
            Err(e) => return Err(e.to_string()),
        }
    }
    *lock(&job.transcripts) = Some(out);
    Ok(())
}

#[cfg(feature = "speech-download")]
fn download(models_dir: &Path, m: &'static ModelInfo, p: &Progress, share: u64, stopped: &dyn Fn() -> bool) -> Result<(), String> {
    set_status(p, 0, format!("Downloading {}", m.name));
    let r = models::download(models_dir, m, &mut |done, total, _| {
        let frac = if total == 0 { 0.0 } else { done as f64 / total as f64 };
        set_status(p, (share as f64 * frac.clamp(0.0, 1.0)) as u64, format!("Downloading {} {:.0}%", m.name, frac * 100.0));
        !stopped()
    });
    match r {
        Ok(()) => Ok(()),
        Err(_) if stopped() => Err(STOPPED.into()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(feature = "speech-download"))]
fn download(_: &Path, m: &'static ModelInfo, _: &Progress, _: u64, _: &dyn Fn() -> bool) -> Result<(), String> {
    Err(SpeechError::NotInstalled(m.id.into()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::SpeechJob;

    fn job(model: &'static ModelInfo, audio: Vec<Vec<f32>>) -> SpeechJob {
        let progress = Arc::new(Progress::default());
        progress.total.store(STEPS, Ordering::Relaxed);
        SpeechJob { id: 1, model, audio, options: Default::default(), progress, result: Arc::default(), transcripts: Arc::default() }
    }

    fn wait(progress: &Progress) {
        let t0 = web_time::Instant::now();
        while !progress.finished.load(Ordering::Relaxed) {
            assert!(t0.elapsed().as_secs() < 30, "the job never finished");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A job ends (with an error, never a hang or a panic) when its model can't be had, and the
    /// worker then takes the next job.
    #[test]
    fn a_job_without_its_model_fails_and_the_worker_goes_on() {
        let dir = std::env::temp_dir().join(format!("filmcraft-native-speech-{}", std::process::id()));
        let host = NativeSpeech::new(dir);
        let m = models::find("whisper-tiny").unwrap();
        assert!(!host.installed(m.id));
        for _ in 0..2 {
            let j = job(m, vec![vec![0.0; 16_000]]);
            let (progress, result, transcripts) = (j.progress.clone(), j.result.clone(), j.transcripts.clone());
            // stopped before it starts, so no download is attempted
            progress.cancel.store(true, Ordering::Relaxed);
            host.start(j);
            wait(&progress);
            assert_eq!(lock(&result).clone().unwrap().unwrap_err(), STOPPED);
            assert!(lock(&transcripts).is_none());
        }
    }
}
