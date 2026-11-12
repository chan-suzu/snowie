//! Fireworks, the snowplow and the local clock used for the New Year countdown.

use crate::geom::V2;
use std::f32::consts::{PI, TAU};

// ------------------------------------------------------------------ clock

#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// seconds until the next 1 January 00:00 local time
    pub secs_to_ny: i64,
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Local date and time: (year, month, day, hour, minute, second).
#[cfg(windows)]
fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (st.wYear as i32, st.wMonth as u32, st.wDay as u32, st.wHour as u32, st.wMinute as u32, st.wSecond as u32)
}

#[cfg(unix)]
fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    // SAFETY: localtime_r only writes the struct we hand it
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (
            tm.tm_year + 1900,
            tm.tm_mon as u32 + 1,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        )
    }
}

pub fn local_clock() -> Clock {
    let (y, mo, d, h, mi, s) = local_time();
    let now = days_from_civil(y, mo, d) * 86400 + (h * 3600 + mi * 60 + s) as i64;
    let ny = days_from_civil(y + 1, 1, 1) * 86400;
    Clock { year: y, month: mo, day: d, hour: h, minute: mi, secs_to_ny: ny - now }
}

pub fn countdown_text(secs: i64) -> String {
    let d = secs / 86400;
    let h = secs % 86400 / 3600;
    let m = secs % 3600 / 60;
    let s = secs % 60;
    if d > 0 {
        format!("До Нового года {d} д {h:02}:{m:02}:{s:02}")
    } else {
        format!("До Нового года {h:02}:{m:02}:{s:02}")
    }
}

// ------------------------------------------------------------------ fireworks

pub const PALETTE: [(f32, f32, f32); 8] = [
    (1.0, 0.25, 0.25), // red
    (1.0, 0.75, 0.2),  // gold
    (0.3, 1.0, 0.4),   // green
    (0.3, 0.6, 1.0),   // blue
    (0.85, 0.35, 1.0), // violet
    (1.0, 0.45, 0.8),  // pink
    (0.4, 1.0, 1.0),   // cyan
    (1.0, 1.0, 0.95),  // white
];

pub struct Spark {
    pub pos: V2,
    pub vel: V2,
    pub life: f32,
    pub max_life: f32,
    pub color: u8,
    pub size: f32,
    pub twinkle: bool,
    drag: f32,
    grav: f32,
}

pub struct Rocket {
    pub pos: V2,
    pub vel: V2,
    pub color: u8,
    kind: u8,
    power: f32,
}

#[derive(Default)]
pub struct Fireworks {
    pub rockets: Vec<Rocket>,
    pub sparks: Vec<Spark>,
    /// for sounds: (burst?, where, power)
    pub events: Vec<(bool, V2, f32)>,
    show_left: f32,
    rate: f32,
    acc: f32,
}

const MAX_SPARKS: usize = 9000;

impl Fireworks {
    pub fn active(&self) -> bool {
        self.show_left > 0.0 || !self.rockets.is_empty() || !self.sparks.is_empty()
    }

    /// Keep launching `rate` rockets per second for `secs` seconds.
    pub fn show(&mut self, secs: f32, rate: f32) {
        if self.show_left <= 0.0 {
            self.rate = rate;
            self.acc = 1.0; // first rocket right away
        } else {
            self.rate = self.rate.max(rate);
        }
        self.show_left = self.show_left.max(secs);
    }

    /// How many rockets should be launched this frame.
    pub fn due(&mut self, dt: f32) -> usize {
        if self.show_left <= 0.0 {
            return 0;
        }
        self.show_left -= dt;
        self.acc += self.rate * dt;
        let n = self.acc.floor();
        self.acc -= n;
        n as usize
    }

    /// `apex` is how high above `from` the rocket should burst.
    pub fn launch(&mut self, from: V2, apex: f32, s: f32, rng: &mut fastrand::Rng) {
        let g = 420.0 * s;
        let vy = -(2.0 * g * apex).sqrt();
        let vx = (rng.f32() - 0.5) * 0.35 * vy.abs();
        self.events.push((false, from, 0.0));
        self.rockets.push(Rocket {
            pos: from,
            vel: V2(vx, vy),
            color: rng.u8(0..PALETTE.len() as u8),
            kind: rng.u8(0..5),
            power: 0.8 + rng.f32() * 0.5,
        });
    }

    pub fn update(&mut self, dt: f32, s: f32, rng: &mut fastrand::Rng) {
        let g = 420.0 * s;
        let mut bursts = Vec::new();
        self.rockets.retain_mut(|r| {
            r.vel.y += g * dt;
            r.pos += r.vel * dt;
            if r.vel.y > -40.0 * s {
                bursts.push((r.pos, r.color, r.kind, r.power));
                return false;
            }
            true
        });
        // smoke-and-sparks trail
        for r in &self.rockets {
            if self.sparks.len() < MAX_SPARKS {
                self.sparks.push(Spark {
                    pos: r.pos + V2((rng.f32() - 0.5) * 2.0 * s, 0.0),
                    vel: V2((rng.f32() - 0.5) * 30.0 * s, 40.0 * s * rng.f32()),
                    life: 0.45,
                    max_life: 0.45,
                    color: 1,
                    size: 2.2 * s,
                    twinkle: false,
                    drag: 2.0,
                    grav: 30.0 * s,
                });
            }
        }
        for (p, c, k, pw) in bursts {
            self.burst(p, c, k, pw, s, rng);
        }
        for sp in &mut self.sparks {
            sp.vel.y += sp.grav * dt;
            sp.vel = sp.vel * (-sp.drag * dt).exp();
            sp.pos += sp.vel * dt;
            sp.life -= dt;
        }
        self.sparks.retain(|sp| sp.life > 0.0);
    }

    fn burst(&mut self, at: V2, color: u8, kind: u8, power: f32, s: f32, rng: &mut fastrand::Rng) {
        self.events.push((true, at, power));
        let speed = 230.0 * s * power;
        let second = rng.u8(0..PALETTE.len() as u8);
        let mut jitter = fastrand::Rng::with_seed(rng.u64(..));
        let mut push = |v: V2, color: u8, life: f32, size: f32, twinkle: bool, drag: f32, grav: f32| {
            if self.sparks.len() < MAX_SPARKS {
                let life = life * (0.8 + jitter.f32() * 0.4);
                self.sparks.push(Spark { pos: at, vel: v, life, max_life: life, color, size, twinkle, drag, grav });
            }
        };
        match kind {
            // peony: a filled sphere, two colours
            0 => {
                for i in 0..130 {
                    let a = rng.f32() * TAU;
                    let sp = speed * rng.f32().powf(0.3);
                    let c = if i % 3 == 0 { second } else { color };
                    push(V2::from_angle(a) * sp, c, 1.8, 2.8 * s, false, 1.6, 90.0 * s);
                }
            }
            // ring
            1 => {
                let tilt = 0.4 + rng.f32() * 0.6;
                for i in 0..80 {
                    let a = i as f32 / 80.0 * TAU;
                    let v = V2(a.cos(), a.sin() * tilt) * speed * 1.1;
                    push(v, color, 1.6, 3.0 * s, false, 1.5, 80.0 * s);
                }
            }
            // golden willow with long drooping tails
            2 => {
                for _ in 0..110 {
                    let a = rng.f32() * TAU;
                    let sp = speed * 0.8 * rng.f32().powf(0.35);
                    push(V2::from_angle(a) * sp, 1, 3.4, 2.4 * s, false, 2.1, 70.0 * s);
                }
            }
            // crackling white glitter
            3 => {
                for _ in 0..140 {
                    let a = rng.f32() * TAU;
                    let sp = speed * 0.9 * rng.f32().powf(0.4);
                    push(V2::from_angle(a) * sp, 7, 1.5, 2.6 * s, true, 1.8, 100.0 * s);
                }
                for _ in 0..30 {
                    let a = rng.f32() * TAU;
                    push(V2::from_angle(a) * speed * 0.6, color, 1.2, 3.0 * s, false, 1.5, 80.0 * s);
                }
            }
            // heart
            _ => {
                for i in 0..90 {
                    let t = i as f32 / 90.0 * TAU;
                    let x = 16.0 * t.sin().powi(3);
                    let y = -(13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos());
                    push(V2(x, y) / 16.0 * speed, if i % 2 == 0 { 0 } else { 5 }, 1.7, 3.0 * s, false, 1.4, 60.0 * s);
                }
            }
        }
        // bright flash in the middle
        for _ in 0..12 {
            push(V2::from_angle(rng.f32() * PI * 2.0) * speed * 0.15, 7, 0.25, 7.0 * s, false, 3.0, 0.0);
        }
    }
}

// ------------------------------------------------------------------ snowplow

pub struct Plow {
    pub floor: usize,
    pub x: f32,
    pub dir: f32,
    /// snow in front of the blade
    pub load: f32,
    pub wheel: f32,
    pub t: f32,
}

/// Truck geometry in px at 100% scale, x measured along the driving direction from the truck center.
pub const PLOW_HALF_LEN: f32 = 80.0;
pub const PLOW_BLADE: f32 = 86.0;
pub const PLOW_SPEED: f32 = 170.0;
