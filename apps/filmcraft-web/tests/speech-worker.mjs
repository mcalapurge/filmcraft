// Optional real Whisper worker check for smoke.mjs --speech-media <served speech.wav>.
// Downloads the pinned whisper-tiny model into the test browser's temporary OPFS profile.
export async function checkSpeechWorker({ evaluate, until, media }) {
  const exec = (id, params = {}) => evaluate(`filmcraft.execute(${JSON.stringify(id)}, ${JSON.stringify(params)})`);
  const imported = await evaluate(`filmcraft.importUrl(${JSON.stringify(media)})`);
  const item = imported.items?.[0];
  if (!Number.isSafeInteger(item)) throw new Error("speech fixture did not import");
  await exec("file.newSequence", { fromItem: item, name: "Speech worker check" });
  const started = await exec("transcript.generate", {
    items: [item], model: "whisper-tiny", language: "en", diarize: false,
    then: [{ command: "captions.createLive", params: { source: "transcript", preset: "Pop" } }],
  });
  if (!Number.isSafeInteger(started.job)) throw new Error("transcription did not return a background job");
  await until(`filmcraft.execute("jobs.list").then(jobs => jobs.some(j => j.id === ${started.job} && j.finished))`, 600000, 250);
  const jobs = await exec("jobs.list");
  const job = jobs.find(j => j.id === started.job);
  if (!job?.finished || !job.result || job.result.error) throw new Error("speech job failed: " + JSON.stringify(job));
  const transcript = await exec("transcript.inspect");
  if (transcript.words.length < 2) throw new Error("speech fixture produced fewer than two words");
  const words = transcript.words;
  for (let i = 0; i < words.length; i++) {
    if (words[i].start < 0 || words[i].end < words[i].start || (i && words[i].start < words[i - 1].start)) {
      throw new Error("speech word timings are invalid");
    }
  }
  const captions = await exec("captions.list");
  if (!captions.tracks.some(t => t.captions.some(c => c.words?.length))) throw new Error("speech followup did not create live captions");
  const caption = captions.tracks.flatMap(t => t.captions).find(c => c.words?.length);
  let rejected = false;
  try {
    await exec("captions.setWord", { caption: caption.id, index: 4294967296, text: "WRAPPED" });
  } catch {
    rejected = true;
  }
  if (!rejected) throw new Error("a 64-bit word index wrapped on wasm32");
  await exec("edit.undo");
  const undone = await exec("captions.list");
  if (undone.tracks.length) throw new Error("live captions were not one undo step");
  await exec("edit.redo");
  const models = await exec("transcript.models");
  if (!models.models.some(m => m.id === "whisper-tiny" && m.installed)) throw new Error("verified model was not retained in OPFS");
  return { ok: true, job: started.job, text: words.map(w => w.text).join(" "), words: words.length };
}
