// FilmCraft speech worker: Whisper speech-to-text off the UI thread (apps/filmcraft-web/src/speech.rs).
//
// Runs its own instance of the app's wasm module and calls only its `speech_worker_*` exports.
// The model is read from OPFS (`speech/<model>/<file>`, downloaded and verified by the page) and
// stays loaded for the next job. Messages:
//   page → worker  {type: "transcribe", job, model, files: [name…], options: json, audio: [Float32Array…]}
//   worker → page  {type: "progress", job, index, fraction, status}
//                  {type: "done", job, transcripts: [json…], seconds}
//                  {type: "error", job, message}
// A job can't be interrupted here: the page stops one by terminating the worker.

// the page's `?v=<build>`: the glue and the wasm are cached as immutable
const v = new URL(import.meta.url).search;
const ready = import("./filmcraft_web.js" + v).then(async (glue) => {
  await glue.default({ module_or_path: new URL("filmcraft_web_bg.wasm" + v, import.meta.url) });
  return glue;
});
let loaded = null;

async function readModelFile(model, name) {
  let dir = await navigator.storage.getDirectory();
  for (const d of ["speech", model]) dir = await dir.getDirectoryHandle(d);
  const file = await (await dir.getFileHandle(name)).getFile();
  return new Uint8Array(await file.arrayBuffer());
}

async function transcribe(m) {
  const glue = await ready;
  if (loaded !== m.model) {
    loaded = null;
    postMessage({ type: "progress", job: m.job, index: 0, fraction: 0, status: "Loading the speech model" });
    const bytes = {};
    for (const name of m.files) bytes[name] = await readModelFile(m.model, name);
    glue.speech_worker_load(m.model, bytes["config.json"], bytes["generation_config.json"], bytes["tokenizer.json"], bytes["model.safetensors"]);
    loaded = m.model;
  }
  const t0 = performance.now();
  const transcripts = [];
  m.audio.forEach((audio, index) => {
    const progress = (fraction, status) => postMessage({ type: "progress", job: m.job, index, fraction, status });
    transcripts.push(glue.speech_worker_transcribe(audio, m.options, progress));
  });
  postMessage({ type: "done", job: m.job, transcripts, seconds: (performance.now() - t0) / 1000 });
}

onmessage = (e) => {
  const m = e.data;
  if (m && m.type === "transcribe") {
    transcribe(m).catch((err) => {
      loaded = null;
      postMessage({ type: "error", job: m.job, message: String((err && err.message) || err) });
    });
  }
};
