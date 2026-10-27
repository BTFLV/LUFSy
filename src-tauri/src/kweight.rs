//! ITU-R BS.1770 K-weighting (high shelf + RLB high-pass) for any sample rate.
//! Coefficients follow the bilinear-transform design used by libebur128.

use std::f64::consts::PI;

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 3],
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline(always)]
    fn tick(&mut self, x: f64) -> f64 {
        // Transposed direct form II.
        let y = self.b[0] * x + self.z1;
        self.z1 = self.b[1] * x - self.a[1] * y + self.z2;
        self.z2 = self.b[2] * x - self.a[2] * y;
        y
    }
}

#[derive(Clone, Copy)]
pub struct KWeighting {
    shelf: Biquad,
    highpass: Biquad,
}

impl KWeighting {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f64;

        let f0 = 1681.974450955533;
        let g = 3.999843853973347;
        let q = 0.7071752369554196;
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad {
            b: [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z1: 0.0,
            z2: 0.0,
        };

        let f0 = 38.13547087602444;
        let q = 0.5003270373238773;
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        let highpass = Biquad {
            b: [1.0, -2.0, 1.0],
            a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z1: 0.0,
            z2: 0.0,
        };

        Self { shelf, highpass }
    }

    #[inline(always)]
    pub fn tick(&mut self, x: f32) -> f64 {
        self.highpass.tick(self.shelf.tick(x as f64))
    }
}

/// Mean-square energy (summed over channels) → LUFS.
#[inline]
pub fn energy_to_lufs(e: f64) -> f64 {
    if e <= 1e-20 {
        -200.0
    } else {
        -0.691 + 10.0 * e.log10()
    }
}

#[inline]
pub fn lufs_to_energy(l: f64) -> f64 {
    10f64.powf((l + 0.691) / 10.0)
}
