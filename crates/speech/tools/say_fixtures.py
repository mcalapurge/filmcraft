"""Word-timed speech fixtures for `tests/whisper_model.rs` (macOS: `say` + ffmpeg).

Each word is synthesised on its own, trimmed to its voiced part and joined to the others with
seeded random pauses, so the true bounds of every word are known exactly. Writes `<name>.f32`
(mono 16 kHz f32le), `<name>.txt` (the text) and `<name>.words` (`start end word` per line, seconds),
plus an `<name>n` copy with white noise (σ 0.01) added.

    python3 crates/speech/tools/say_fixtures.py target/fixtures/speech/say
"""

import math
import os
import random
import struct
import subprocess
import sys
import tempfile

SR = 16000
TEXTS = {
    "a": "The quick brown fox jumps over the lazy dog while the farmer watches from his old wooden porch",
    "b": "Please remember to bring your camera tomorrow because we are filming the interview at nine o'clock",
    "c": "Editing video is mostly about rhythm and timing so every caption must appear exactly when the word is spoken",
}


def synth(word, tmp):
    aiff = os.path.join(tmp, "w.aiff")
    subprocess.run(["say", "-o", aiff, word], check=True)
    raw = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", aiff, "-ar", str(SR), "-ac", "1", "-f", "f32le", "-"], check=True, capture_output=True).stdout
    x = struct.unpack("<%df" % (len(raw) // 4), raw)
    fr = SR // 100
    db = [10 * math.log10(sum(v * v for v in x[i:i + fr]) / fr + 1e-12) for i in range(0, len(x), fr)]
    voiced = [i for i, d in enumerate(db) if d > -45]
    return list(x[voiced[0] * fr:(voiced[-1] + 1) * fr])


def write(out_dir, name, text, samples, truth):
    with open(os.path.join(out_dir, name + ".f32"), "wb") as f:
        f.write(struct.pack("<%df" % len(samples), *samples))
    with open(os.path.join(out_dir, name + ".txt"), "w") as f:
        f.write(text + "\n")
    with open(os.path.join(out_dir, name + ".words"), "w") as f:
        f.write("".join(f"{a:.3f} {b:.3f} {w}\n" for w, a, b in truth))


def main():
    out_dir = sys.argv[1]
    os.makedirs(out_dir, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for name, text in TEXTS.items():
            rng = random.Random(7)
            audio = [0.0] * (2 * SR)
            truth = []
            for w in text.split():
                x = synth(w, tmp)
                start = len(audio)
                audio += x
                truth.append((w, start / SR, len(audio) / SR))
                audio += [0.0] * int(SR * rng.uniform(0.05, 0.6))
            audio += [0.0] * int(SR * 1.5)
            write(out_dir, name, text, audio, truth)
            write(out_dir, name + "n", text, [v + rng.gauss(0, 0.01) for v in audio], truth)


if __name__ == "__main__":
    main()
