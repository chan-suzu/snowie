//! Audio: a small mixer on `waveOut` that plays the music and the snow sounds together, in
//! stereo, panned by where on screen things happen. Every sound is synthesized at start-up
//! (filtered noise grains for snow, sine sweeps for whooshes and whistles), so there are no
//! audio files. When nothing plays the thread blocks and costs nothing.

use std::f32::consts::TAU;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;
use windows::core::PSTR;
use windows::Win32::Media::Audio::*;

pub const SR: u32 = 24000;
const FRAMES: usize = 480; // 20 ms per buffer
const BUFFERS: usize = 4;

#[derive(Clone, Copy, Debug)]
pub enum Sfx {
    /// snow scooped up
    Grab,
    /// rolling the ball over a drift; radius in px
    Roll(f32),
    /// thrown; speed 0..1
    Throw(f32),
    /// a ball breaks; strength 0..2
    Splat(f32),
    /// a ball settles on a drift or a stack; radius in px
    Plop(f32),
    /// the third ball is on: a snowman
    Snowman,
    /// a loose clump falls apart
    Crumble,
    /// firework rocket goes up
    Launch,
    /// firework bursts; size 0..1.5
    Boom(f32),
}

enum Cmd {
    Play(Sfx, f32),
    Music(Option<Arc<Vec<i16>>>),
    Volumes {
        music: f32,
        sfx: f32,
    },
    /// snowplow engine loudness 0..1 and pan
    Engine(f32, f32),
}

#[derive(Clone)]
pub struct Audio {
    tx: Sender<Cmd>,
}

impl Audio {
    pub fn start() -> Audio {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("snowie-audio".into()).spawn(move || run(rx)).ok();
        Audio { tx }
    }

    /// `pan` from -1 (left) to 1 (right).
    pub fn play(&self, sfx: Sfx, pan: f32) {
        let _ = self.tx.send(Cmd::Play(sfx, pan.clamp(-1.0, 1.0)));
    }

    pub fn music(&self, track: Option<Arc<Vec<i16>>>) {
        let _ = self.tx.send(Cmd::Music(track));
    }

    pub fn volumes(&self, music: f32, sfx: f32) {
        let _ = self.tx.send(Cmd::Volumes { music, sfx });
    }

    pub fn engine(&self, level: f32, pan: f32) {
        let _ = self.tx.send(Cmd::Engine(level, pan));
    }
}

// ------------------------------------------------------------------ synthesis

struct Bank {
    grab: Vec<Vec<f32>>,
    roll: Vec<Vec<f32>>,
    throw: Vec<f32>,
    splat: Vec<Vec<f32>>,
    plop: Vec<f32>,
    snowman: Vec<f32>,
    crumble: Vec<f32>,
    launch: Vec<f32>,
    boom: Vec<Vec<f32>>,
}

fn secs(n: f32) -> usize {
    (n * SR as f32) as usize
}

/// Two-pole resonant band-pass (state-variable filter), returns the band output.
struct Svf {
    low: f32,
    band: f32,
}

impl Svf {
    fn new() -> Svf {
        Svf { low: 0.0, band: 0.0 }
    }
    fn band(&mut self, x: f32, freq: f32, q: f32) -> f32 {
        let f = 2.0 * (std::f32::consts::PI * freq / SR as f32).sin();
        let high = x - self.low - self.band / q;
        self.band += f * high;
        self.low += f * self.band;
        self.band
    }
    fn low(&mut self, x: f32, freq: f32) -> f32 {
        self.band(x, freq, 0.7);
        self.low
    }
}

/// Snow crunch: many tiny filtered noise grains, the way compacting snow crackles.
fn crunch(rng: &mut fastrand::Rng, len: f32, grains: usize, center: f32, gain: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; secs(len + 0.03)];
    for g in 0..grains {
        let t0 = secs(len * (g as f32 / grains as f32).powf(0.8) + rng.f32() * 0.01);
        let glen = secs(0.003 + rng.f32() * 0.009);
        let freq = center * (0.6 + rng.f32() * 0.9);
        let amp = gain * (0.4 + rng.f32() * 0.6) * (1.0 - g as f32 / grains as f32 * 0.6);
        let mut f = Svf::new();
        for i in 0..glen {
            if t0 + i >= out.len() {
                break;
            }
            let env = (-(i as f32) / (glen as f32 * 0.3)).exp();
            out[t0 + i] += f.band((rng.f32() * 2.0 - 1.0) * env, freq, 2.5) * amp;
        }
    }
    out
}

fn whoosh(rng: &mut fastrand::Rng) -> Vec<f32> {
    let n = secs(0.32);
    let mut f = Svf::new();
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let env = (t / 0.12).min(1.0) * (1.0 - t).powf(1.6);
            let freq = 350.0 + 1400.0 * t.powf(0.7);
            f.band(rng.f32() * 2.0 - 1.0, freq, 3.0) * env * 0.9
        })
        .collect()
}

fn thud(rng: &mut fastrand::Rng, f0: f32, f1: f32, len: f32, noise: f32) -> Vec<f32> {
    let n = secs(len);
    let mut ph = 0.0f32;
    let mut lp = Svf::new();
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let k = i as f32 / n as f32;
            ph += TAU * (f0 + (f1 - f0) * k) / SR as f32;
            let body = ph.sin() * (-t / (len * 0.35)).exp();
            let hiss = lp.low(rng.f32() * 2.0 - 1.0, 1400.0) * (-t / (len * 0.25)).exp() * noise;
            body * 0.8 + hiss
        })
        .collect()
}

fn mix_into(a: &mut Vec<f32>, b: &[f32], at: usize, gain: f32) {
    if a.len() < at + b.len() {
        a.resize(at + b.len(), 0.0);
    }
    for (i, x) in b.iter().enumerate() {
        a[at + i] += x * gain;
    }
}

fn chime() -> Vec<f32> {
    let mut out = Vec::new();
    for (k, f) in [1046.5f32, 1318.5, 1568.0, 2093.0].iter().enumerate() {
        let n = secs(1.2);
        let tone: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                ((TAU * f * t).sin() + 0.3 * (TAU * f * 2.0 * t).sin() * (-t / 0.2).exp()) * (-t / 0.45).exp() * 0.35
            })
            .collect();
        mix_into(&mut out, &tone, secs(k as f32 * 0.09), 1.0);
    }
    out
}

fn whistle(rng: &mut fastrand::Rng) -> Vec<f32> {
    let n = secs(1.1);
    let mut ph = 0.0f32;
    let mut f = Svf::new();
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let freq = 900.0 + 1700.0 * t + 40.0 * (TAU * 9.0 * t * 1.1).sin();
            ph += TAU * freq / SR as f32;
            let env = (t / 0.05).min(1.0) * (1.0 - t).powf(0.8);
            (ph.sin() * 0.22 + f.band(rng.f32() * 2.0 - 1.0, 3000.0, 1.0) * 0.25) * env
        })
        .collect()
}

fn boom(rng: &mut fastrand::Rng) -> Vec<f32> {
    let n = secs(1.6);
    let mut lp = Svf::new();
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let noise = rng.f32() * 2.0 - 1.0;
            lp.low(noise, 260.0 + 600.0 * (-t / 0.05).exp()) * 2.2 * (-t / 0.35).exp()
        })
        .collect();
    // the boom itself sets the level; the crackle stays well under it
    normalize(&mut out, 0.9);
    // crackle tail
    for _ in 0..90 {
        let at = secs(0.25 + rng.f32() * 1.1);
        let freq = 2500.0 + rng.f32() * 3000.0;
        let click = crunch(rng, 0.004, 1, freq, 0.5);
        let mut click = click;
        normalize(&mut click, 1.0);
        mix_into(&mut out, &click, at, 0.12 + rng.f32() * 0.12);
    }
    out
}

fn normalize(v: &mut [f32], peak: f32) {
    let m = v.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-6);
    v.iter_mut().for_each(|x| *x *= peak / m);
}

fn bank() -> Bank {
    let mut rng = fastrand::Rng::with_seed(7);
    let norm = |mut v: Vec<f32>, p: f32| {
        normalize(&mut v, p);
        v
    };
    let grab =
        (0..3).map(|k| norm(crunch(&mut fastrand::Rng::with_seed(10 + k), 0.18, 28, 2600.0, 1.0), 0.55)).collect();
    let roll = (0..4).map(|k| norm(crunch(&mut fastrand::Rng::with_seed(k), 0.07, 9, 1800.0, 1.0), 0.3)).collect();
    let throw = norm(whoosh(&mut rng), 0.45);
    let splat = (0..3)
        .map(|k| {
            let mut r = fastrand::Rng::with_seed(100 + k);
            let mut v = thud(&mut r, 95.0, 50.0, 0.28, 0.9);
            mix_into(&mut v, &crunch(&mut r, 0.22, 34, 1700.0, 1.0), 0, 0.7);
            norm(v, 0.8)
        })
        .collect();
    let mut plop = thud(&mut rng, 140.0, 80.0, 0.14, 0.4);
    mix_into(&mut plop, &crunch(&mut rng, 0.06, 8, 2200.0, 1.0), 0, 0.4);
    let plop = norm(plop, 0.5);
    let snowman = norm(chime(), 0.5);
    let crumble = norm(crunch(&mut rng, 0.38, 40, 2000.0, 1.0), 0.5);
    let launch = norm(whistle(&mut rng), 0.35);
    let boom = (0..3).map(|k| norm(boom(&mut fastrand::Rng::with_seed(200 + k)), 0.9)).collect();
    Bank { grab, roll, throw, splat, plop, snowman, crumble, launch, boom }
}

/// Writes every sound to `dir` as a WAV file (for `--export-sounds`).
pub fn export(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let b = bank();
    let mut list: Vec<(&str, &Vec<f32>)> = vec![
        ("grab", &b.grab[0]),
        ("roll", &b.roll[0]),
        ("throw", &b.throw),
        ("splat", &b.splat[0]),
        ("plop", &b.plop),
        ("snowman", &b.snowman),
        ("crumble", &b.crumble),
        ("launch", &b.launch),
        ("boom", &b.boom[0]),
    ];
    for (name, v) in list.drain(..) {
        let mut wav = Vec::new();
        let n = (v.len() * 2) as u32;
        wav.extend(b"RIFF");
        wav.extend((36 + n).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(SR.to_le_bytes());
        wav.extend((SR * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(n.to_le_bytes());
        for x in v {
            wav.extend(((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(dir.join(format!("{name}.wav")), wav)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ mixer

struct Voice {
    buf: Arc<Vec<f32>>,
    pos: f32,
    rate: f32,
    gain: f32,
    pan: f32,
}

fn run(rx: Receiver<Cmd>) {
    let bank = bank();
    let arc = |v: &Vec<f32>| Arc::new(v.clone());
    let banks = (
        bank.grab.iter().map(arc).collect::<Vec<_>>(),
        bank.roll.iter().map(arc).collect::<Vec<_>>(),
        arc(&bank.throw),
        bank.splat.iter().map(arc).collect::<Vec<_>>(),
        arc(&bank.plop),
        arc(&bank.snowman),
        arc(&bank.crumble),
        arc(&bank.launch),
        bank.boom.iter().map(arc).collect::<Vec<_>>(),
    );
    let mut rng = fastrand::Rng::new();

    let fmt = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 2,
        nSamplesPerSec: SR,
        nAvgBytesPerSec: SR * 4,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut h = HWAVEOUT::default();
    if unsafe { waveOutOpen(Some(&mut h), WAVE_MAPPER, &fmt, None, None, CALLBACK_NULL) } != 0 {
        // no audio device: keep draining commands so senders never block
        while rx.recv().is_ok() {}
        return;
    }
    let mut bufs: Vec<Vec<i16>> = (0..BUFFERS).map(|_| vec![0i16; FRAMES * 2]).collect();
    let mut hdrs: Vec<WAVEHDR> = bufs
        .iter_mut()
        .map(|b| WAVEHDR {
            lpData: PSTR(b.as_mut_ptr() as *mut u8),
            dwBufferLength: (b.len() * 2) as u32,
            dwFlags: WHDR_DONE,
            ..Default::default()
        })
        .collect();
    let hsize = std::mem::size_of::<WAVEHDR>() as u32;
    for hd in hdrs.iter_mut() {
        unsafe { waveOutPrepareHeader(h, hd, hsize) };
        hd.dwFlags |= WHDR_DONE;
    }

    let mut voices: Vec<Voice> = Vec::new();
    let mut music: Option<Arc<Vec<i16>>> = None;
    let mut music_pos = 0usize;
    let (mut music_vol, mut sfx_vol) = (0.35f32, 0.5f32);
    let (mut engine, mut engine_pan, mut engine_cur, mut engine_ph) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut pan_cur = 0.0f32;
    let mut engine_lp = Svf::new();
    let mut mix = vec![0.0f32; FRAMES * 2];

    loop {
        let busy = !voices.is_empty() || music.is_some() || engine > 0.0 || engine_cur > 0.001;
        let pending = hdrs.iter().any(|hd| hd.dwFlags & WHDR_DONE == 0);
        // idle: wait for something to play without spinning
        let first = if busy || pending {
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            }
        };
        for cmd in first.into_iter().chain(rx.try_iter()) {
            match cmd {
                Cmd::Play(sfx, pan) => {
                    let pick = |v: &Vec<Arc<Vec<f32>>>, rng: &mut fastrand::Rng| v[rng.usize(..v.len())].clone();
                    let (buf, gain, rate) = match sfx {
                        Sfx::Grab => (pick(&banks.0, &mut rng), 1.0, 0.9 + rng.f32() * 0.25),
                        Sfx::Roll(r) => (pick(&banks.1, &mut rng), 0.8, (1.3 - r / 120.0).clamp(0.6, 1.3)),
                        Sfx::Throw(v) => (banks.2.clone(), 0.3 + v.min(1.0) * 0.8, 0.85 + v.min(1.0) * 0.35),
                        Sfx::Splat(k) => {
                            (pick(&banks.3, &mut rng), (0.35 + k * 0.45).min(1.3), (1.15 - k * 0.15).clamp(0.75, 1.2))
                        }
                        Sfx::Plop(r) => (banks.4.clone(), 0.9, (1.4 - r / 80.0).clamp(0.6, 1.4)),
                        Sfx::Snowman => (banks.5.clone(), 0.9, 1.0),
                        Sfx::Crumble => (banks.6.clone(), 0.9, 0.9 + rng.f32() * 0.2),
                        Sfx::Launch => (banks.7.clone(), 0.7, 0.9 + rng.f32() * 0.25),
                        Sfx::Boom(k) => (pick(&banks.8, &mut rng), (0.4 + k * 0.5).min(1.2), 0.85 + rng.f32() * 0.3),
                    };
                    if voices.len() < 48 {
                        voices.push(Voice { buf, pos: 0.0, rate, gain, pan });
                    }
                }
                Cmd::Music(m) => {
                    music = m;
                    music_pos = 0;
                }
                Cmd::Volumes { music: m, sfx: s } => {
                    music_vol = m;
                    sfx_vol = s;
                }
                Cmd::Engine(l, p) => {
                    engine = l;
                    engine_pan = p;
                }
            }
        }
        for i in 0..BUFFERS {
            if hdrs[i].dwFlags & WHDR_DONE == 0 {
                continue;
            }
            let busy = !voices.is_empty() || music.is_some() || engine > 0.0 || engine_cur > 0.001;
            if !busy {
                break;
            }
            mix.iter_mut().for_each(|x| *x = 0.0);
            // music, mono, looping
            if let Some(m) = &music {
                for f in 0..FRAMES {
                    let x = m[music_pos] as f32 / 32768.0 * music_vol;
                    mix[f * 2] += x;
                    mix[f * 2 + 1] += x;
                    music_pos = (music_pos + 1) % m.len();
                }
            }
            // effects, resampled for pitch variation and panned with equal power
            voices.retain_mut(|v| {
                let a = (v.pan + 1.0) * std::f32::consts::FRAC_PI_4;
                let (gl, gr) = (a.cos() * v.gain * sfx_vol, a.sin() * v.gain * sfx_vol);
                for f in 0..FRAMES {
                    let p = v.pos as usize;
                    if p + 1 >= v.buf.len() {
                        return false;
                    }
                    let fr = v.pos - p as f32;
                    let x = v.buf[p] * (1.0 - fr) + v.buf[p + 1] * fr;
                    mix[f * 2] += x * gl;
                    mix[f * 2 + 1] += x * gr;
                    v.pos += v.rate;
                }
                true
            });
            // snowplow engine: a low, rough rumble that fades in and out
            if engine > 0.0 || engine_cur > 0.001 {
                for f in 0..FRAMES {
                    engine_cur += (engine - engine_cur) * 0.0005;
                    // glide the pan sample by sample, so the sound moves smoothly between the ears
                    if engine_cur < 0.001 {
                        pan_cur = engine_pan; // a fresh start begins where the truck is
                    }
                    pan_cur += (engine_pan - pan_cur) * 0.0004;
                    let a = (pan_cur + 1.0) * std::f32::consts::FRAC_PI_4;
                    engine_ph += 42.0 / SR as f32;
                    let saw = (engine_ph.fract() * 2.0 - 1.0) + 0.5 * ((engine_ph * 2.0).fract() * 2.0 - 1.0);
                    let rough = engine_lp.low(rng.f32() * 2.0 - 1.0, 180.0) * 2.5;
                    let trem = 0.8 + 0.2 * (TAU * 7.0 * engine_ph / 42.0).sin();
                    let x = (saw * 0.25 + rough) * trem * engine_cur * 0.22 * sfx_vol;
                    mix[f * 2] += x * a.cos();
                    mix[f * 2 + 1] += x * a.sin();
                }
            }
            for (o, x) in bufs[i].iter_mut().zip(&mix) {
                // soft clip so many simultaneous sounds never crackle
                *o = (x.tanh() * 32000.0) as i16;
            }
            unsafe { waveOutWrite(h, &mut hdrs[i], hsize) };
        }
    }
    unsafe {
        waveOutReset(h);
        for hd in hdrs.iter_mut() {
            waveOutUnprepareHeader(h, hd, hsize);
        }
        waveOutClose(h);
    }
}
