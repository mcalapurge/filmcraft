//! Whisper end-to-end on real speech, when the weights are on this machine.
//!
//! Never downloads. Looks for the model in `$FILMCRAFT_MODELS_DIR/<id>/`, then
//! `<repo>/target/models/<id>/` (put a model there by hand, or let the app download it into the
//! data directory and point `FILMCRAFT_MODELS_DIR` at `<data dir>/models`). Speech samples are
//! `<repo>/target/fixtures/speech/**/<name>.f32` (mono 16 kHz little-endian f32) with the
//! reference text in `<name>.txt`; without either the test prints SKIPPED.
#![cfg(feature = "whisper")]

use std::path::{Path, PathBuf};

use filmcraft_speech::{Options, Transcriber, models, word_error_rate};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn model_dir(id: &str) -> Option<PathBuf> {
    let m = models::find(id)?;
    let mut roots: Vec<PathBuf> = std::env::var_os("FILMCRAFT_MODELS_DIR").map(PathBuf::from).into_iter().collect();
    roots.push(repo().join("target/models"));
    // worktrees: the main checkout's target
    if let Ok(main) = std::fs::canonicalize(repo())
        && let Some(p) = main.ancestors().find(|p| p.join(".git").is_dir())
    {
        roots.push(p.join("target/models"));
    }
    roots.into_iter().find(|r| models::installed(r, m)).map(|r| models::model_dir(&r, m))
}

fn samples() -> Vec<(PathBuf, String)> {
    let mut roots = vec![repo().join("target/fixtures/speech")];
    if let Ok(main) = std::fs::canonicalize(repo())
        && let Some(p) = main.ancestors().find(|p| p.join(".git").is_dir())
    {
        roots.push(p.join("target/fixtures/speech"));
    }
    let mut out = Vec::new();
    for r in roots {
        let Ok(dirs) = std::fs::read_dir(&r) else { continue };
        for d in dirs.flatten() {
            let Ok(files) = std::fs::read_dir(d.path()) else { continue };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "f32")
                    && let Ok(txt) = std::fs::read_to_string(p.with_extension("txt"))
                {
                    out.push((p, txt.trim().to_string()));
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort();
    out
}

fn read_f32(p: &Path) -> Vec<f32> {
    std::fs::read(p).unwrap().as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

#[test]
fn tiny_model_transcribes_public_domain_speech() {
    let Some(dir) = model_dir("whisper-tiny") else {
        eprintln!("SKIPPED: whisper-tiny weights not found (see the test's module docs)");
        return;
    };
    let samples = samples();
    if samples.is_empty() {
        eprintln!("SKIPPED: no speech samples in target/fixtures/speech");
        return;
    }
    let w = filmcraft_speech::whisper::Whisper::load(&dir, "whisper-tiny").unwrap();
    let opts = Options { language: Some("en".into()), diarize: false, ..Default::default() };
    let (mut errs, mut words) = (0.0, 0usize);
    for (p, reference) in samples.iter().take(8) {
        let audio = read_f32(p);
        let t = w.transcribe(&audio, &opts, &mut |_, _| true).unwrap();
        let hyp = t.text();
        let n = reference.split_whitespace().count();
        errs += word_error_rate(reference, &hyp) * n as f64;
        words += n;
        // word times: in order, inside the audio
        let end = filmcraft_speech::sample_tick(audio.len() as i64);
        t.check().unwrap();
        assert!(t.words.iter().all(|x| x.end <= end), "{}", p.display());
    }
    let wer = errs / words.max(1) as f64;
    eprintln!("whisper-tiny WER {:.1}% over {words} words", wer * 100.0);
    assert!(wer < 0.25, "WER {wer}");
}

/// Word timing against samples that carry true word bounds in `<name>.words` (one
/// `start end word` line per word, seconds). Make them by synthesising each word on its own and
/// joining them with known pauses (`docs/transcripts.md` ▸ Testing). Guards against alignment
/// that shifts every word (it once put each word on the next one's audio, 0.3 s late).
#[test]
fn word_times_match_the_reference_bounds() {
    let Some((id, dir)) = ["whisper-base", "whisper-tiny", "whisper-small"].into_iter().find_map(|id| model_dir(id).map(|d| (id, d))) else {
        eprintln!("SKIPPED: no Whisper weights found (see the test's module docs)");
        return;
    };
    let timed: Vec<(PathBuf, Vec<(f64, f64, String)>)> = samples()
        .into_iter()
        .filter_map(|(p, _)| {
            let truth = std::fs::read_to_string(p.with_extension("words")).ok()?;
            let words = truth
                .lines()
                .filter_map(|l| {
                    let mut f = l.split_whitespace();
                    Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?, filmcraft_project::transcript::normalize_word(f.next()?)))
                })
                .collect();
            Some((p, words))
        })
        .collect();
    if timed.is_empty() {
        eprintln!("SKIPPED: no samples with `.words` timings in target/fixtures/speech");
        return;
    }
    let w = filmcraft_speech::whisper::Whisper::load(&dir, id).unwrap();
    let opts = Options { language: Some("en".into()), diarize: false, ..Default::default() };
    let secs = |t: filmcraft_time::Tick| t.0 as f64 / filmcraft_time::TICKS_PER_SECOND as f64;
    let (mut start_err, mut end_err) = (Vec::new(), Vec::new());
    for (p, truth) in &timed {
        let t = w.transcribe(&read_f32(p), &opts, &mut |_, _| true).unwrap();
        // pair words in order by text, skipping misrecognised ones
        let mut hyp = t.words.iter().peekable();
        for (s, e, text) in truth {
            let Some(h) = hyp.clone().position(|h| h.normalized() == *text).and_then(|k| hyp.nth(k)) else { continue };
            start_err.push(secs(h.start) - s);
            end_err.push(secs(h.end) - e);
        }
    }
    let n = start_err.len().max(1) as f64;
    let mean_start = start_err.iter().sum::<f64>() / n;
    let end_mae = end_err.iter().map(|x| x.abs()).sum::<f64>() / n;
    eprintln!("{id}: {} words, mean start error {mean_start:+.3}s, end MAE {end_mae:.3}s", start_err.len());
    assert!(start_err.len() * 2 >= timed.iter().map(|t| t.1.len()).sum::<usize>(), "too few words recognised");
    // words never start late on average (captions would trail the speech) and end near the truth
    assert!(mean_start <= 0.1, "words start {mean_start:+.3}s late on average");
    assert!(end_mae <= 0.25, "word ends are off by {end_mae:.3}s on average");
}
