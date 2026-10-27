use std::f32::consts::TAU;
use std::path::PathBuf;

use lufsy_lib::analysis::{self, integrated_lufs, Analysis};
use lufsy_lib::kweight::KWeighting;
use lufsy_lib::limiter::Limiter;
use lufsy_lib::plan::{self, FadeCurve, Params};
use lufsy_lib::render::{self, Format};

fn params(start: f64, end: f64) -> Params {
    Params {
        start_s: start,
        end_s: end,
        target_lufs: None,
        strength: 1.0,
        window_s: 20.0,
        smooth_s: 4.0,
        max_boost_db: 12.0,
        max_cut_db: 12.0,
        limiter: true,
        ceiling_db: -1.0,
        fade_in_s: 0.0,
        fade_out_s: 0.0,
        fade_curve: FadeCurve::Smooth,
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lufsy-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Stereo 1 kHz sine, `segments` of (seconds, dBFS), written as 32-bit float WAV.
fn write_tone(path: &PathBuf, rate: u32, segments: &[(f32, f32)]) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    let mut n = 0u64;
    for &(secs, db) in segments {
        let a = 10f32.powf(db / 20.0);
        for _ in 0..(secs * rate as f32) as u64 {
            let s = a * (TAU * 1000.0 * n as f32 / rate as f32).sin();
            w.write_sample(s).unwrap();
            w.write_sample(s).unwrap();
            n += 1;
        }
    }
    w.finalize().unwrap();
}

#[test]
fn sine_at_minus_20_dbfs_reads_minus_20_lufs() {
    for rate in [44_100, 48_000, 96_000] {
        let path = tmp(&format!("sine-{rate}.wav"));
        write_tone(&path, rate, &[(10.0, -20.0)]);
        let a = analysis::analyze(&path, |_| {}, || false).unwrap();
        let l = integrated_lufs(&a.energy);
        assert!((l + 20.0).abs() < 0.1, "{rate} Hz: {l} LUFS");
    }
}

#[test]
fn k_weighting_is_stable_and_near_unity_at_1k() {
    let mut k = KWeighting::new(48_000);
    let mut e = 0.0;
    let n = 48_000;
    for i in 0..n {
        let y = k.tick((TAU * 1000.0 * i as f32 / 48_000.0).sin());
        if i > n / 2 {
            e += y * y;
        }
    }
    let gain_db = 10.0 * (e / (n / 2) as f64 / 0.5).log10();
    assert!(gain_db.abs() < 0.8, "{gain_db} dB at 1 kHz");
}

#[test]
fn limiter_never_exceeds_ceiling_and_keeps_length() {
    let mut lim = Limiter::new(48_000, -1.0, 2.0, 80.0);
    let ceiling = 10f32.powf(-1.0 / 20.0);
    let mut seed = 7u32;
    let mut out = Vec::new();
    let n = 200_000;
    for i in 0..n {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
        // Occasional single-sample spikes are the hardest case for a lookahead.
        let x = if i % 9_973 == 0 { 4.0 } else { noise * 1.8 };
        if let Some(y) = lim.process([x, -x * 0.5]) {
            out.push(y);
        }
    }
    lim.flush(|y| out.push(y));
    assert_eq!(out.len(), n);
    for y in &out {
        assert!(y[0].abs() <= ceiling + 1e-6 && y[1].abs() <= ceiling + 1e-6);
    }
}

#[test]
fn limiter_leaves_quiet_signal_untouched() {
    let mut lim = Limiter::new(44_100, -1.0, 2.0, 80.0);
    let delay = lim.latency();
    let input: Vec<f32> = (0..10_000).map(|i| 0.5 * (i as f32 * 0.01).sin()).collect();
    let mut out = Vec::new();
    for &x in &input {
        if let Some(y) = lim.process([x, x]) {
            out.push(y[0]);
        }
    }
    assert_eq!(out.len(), input.len() - delay);
    for (o, i) in out.iter().zip(&input) {
        assert_eq!(o, i);
    }
}

fn synthetic(levels_db: &[f64], secs_each: usize) -> Analysis {
    let blocks_each = secs_each * 10;
    let mut energy = Vec::new();
    for &l in levels_db {
        // Alternate loud and slightly quieter bars so the top-30 % estimate has work to do.
        for i in 0..blocks_each {
            let v = if (i / 20) % 4 == 3 { l - 6.0 } else { l };
            energy.push(lufsy_lib::kweight::lufs_to_energy(v));
        }
    }
    let n = energy.len();
    Analysis {
        path: String::new(),
        sample_rate: 48_000,
        source_channels: 2,
        codec: "TEST".into(),
        frames: n as u64 * 4_800,
        block_frames: 4_800,
        energy,
        peak: vec![0.3; n],
        rms: vec![0.1; n],
    }
}

#[test]
fn plan_levels_tracks_and_keeps_reference() {
    let a = synthetic(&[-10.0, -18.0, -10.0, -14.0], 120);
    let (_, r) = plan::compute(&a, &params(0.0, 480.0));
    assert!(r.spread_before > 6.0, "before {}", r.spread_before);
    assert!(r.spread_after < 1.5, "after {}", r.spread_after);
    // The quiet track gets lifted by about its distance to the median.
    let mid_quiet = r.gain_db[180 * 10];
    let mid_loud = r.gain_db[60 * 10];
    assert!((mid_quiet - mid_loud - 8.0).abs() < 1.0, "{mid_quiet} vs {mid_loud}");
}

#[test]
fn plan_respects_limits_strength_and_target() {
    let a = synthetic(&[-10.0, -30.0, -10.0], 120);
    let mut p = params(0.0, 360.0);
    p.max_boost_db = 4.0;
    let (_, r) = plan::compute(&a, &p);
    assert!(r.gain_db.iter().all(|&g| g <= 4.0 + 1e-3));

    let mut p = params(0.0, 360.0);
    p.strength = 0.0;
    p.target_lufs = Some(r.reference_lufs + 3.0);
    let (_, r2) = plan::compute(&a, &p);
    assert!(r2.gain_db.iter().all(|&g| (g - 3.0).abs() < 1e-3));
}

#[test]
fn fade_curves_hit_their_ends() {
    for c in [FadeCurve::Linear, FadeCurve::Smooth, FadeCurve::Log] {
        assert_eq!(c.gain(0.0), 0.0);
        assert!((c.gain(1.0) - 1.0).abs() < 1e-9);
        assert!(c.gain(0.5) > 0.0 && c.gain(0.5) < 1.0);
    }
}

#[test]
fn render_cuts_fades_levels_and_measures() {
    let input = tmp("set.wav");
    // A foreign tail, then a loud and a quiet track.
    write_tone(&input, 48_000, &[(5.0, -30.0), (40.0, -12.0), (40.0, -24.0)]);
    let a = analysis::analyze(&input, |_| {}, || false).unwrap();

    let mut p = params(5.0, 85.0);
    p.fade_in_s = 1.0;
    p.fade_out_s = 2.0;
    p.max_boost_db = 12.0;
    let (plan, _) = plan::compute(&a, &p);

    for format in [Format::Wav24, Format::Wav16, Format::Mp3] {
        let ext = if format == Format::Mp3 { "mp3" } else { "wav" };
        let out = tmp(&format!("out-{format:?}.{ext}"));
        let r = render::render(&a, &plan, &p, format, &out, |_| {}, || false).unwrap();
        assert!((r.duration_s - 80.0).abs() < 0.01, "{format:?}: {} s", r.duration_s);
        assert!(r.true_peak_db < -0.5, "{format:?}: TP {}", r.true_peak_db);

        let b = analysis::analyze(&out, |_| {}, || false).unwrap();
        if format != Format::Mp3 {
            assert_eq!(b.frames, 80 * 48_000);
            assert!(b.peak[0] < 0.01, "fade-in starts at silence");
            assert!(*b.peak.last().unwrap() < 0.05, "fade-out ends near silence");
        }
        // Both halves now sit within a few dB of each other.
        let first = integrated_lufs(&b.energy[100..350]);
        let second = integrated_lufs(&b.energy[500..750]);
        assert!((first - second).abs() < 2.0, "{format:?}: {first} vs {second}");
    }
}
