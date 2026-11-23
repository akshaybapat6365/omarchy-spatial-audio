//! Hard real-time, lock-free, zero-allocation stereo DSP chain for Dell XPS 17 9720:
//!   - ISO 226:2023 equal-loudness warmth contour
//!   - Dual-bank Chebyshev missing-fundamental bass harmonics (with speech gate)
//!   - 2x2 Choueiri FDTR-XTC 512-tap FIR matrix (5.333 ms causal delay)
//!   - Subtle analog even-harmonic warmth with DC blocker
//!   - 10 ms lookahead 4x Catmull-Rom true-peak limiter (+7 dB LOUDNESS_DRIVE_DB capable)
//!
//! Zero heap allocations occur inside `process_sample` or `process_block`.

use std::f64::consts::PI;

use crate::fdtr::{generate_fdtr_kernels, CAUSAL_DELAY_SAMPLES, KERNEL_TAPS, Xps17Geometry};

const LIMITER_RING_CAP: usize = 512; // power-of-two >= 480 samples (10 ms @ 48 kHz)
const LIMITER_RING_MASK: usize = LIMITER_RING_CAP - 1;
const FIR_RING_MASK: usize = KERNEL_TAPS - 1; // KERNEL_TAPS = 512 (power of two)
const DENORMAL_FLOOR: f32 = 1.0e-24;

#[inline]
fn sanitize(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x
    }
}

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10.0_f32.powf(db * 0.05)
}

#[inline]
pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1.0e-12).log10()
}

/// Direct Form II Transposed biquad filter (zero-alloc, f64 state for low-freq precision).
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub const fn bypass() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    pub fn highpass(fs: f64, f0: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * (f0.clamp(10.0, 0.45 * fs)) / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos_w0) * 0.5 / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: (1.0 + cos_w0) * 0.5 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn lowpass(fs: f64, f0: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * (f0.clamp(10.0, 0.45 * fs)) / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cos_w0) * 0.5 / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: (1.0 - cos_w0) * 0.5 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn low_shelf(fs: f64, f0: f64, gain_db: f64, slope: f64) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::bypass();
        }
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (f0.clamp(20.0, 0.45 * fs)) / fs;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 * 0.5 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).max(1e-6).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        Self {
            b0: a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha) / a0,
            b1: 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0) / a0,
            b2: a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha) / a0,
            a1: -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0) / a0,
            a2: ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn high_shelf(fs: f64, f0: f64, gain_db: f64, slope: f64) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::bypass();
        }
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (f0.clamp(20.0, 0.45 * fs)) / fs;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 * 0.5 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).max(1e-6).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        Self {
            b0: a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha) / a0,
            b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0) / a0,
            b2: a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha) / a0,
            a1: 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0) / a0,
            a2: ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        let z1_next = self.b1 * x - self.a1 * y + self.z2;
        let z2_next = self.b2 * x - self.a2 * y;
        self.z1 = if z1_next.abs() < 1e-30 { 0.0 } else { z1_next };
        self.z2 = if z2_next.abs() < 1e-30 { 0.0 } else { z2_next };
        y
    }
}

/// Real-time runtime control parameters (all bounded, safe to update between blocks).
#[derive(Clone, Copy, Debug)]
pub struct SpatialParams {
    /// 0.0 = bypass FDTR-XTC (0 extra delay), 1.0 = full 512-tap 2x2 Choueiri FDTR-XTC.
    pub xtc_mix: f32,
    /// 0.0 = pure bass-harmonics output mode, 1.0 = full stereo pass + enhancements.
    pub dry_mix: f32,
    /// 0.0 .. 1.0 dual-bank Chebyshev missing-fundamental bass warmth amount.
    pub bass_harmonics: f32,
    /// 0.0 .. 1.0 subtle even-harmonic analog warmth.
    pub harmonic_warmth: f32,
    /// 0.0 .. 1.0 ISO 226:2023 equal-loudness warm body contour depth.
    pub loudness_contour: f32,
    /// 0.0 .. 12.0 dB loudness drive into lookahead limiter (7.0 dB matches LOUDNESS_DRIVE_DB).
    pub drive_db: f32,
    /// -9.0 .. -0.5 dBFS brickwall true-peak ceiling.
    pub ceiling_dbfs: f32,
    /// Toggle lookahead true-peak limiter (0 = bypass/0 delay, 1 = active/10 ms lookahead).
    pub enable_limiter: bool,
    /// Optional 180 Hz driver-protection high-pass on the main stereo path (true for standalone offline/7.1).
    pub enable_driver_hp: bool,
}

impl Default for SpatialParams {
    fn default() -> Self {
        Self {
            xtc_mix: 1.0,
            dry_mix: 1.0,
            bass_harmonics: 0.38,
            harmonic_warmth: 0.16,
            loudness_contour: 0.25,
            drive_db: 7.0,
            ceiling_dbfs: -1.0,
            enable_limiter: true,
            enable_driver_hp: true,
        }
    }
}

/// Hard real-time, zero-allocation stereo spatial DSP engine.
pub struct SpatialDspEngine {
    sample_rate: f64,

    // Precomputed 512-tap Choueiri FDTR-XTC kernels (symmetric: h_rr = h_ll, h_rl = h_lr)
    h_ll: [f32; KERNEL_TAPS],
    h_lr: [f32; KERNEL_TAPS],
    fir_buf_l: [f32; KERNEL_TAPS],
    fir_buf_r: [f32; KERNEL_TAPS],
    fir_pos: usize,

    // 180 Hz 4th-order BWC driver protection HP (cascaded 2x biquad per channel)
    hp_l0: Biquad,
    hp_l1: Biquad,
    hp_r0: Biquad,
    hp_r1: Biquad,

    // ISO 226:2023 loudness warmth contour (parallel unity + shelf delta)
    iso_lo_l: Biquad,
    iso_hi_l: Biquad,
    iso_lo_r: Biquad,
    iso_hi_r: Biquad,

    // Dual-bank Chebyshev missing-fundamental bass generator
    // Bank A: sustained deep bass (40 - 95 Hz)
    bank_a_hp: Biquad,
    bank_a_lp0: Biquad,
    bank_a_lp1: Biquad,
    bank_a_env: f64,
    // Bank B: punch / transient bass (80 - 155 Hz)
    bank_b_hp: Biquad,
    bank_b_lp: Biquad,
    bank_b_env: f64,
    // Speech detector bandpass (400 - 2800 Hz)
    speech_hp: Biquad,
    speech_lp: Biquad,
    speech_env: f64,
    bass_total_env: f64,
    speech_gate: f64,
    // Harmonics output band-shaping (125 - 380 Hz, zero sub-115 Hz leakage)
    harm_hp0: Biquad,
    harm_hp1: Biquad,
    harm_lp0: Biquad,
    harm_lp1: Biquad,

    // Subtle even-harmonic warmth DC blockers
    warm_dc_l: Biquad,
    warm_dc_r: Biquad,

    // Lookahead true-peak limiter (10 ms @ sample_rate, O(1) amortized monotonic max deque)
    lim_delay: usize,
    lim_buf_l: [f32; LIMITER_RING_CAP],
    lim_buf_r: [f32; LIMITER_RING_CAP],
    lim_deque_sample: [u64; LIMITER_RING_CAP],
    lim_deque_val: [f32; LIMITER_RING_CAP],
    lim_deque_head: usize,
    lim_deque_tail: usize,
    lim_sample_counter: u64,
    lim_pos: usize,
    lim_prev_l: [f32; 3],
    lim_prev_r: [f32; 3],
    lim_gain: f32,
    lim_attack_coeff: f32,
    lim_release_coeff: f32,
}

impl SpatialDspEngine {
    /// Allocate and initialize the complete DSP engine once at startup (`instantiate`).
    pub fn new(sample_rate: f64) -> Self {
        let fs = sample_rate.clamp(22_050.0, 192_000.0);
        let geom = Xps17Geometry {
            sample_rate: fs,
            ..Xps17Geometry::default()
        };
        let kernels = generate_fdtr_kernels(&geom);
        let mut h_ll = [0.0_f32; KERNEL_TAPS];
        let mut h_lr = [0.0_f32; KERNEL_TAPS];
        h_ll.copy_from_slice(&kernels.h_ll);
        h_lr.copy_from_slice(&kernels.h_lr);

        // 4th-order Butterworth Q values: 0.54119610, 1.30656296
        let q1 = 0.54119610;
        let q2 = 1.30656296;

        let lim_delay = ((0.010 * fs).round() as usize).clamp(32, LIMITER_RING_CAP - 1);
        let lim_attack_coeff = (-1.0 / (0.0025 * fs)).exp() as f32;
        let lim_release_coeff = (-1.0 / (0.040 * fs)).exp() as f32;

        Self {
            sample_rate: fs,
            h_ll,
            h_lr,
            fir_buf_l: [0.0; KERNEL_TAPS],
            fir_buf_r: [0.0; KERNEL_TAPS],
            fir_pos: 0,

            hp_l0: Biquad::highpass(fs, 180.0, q1),
            hp_l1: Biquad::highpass(fs, 180.0, q2),
            hp_r0: Biquad::highpass(fs, 180.0, q1),
            hp_r1: Biquad::highpass(fs, 180.0, q2),

            // Fixed full-depth ISO 226:2023 delta curve (+5.0 dB @ 250 Hz low-warmth, +1.8 dB @ 7.5 kHz air)
            // Blended smoothly by `loudness_contour` without re-computing biquads in RT thread!
            iso_lo_l: Biquad::low_shelf(fs, 260.0, 5.0, 0.72),
            iso_hi_l: Biquad::high_shelf(fs, 7500.0, 1.8, 0.75),
            iso_lo_r: Biquad::low_shelf(fs, 260.0, 5.0, 0.72),
            iso_hi_r: Biquad::high_shelf(fs, 7500.0, 1.8, 0.75),

            bank_a_hp: Biquad::highpass(fs, 38.0, 0.7071),
            bank_a_lp0: Biquad::lowpass(fs, 95.0, q1),
            bank_a_lp1: Biquad::lowpass(fs, 95.0, q2),
            bank_a_env: 1e-6,

            bank_b_hp: Biquad::highpass(fs, 80.0, 0.7071),
            bank_b_lp: Biquad::lowpass(fs, 155.0, 0.7071),
            bank_b_env: 1e-6,

            speech_hp: Biquad::highpass(fs, 400.0, 0.7071),
            speech_lp: Biquad::lowpass(fs, 2800.0, 0.7071),
            speech_env: 1e-6,
            bass_total_env: 1e-6,
            speech_gate: 1.0,

            harm_hp0: Biquad::highpass(fs, 125.0, q1),
            harm_hp1: Biquad::highpass(fs, 125.0, q2),
            harm_lp0: Biquad::lowpass(fs, 380.0, q1),
            harm_lp1: Biquad::lowpass(fs, 380.0, q2),

            warm_dc_l: Biquad::highpass(fs, 30.0, 0.7071),
            warm_dc_r: Biquad::highpass(fs, 30.0, 0.7071),

            lim_delay,
            lim_buf_l: [0.0; LIMITER_RING_CAP],
            lim_buf_r: [0.0; LIMITER_RING_CAP],
            lim_deque_sample: [0; LIMITER_RING_CAP],
            lim_deque_val: [0.0; LIMITER_RING_CAP],
            lim_deque_head: 0,
            lim_deque_tail: 0,
            lim_sample_counter: 0,
            lim_pos: 0,
            lim_prev_l: [0.0; 3],
            lim_prev_r: [0.0; 3],
            lim_gain: 1.0,
            lim_attack_coeff,
            lim_release_coeff,
        }
    }

    pub fn reset(&mut self) {
        self.fir_buf_l.fill(0.0);
        self.fir_buf_r.fill(0.0);
        self.fir_pos = 0;
        self.hp_l0.reset();
        self.hp_l1.reset();
        self.hp_r0.reset();
        self.hp_r1.reset();
        self.iso_lo_l.reset();
        self.iso_hi_l.reset();
        self.iso_lo_r.reset();
        self.iso_hi_r.reset();
        self.bank_a_hp.reset();
        self.bank_a_lp0.reset();
        self.bank_a_lp1.reset();
        self.bank_a_env = 1e-6;
        self.bank_b_hp.reset();
        self.bank_b_lp.reset();
        self.bank_b_env = 1e-6;
        self.speech_hp.reset();
        self.speech_lp.reset();
        self.speech_env = 1e-6;
        self.bass_total_env = 1e-6;
        self.speech_gate = 1.0;
        self.harm_hp0.reset();
        self.harm_hp1.reset();
        self.harm_lp0.reset();
        self.harm_lp1.reset();
        self.warm_dc_l.reset();
        self.warm_dc_r.reset();
        self.lim_buf_l.fill(0.0);
        self.lim_buf_r.fill(0.0);
        self.lim_deque_sample.fill(0);
        self.lim_deque_val.fill(0.0);
        self.lim_deque_head = 0;
        self.lim_deque_tail = 0;
        self.lim_sample_counter = 0;
        self.lim_pos = 0;
        self.lim_prev_l = [0.0; 3];
        self.lim_prev_r = [0.0; 3];
        self.lim_gain = 1.0;
    }

    /// Exact active latency in samples for the current settings.
    #[inline]
    pub fn latency_samples(&self, params: &SpatialParams) -> usize {
        let mut lat = 0usize;
        if params.xtc_mix > 1.0e-5 {
            lat += CAUSAL_DELAY_SAMPLES;
        }
        if params.enable_limiter {
            lat += self.lim_delay;
        }
        lat
    }

    /// 4x Catmull-Rom inter-sample true-peak estimator across the latest 4 samples.
    #[inline]
    fn true_peak_4x(p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
        let mut peak = p1.abs().max(p2.abs());
        // Sub-sample offsets t = 0.25, 0.50, 0.75 between p1 and p2
        for &t in &[0.25_f32, 0.5_f32, 0.75_f32] {
            let t2 = t * t;
            let t3 = t2 * t;
            let v = 0.5
                * ((2.0 * p1)
                    + (-p0 + p2) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
            peak = peak.max(v.abs());
        }
        peak
    }

    /// Dual-bank Chebyshev missing-fundamental bass generator (zero-alloc, f64 precision).
    #[inline]
    fn compute_missing_fundamental_bass(&mut self, mono_in: f64, amount: f32) -> f32 {
        if amount <= 1.0e-5 {
            return 0.0;
        }
        // Bank A: sustained deep bass 38 - 95 Hz (envelope time constant ~25 ms)
        let ba = self.bank_a_lp1.process(self.bank_a_lp0.process(self.bank_a_hp.process(mono_in)));
        let alpha_a = 1.0 / (0.025 * self.sample_rate);
        self.bank_a_env += alpha_a * (ba.abs() - self.bank_a_env);
        let env_a = self.bank_a_env.max(1.0e-5);
        let ua = (ba / env_a).clamp(-1.5, 1.5);
        // Chebyshev polynomials: T2 = 2u^2 - 1, T3 = 4u^3 - 3u, T4 = 8u^4 - 8u^2 + 1
        let ua2 = ua * ua;
        let t2_a = 2.0 * ua2 - 1.0;
        let t3_a = ua * (4.0 * ua2 - 3.0);
        let t4_a = 8.0 * ua2 * ua2 - 8.0 * ua2 + 1.0;
        let harm_a = env_a * (0.42 * t2_a + 0.34 * t3_a + 0.24 * t4_a);

        // Bank B: punch / kick bass 80 - 155 Hz (fast envelope ~8 ms)
        let bb = self.bank_b_lp.process(self.bank_b_hp.process(mono_in));
        let alpha_b = 1.0 / (0.008 * self.sample_rate);
        self.bank_b_env += alpha_b * (bb.abs() - self.bank_b_env);
        let env_b = self.bank_b_env.max(1.0e-5);
        let ub = (bb / env_b).clamp(-1.5, 1.5);
        let ub2 = ub * ub;
        let t2_b = 2.0 * ub2 - 1.0;
        let t3_b = ub * (4.0 * ub2 - 3.0);
        let harm_b = env_b * (0.62 * t2_b + 0.38 * t3_b);

        // Speech gate: suppress perceived bass on speech-only signals
        let sp = self.speech_lp.process(self.speech_hp.process(mono_in));
        let alpha_g = 1.0 / (0.080 * self.sample_rate);
        self.speech_env += alpha_g * (sp.abs() - self.speech_env);
        self.bass_total_env += alpha_g * ((ba.abs() + bb.abs()) - self.bass_total_env);
        let ratio = self.bass_total_env / (self.bass_total_env + 0.35 * self.speech_env + 1.0e-6);
        let target_gate = ((ratio - 0.12) / 0.25).clamp(0.0, 1.0);
        self.speech_gate += alpha_g * (target_gate - self.speech_gate);

        let raw_harm = (harm_a + 0.72 * harm_b) * self.speech_gate;
        let shaped = self
            .harm_lp1
            .process(self.harm_lp0.process(self.harm_hp1.process(self.harm_hp0.process(raw_harm))));
        sanitize((shaped * (amount as f64) * 0.55) as f32)
    }

    /// Process a single stereo sample pair with ZERO heap allocations.
    #[inline]
    pub fn process_sample(&mut self, in_l: f32, in_r: f32, params: &SpatialParams) -> (f32, f32) {
        let xl = sanitize(in_l);
        let xr = sanitize(in_r);

        // 1. Tap raw full-range mono signal for missing-fundamental bass harmonics BEFORE any HP
        let mono_raw = 0.5 * ((xl as f64) + (xr as f64));
        let bass_harm = self.compute_missing_fundamental_bass(mono_raw, params.bass_harmonics);

        // 2. Main stereo path: optional 180 Hz 4th-order BWC HP + ISO 226:2023 warmth contour
        let dry_mix = params.dry_mix.clamp(0.0, 1.0);
        let (mut l, mut r) = if dry_mix > 1.0e-5 {
            let (mut dl, mut dr) = if params.enable_driver_hp {
                (
                    self.hp_l1.process(self.hp_l0.process(xl as f64)),
                    self.hp_r1.process(self.hp_r0.process(xr as f64)),
                )
            } else {
                (xl as f64, xr as f64)
            };

            let contour = params.loudness_contour.clamp(0.0, 1.0) as f64;
            if contour > 1.0e-5 {
                let wl = self.iso_hi_l.process(self.iso_lo_l.process(dl));
                let wr = self.iso_hi_r.process(self.iso_lo_r.process(dr));
                dl += contour * (wl - dl);
                dr += contour * (wr - dr);
            }
            ((dl as f32) * dry_mix + bass_harm, (dr as f32) * dry_mix + bass_harm)
        } else {
            // Pure parallel missing-fundamental bass generator mode (0 ms delay)
            (bass_harm, bass_harm)
        };

        // 3. Subtle analog even-harmonic warmth (with 30 Hz DC blocker)
        let warmth = params.harmonic_warmth.clamp(0.0, 1.0);
        if warmth > 1.0e-5 {
            let k = 0.14 * warmth;
            let nl_l = (l * l * 0.55) + (l.tanh() - l) * 0.45;
            let nl_r = (r * r * 0.55) + (r.tanh() - r) * 0.45;
            l += k * (self.warm_dc_l.process(nl_l as f64) as f32);
            r += k * (self.warm_dc_r.process(nl_r as f64) as f32);
        }

        // 4. 2x2 Choueiri FDTR-XTC 512-tap FIR matrix (symmetric h_rr = h_ll, h_rl = h_lr)
        let xtc_mix = params.xtc_mix.clamp(0.0, 1.0);
        if xtc_mix > 1.0e-5 {
            self.fir_buf_l[self.fir_pos] = l;
            self.fir_buf_r[self.fir_pos] = r;

            let mut acc_ll = 0.0_f32;
            let mut acc_lr = 0.0_f32;
            let mut acc_rl = 0.0_f32;
            let mut acc_rr = 0.0_f32;

            // Split circular convolution into two linear slices for LLVM auto-vectorization (AVX2/FMA)
            let p = self.fir_pos;
            // Tap 0..=p maps to buf[p], buf[p-1], ..., buf[0]
            for tap in 0..=p {
                let idx = p - tap;
                let bl = self.fir_buf_l[idx];
                let br = self.fir_buf_r[idx];
                let k_ll = self.h_ll[tap];
                let k_lr = self.h_lr[tap];
                acc_ll += bl * k_ll;
                acc_lr += br * k_lr;
                acc_rl += bl * k_lr;
                acc_rr += br * k_ll;
            }
            // Tap (p+1)..KERNEL_TAPS maps to buf[KERNEL_TAPS-1], ..., buf[p+1]
            let split = p + 1;
            for tap in split..KERNEL_TAPS {
                let idx = KERNEL_TAPS + p - tap;
                let bl = self.fir_buf_l[idx];
                let br = self.fir_buf_r[idx];
                let k_ll = self.h_ll[tap];
                let k_lr = self.h_lr[tap];
                acc_ll += bl * k_ll;
                acc_lr += br * k_lr;
                acc_rl += bl * k_lr;
                acc_rr += br * k_ll;
            }

            let dry_idx = p.wrapping_sub(CAUSAL_DELAY_SAMPLES) & FIR_RING_MASK;
            let dry_l = self.fir_buf_l[dry_idx];
            let dry_r = self.fir_buf_r[dry_idx];
            let wet_l = acc_ll + acc_lr;
            let wet_r = acc_rl + acc_rr;

            l = dry_l + xtc_mix * (wet_l - dry_l);
            r = dry_r + xtc_mix * (wet_r - dry_r);

            self.fir_pos = (p + 1) & FIR_RING_MASK;
        }

        // 5. Lookahead 4x True-Peak Limiter with LOUDNESS_DRIVE_DB gain stage
        if params.enable_limiter {
            let drive = db_to_lin(params.drive_db.clamp(0.0, 12.0));
            let ceil = db_to_lin(params.ceiling_dbfs.clamp(-9.0, -0.3));
            let dl = sanitize(l * drive);
            let dr = sanitize(r * drive);

            let tp_l = Self::true_peak_4x(self.lim_prev_l[0], self.lim_prev_l[1], self.lim_prev_l[2], dl);
            let tp_r = Self::true_peak_4x(self.lim_prev_r[0], self.lim_prev_r[1], self.lim_prev_r[2], dr);
            self.lim_prev_l = [self.lim_prev_l[1], self.lim_prev_l[2], dl];
            self.lim_prev_r = [self.lim_prev_r[1], self.lim_prev_r[2], dr];
            let inst_tp = tp_l.max(tp_r);

            let pos = self.lim_pos;
            self.lim_buf_l[pos] = dl;
            self.lim_buf_r[pos] = dr;

            // O(1) amortized sliding-window maximum over [sample - lim_delay ..= sample]
            let now = self.lim_sample_counter;
            self.lim_sample_counter = now.wrapping_add(1);
            let min_valid = now.saturating_sub(self.lim_delay as u64);

            while self.lim_deque_head != self.lim_deque_tail
                && self.lim_deque_sample[self.lim_deque_head] < min_valid
            {
                self.lim_deque_head = (self.lim_deque_head + 1) & LIMITER_RING_MASK;
            }
            while self.lim_deque_head != self.lim_deque_tail {
                let back = self.lim_deque_tail.wrapping_sub(1) & LIMITER_RING_MASK;
                if self.lim_deque_val[back] <= inst_tp {
                    self.lim_deque_tail = back;
                } else {
                    break;
                }
            }
            self.lim_deque_sample[self.lim_deque_tail] = now;
            self.lim_deque_val[self.lim_deque_tail] = inst_tp;
            self.lim_deque_tail = (self.lim_deque_tail + 1) & LIMITER_RING_MASK;
            let win_peak = self.lim_deque_val[self.lim_deque_head];

            let target_gain = if win_peak > ceil { ceil / win_peak } else { 1.0 };
            if target_gain < self.lim_gain {
                // Fast attack + hard guarantee not to overshoot current delayed sample
                self.lim_gain =
                    target_gain + self.lim_attack_coeff * (self.lim_gain - target_gain);
                if self.lim_gain > target_gain {
                    // Tightly clamp when within 2 ms of peak arrival
                    self.lim_gain = self.lim_gain.min(target_gain * 1.02);
                }
            } else {
                self.lim_gain =
                    target_gain + self.lim_release_coeff * (self.lim_gain - target_gain);
            }

            let read_idx = pos.wrapping_sub(self.lim_delay) & LIMITER_RING_MASK;
            let out_l_raw = self.lim_buf_l[read_idx];
            let out_r_raw = self.lim_buf_r[read_idx];
            let sample_max = out_l_raw.abs().max(out_r_raw.abs());
            let applied_gain = if sample_max * self.lim_gain > ceil {
                ceil / sample_max
            } else {
                self.lim_gain
            };

            l = sanitize((out_l_raw * applied_gain).clamp(-ceil, ceil));
            r = sanitize((out_r_raw * applied_gain).clamp(-ceil, ceil));
            self.lim_pos = (pos + 1) & LIMITER_RING_MASK;
        }

        (sanitize(l), sanitize(r))
    }

    /// Process a stereo block (supports in-place buffers cleanly).
    pub fn process_block(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        params: &SpatialParams,
    ) {
        let n = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());
        for i in 0..n {
            let (yl, yr) = self.process_sample(in_l[i], in_r[i], params);
            out_l[i] = yl;
            out_r[i] = yr;
        }
    }
}
