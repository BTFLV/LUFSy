//! Pass 1: decode the whole recording once and reduce it to 100 ms blocks.
//!
//! Everything the planner and the waveform view need is derived from these
//! blocks, so moving a slider never touches the audio file again.

use std::path::Path;

use serde::Serialize;

use crate::decode::StereoDecoder;
use crate::kweight::{energy_to_lufs, KWeighting};

pub const BLOCK_SECONDS: f64 = 0.1;

pub struct Analysis {
    pub path: String,
    pub sample_rate: u32,
    pub source_channels: usize,
    pub codec: String,
    pub frames: u64,
    pub block_frames: usize,
    /// K-weighted mean-square energy per block, summed over L and R.
    pub energy: Vec<f64>,
    /// Sample peak per block (linear, max of both channels).
    pub peak: Vec<f32>,
    /// Unweighted RMS per block (linear), for drawing.
    pub rms: Vec<f32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDto {
    pub path: String,
    pub file_name: String,
    pub sample_rate: u32,
    pub channels: usize,
    pub codec: String,
    pub frames: u64,
    pub duration_s: f64,
    pub block_s: f64,
    pub peak_db: Vec<f32>,
    pub rms_db: Vec<f32>,
    /// Short-term (3 s) loudness centred on each block.
    pub short_term_lufs: Vec<f32>,
    pub integrated_lufs: f64,
    pub max_peak_db: f64,
    /// Suggested cut: first and last second with programme above -50 LUFS.
    pub content_start_s: f64,
    pub content_end_s: f64,
}

pub fn analyze(path: &Path, mut progress: impl FnMut(f64), cancelled: impl Fn() -> bool) -> Result<Analysis, String> {
    let mut dec = StereoDecoder::open(path)?;
    let rate = dec.sample_rate;
    let block_frames = ((rate as f64) * BLOCK_SECONDS).round() as usize;
    let total = dec.total_frames;

    let mut kl = KWeighting::new(rate);
    let mut kr = KWeighting::new(rate);

    let mut energy = Vec::new();
    let mut peak = Vec::new();
    let mut rms = Vec::new();

    let (mut acc_k, mut acc_raw, mut acc_peak, mut in_block) = (0.0f64, 0.0f64, 0.0f32, 0usize);
    let mut frames: u64 = 0;
    let mut buf = Vec::new();
    let mut last_report = 0u64;

    while dec.next_chunk(&mut buf)? {
        if cancelled() {
            return Err("Abgebrochen".into());
        }
        for f in buf.as_chunks::<2>().0 {
            let (l, r) = (f[0], f[1]);
            let yl = kl.tick(l);
            let yr = kr.tick(r);
            acc_k += yl * yl + yr * yr;
            acc_raw += (l as f64) * (l as f64) + (r as f64) * (r as f64);
            acc_peak = acc_peak.max(l.abs()).max(r.abs());
            in_block += 1;
            if in_block == block_frames {
                energy.push(acc_k / block_frames as f64);
                rms.push((acc_raw / (2 * block_frames) as f64).sqrt() as f32);
                peak.push(acc_peak);
                acc_k = 0.0;
                acc_raw = 0.0;
                acc_peak = 0.0;
                in_block = 0;
            }
        }
        frames += (buf.len() / 2) as u64;
        if let Some(t) = total {
            if frames - last_report > rate as u64 * 20 {
                last_report = frames;
                progress((frames as f64 / t as f64).min(1.0));
            }
        }
    }
    if in_block > 0 {
        energy.push(acc_k / in_block as f64);
        rms.push((acc_raw / (2 * in_block) as f64).sqrt() as f32);
        peak.push(acc_peak);
    }
    if frames == 0 {
        return Err("Die Datei enthält kein Audio".into());
    }
    progress(1.0);

    Ok(Analysis {
        path: path.to_string_lossy().into_owned(),
        sample_rate: rate,
        source_channels: dec.source_channels,
        codec: dec.codec.clone(),
        frames,
        block_frames,
        energy,
        peak,
        rms,
    })
}

/// Centred moving mean of block energies over `width` blocks, clipped to `[lo, hi)`.
pub fn centred_mean(energy: &[f64], width: usize, lo: usize, hi: usize) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(hi - lo + 1);
    prefix.push(0.0);
    for e in &energy[lo..hi] {
        prefix.push(prefix.last().unwrap() + e);
    }
    let half = width / 2;
    (0..hi - lo)
        .map(|i| {
            let a = i.saturating_sub(half);
            let b = (i + half + 1).min(hi - lo);
            (prefix[b] - prefix[a]) / (b - a) as f64
        })
        .collect()
}

/// BS.1770-4 integrated loudness from 100 ms block energies:
/// 400 ms gating blocks with 75 % overlap, -70 LUFS absolute and -10 LU relative gate.
pub fn integrated_lufs(energy: &[f64]) -> f64 {
    if energy.len() < 4 {
        return energy_to_lufs(energy.iter().sum::<f64>() / energy.len().max(1) as f64);
    }
    let gating: Vec<f64> = energy.windows(4).map(|w| w.iter().sum::<f64>() / 4.0).collect();
    let abs_gate = crate::kweight::lufs_to_energy(-70.0);
    let above: Vec<f64> = gating.iter().copied().filter(|&e| e > abs_gate).collect();
    if above.is_empty() {
        return -200.0;
    }
    let ungated = above.iter().sum::<f64>() / above.len() as f64;
    let rel_gate = ungated * 10f64.powf(-10.0 / 10.0);
    let gated: Vec<f64> = above.into_iter().filter(|&e| e > rel_gate).collect();
    energy_to_lufs(gated.iter().sum::<f64>() / gated.len().max(1) as f64)
}

/// Two decimals are plenty for drawing and keep the JSON for long sets small.
pub fn r2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

fn to_db(x: f32) -> f32 {
    if x <= 1e-9 {
        -180.0
    } else {
        20.0 * x.log10()
    }
}

impl Analysis {
    pub fn block_s(&self) -> f64 {
        self.block_frames as f64 / self.sample_rate as f64
    }

    pub fn duration_s(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    pub fn to_dto(&self) -> AnalysisDto {
        let n = self.energy.len();
        let st = centred_mean(&self.energy, 30, 0, n);
        let st_lufs: Vec<f32> = st.iter().map(|&e| r2(energy_to_lufs(e).max(-90.0) as f32)).collect();

        let bs = self.block_s();
        let gate = crate::kweight::lufs_to_energy(-50.0);
        let first = st.iter().position(|&e| e > gate);
        let last = st.iter().rposition(|&e| e > gate);
        let (content_start_s, content_end_s) = match (first, last) {
            // Short-term values smear 1.5 s around the onset; step back so nothing is clipped.
            (Some(a), Some(b)) => (
                ((a as f64) * bs + 1.0).max(0.0).min(self.duration_s()),
                ((b as f64 + 1.0) * bs - 1.0).max(0.0).min(self.duration_s()),
            ),
            _ => (0.0, self.duration_s()),
        };

        AnalysisDto {
            path: self.path.clone(),
            file_name: Path::new(&self.path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            sample_rate: self.sample_rate,
            channels: self.source_channels,
            codec: self.codec.clone(),
            frames: self.frames,
            duration_s: self.duration_s(),
            block_s: bs,
            peak_db: self.peak.iter().map(|&p| r2(to_db(p).max(-90.0))).collect(),
            rms_db: self.rms.iter().map(|&p| r2(to_db(p).max(-90.0))).collect(),
            short_term_lufs: st_lufs,
            integrated_lufs: integrated_lufs(&self.energy),
            max_peak_db: to_db(self.peak.iter().copied().fold(0.0, f32::max)) as f64,
            content_start_s,
            content_end_s,
        }
    }
}
