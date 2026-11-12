//! New Year music. Both tunes are public domain ("Jingle Bells", J. Pierpont, 1857, and the
//! traditional English carol "We Wish You a Merry Christmas"); the arrangement is synthesized
//! here at start-up (music box, plucked bass, sleigh bells), so the project ships no audio
//! files and no third-party recordings.

use std::f32::consts::TAU;

const SR: u32 = crate::audio::SR;

struct Song {
    bpm: f32,
    beats_per_bar: f32,
    /// beats before the first full bar
    pickup: f32,
    transpose: i32,
    /// "E5:1 G5:0.5 R:1 ...", duration in beats (default 1); '|' is ignored
    melody: &'static str,
    /// one bass root per bar (counted after the pickup)
    bass: &'static str,
    sleigh_bells: bool,
}

const JINGLE_BELLS: Song = Song {
    bpm: 132.0,
    beats_per_bar: 4.0,
    pickup: 0.0,
    transpose: 0,
    melody: "\
        G4 E5 D5 C5 | G4:3 G4:0.5 G4:0.5 | G4 E5 D5 C5 | A4:4 | A4 F5 E5 D5 | B4:4 | G5 G5 F5 D5 | E5:4 |\
        G4 E5 D5 C5 | G4:4 | G4 E5 D5 C5 | A4:3 A4 | A4 F5 E5 D5 | G5 G5 G5 G5 | A5 G5 F5 D5 | C5:2 G5:2 |\
        E5 E5 E5:2 | E5 E5 E5:2 | E5 G5 C5:1.5 D5:0.5 | E5:4 | F5 F5 F5:1.5 F5:0.5 | F5 E5 E5 E5:0.5 E5:0.5 |\
        E5 D5 D5 E5 | D5:2 G5:2 |\
        E5 E5 E5:2 | E5 E5 E5:2 | E5 G5 C5:1.5 D5:0.5 | E5:4 | F5 F5 F5:1.5 F5:0.5 | F5 E5 E5 E5:0.5 E5:0.5 |\
        G5 G5 F5 D5 | C5:4",
    bass: "C3 C3 C3 F2 F2 G2 G2 C3  C3 C3 C3 F2 F2 G2 G2 C3  C3 C3 C3 C3 F2 C3 D3 G2  C3 C3 C3 C3 F2 C3 G2 C3",
    sleigh_bells: true,
};

const WE_WISH: Song = Song {
    bpm: 150.0,
    beats_per_bar: 3.0,
    pickup: 1.0,
    transpose: 12,
    melody: "\
        D4 |\
        G4 G4:0.5 A4:0.5 G4:0.5 F#4:0.5 | E4 E4 E4 | A4 A4:0.5 B4:0.5 A4:0.5 G4:0.5 | F#4 D4 D4 |\
        B4 B4:0.5 C5:0.5 B4:0.5 A4:0.5 | G4 E4 D4:0.5 D4:0.5 | E4 A4 F#4 | G4:2 D4 |\
        G4 G4:0.5 A4:0.5 G4:0.5 F#4:0.5 | E4 E4 E4 | A4 A4:0.5 B4:0.5 A4:0.5 G4:0.5 | F#4 D4 D4 |\
        B4 B4:0.5 C5:0.5 B4:0.5 A4:0.5 | G4 E4 D4:0.5 D4:0.5 | E4 A4 F#4 | G4:2 D4 |\
        G4 G4 G4 | F#4:2 F#4 | G4 F#4 E4 | D4:2 A4 | B4 A4 G4 | D5 D4 D4:0.5 D4:0.5 | E4 A4 F#4 | G4:2 D4 |\
        G4 G4:0.5 A4:0.5 G4:0.5 F#4:0.5 | E4 E4 E4 | A4 A4:0.5 B4:0.5 A4:0.5 G4:0.5 | F#4 D4 D4 |\
        B4 B4:0.5 C5:0.5 B4:0.5 A4:0.5 | G4 E4 D4:0.5 D4:0.5 | E4 A4 F#4 | G4:3",
    bass: "G2 C3 A2 D3 B2 E2 D3 G2  G2 C3 A2 D3 B2 E2 D3 G2  G2 D3 E2 D3 G2 B2 D3 G2  G2 C3 A2 D3 B2 E2 D3 G2",
    sleigh_bells: false,
};

fn note_freq(name: &str) -> Option<f32> {
    let b = name.as_bytes();
    let semi = match b.first()? {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let (acc, rest) = match b.get(1) {
        Some(b'#') => (1, &name[2..]),
        Some(b'b') => (-1, &name[2..]),
        _ => (0, &name[1..]),
    };
    let oct: i32 = rest.parse().ok()?;
    let midi = 12 * (oct + 1) + semi + acc;
    Some(440.0 * 2f32.powf((midi - 69) as f32 / 12.0))
}

fn add_voice(out: &mut [f32], at: usize, secs: f32, f: impl Fn(f32) -> f32) {
    let len = (secs * SR as f32) as usize;
    for i in 0..len.min(out.len().saturating_sub(at)) {
        out[at + i] += f(i as f32 / SR as f32);
    }
}

/// Music-box tine: slightly inharmonic partials with their own decays.
fn bell(out: &mut [f32], at: usize, f: f32, amp: f32) {
    const P: [(f32, f32, f32); 5] =
        [(1.0, 1.0, 1.1), (2.0, 0.42, 0.55), (3.01, 0.16, 0.3), (4.17, 0.09, 0.18), (5.43, 0.05, 0.1)];
    add_voice(out, at, 2.2, |t| {
        let att = (t / 0.003).min(1.0);
        let mut s = 0.0;
        for (r, g, d) in P {
            s += g * (TAU * f * r * t).sin() * (-t / d).exp();
        }
        amp * att * s
    });
}

fn pluck(out: &mut [f32], at: usize, f: f32, amp: f32) {
    add_voice(out, at, 1.2, |t| {
        let att = (t / 0.006).min(1.0);
        let s = (TAU * f * t).sin() + 0.3 * (TAU * 2.0 * f * t).sin() * (-t / 0.15).exp();
        amp * att * s * (-t / 0.4).exp()
    });
}

fn jingle(out: &mut [f32], at: usize, amp: f32, rng: &mut fastrand::Rng) {
    let freqs = [5230.0, 6370.0, 7410.0, 8830.0, 10120.0];
    let phases: Vec<f32> = freqs.iter().map(|_| rng.f32() * TAU).collect();
    let len = (0.16 * SR as f32) as usize;
    let mut prev = 0.0;
    for i in 0..len.min(out.len().saturating_sub(at)) {
        let t = i as f32 / SR as f32;
        let n = rng.f32() * 2.0 - 1.0;
        let hp = n - prev; // crude high-pass
        prev = n;
        let mut metal = 0.0;
        for (f, p) in freqs.iter().zip(&phases) {
            metal += (TAU * f * t + p).sin();
        }
        let shake = 1.0 + 0.6 * (TAU * 28.0 * t).sin();
        out[at + i] += amp * (-t / 0.04).exp() * shake * (hp * 0.5 + metal * 0.12);
    }
}

fn render_song(song: &Song, rng: &mut fastrand::Rng) -> Vec<f32> {
    let spb = 60.0 / song.bpm;
    let k = 2f32.powf(song.transpose as f32 / 12.0);
    let mut notes = Vec::new();
    let mut t = 0.0f32;
    for tok in song.melody.split_whitespace().filter(|t| *t != "|") {
        let (name, dur) = match tok.split_once(':') {
            Some((n, d)) => (n, d.parse::<f32>().unwrap_or(1.0)),
            None => (tok, 1.0),
        };
        if let Some(f) = note_freq(name) {
            notes.push((t, f * k));
        }
        t += dur;
    }
    let total = t;
    let mut out = vec![0.0f32; ((total * spb + 2.5) * SR as f32) as usize];
    let pos = |beat: f32| (beat * spb * SR as f32) as usize;
    for (beat, f) in notes {
        bell(&mut out, pos(beat), f, 0.30);
        bell(&mut out, pos(beat) + 40, f * 2.0, 0.05); // a little sparkle an octave up
    }
    for (bar, root) in song.bass.split_whitespace().enumerate() {
        let Some(f) = note_freq(root) else { continue };
        let b0 = song.pickup + bar as f32 * song.beats_per_bar;
        if b0 >= total {
            break;
        }
        pluck(&mut out, pos(b0), f, 0.26);
        if song.beats_per_bar == 4.0 {
            pluck(&mut out, pos(b0 + 2.0), f * 1.5, 0.18);
        } else {
            pluck(&mut out, pos(b0 + 1.0), f * 2.0, 0.10);
            pluck(&mut out, pos(b0 + 2.0), f * 1.5, 0.10);
        }
    }
    if song.sleigh_bells {
        let mut beat = song.pickup;
        while beat < total {
            let accent = if beat.fract() == 0.0 { 0.16 } else { 0.09 };
            jingle(&mut out, pos(beat), accent, rng);
            beat += 0.5;
        }
    }
    out
}

/// A little room: four parallel feedback combs mixed under the dry signal.
fn reverb(dry: &mut [f32]) {
    let mut wet = vec![0.0f32; dry.len()];
    for (d, g) in [(0.0297f32, 0.62f32), (0.0371, 0.6), (0.0411, 0.58), (0.0437, 0.56)] {
        let d = (d * SR as f32) as usize;
        let mut y = vec![0.0f32; dry.len()];
        for i in 0..dry.len() {
            y[i] = dry[i] + if i >= d { g * y[i - d] } else { 0.0 };
            wet[i] += y[i];
        }
    }
    for (x, w) in dry.iter_mut().zip(wet) {
        *x += w * 0.07;
    }
}

/// Renders the playlist as 16-bit mono samples at the mixer rate.
pub fn render_samples() -> Vec<i16> {
    let mut rng = fastrand::Rng::with_seed(2025);
    let mut mix: Vec<f32> = Vec::new();
    for song in [&JINGLE_BELLS, &WE_WISH] {
        mix.extend(render_song(song, &mut rng));
        mix.extend(std::iter::repeat_n(0.0, SR as usize * 2));
    }
    reverb(&mut mix);
    let peak = mix.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-6);
    let gain = 0.85 / peak;
    mix.into_iter().map(|x| ((x * gain).clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

/// The playlist as a complete WAV file (for `--export-music`).
pub fn render_wav() -> Vec<u8> {
    let samples = render_samples();
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend(b"RIFF");
    wav.extend((36 + data_len).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes()); // PCM
    wav.extend(1u16.to_le_bytes()); // mono
    wav.extend(SR.to_le_bytes());
    wav.extend((SR * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_len.to_le_bytes());
    for x in samples {
        wav.extend(x.to_le_bytes());
    }
    wav
}
