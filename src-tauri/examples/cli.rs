//! Headless run of the full chain, for testing and benchmarking.
//!
//! cargo run --release --example cli -- in.wav out.wav [wav16|wav24|wav32f|mp3]

use std::path::Path;
use std::time::Instant;

use lufsy_lib::{analysis, plan, render};

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("input");
    let output = args.next().expect("output");
    let format = match args.next().as_deref() {
        Some("wav16") => render::Format::Wav16,
        Some("wav32f") => render::Format::Wav32f,
        Some("mp3") => render::Format::Mp3,
        _ => render::Format::Wav24,
    };

    let t = Instant::now();
    let a = analysis::analyze(Path::new(&input), |_| {}, || false).unwrap();
    let dto = a.to_dto();
    println!(
        "analysis   {:>6.2} s  {:.1} min  {} Hz  {}  in {:.1} LUFS  content {:.1}–{:.1} s",
        t.elapsed().as_secs_f64(),
        dto.duration_s / 60.0,
        dto.sample_rate,
        dto.codec,
        dto.integrated_lufs,
        dto.content_start_s,
        dto.content_end_s
    );

    let params = plan::Params {
        start_s: dto.content_start_s,
        end_s: dto.content_end_s,
        target_lufs: None,
        strength: 1.0,
        window_s: 45.0,
        smooth_s: 10.0,
        max_boost_db: 6.0,
        max_cut_db: 12.0,
        limiter: true,
        ceiling_db: -1.0,
        fade_in_s: 2.0,
        fade_out_s: 6.0,
        fade_curve: plan::FadeCurve::Smooth,
    };
    let t = Instant::now();
    let (p, report) = plan::compute(&a, &params);
    println!(
        "plan       {:>6.3} s  ref {:.1}  spread {:.1} → {:.1} LU  est {:.1} → {:.1} LUFS  boost {:.1} cut {:.1}  limiter {:.1}% max {:.1} dB",
        t.elapsed().as_secs_f64(),
        report.reference_lufs,
        report.spread_before,
        report.spread_after,
        report.lufs_before,
        report.lufs_after,
        report.max_boost_used,
        report.max_cut_used,
        report.limiter_share * 100.0,
        report.limiter_max_db
    );

    let t = Instant::now();
    let r = render::render(&a, &p, &params, format, Path::new(&output), |_| {}, || false).unwrap();
    println!(
        "render     {:>6.2} s  {:.1} s out  {:.1} LUFS  TP {:.2} dBTP  LRA {:.1} LU  limiter max {:.1} dB",
        t.elapsed().as_secs_f64(),
        r.duration_s,
        r.integrated_lufs,
        r.true_peak_db,
        r.loudness_range,
        r.limiter_max_db
    );

    // Verify: re-analyse the output and compare per-track loudness spread.
    let b = analysis::analyze(Path::new(&output), |_| {}, || false).unwrap();
    let pb = plan::Params {
        start_s: 0.0,
        end_s: b.duration_s(),
        ..params
    };
    let (_, after) = plan::compute(&b, &pb);
    println!(
        "re-measure            spread of output {:.1} LU, {:.1} LUFS",
        after.spread_before, after.lufs_before
    );
}
