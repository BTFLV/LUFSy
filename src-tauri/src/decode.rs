//! Streaming decoder: any format symphonia knows, delivered as interleaved stereo f32.
//!
//! Sets can be hours long, so nothing here holds the whole file in memory.
//! Mono sources are duplicated to both sides, multichannel sources keep L/R.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct StereoDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    sample_buf: Option<SampleBuffer<f32>>,
    pub sample_rate: u32,
    pub source_channels: usize,
    /// Frame count from the container, if it knows it (used for progress only).
    pub total_frames: Option<u64>,
    pub codec: String,
}

impl StereoDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Datei lässt sich nicht öffnen: {e}"))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("Format nicht erkannt: {e}"))?;
        let format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("Keine Audiospur gefunden")?;
        let params = track.codec_params.clone();
        let track_id = track.id;

        let sample_rate = params.sample_rate.ok_or("Samplerate unbekannt")?;
        let source_channels = params.channels.map(|c| c.count()).unwrap_or(2);
        let codec = symphonia::default::get_codecs()
            .get_codec(params.codec)
            .map(|d| d.short_name.to_uppercase())
            .unwrap_or_else(|| "?".into());

        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| format!("Codec nicht unterstützt: {e}"))?;

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_buf: None,
            sample_rate,
            source_channels,
            total_frames: params.n_frames,
            codec,
        })
    }

    /// Clears `out` and fills it with the next chunk of interleaved stereo samples.
    /// Returns `Ok(false)` at end of stream.
    pub fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        out.clear();
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(format!("Lesefehler: {e}")),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // A corrupt frame in a long recording should not kill the whole job.
                Err(SymError::DecodeError(_)) => continue,
                Err(SymError::IoError(_)) => continue,
                Err(e) => return Err(format!("Dekodierfehler: {e}")),
            };

            let spec = *decoded.spec();
            let cap = decoded.capacity() as u64;
            let needs_new = self.sample_buf.as_ref().is_none_or(|b| b.capacity() < cap as usize);
            if needs_new {
                self.sample_buf = Some(SampleBuffer::new(cap, spec));
            }
            let buf = self.sample_buf.as_mut().unwrap();
            buf.copy_interleaved_ref(decoded);
            let samples = buf.samples();
            let ch = spec.channels.count().max(1);
            let frames = samples.len() / ch;
            if frames == 0 {
                continue;
            }

            out.reserve(frames * 2);
            match ch {
                1 => {
                    for &s in samples {
                        out.push(s);
                        out.push(s);
                    }
                }
                2 => out.extend_from_slice(samples),
                _ => {
                    for f in samples.chunks_exact(ch) {
                        out.push(f[0]);
                        out.push(f[1]);
                    }
                }
            }
            return Ok(true);
        }
    }
}
