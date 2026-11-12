# Changelog

## 0.1.1 — 2026-11-12

### Project
- macOS builds are signed with a Developer ID and notarized by Apple, so they
  open without Gatekeeper warnings. The disk image is notarized as well.
- macOS builds target Apple Silicon (arm64) only.

## 0.1.0 — 2026-10-27

First public release.

### Levelling
- Offline loudness levelling for recorded DJ sets: local level from the loudest
  30 % of 3 s short-term loudness in a centred window, corrected towards the
  set's median level and smoothed with a zero-phase filter.
- Keep the set's own level or move it to a fixed target between −20 and
  −5 LUFS. Strength, window, smoothing and maximum raise/cut are adjustable.
- Mostly silent stretches hold their gain instead of being raised.
- Stereo-linked lookahead limiter (2 ms, 80 ms release) with an adjustable
  ceiling that the output never exceeds.

### Editing
- Cut with draggable IN/OUT handles, typed times or the playhead (`I`/`O`);
  one click trims leading and trailing silence.
- Fade-in and fade-out with linear, smooth (S) or logarithmic curves.
- Timeline with before/after waveform, local loudness against the target, gain
  curve and the limiter's expected work; zoom and pan.
- Preview playback with instant switching between Original and LUFSy.

### Formats
- Reads WAV, AIFF, FLAC, MP3, M4A/AAC and Ogg Vorbis.
- Writes WAV 16-bit (TPDF dither), 24-bit, 32-bit float and MP3 320 kbit/s.
- Every export is measured with an EBU R128 meter: integrated loudness, true
  peak and loudness range.

### Project
- macOS and Windows (x64) builds from GitHub Actions, not yet
  code-signed.
- MIT license.
