//! The gain plan: how loud is the set at each moment, and how far do we move it.
//!
//! A live rider only knows the past and has to react late. Here the whole set
//! is known, so the level estimate is centred on each moment (it looks ahead as
//! far as it looks back) and the gain curve is smoothed with a zero-phase
//! filter. Tracks change level exactly where the mix changes them, not seconds
//! after.
//!
//! "Level" is the loudness of the loud part of a window: the top 30 % of its
//! 3 s short-term values. Breakdowns and intros are quieter on purpose and
//! should stay that way, so they barely move the estimate.

use serde::{Deserialize, Serialize};

use crate::analysis::{centred_mean, integrated_lufs, r2, Analysis};
use crate::kweight::{energy_to_lufs, lufs_to_energy};

const EVAL_STEP: usize = 5; // blocks, 0.5 s
const SHORT_TERM_BLOCKS: usize = 30; // 3 s
const TOP_FRACTION: f64 = 0.3;
const SILENCE_LUFS: f64 = -45.0;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum FadeCurve {
    Linear,
    Smooth,
    Log,
}

impl FadeCurve {
    /// Gain for fade position `x` in 0..=1 (0 = silent, 1 = full level).
    pub fn gain(self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => x,
            FadeCurve::Smooth => 0.5 - 0.5 * (std::f64::consts::PI * x).cos(),
            // Straight line in dB over 60 dB, pinned to exactly 0 at the start.
            FadeCurve::Log => ((10f64.powf(3.0 * (x - 1.0)) - 0.001) / 0.999).max(0.0),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    pub start_s: f64,
    pub end_s: f64,
    /// `None` keeps the set's own typical level.
    pub target_lufs: Option<f64>,
    /// 0..=1, how much of each deviation is corrected.
    pub strength: f64,
    pub window_s: f64,
    pub smooth_s: f64,
    pub max_boost_db: f64,
    pub max_cut_db: f64,
    pub limiter: bool,
    pub ceiling_db: f64,
    pub fade_in_s: f64,
    pub fade_out_s: f64,
    pub fade_curve: FadeCurve,
}

pub struct Plan {
    /// First block of the cut.
    pub b0: usize,
    /// Gain in dB per block from `b0`.
    pub gain_db: Vec<f32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub b0: usize,
    pub gain_db: Vec<f32>,
    /// Local level before and after, per block from `b0`.
    pub level_before: Vec<f32>,
    pub level_after: Vec<f32>,
    /// Estimated limiter gain reduction per block from `b0` (dB, >= 0).
    pub limit_db: Vec<f32>,
    pub reference_lufs: f64,
    pub target_lufs: f64,
    pub lufs_before: f64,
    pub lufs_after: f64,
    /// P90 - P10 of the local level, in LU.
    pub spread_before: f64,
    pub spread_after: f64,
    pub max_boost_used: f64,
    pub max_cut_used: f64,
    pub limiter_share: f64,
    pub limiter_max_db: f64,
}

pub fn block_range(a: &Analysis, p: &Params) -> (usize, usize) {
    let n = a.energy.len();
    let bs = a.block_s();
    let b0 = ((p.start_s.max(0.0) / bs).floor() as usize).min(n.saturating_sub(1));
    let b1 = ((p.end_s / bs).ceil() as usize).clamp(b0 + 1, n);
    (b0, b1)
}

/// Centred moving average with shrinking edges.
fn box_smooth(x: &[f64], half: usize) -> Vec<f64> {
    if half == 0 || x.len() < 2 {
        return x.to_vec();
    }
    let mut prefix = Vec::with_capacity(x.len() + 1);
    prefix.push(0.0);
    for v in x {
        prefix.push(prefix.last().unwrap() + v);
    }
    (0..x.len())
        .map(|i| {
            let a = i.saturating_sub(half);
            let b = (i + half + 1).min(x.len());
            (prefix[b] - prefix[a]) / (b - a) as f64
        })
        .collect()
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let i = pos.floor() as usize;
    let f = pos - i as f64;
    let j = (i + 1).min(sorted.len() - 1);
    sorted[i] * (1.0 - f) + sorted[j] * f
}

fn spread(levels: &[f64]) -> f64 {
    let mut s: Vec<f64> = levels.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    percentile(&s, 0.9) - percentile(&s, 0.1)
}

/// Linear interpolation of values sampled every `EVAL_STEP` blocks back to blocks.
fn upsample(eval: &[f64], len: usize) -> Vec<f64> {
    (0..len)
        .map(|i| {
            let pos = i as f64 / EVAL_STEP as f64;
            let k = pos.floor() as usize;
            if k + 1 >= eval.len() {
                *eval.last().unwrap()
            } else {
                let f = pos - k as f64;
                eval[k] * (1.0 - f) + eval[k + 1] * f
            }
        })
        .collect()
}

/// Returns the plan for rendering and the full report for the UI.
pub fn compute(a: &Analysis, p: &Params) -> (Plan, PlanDto) {
    let (b0, b1) = block_range(a, p);
    let len = b1 - b0;
    let bs = a.block_s();

    let st = centred_mean(&a.energy, SHORT_TERM_BLOCKS, b0, b1);

    // Local level at every evaluation point.
    let half_win = ((p.window_s.max(1.0) / bs) / 2.0).round() as usize;
    let silence = lufs_to_energy(SILENCE_LUFS);
    let n_eval = len.div_ceil(EVAL_STEP);
    let mut scratch: Vec<f64> = Vec::with_capacity(2 * half_win + 1);
    let mut level: Vec<Option<f64>> = (0..n_eval)
        .map(|k| {
            let c = k * EVAL_STEP;
            let lo = c.saturating_sub(half_win);
            let hi = (c + half_win + 1).min(len);
            scratch.clear();
            scratch.extend(st[lo..hi].iter().copied().filter(|&e| e > silence));
            // Mostly silence: nothing to measure, the gain is held instead.
            if scratch.len() * 5 < hi - lo {
                return None;
            }
            let top = ((scratch.len() as f64 * TOP_FRACTION).ceil() as usize).max(1);
            let idx = scratch.len() - top;
            scratch.select_nth_unstable_by(idx, |a, b| a.partial_cmp(b).unwrap());
            let mean = scratch[idx..].iter().sum::<f64>() / top as f64;
            Some(energy_to_lufs(mean))
        })
        .collect();

    // Fill silent gaps by interpolating between the measured neighbours.
    let known: Vec<usize> = (0..n_eval).filter(|&k| level[k].is_some()).collect();
    if known.is_empty() {
        level.iter_mut().for_each(|l| *l = Some(-20.0));
    } else {
        for k in 0..n_eval {
            if level[k].is_some() {
                continue;
            }
            let next = known.partition_point(|&j| j < k);
            let v = match (next.checked_sub(1).map(|i| known[i]), known.get(next)) {
                (Some(a), Some(&b)) => {
                    let (la, lb) = (level[a].unwrap(), level[b].unwrap());
                    la + (lb - la) * (k - a) as f64 / (b - a) as f64
                }
                (Some(a), None) => level[a].unwrap(),
                (None, Some(&b)) => level[b].unwrap(),
                (None, None) => unreachable!(),
            };
            level[k] = Some(v);
        }
    }
    let level: Vec<f64> = level.into_iter().map(Option::unwrap).collect();

    // Reference: the set's typical level, i.e. the median of the measured points.
    let mut measured: Vec<f64> = known.iter().map(|&k| level[k]).collect();
    if measured.is_empty() {
        measured = level.clone();
    }
    measured.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let reference = percentile(&measured, 0.5);
    let target = p.target_lufs.unwrap_or(reference);
    let offset = target - reference;

    let correction: Vec<f64> = level
        .iter()
        .map(|&l| (p.strength.clamp(0.0, 1.0) * (reference - l)).clamp(-p.max_cut_db.max(0.0), p.max_boost_db.max(0.0)))
        .collect();

    // Two passes of a centred box filter ≈ a triangular, zero-phase smoother.
    let half_smooth = ((p.smooth_s.max(0.0) / bs / EVAL_STEP as f64) / 2.0).round() as usize;
    let correction = box_smooth(&box_smooth(&correction, half_smooth), half_smooth);

    let gain_eval: Vec<f64> = correction.iter().map(|c| c + offset).collect();
    let gain = upsample(&gain_eval, len);
    let level_b = upsample(&level, len);

    // What the limiter will have to do, from block sample peaks.
    let mut limit_db = vec![0f32; len];
    let mut limited = 0usize;
    let mut limiter_max: f64 = 0.0;
    for i in 0..len {
        let pk = a.peak[b0 + i].max(1e-9) as f64;
        let over = 20.0 * pk.log10() + gain[i] - p.ceiling_db;
        if over > 0.0 {
            limit_db[i] = over as f32;
            limited += 1;
            limiter_max = limiter_max.max(over);
        }
    }

    let out_energy: Vec<f64> = (0..len)
        .map(|i| a.energy[b0 + i] * 10f64.powf(gain[i] / 10.0))
        .collect();

    let level_after_eval: Vec<f64> = level.iter().zip(&gain_eval).map(|(l, g)| l + g).collect();
    let known_after: Vec<f64> = known.iter().map(|&k| level_after_eval[k]).collect();
    let known_before: Vec<f64> = known.iter().map(|&k| level[k]).collect();

    let dto = PlanDto {
        b0,
        gain_db: gain.iter().map(|&g| r2(g as f32)).collect(),
        level_before: level_b.iter().map(|&l| r2(l as f32)).collect(),
        level_after: level_b.iter().zip(&gain).map(|(l, g)| r2((l + g) as f32)).collect(),
        limit_db: limit_db.iter().map(|&l| r2(l)).collect(),
        reference_lufs: reference,
        target_lufs: target,
        lufs_before: integrated_lufs(&a.energy[b0..b1]),
        lufs_after: integrated_lufs(&out_energy),
        spread_before: spread(&known_before),
        spread_after: spread(&known_after),
        max_boost_used: correction.iter().copied().fold(0.0, f64::max),
        max_cut_used: -correction.iter().copied().fold(0.0, f64::min),
        limiter_share: if p.limiter { limited as f64 / len as f64 } else { 0.0 },
        limiter_max_db: if p.limiter { limiter_max } else { 0.0 },
    };

    (
        Plan {
            b0,
            gain_db: gain.iter().map(|&g| g as f32).collect(),
        },
        dto,
    )
}
