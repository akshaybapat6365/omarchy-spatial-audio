//! Pure-std Radix-2 Cooley-Tukey FFT/IFFT and Kaiser window Bessel utilities.
//! Zero external crate dependencies for deterministic, fast compilation.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, Default)]
pub struct Complex64 {
    pub re: f64,
    pub im: f64,
}

impl Complex64 {
    #[inline]
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    #[inline]
    pub fn from_polar(r: f64, theta: f64) -> Self {
        Self {
            re: r * theta.cos(),
            im: r * theta.sin(),
        }
    }

    #[inline]
    pub fn conj(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }

    #[inline]
    pub fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    #[inline]
    pub fn norm(self) -> f64 {
        self.norm_sqr().sqrt()
    }

    #[inline]
    pub fn add(self, rhs: Self) -> Self {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }

    #[inline]
    pub fn sub(self, rhs: Self) -> Self {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }

    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }

    #[inline]
    pub fn div(self, rhs: Self) -> Self {
        let d = rhs.norm_sqr();
        Self {
            re: (self.re * rhs.re + self.im * rhs.im) / d,
            im: (self.im * rhs.re - self.re * rhs.im) / d,
        }
    }

    #[inline]
    pub fn scale(self, s: f64) -> Self {
        Self {
            re: self.re * s,
            im: self.im * s,
        }
    }
}

/// In-place radix-2 Cooley-Tukey FFT (`inverse = false`) or IFFT (`inverse = true`).
/// Panics if `buf.len()` is not a power of two.
pub fn fft_in_place(buf: &mut [Complex64], inverse: bool) {
    let n = buf.len();
    assert!(n.is_power_of_two(), "FFT length must be a power of two");
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(i, j);
        }
    }

    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2usize;
    while len <= n {
        let half = len >> 1;
        let angle = sign * 2.0 * PI / (len as f64);
        let wlen = Complex64::new(angle.cos(), angle.sin());
        let mut i = 0usize;
        while i < n {
            let mut w = Complex64::new(1.0, 0.0);
            for k in 0..half {
                let u = buf[i + k];
                let v = buf[i + k + half].mul(w);
                buf[i + k] = u.add(v);
                buf[i + k + half] = u.sub(v);
                w = w.mul(wlen);
            }
            i += len;
        }
        len <<= 1;
    }

    if inverse {
        let inv_n = 1.0 / (n as f64);
        for x in buf.iter_mut() {
            *x = x.scale(inv_n);
        }
    }
}

/// Forward real-input FFT returning the `n/2 + 1` non-negative frequency bins.
pub fn rfft(signal: &[f64], n_fft: usize) -> Vec<Complex64> {
    assert!(n_fft.is_power_of_two() && n_fft >= signal.len());
    let mut buf = vec![Complex64::default(); n_fft];
    for (i, &s) in signal.iter().enumerate() {
        buf[i] = Complex64::new(s, 0.0);
    }
    fft_in_place(&mut buf, false);
    buf.truncate(n_fft / 2 + 1);
    buf
}

/// Inverse FFT from `n_fft/2 + 1` conjugate-symmetric bins back to `n_fft` real samples.
pub fn irfft(half_spec: &[Complex64], n_fft: usize) -> Vec<f64> {
    assert!(n_fft.is_power_of_two());
    assert_eq!(half_spec.len(), n_fft / 2 + 1);
    let mut buf = vec![Complex64::default(); n_fft];
    buf[0] = Complex64::new(half_spec[0].re, 0.0);
    buf[n_fft / 2] = Complex64::new(half_spec[n_fft / 2].re, 0.0);
    for k in 1..(n_fft / 2) {
        buf[k] = half_spec[k];
        buf[n_fft - k] = half_spec[k].conj();
    }
    fft_in_place(&mut buf, true);
    buf.into_iter().map(|c| c.re).collect()
}

/// Modified Bessel function of the first kind, order 0 (`I_0(x)`), accurate to <1e-14.
pub fn bessel_i0(x: f64) -> f64 {
    let ax = x.abs();
    let half_x_sq = 0.25 * ax * ax;
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..=50 {
        let kf = k as f64;
        term *= half_x_sq / (kf * kf);
        sum += term;
        if term < 1e-16 * sum {
            break;
        }
    }
    sum
}

/// Symmetric Kaiser window of length `n` centered cleanly at `center_tap` (e.g. `n / 2`).
pub fn kaiser_window_centered(n: usize, center_tap: usize, beta: f64) -> Vec<f64> {
    let denom = bessel_i0(beta);
    let half = center_tap as f64;
    (0..n)
        .map(|i| {
            let x = (i as f64 - half) / half;
            let arg = (1.0 - x * x).max(0.0).sqrt();
            bessel_i0(beta * arg) / denom
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_roundtrip_and_parseval() {
        let n = 512;
        let x: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * 17.0 * i as f64 / n as f64).sin() + 0.5 * (2.0 * PI * 63.0 * i as f64 / n as f64).cos())
            .collect();
        let spec = rfft(&x, n);
        let y = irfft(&spec, n);
        let max_err = x
            .iter()
            .zip(y.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f64, f64::max);
        assert!(max_err < 1e-12, "roundtrip max_err={max_err}");

        let time_energy: f64 = x.iter().map(|v| v * v).sum();
        let mut freq_energy = spec[0].norm_sqr() + spec[n / 2].norm_sqr();
        for k in 1..(n / 2) {
            freq_energy += 2.0 * spec[k].norm_sqr();
        }
        freq_energy /= n as f64;
        assert!((time_energy - freq_energy).abs() < 1e-10);
    }
}
