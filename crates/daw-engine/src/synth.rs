//! A small polyphonic sine/saw synth with a click-free attack and release.

const VOICES: usize = 16;
const ATTACK_SECONDS: f32 = 0.003;
const RELEASE_SECONDS: f32 = 0.06;
const VOICE_LEVEL: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Waveform {
    Sine,
    #[default]
    Saw,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Held,
    Release,
}

#[derive(Clone, Copy)]
struct Voice {
    key: u8,
    stage: Stage,
    phase: f32,
    increment: f32,
    velocity: f32,
    envelope: f32,
    age: u64,
}

const IDLE_VOICE: Voice = Voice {
    key: 0,
    stage: Stage::Idle,
    phase: 0.0,
    increment: 0.0,
    velocity: 0.0,
    envelope: 0.0,
    age: 0,
};

pub struct Synth {
    voices: [Voice; VOICES],
    sample_rate: f32,
    attack_step: f32,
    release_step: f32,
    clock: u64,
    pub waveform: Waveform,
}

impl Synth {
    pub fn new(sample_rate: f32) -> Synth {
        Synth {
            voices: [IDLE_VOICE; VOICES],
            sample_rate,
            attack_step: 1.0 / (ATTACK_SECONDS * sample_rate),
            release_step: 1.0 / (RELEASE_SECONDS * sample_rate),
            clock: 0,
            waveform: Waveform::default(),
        }
    }

    pub fn note_on(&mut self, key: u8, velocity: f32) {
        self.clock += 1;
        let slot = self
            .voices
            .iter()
            .position(|v| v.stage != Stage::Idle && v.stage != Stage::Release && v.key == key)
            .or_else(|| self.voices.iter().position(|v| v.stage == Stage::Idle))
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map_or(0, |(i, _)| i)
            });
        let hz = 440.0 * 2f32.powf((f32::from(key) - 69.0) / 12.0);
        let v = &mut self.voices[slot];
        // Keep the envelope when stealing so the new note does not click.
        *v = Voice {
            key,
            stage: Stage::Attack,
            phase: 0.0,
            increment: hz / self.sample_rate,
            velocity: velocity.clamp(0.0, 1.0),
            envelope: v.envelope,
            age: self.clock,
        };
    }

    pub fn note_off(&mut self, key: u8) {
        for v in &mut self.voices {
            if v.key == key && matches!(v.stage, Stage::Attack | Stage::Held) {
                v.stage = Stage::Release;
            }
        }
    }

    /// Releases every sounding note (with the normal release tail).
    pub fn release_all(&mut self) {
        for v in &mut self.voices {
            if matches!(v.stage, Stage::Attack | Stage::Held) {
                v.stage = Stage::Release;
            }
        }
    }

    pub fn is_silent(&self) -> bool {
        self.voices.iter().all(|v| v.stage == Stage::Idle)
    }

    pub fn next_sample(&mut self) -> f32 {
        let mut sum = 0.0;
        for v in &mut self.voices {
            match v.stage {
                Stage::Idle => continue,
                Stage::Attack => {
                    v.envelope += self.attack_step;
                    if v.envelope >= 1.0 {
                        v.envelope = 1.0;
                        v.stage = Stage::Held;
                    }
                }
                Stage::Held => {}
                Stage::Release => {
                    v.envelope -= self.release_step;
                    if v.envelope <= 0.0 {
                        *v = IDLE_VOICE;
                        continue;
                    }
                }
            }
            let wave = match self.waveform {
                Waveform::Sine => (v.phase * std::f32::consts::TAU).sin(),
                Waveform::Saw => 2.0 * v.phase - 1.0 - poly_blep(v.phase, v.increment),
            };
            sum += wave * v.velocity * v.envelope * VOICE_LEVEL;
            v.phase += v.increment;
            if v.phase >= 1.0 {
                v.phase -= 1.0;
            }
        }
        sum
    }
}

/// Band-limited correction for the saw's discontinuity.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(synth: &mut Synth, n: usize) -> Vec<f32> {
        (0..n).map(|_| synth.next_sample()).collect()
    }

    #[test]
    fn sine_has_expected_frequency() {
        let mut s = Synth::new(48_000.0);
        s.waveform = Waveform::Sine;
        s.note_on(69, 1.0); // A4 = 440 Hz
        let out = render(&mut s, 48_000);
        let crossings = out.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((439..=441).contains(&crossings), "crossings = {crossings}");
    }

    #[test]
    fn note_off_decays_to_silence() {
        let mut s = Synth::new(48_000.0);
        s.note_on(60, 1.0);
        render(&mut s, 4_800);
        assert!(!s.is_silent());
        s.note_off(60);
        render(&mut s, 4_800);
        assert!(s.is_silent());
    }

    #[test]
    fn output_is_bounded_with_full_polyphony() {
        let mut s = Synth::new(44_100.0);
        for k in 40..80 {
            s.note_on(k, 1.0); // more notes than voices: exercises stealing
        }
        let peak = render(&mut s, 44_100)
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            peak.is_finite() && peak < VOICES as f32 * VOICE_LEVEL * 1.1,
            "peak = {peak}"
        );
    }
}
