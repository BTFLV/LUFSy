//! Writes a synthetic 150 BPM DJ set for testing and for the README screenshots.
//!
//! Six tracks at deliberately uneven levels, each with a breakdown, mixed into
//! each other over 32 beats. Before the set the previous DJ's last track fades
//! out, after it there is a few seconds of room noise.
//!
//! cargo run --release --example demo_set -- demo-set.wav [minutes]

use std::f32::consts::TAU;

const SR: f32 = 44_100.0;
const BPM: f32 = 150.0;

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// One track: level offset, root note (Hz) and brightness.
struct Track {
    gain_db: f32,
    root: f32,
    bright: f32,
}

struct Voice {
    hp: [f32; 2],
    lp: [f32; 2],
}

fn saw(phase: f32) -> f32 {
    2.0 * (phase - phase.floor()) - 1.0
}

fn track_sample(tr: &Track, v: &mut Voice, rng: &mut Rng, t: f32, local: f32, len: f32) -> [f32; 2] {
    let beat = 60.0 / BPM;
    let b = t / beat;
    let ph = b.fract();
    let bar = (local / (beat * 4.0)).floor() as i32;
    let bars = (len / (beat * 4.0)) as i32;
    let breakdown = bar >= bars / 2 - 4 && bar < bars / 2 + 4;

    let mut l = 0.0;
    let mut r = 0.0;

    if !breakdown {
        // Kick: pitch sweep with a hard transient and soft saturation.
        let f = 48.0 + 260.0 * (-ph * 38.0).exp();
        let kick = ((TAU * f * ph * beat).sin() * (-ph * 5.0).exp() * 2.2).tanh();
        // Off-beat bass.
        let off = (ph - 0.5).max(0.0) * 2.0;
        let bass = if ph >= 0.5 {
            saw(tr.root * 0.5 * t) * (-off * 4.0).exp() * 0.35
        } else {
            0.0
        };
        // Hats on the off-beat, high-passed noise.
        let n = rng.next();
        v.hp[0] += 0.6 * (n - v.hp[0]);
        let hat = (n - v.hp[0]) * (-((ph * 2.0).fract()) * 30.0).exp() * 0.22 * tr.bright;
        l += 0.9 * kick + bass + hat;
        r += 0.9 * kick + bass + hat * 0.8;
    }

    // Lead / pad: detuned saws through a one-pole low-pass, wider in the breakdown.
    let chord = [1.0, 1.189, 1.498];
    let mut ls = 0.0;
    let mut rs = 0.0;
    for (i, c) in chord.iter().enumerate() {
        let f = tr.root * 2.0 * c;
        ls += saw(f * t * 1.003 + i as f32 * 0.31);
        rs += saw(f * t * 0.997 + i as f32 * 0.57);
    }
    let gate = if breakdown {
        1.0
    } else {
        0.35 + 0.65 * (-(b * 2.0).fract() * 3.0).exp()
    };
    let cut = (0.04 + 0.12 * tr.bright)
        * if breakdown {
            0.6 + 0.4 * ((local / len) * 6.0).fract()
        } else {
            1.0
        };
    v.lp[0] += cut * (ls - v.lp[0]);
    v.lp[1] += cut * (rs - v.lp[1]);
    let pad = if breakdown { 0.22 } else { 0.14 };
    l += v.lp[0] * pad * gate;
    r += v.lp[1] * pad * gate;

    // Riser into the drop.
    if breakdown && bar >= bars / 2 + 2 {
        let p = ((local / (beat * 4.0)) - (bars / 2 + 2) as f32) / 2.0;
        let n = rng.next();
        v.hp[1] += 0.3 * (n - v.hp[1]);
        let riser = (n - v.hp[1]) * p * p * 0.3;
        l += riser;
        r -= riser;
    }

    let g = 10f32.powf(tr.gain_db / 20.0);
    [l * g, r * g]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("output path");
    let minutes: f32 = args.next().map(|m| m.parse().expect("minutes")).unwrap_or(20.0);

    let tracks = [
        Track {
            gain_db: -5.0,
            root: 55.0,
            bright: 1.0,
        },
        Track {
            gain_db: -12.0,
            root: 49.0,
            bright: 0.7,
        },
        Track {
            gain_db: -3.0,
            root: 58.3,
            bright: 1.2,
        },
        Track {
            gain_db: -10.0,
            root: 52.0,
            bright: 0.9,
        },
        Track {
            gain_db: -6.0,
            root: 46.2,
            bright: 1.1,
        },
        Track {
            gain_db: -14.0,
            root: 55.0,
            bright: 0.8,
        },
    ];
    let beat = 60.0 / BPM;
    let xfade = 32.0 * beat;
    let track_len = minutes * 60.0 / tracks.len() as f32 + xfade;
    let pre = 9.0;
    let set_len = tracks.len() as f32 * (track_len - xfade) + xfade;
    let total = pre + set_len + 5.0;

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SR as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    let mut rng = Rng(0x1234_5678);
    let mut voices: Vec<Voice> = (0..=tracks.len())
        .map(|_| Voice {
            hp: [0.0; 2],
            lp: [0.0; 2],
        })
        .collect();
    let prev = Track {
        gain_db: -4.0,
        root: 61.7,
        bright: 1.0,
    };

    for n in 0..(total * SR) as usize {
        let t = n as f32 / SR;
        let mut s = [rng.next() * 0.0003, rng.next() * 0.0003];
        if t < pre {
            // The previous DJ's last minute, already fading out.
            let fade = 1.0 - t / pre;
            let x = track_sample(&prev, &mut voices[tracks.len()], &mut rng, t, 60.0 + t, 200.0);
            s[0] += x[0] * fade;
            s[1] += x[1] * fade;
        } else if t < pre + set_len {
            let ts = t - pre;
            for (i, tr) in tracks.iter().enumerate() {
                let start = i as f32 * (track_len - xfade);
                let local = ts - start;
                if local < 0.0 || local > track_len {
                    continue;
                }
                let mut env = 1.0;
                if i > 0 && local < xfade {
                    env *= (local / xfade).sqrt();
                }
                if i + 1 < tracks.len() && local > track_len - xfade {
                    env *= ((track_len - local) / xfade).sqrt();
                }
                let x = track_sample(tr, &mut voices[i], &mut rng, t, local, track_len);
                s[0] += x[0] * env;
                s[1] += x[1] * env;
            }
        }
        for c in s {
            w.write_sample((c.clamp(-1.0, 1.0) * 32767.0) as i16).expect("write");
        }
    }
    w.finalize().expect("finalize");
}
