//! Stereo-linked lookahead peak limiter.
//!
//! For every sample we know the gain `r` that would keep it under the ceiling.
//! Taking the minimum of `r` over ±L samples and then averaging that over ±L
//! gives a smooth envelope that is guaranteed to be at or below `r` at every
//! sample: each average only contains minima that already include it. A slow
//! release on top keeps the limiter from pumping between kicks.
//! Total delay is 2L samples; `flush` drains it.

use std::collections::VecDeque;

pub struct Limiter {
    ceiling: f32,
    half: usize,
    n: u64,
    mins: VecDeque<(u64, f32)>,
    avg_hist: VecDeque<f32>,
    avg_sum: f64,
    delay: VecDeque<[f32; 2]>,
    env: f32,
    release: f32,
    pub max_reduction: f32,
}

impl Limiter {
    pub fn new(sample_rate: u32, ceiling_db: f64, lookahead_ms: f64, release_ms: f64) -> Self {
        let half = ((sample_rate as f64 * lookahead_ms / 1000.0) / 2.0).round().max(1.0) as usize;
        let width = 2 * half + 1;
        Self {
            ceiling: 10f64.powf(ceiling_db / 20.0) as f32,
            half,
            n: 0,
            mins: VecDeque::with_capacity(width + 1),
            avg_hist: std::iter::repeat_n(1.0, width).collect(),
            avg_sum: width as f64,
            delay: VecDeque::with_capacity(2 * half + 1),
            env: 1.0,
            release: (-1.0 / (sample_rate as f64 * release_ms / 1000.0)).exp() as f32,
            max_reduction: 1.0,
        }
    }

    pub fn latency(&self) -> usize {
        2 * self.half
    }

    #[inline]
    pub fn process(&mut self, frame: [f32; 2]) -> Option<[f32; 2]> {
        let width = 2 * self.half + 1;
        let pk = frame[0].abs().max(frame[1].abs());
        let r = if pk > self.ceiling { self.ceiling / pk } else { 1.0 };

        // Sliding minimum over the last `width` requirements (monotonic deque).
        while matches!(self.mins.back(), Some(&(_, v)) if v >= r) {
            self.mins.pop_back();
        }
        self.mins.push_back((self.n, r));
        while self.mins.front().is_some_and(|&(i, _)| i + width as u64 <= self.n) {
            self.mins.pop_front();
        }
        let m = self.mins.front().unwrap().1;
        self.n += 1;

        // Moving average of the minima.
        self.avg_hist.push_back(m);
        self.avg_sum += m as f64 - self.avg_hist.pop_front().unwrap() as f64;
        let s = (self.avg_sum / width as f64) as f32;

        self.env = if s < self.env {
            s
        } else {
            s + (self.env - s) * self.release
        };
        self.max_reduction = self.max_reduction.min(self.env);

        self.delay.push_back(frame);
        if self.delay.len() <= 2 * self.half {
            return None;
        }
        let x = self.delay.pop_front().unwrap();
        // Rounding in the running sum can leave a hair above the ceiling.
        let c = self.ceiling;
        Some([(x[0] * self.env).clamp(-c, c), (x[1] * self.env).clamp(-c, c)])
    }

    pub fn flush(&mut self, mut out: impl FnMut([f32; 2])) {
        for _ in 0..2 * self.half {
            if let Some(f) = self.process([0.0, 0.0]) {
                out(f);
            }
        }
    }
}
