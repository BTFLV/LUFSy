//! Pass 2: decode again, apply cut, gain plan, limiter and fades, encode, and
//! measure the result with an independent EBU R128 meter.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

use crate::analysis::Analysis;
use crate::decode::StereoDecoder;
use crate::limiter::Limiter;
use crate::plan::{Params, Plan};

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Wav16,
    Wav24,
    Wav32f,
    Mp3,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub path: String,
    pub duration_s: f64,
    pub integrated_lufs: f64,
    pub true_peak_db: f64,
    pub loudness_range: f64,
    pub limiter_max_db: f64,
}

enum Sink {
    Wav(hound::WavWriter<BufWriter<File>>, Format, Dither),
    Mp3(mp3lame_encoder::Encoder, BufWriter<File>, Vec<u8>),
}

/// TPDF dither for the 16-bit export.
struct Dither(u64);

impl Dither {
    #[inline]
    fn uniform(&mut self) -> f32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32) / (1u64 << 24) as f32
    }
    #[inline]
    fn tpdf(&mut self) -> f32 {
        self.uniform() - self.uniform()
    }
}

impl Sink {
    fn create(path: &Path, format: Format, rate: u32) -> Result<Self, String> {
        let io = |e: std::io::Error| format!("Schreiben fehlgeschlagen: {e}");
        match format {
            Format::Mp3 => {
                if rate > 48_000 {
                    return Err(format!(
                        "MP3 geht nur bis 48 kHz, das Set hat {} kHz. Exportiere als WAV.",
                        rate as f64 / 1000.0
                    ));
                }
                use mp3lame_encoder::{Bitrate, Builder, Quality};
                let mut b = Builder::new().ok_or("MP3-Encoder nicht verfügbar")?;
                let err = |e: mp3lame_encoder::BuildError| format!("MP3-Encoder: {e:?}");
                b.set_num_channels(2).map_err(err)?;
                b.set_sample_rate(rate).map_err(err)?;
                b.set_brate(Bitrate::Kbps320).map_err(err)?;
                // LAME -q 2: its recommended setting; -q 0 is several times slower for no audible gain.
                b.set_quality(Quality::NearBest).map_err(err)?;
                let enc = b.build().map_err(err)?;
                let file = File::create(path).map_err(io)?;
                Ok(Sink::Mp3(enc, BufWriter::new(file), Vec::new()))
            }
            _ => {
                let (bits, fmt) = match format {
                    Format::Wav16 => (16, hound::SampleFormat::Int),
                    Format::Wav24 => (24, hound::SampleFormat::Int),
                    _ => (32, hound::SampleFormat::Float),
                };
                let spec = hound::WavSpec {
                    channels: 2,
                    sample_rate: rate,
                    bits_per_sample: bits,
                    sample_format: fmt,
                };
                let w = hound::WavWriter::create(path, spec).map_err(|e| format!("Schreiben fehlgeschlagen: {e}"))?;
                Ok(Sink::Wav(w, format, Dither(0x9E3779B97F4A7C15)))
            }
        }
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        let werr = |e: hound::Error| format!("Schreiben fehlgeschlagen: {e}");
        match self {
            Sink::Wav(w, Format::Wav16, d) => {
                for &s in samples {
                    let v = (s * 32767.0 + d.tpdf()).round().clamp(-32768.0, 32767.0) as i16;
                    w.write_sample(v).map_err(werr)?;
                }
            }
            Sink::Wav(w, Format::Wav24, _) => {
                for &s in samples {
                    let v = (s as f64 * 8_388_607.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                    w.write_sample(v).map_err(werr)?;
                }
            }
            Sink::Wav(w, _, _) => {
                for &s in samples {
                    w.write_sample(s).map_err(werr)?;
                }
            }
            Sink::Mp3(enc, file, buf) => {
                buf.clear();
                buf.reserve(samples.len() * 5 / 8 + 7200);
                enc.encode_to_vec(mp3lame_encoder::InterleavedPcm(samples), buf)
                    .map_err(|e| format!("MP3-Encoder: {e:?}"))?;
                file.write_all(buf)
                    .map_err(|e| format!("Schreiben fehlgeschlagen: {e}"))?;
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<(), String> {
        match self {
            Sink::Wav(w, _, _) => w.finalize().map_err(|e| format!("Schreiben fehlgeschlagen: {e}")),
            Sink::Mp3(mut enc, mut file, mut buf) => {
                buf.clear();
                buf.reserve(7200);
                enc.flush_to_vec::<mp3lame_encoder::FlushNoGap>(&mut buf)
                    .map_err(|e| format!("MP3-Encoder: {e:?}"))?;
                file.write_all(&buf).map_err(|e| e.to_string())?;
                file.flush().map_err(|e| e.to_string())
            }
        }
    }
}

pub fn render(
    a: &Analysis,
    plan: &Plan,
    p: &Params,
    format: Format,
    out_path: &Path,
    mut progress: impl FnMut(f64),
    cancelled: impl Fn() -> bool,
) -> Result<Report, String> {
    let mut dec = StereoDecoder::open(Path::new(&a.path))?;
    let rate = a.sample_rate;
    let sr = rate as f64;
    let start = (p.start_s.max(0.0) * sr).round() as u64;
    let end = ((p.end_s * sr).round() as u64).min(a.frames).max(start + 1);
    let out_len = end - start;

    let mut sink = Sink::create(out_path, format, rate)?;
    let mut meter =
        EbuR128::new(2, rate, Mode::I | Mode::LRA | Mode::TRUE_PEAK).map_err(|e| format!("Messung: {e}"))?;
    let mut limiter = p.limiter.then(|| Limiter::new(rate, p.ceiling_db, 2.0, 80.0));

    // Linear gain at each block centre; interpolated per sample in between.
    let gains: Vec<f32> = plan.gain_db.iter().map(|g| 10f32.powf(g / 20.0)).collect();
    let bf = a.block_frames as f64;
    let gain_at = |frame: u64| -> f32 {
        let pos = (frame as f64 + 0.5) / bf - 0.5 - plan.b0 as f64;
        if pos <= 0.0 {
            return gains[0];
        }
        let k = pos.floor() as usize;
        if k + 1 >= gains.len() {
            return *gains.last().unwrap();
        }
        let f = (pos - k as f64) as f32;
        gains[k] + (gains[k + 1] - gains[k]) * f
    };

    let fade_in = (p.fade_in_s.max(0.0) * sr) as u64;
    let fade_out = (p.fade_out_s.max(0.0) * sr) as u64;
    let curve = p.fade_curve;
    let fade_at = |i: u64| -> f32 {
        let mut g = 1.0f64;
        if fade_in > 0 && i < fade_in {
            g *= curve.gain(i as f64 / fade_in as f64);
        }
        let remaining = out_len - 1 - i.min(out_len - 1);
        if fade_out > 0 && remaining < fade_out {
            g *= curve.gain(remaining as f64 / fade_out as f64);
        }
        g as f32
    };

    let mut buf = Vec::new();
    let mut out: Vec<f32> = Vec::with_capacity(1 << 16);
    let mut frame: u64 = 0;
    let mut written: u64 = 0;
    let mut last_report = 0u64;

    let emit = |x: [f32; 2], out: &mut Vec<f32>, written: &mut u64| {
        if *written >= out_len {
            return;
        }
        let g = fade_at(*written);
        out.push(x[0] * g);
        out.push(x[1] * g);
        *written += 1;
    };

    'decode: while dec.next_chunk(&mut buf)? {
        if cancelled() {
            drop(sink);
            let _ = std::fs::remove_file(out_path);
            return Err("Abgebrochen".into());
        }
        out.clear();
        for f in buf.as_chunks::<2>().0 {
            if frame >= end {
                break 'decode;
            }
            if frame >= start {
                let g = gain_at(frame);
                let x = [f[0] * g, f[1] * g];
                match limiter.as_mut() {
                    Some(l) => {
                        if let Some(y) = l.process(x) {
                            emit(y, &mut out, &mut written);
                        }
                    }
                    None => emit(x, &mut out, &mut written),
                }
            }
            frame += 1;
        }
        if !out.is_empty() {
            meter.add_frames_f32(&out).map_err(|e| format!("Messung: {e}"))?;
            sink.write(&out)?;
        }
        if frame - last_report > rate as u64 * 20 {
            last_report = frame;
            progress(((frame.saturating_sub(start)) as f64 / out_len as f64).clamp(0.0, 1.0));
        }
    }

    out.clear();
    if let Some(l) = limiter.as_mut() {
        l.flush(|y| emit(y, &mut out, &mut written));
    }
    if !out.is_empty() {
        meter.add_frames_f32(&out).map_err(|e| format!("Messung: {e}"))?;
        sink.write(&out)?;
    }
    sink.finish()?;
    progress(1.0);

    let tp = (0..2).map(|c| meter.true_peak(c).unwrap_or(0.0)).fold(0.0f64, f64::max);
    Ok(Report {
        path: out_path.to_string_lossy().into_owned(),
        duration_s: written as f64 / sr,
        integrated_lufs: meter.loudness_global().unwrap_or(f64::NEG_INFINITY),
        true_peak_db: if tp > 0.0 { 20.0 * tp.log10() } else { -180.0 },
        loudness_range: meter.loudness_range().unwrap_or(0.0),
        limiter_max_db: limiter.map_or(0.0, |l| -20.0 * (l.max_reduction.max(1e-9) as f64).log10()),
    })
}
