//! `transcript.generate` / `transcript.downloadModel` as [`SpeechHost`] jobs (the web build's
//! speech worker): the command returns a job, [`crate::transcript::poll`] reads the audio and
//! hands it over, and the finished job's transcripts are applied.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::transcript::{SpeechHost, SpeechJob};
use crate::{Event, Session};
use filmcraft_project::{ItemId, Transcript, Word};
use filmcraft_time::Tick;

/// A host that keeps the jobs it gets; the tests finish them.
#[derive(Default)]
struct FakeHost {
    installed: Mutex<BTreeSet<String>>,
    jobs: Mutex<Vec<SpeechJob>>,
    cancelled: Mutex<Vec<u64>>,
    /// Media time of the word each finished job's transcripts hold (inside the A1 clip).
    word_at: Mutex<Tick>,
}

impl SpeechHost for FakeHost {
    fn installed(&self, id: &str) -> bool {
        self.installed.lock().unwrap().contains(id)
    }
    fn start(&self, job: SpeechJob) {
        self.jobs.lock().unwrap().push(job);
    }
    fn cancel(&self, id: u64) {
        self.cancelled.lock().unwrap().push(id);
        for j in self.jobs.lock().unwrap().iter().filter(|j| j.id == id) {
            *j.result.lock().unwrap() = Some(Err("stopped".into()));
            j.progress.finished.store(true, Ordering::Relaxed);
        }
    }
}

impl FakeHost {
    /// Finish job `id` with one transcript per clip: "hello", half a second long, at `word_at`.
    fn complete(&self, id: u64) {
        let at = *self.word_at.lock().unwrap();
        let jobs = self.jobs.lock().unwrap();
        let j = jobs.iter().find(|j| j.id == id).unwrap();
        let ts = j
            .audio
            .iter()
            .map(|_| Transcript { language: "en".into(), words: vec![Word::new("hello", at, at + Tick::from_seconds_f64(0.5))], ..Default::default() })
            .collect();
        *j.transcripts.lock().unwrap() = Some(ts);
        *j.result.lock().unwrap() =
            Some(Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: 0.0, bytes: 0, render_fps: 0.0, extra_files: Vec::new() }));
        j.progress.finished.store(true, Ordering::Relaxed);
    }
}

/// The demo project with a fake speech host and no transcriber.
fn session() -> (Session, Arc<FakeHost>, ItemId) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let a = &s.active_sequence().unwrap().audio_tracks[0].items[0];
    let (item, source_in) = (a.item, a.source_in);
    let host = Arc::new(FakeHost::default());
    *host.word_at.lock().unwrap() = source_in + Tick::from_seconds_f64(0.1);
    s.transcriber = None;
    s.speech_host = Some(host.clone());
    (s, host, item)
}

/// Poll until the host has job `id` (the audio is read a block at a time).
fn poll_until_started(s: &mut Session, host: &FakeHost, id: u64) {
    for _ in 0..10_000 {
        if host.jobs.lock().unwrap().iter().any(|j| j.id == id) {
            return;
        }
        s.poll_persistence();
    }
    panic!("job {id} never reached the host");
}

fn toasts(s: &mut Session) -> Vec<(String, bool)> {
    s.events
        .drain(..)
        .filter_map(|e| match e {
            Event::Toast { message, error } => Some((message, error)),
            _ => None,
        })
        .collect()
}

#[test]
fn generate_runs_as_a_host_job_and_applies_when_it_finishes() {
    let (mut s, host, item) = session();
    assert!(s.is_enabled("transcript.generate"), "a speech host enables Transcribe without the whisper feature");
    let r = s.execute("transcript.generate", json!({"items": [item.0], "model": "whisper-tiny", "language": "de", "diarize": false})).unwrap();
    let id = r["job"].as_u64().unwrap();
    assert_eq!(r["model"], "whisper-tiny");
    assert!(s.project.transcripts.is_empty(), "nothing is applied before the job ends");

    poll_until_started(&mut s, &host, id);
    {
        let jobs = host.jobs.lock().unwrap();
        let j = &jobs[0];
        assert_eq!(j.model.id, "whisper-tiny");
        assert_eq!(j.options.language.as_deref(), Some("de"));
        assert!(!j.options.diarize);
        let dur = match &s.project.item(item).unwrap().kind {
            filmcraft_project::ItemKind::Media(m) => m.duration(),
            _ => unreachable!(),
        };
        let expected = dur.to_units_floor(filmcraft_speech::SAMPLE_RATE as i64) as usize;
        assert_eq!(j.audio.len(), 1);
        assert_eq!(j.audio[0].len(), expected, "the whole clip, read in blocks");
        assert!(j.audio[0].iter().any(|x| *x != 0.0), "real samples, not silence");
    }
    s.poll_persistence();
    assert!(s.project.transcripts.is_empty(), "still running");

    host.complete(id);
    s.poll_persistence();
    assert_eq!(s.project.transcripts[&item].words[0].text, "hello");
    assert!(toasts(&mut s).iter().any(|(m, e)| m.starts_with("Transcribed 1 clip") && !e));
    assert!(s.transcript_jobs.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.transcripts.is_empty(), "one undo step");
}

#[test]
fn then_commands_run_after_the_job() {
    let (mut s, host, item) = session();
    let then = json!([{"command": "captions.createLive", "params": {"maxWords": 2, "source": "transcript"}}]);
    let r = s.execute("transcript.generate", json!({"items": [item.0], "then": then})).unwrap();
    let id = r["job"].as_u64().unwrap();
    poll_until_started(&mut s, &host, id);
    assert!(s.active_sequence().unwrap().caption_tracks.is_empty(), "not before the transcript exists");
    host.complete(id);
    s.poll_persistence();
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.len(), 1, "captions made from the new transcript");
    assert_eq!(q.caption_tracks[0].captions[0].text, "hello");
}

#[test]
fn then_also_runs_when_transcribing_in_the_command() {
    let (mut s, host, item) = session();
    s.speech_host = None;
    let at = *host.word_at.lock().unwrap();
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    t.words.push(Word::new("hi", at, at + Tick::from_seconds_f64(0.3)));
    s.transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: t, id: "fixed".into() }));
    let then = json!([{"command": "captions.createLive", "params": {"source": "transcript"}}]);
    let r = s.execute("transcript.generate", json!({"items": [item.0], "then": then})).unwrap();
    assert!(r["then"][0]["result"].is_object(), "{r}");
    assert_eq!(s.active_sequence().unwrap().caption_tracks.len(), 1);
}

#[test]
fn hostile_then_is_refused() {
    let (mut s, _, item) = session();
    for then in [
        json!(3),
        json!([{"params": {}}]),
        json!([{"command": "transcript.generate"}]),
        json!([{"command": "captions.createLive", "params": 42}]),
        json!(vec![json!({"command": "captions.createLive"}); 17]),
    ] {
        assert!(s.execute("transcript.generate", json!({"items": [item.0], "then": then})).is_err(), "{then}");
    }
    assert!(s.transcript_jobs.is_empty(), "no job for a refused command");
}

#[test]
fn cancelling_stops_the_host_job_and_changes_nothing() {
    let (mut s, host, item) = session();
    let id = s.execute("transcript.generate", json!({"items": [item.0]})).unwrap()["job"].as_u64().unwrap();
    poll_until_started(&mut s, &host, id);
    s.execute("jobs.cancel", json!({"job": id})).unwrap();
    s.poll_persistence();
    assert_eq!(*host.cancelled.lock().unwrap(), [id]);
    s.poll_persistence();
    assert!(s.project.transcripts.is_empty());
    assert!(s.transcript_jobs.is_empty());
    assert!(toasts(&mut s).iter().all(|(_, e)| !e), "a stop is not an error");
}

#[test]
fn cancelling_while_reading_audio_never_reaches_the_host() {
    let (mut s, host, item) = session();
    let id = s.execute("transcript.generate", json!({"items": [item.0]})).unwrap()["job"].as_u64().unwrap();
    s.execute("jobs.cancel", json!({"job": id})).unwrap();
    s.poll_persistence();
    s.poll_persistence();
    assert!(host.jobs.lock().unwrap().is_empty());
    assert!(host.cancelled.lock().unwrap().is_empty());
    assert!(s.transcript_jobs.is_empty());
}

#[test]
fn clips_without_audio_are_skipped_and_none_left_is_an_error() {
    let (mut s, host, item) = session();
    // a still: media without audio
    let dir = std::env::temp_dir().join(format!("filmcraft-speech-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("still.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 40, 40, 255])).save(&png).unwrap();
    let r = s.execute("file.import", json!({"paths": [png.to_string_lossy()]})).unwrap();
    let video_only = ItemId(r["items"][0].as_u64().unwrap());
    assert!(s.source(video_only).is_some_and(|src| !src.info().has_audio()));
    let id = s.execute("transcript.generate", json!({"items": [video_only.0, item.0]})).unwrap()["job"].as_u64().unwrap();
    poll_until_started(&mut s, &host, id);
    assert_eq!(host.jobs.lock().unwrap()[0].audio.len(), 1, "only the clip with audio");

    let id = s.execute("transcript.generate", json!({"items": [video_only.0]})).unwrap()["job"].as_u64().unwrap();
    for _ in 0..5 {
        s.poll_persistence();
    }
    let job = s.jobs.iter().find(|j| j.id == id).unwrap();
    assert!(job.progress.finished.load(Ordering::Relaxed));
    assert!(toasts(&mut s).iter().any(|(m, e)| m.contains("none of the clips has audio") && *e));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn models_and_downloads_go_through_the_host() {
    let (mut s, host, _) = session();
    host.installed.lock().unwrap().insert("whisper-tiny".into());
    let m = s.execute("transcript.models", json!({})).unwrap();
    assert_eq!(m["available"], true);
    assert!(m["dir"].is_null(), "the host keeps the models");
    let installed = |id: &str| m["models"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap()["installed"].clone();
    assert_eq!((installed("whisper-tiny"), installed("whisper-base")), (json!(true), json!(false)));

    assert!(s.is_enabled("transcript.downloadModel"));
    let r = s.execute("transcript.downloadModel", json!({"model": "whisper-tiny"})).unwrap();
    assert_eq!(r["installed"], true, "nothing to download: {r}");
    let r = s.execute("transcript.downloadModel", json!({"model": "whisper-base"})).unwrap();
    let id = r["job"].as_u64().unwrap();
    {
        let jobs = host.jobs.lock().unwrap();
        assert_eq!((jobs.len(), jobs[0].model.id, jobs[0].audio.len()), (1, "whisper-base", 0), "handed over at once, no audio");
    }
    host.complete(id);
    s.poll_persistence();
    assert!(toasts(&mut s).iter().any(|(m, _)| m.starts_with("Downloaded Whisper base")));
    assert!(s.project.transcripts.is_empty());
    assert!(s.execute("transcript.downloadModel", json!({"model": "nope"})).is_err());
}

#[test]
fn a_failed_host_job_reports_its_error() {
    let (mut s, host, item) = session();
    let id = s.execute("transcript.generate", json!({"items": [item.0]})).unwrap()["job"].as_u64().unwrap();
    poll_until_started(&mut s, &host, id);
    {
        let jobs = host.jobs.lock().unwrap();
        *jobs[0].result.lock().unwrap() = Some(Err("download: the server answered 404".into()));
        jobs[0].progress.finished.store(true, Ordering::Relaxed);
    }
    s.poll_persistence();
    assert!(s.project.transcripts.is_empty());
    assert!(toasts(&mut s).iter().any(|(m, e)| m.contains("404") && *e));
}

#[test]
fn pending_speech_jobs_are_bounded() {
    let (mut s, _, _) = session();
    for _ in 0..8 {
        s.execute("transcript.downloadModel", json!({"model": "whisper-tiny"})).unwrap();
    }
    let jobs = s.jobs.len();
    assert!(s.execute("transcript.downloadModel", json!({"model": "whisper-tiny"})).is_err());
    assert_eq!(s.jobs.len(), jobs, "a rejected request must not allocate another job");
}

#[test]
fn finished_transcription_cannot_edit_a_replaced_document_with_reused_ids() {
    let (mut s, host, item) = session();
    let id = s.execute("transcript.generate", json!({"items": [item.0]})).unwrap()["job"].as_u64().unwrap();
    poll_until_started(&mut s, &host, id);
    host.complete(id);
    s.execute("file.openDemoProject", json!({})).unwrap();
    let before = s.project.clone();
    let undo = s.history.undo.len();
    s.poll_persistence();
    assert!(Arc::ptr_eq(&s.project, &before), "a job belongs to the document that started it");
    assert_eq!(s.history.undo.len(), undo);
    assert!(s.transcript_jobs.is_empty());
}

#[test]
fn transcription_followups_cannot_edit_a_different_active_sequence() {
    let (mut s, host, item) = session();
    let original = s.state.active_sequence.unwrap();
    let id = s
        .execute(
            "transcript.generate",
            json!({"items": [item.0], "then": [
                {"command": "captions.createLive", "params": {"source": "transcript"}}
            ]}),
        )
        .unwrap()["job"]
        .as_u64()
        .unwrap();
    poll_until_started(&mut s, &host, id);
    s.execute("file.newSequence", json!({"name": "Other"})).unwrap();
    let other = s.state.active_sequence.unwrap();
    s.execute("timeline.place", json!({"item": item.0, "time": 0})).unwrap();
    host.complete(id);
    s.poll_persistence();
    assert_eq!(s.state.active_sequence, Some(other));
    assert!(s.project.transcripts.contains_key(&item), "the transcript still belongs to its media");
    assert!(s.project.sequence(other).unwrap().caption_tracks.is_empty(), "followups must not edit another timeline");
    assert!(s.project.sequence(original).unwrap().caption_tracks.is_empty());
    assert!(toasts(&mut s).iter().any(|(m, _)| m.contains("follow-up") && m.contains("sequence")));
}
