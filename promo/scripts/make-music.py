#!/usr/bin/env python3
"""Synthesize the promo soundtrack (public/music.wav), timed to the edit.

  0.0 – 6.1 s   the "waiting" intro: a tense drone and a ticking clock
  5.1 – 6.1 s   riser into an impact on the reveal
  6.1 s – outro 120 BPM groove: kick, off-beat hats, bass, side-chained pads
                (Am – F – C – G), a plucked arpeggio from 10.5 s
  "20× faster"  accent hit; a lighter one as the creative-work scene opens
  outro         the beat drops out, the last chord rings
(times come from src/timeline.json)

Needs only numpy. Original and generated, so it's MIT like the rest.
"""
import json
import os
import wave

import numpy as np

SR = 44100
# Scene timing comes from the same file the video uses, so cuts and hits line up.
TL = json.load(open(os.path.join(os.path.dirname(__file__), "..", "src", "timeline.json")))
FPS, TR = TL["fps"], TL["transition"]
starts, _t = {}, 0
for _name, _d in TL["scenes"]:
    starts[_name] = _t / FPS
    _t += _d - TR
LEN = (_t + TR) / FPS  # composition length in seconds
REVEAL, BOOM, CREATIVE, OUTRO = starts["reveal"], starts["speed"] + 88 / FPS, starts["creative"], starts["outro"]
BPM = 120
BEAT = 60 / BPM
N = int(SR * LEN)
rng = np.random.default_rng(7)
L = np.zeros(N)
R = np.zeros(N)


def hz(m):
    return 440 * 2 ** ((m - 69) / 12)


def _filter(x, response):
    """Zero-phase filtering in the frequency domain (fine for offline audio)."""
    f = np.fft.rfftfreq(len(x), 1 / SR)
    return np.fft.irfft(np.fft.rfft(x) * response(f), n=len(x))


def lp(x, cutoff, order=2):
    return _filter(x, lambda f: 1 / np.sqrt(1 + (f / cutoff) ** (2 * order)))


def hp(x, cutoff, order=2):
    return _filter(x, lambda f: 1 / np.sqrt(1 + (cutoff / np.maximum(f, 1e-6)) ** (2 * order)))


def fftconvolve(x, h):
    n = 1 << int(np.ceil(np.log2(len(x) + len(h) - 1)))
    return np.fft.irfft(np.fft.rfft(x, n) * np.fft.rfft(h, n), n)[: len(x) + len(h) - 1]


def add(sig, start, gain=1.0, pan=0.0):
    i = int(start * SR)
    if i >= N:
        return
    j = min(N, i + len(sig))
    s = sig[: j - i] * gain
    L[i:j] += s * np.sqrt(0.5 * (1 - pan))
    R[i:j] += s * np.sqrt(0.5 * (1 + pan))


def saw(f, dur, detune=0.0):
    t = np.arange(int(dur * SR)) / SR
    return 2 * ((t * f * (1 + detune)) % 1) - 1


def adsr(n, a, d, s, r):
    a, d, r = int(a * SR), int(d * SR), int(r * SR)
    e = np.full(n, s)
    e[:a] = np.linspace(0, 1, a) if a else 1
    e[a:a + d] = np.linspace(1, s, len(e[a:a + d]))
    if r:
        e[-r:] *= np.linspace(1, 0, len(e[-r:]))
    return e


def reverb(x, secs=1.6, mix=0.25):
    ir = rng.standard_normal(int(secs * SR)) * np.exp(-np.linspace(0, 6, int(secs * SR)))
    ir = lp(ir, 6000)
    wet = fftconvolve(x, ir)[: len(x)]
    wet /= np.max(np.abs(wet)) + 1e-9
    return x * (1 - mix) + wet * mix * np.max(np.abs(x))


chords = [[57, 60, 64, 69], [53, 57, 60, 65], [48, 52, 55, 60], [55, 59, 62, 67]]  # Am F C G
roots = [45, 41, 36, 43]
bar = 4 * BEAT

# --- intro: tense drone + ticking clock -----------------------------------
t = np.arange(int(REVEAL * SR)) / SR
drone = (saw(hz(33), REVEAL) + saw(hz(33), REVEAL, 0.004) + 0.5 * np.sin(2 * np.pi * hz(45) * t))
drone = lp(drone, 380) * np.linspace(0.15, 0.55, len(t))
add(drone * 0.35, 0)
for k in range(int(REVEAL / BEAT)):
    click = hp(rng.standard_normal(int(0.03 * SR)), 3000) * np.exp(-np.linspace(0, 12, int(0.03 * SR)))
    add(click, k * BEAT, 0.18 if k % 2 else 0.26, pan=-0.3 if k % 2 else 0.3)

# --- riser into the reveal ----------------------------------------------------
rl = 1.0
noise = rng.standard_normal(int(rl * SR))
rise = np.concatenate([hp(noise[i:i + 2205], 400 + 9000 * (i / len(noise))) for i in range(0, len(noise), 2205)])
add(rise * np.linspace(0, 1, len(rise)) ** 2 * 0.35, REVEAL - rl)


def impact(at, gain=1.0):
    d = 1.6
    t = np.arange(int(d * SR)) / SR
    f = 30 + 90 * np.exp(-t * 18)
    boom = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 2.4)
    crash = hp(rng.standard_normal(len(t)), 2500) * np.exp(-t * 3.2) * 0.35
    add((boom + crash) * gain, at)


impact(REVEAL, 0.9)
impact(BOOM, 0.6)
impact(CREATIVE, 0.4)

# --- main groove -----------------------------------------------------------------
pump = np.ones(N)  # side-chain envelope from the kick
beats = np.arange(REVEAL, OUTRO - 0.01, BEAT)
for k, b in enumerate(beats):
    d = 0.42
    tt = np.arange(int(d * SR)) / SR
    f = 46 + 110 * np.exp(-tt * 30)
    kick = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-tt * 9)
    add(kick, b, 0.95)
    i = int(b * SR)
    dip = 1 - 0.6 * np.exp(-np.linspace(0, 6, int(BEAT * SR)))
    pump[i:i + len(dip)] = dip[: max(0, min(len(dip), N - i))]
    # off-beat hat, and a lighter 16th after the first bars
    hat = hp(rng.standard_normal(int(0.05 * SR)), 7000) * np.exp(-np.linspace(0, 9, int(0.05 * SR)))
    add(hat, b + BEAT / 2, 0.22, pan=0.25)
    if b > REVEAL + 2 * bar:
        add(hat, b + BEAT / 4 * 3, 0.08, pan=-0.25)

# bass: root notes in 8ths
for k, b in enumerate(np.arange(REVEAL, OUTRO - 0.01, BEAT / 2)):
    chord_i = int((b - REVEAL) // bar) % 4
    d = BEAT / 2 * 0.9
    tt = np.arange(int(d * SR)) / SR
    note = np.sin(2 * np.pi * hz(roots[chord_i]) * tt) + 0.3 * np.sin(2 * np.pi * 2 * hz(roots[chord_i]) * tt)
    add(note * adsr(len(tt), 0.005, 0.08, 0.7, 0.03), b, 0.28)

# pads: detuned saws per chord, low-passed, wide
pad_l = np.zeros(N)
pad_r = np.zeros(N)
for k, b in enumerate(np.arange(REVEAL, LEN, bar)):
    chord = chords[k % 4]
    d = min(bar + 0.6, LEN - b)
    n = int(d * SR)
    sl = sum(saw(hz(m), d, -0.003) for m in chord)
    sr_ = sum(saw(hz(m), d, 0.003) for m in chord)
    env = adsr(n, 0.25, 0.4, 0.8, 0.5)
    i = int(b * SR)
    j = min(N, i + n)
    pad_l[i:j] += (lp(sl, 1800) * env)[: j - i]
    pad_r[i:j] += (lp(sr_, 1800) * env)[: j - i]
L += reverb(pad_l * pump, 2.2, 0.35) * 0.05
R += reverb(pad_r * pump, 2.2, 0.35) * 0.05

# arpeggio: plucked chord tones in 16ths from "how it works" on
arp = np.zeros(N)
start = 10.5
for k, b in enumerate(np.arange(start, OUTRO + bar, BEAT / 4)):
    chord = chords[int((b - REVEAL) // bar) % 4]
    m = chord[[0, 1, 2, 3, 2, 1][k % 6]] + 12
    d = 0.22
    tt = np.arange(int(d * SR)) / SR
    pl = (saw(hz(m), d) * 0.5 + np.sin(2 * np.pi * hz(m) * tt)) * np.exp(-tt * 18)
    i = int(b * SR)
    j = min(N, i + len(pl))
    fade = 1.0 if b < OUTRO else max(0.0, 1 - (b - OUTRO) / bar)
    arp[i:j] += pl[: j - i] * fade
arp = reverb(lp(arp, 4200), 1.4, 0.3)
add(arp, 0, 0.09, pan=0.35)
add(np.concatenate([np.zeros(int(0.1875 * SR)), arp])[:N], 0, 0.05, pan=-0.45)  # dotted echo

# --- master: fades, glue, normalize -----------------------------------------
fade = np.ones(N)
fade[-int(2.5 * SR):] = np.linspace(1, 0, int(2.5 * SR)) ** 1.5
fade[: int(0.4 * SR)] = np.linspace(0, 1, int(0.4 * SR))
mix = np.stack([L, R]) * fade
mix = np.tanh(mix * 1.6) / np.tanh(1.6)
mix /= np.max(np.abs(mix)) * 1.12

out = os.path.join(os.path.dirname(__file__), "..", "public", "music.wav")
with wave.open(out, "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((mix.T * 32767).astype("<i2").tobytes())
print(f"wrote {os.path.normpath(out)} ({LEN:.1f}s)")
