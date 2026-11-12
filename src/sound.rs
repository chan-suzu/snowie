//! Music and sound effects driven by the simulation: shared by every platform.

use crate::config::Config;
use crate::world::World;
use crate::{audio, music};
use std::sync::{Arc, OnceLock};

static MUSIC_PCM: OnceLock<Arc<Vec<i16>>> = OnceLock::new();

/// Music and sound settings, applied to the mixer; everything goes quiet while snow is hidden.
pub struct SoundState {
    pub audio: audio::Audio,
    playing: bool,
    rendering: bool,
    volumes: (f32, f32),
    engine: f32,
    engine_pan: f32,
}

impl SoundState {
    pub fn new() -> SoundState {
        SoundState {
            audio: audio::Audio::start(),
            playing: false,
            rendering: false,
            volumes: (-1.0, -1.0),
            engine: 0.0,
            engine_pan: 0.0,
        }
    }

    pub fn update(&mut self, cfg: &Config, hidden: bool) {
        let want = cfg.music && !hidden && cfg.volume > 0.0;
        let vols = (cfg.volume, if cfg.sounds { cfg.sound_volume } else { 0.0 });
        if vols != self.volumes {
            self.volumes = vols;
            self.audio.volumes(vols.0, vols.1);
        }
        // the track is synthesized on first use only, so a muted Snowie never holds it in memory
        if want && !self.rendering {
            self.rendering = true;
            std::thread::spawn(|| {
                let _ = MUSIC_PCM.set(Arc::new(music::render_samples()));
            });
        }
        match (want, self.playing, MUSIC_PCM.get()) {
            (true, false, Some(pcm)) => {
                self.audio.music(Some(pcm.clone()));
                self.playing = true;
            }
            (false, true, _) => {
                self.audio.music(None);
                self.playing = false;
            }
            _ => {}
        }
        if hidden && self.engine > 0.0 {
            self.engine = 0.0;
            self.audio.engine(0.0, 0.0);
        }
    }

    /// Plays this frame's sounds and keeps the snowplow engine in step with the truck.
    pub fn frame(&mut self, w: &mut World, cfg: &Config) {
        let vr = w.vrect;
        let pan = |x: f32| ((x - vr.l as f32) / vr.w().max(1) as f32 * 2.0 - 1.0).clamp(-1.0, 1.0);
        for (sfx, x) in w.sounds.drain(..) {
            if cfg.sounds {
                self.audio.play(sfx, pan(x));
            }
        }
        // the engine is loud on screen and fades as the truck drives off the edge
        let (level, at) = w
            .plows
            .first()
            .and_then(|p| {
                let f = w.floors.get(p.floor)?;
                let (l, r) = (f.left as f32, f.right() as f32);
                let inside = ((p.x - l).min(r - p.x) / (150.0 * w.scale) + 1.0).clamp(0.0, 1.0);
                Some((inside, p.x))
            })
            .unwrap_or((0.0, 0.0));
        let level = if cfg.sounds { level } else { 0.0 };
        // follow the truck across the screen: the mixer glides between these updates
        let p = pan(at);
        if (level - self.engine).abs() > 0.01
            || (p - self.engine_pan).abs() > 0.005
            || (level == 0.0 && self.engine != 0.0)
        {
            self.engine = level;
            self.engine_pan = p;
            self.audio.engine(level, p);
        }
    }
}
