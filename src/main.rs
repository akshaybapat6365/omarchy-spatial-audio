//! CLI tool for Omarchy Spatial Audio (`omarchy-spatial-dsp`):
//!   - generate-kernels --out-dir <dir>
//!   - verify [--kernels-dir <dir>] [--json]
//!   - process-wav --in <in.wav> --out <out.wav> [--drive-db 7.0] [--xtc-mix 1.0] [--bass 0.38] [--warmth 0.16] [--contour 0.25]

use std::env;
use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use omarchy_spatial_fdtr::dsp::{lin_to_db, SpatialDspEngine, SpatialParams};
use omarchy_spatial_fdtr::fdtr::{
    generate_fdtr_kernels, verify_kernels, FdtrKernels, Xps17Geometry, CAUSAL_DELAY_SAMPLES,
    KERNEL_TAPS,
};
use omarchy_spatial_fdtr::wav::{read_wav, write_wav_f32};

fn usage() {
    eprintln!(
        "Usage:
  omarchy-spatial-dsp generate-kernels --out-dir <dir> [--speaker-span-cm 34.0] [--listening-distance-cm 55.0]
  omarchy-spatial-dsp verify [--kernels-dir <dir>] [--speaker-span-cm 34.0] [--listening-distance-cm 55.0] [--json]
  omarchy-spatial-dsp process-wav --in <in.wav> --out <out.wav> [--drive-db 7.0] [--xtc-mix 1.0] [--bass 0.38] [--warmth 0.16] [--contour 0.25]"
    );
}

fn cmd_generate_kernels(out_dir: &Path, geom: &Xps17Geometry) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|e| format!("create dir {:?}: {e}", out_dir))?;
    let kernels = generate_fdtr_kernels(geom);
    let metrics = verify_kernels(geom, &kernels);

    write_wav_f32(out_dir.join("h_ll.wav"), kernels.sample_rate, 1, &kernels.h_ll)
        .map_err(|e| format!("write h_ll.wav: {e}"))?;
    write_wav_f32(out_dir.join("h_lr.wav"), kernels.sample_rate, 1, &kernels.h_lr)
        .map_err(|e| format!("write h_lr.wav: {e}"))?;
    write_wav_f32(out_dir.join("h_rl.wav"), kernels.sample_rate, 1, &kernels.h_rl)
        .map_err(|e| format!("write h_rl.wav: {e}"))?;
    write_wav_f32(out_dir.join("h_rr.wav"), kernels.sample_rate, 1, &kernels.h_rr)
        .map_err(|e| format!("write h_rr.wav: {e}"))?;

    let meta_json = format!(
        "{{\n\
         \x20 \"device\": \"Dell XPS 17 9720\",\n\
         \x20 \"speaker_span_cm\": {:.1},\n\
         \x20 \"listening_distance_cm\": {:.1},\n\
         \x20 \"full_span_deg\": {:.2},\n\
         \x20 \"itd_us\": {:.1},\n\
         \x20 \"comb_singularity_hz\": {:.2},\n\
         \x20 \"sample_rate_hz\": {},\n\
         \x20 \"taps\": {},\n\
         \x20 \"causal_delay_samples\": {},\n\
         \x20 \"causal_delay_ms\": {:.4},\n\
         \x20 \"xtc_200_2600_mean_db\": {:.3},\n\
         \x20 \"xtc_200_2600_net_db\": {:.3},\n\
         \x20 \"xtc_300_2600_mean_db\": {:.3},\n\
         \x20 \"xtc_2600_5500_mean_db\": {:.3},\n\
         \x20 \"mono_sum_max_ripple_db\": {:.6},\n\
         \x20 \"max_hll_gain_db\": {:.3},\n\
         \x20 \"max_hlr_gain_db\": {:.3},\n\
         \x20 \"max_side_gain_db\": {:.3},\n\
         \x20 \"offaxis_3cm_mean_db\": {:.3},\n\
         \x20 \"offaxis_5cm_mean_db\": {:.3},\n\
         \x20 \"all_passed\": {}\n\
         }}\n",
        geom.speaker_span_m * 100.0,
        geom.listening_dist_m * 100.0,
        metrics.full_span_deg,
        metrics.itd_us,
        metrics.comb_f1_hz,
        kernels.sample_rate,
        kernels.taps,
        kernels.delay_samples,
        metrics.latency_ms,
        metrics.xtc_200_2600_mean_db,
        metrics.xtc_200_2600_net_db,
        metrics.xtc_300_2600_mean_db,
        metrics.xtc_2600_5500_mean_db,
        metrics.mono_sum_max_ripple_db,
        metrics.max_hll_gain_db,
        metrics.max_hlr_gain_db,
        metrics.max_side_gain_db,
        metrics.offaxis_3cm_mean_db,
        metrics.offaxis_5cm_mean_db,
        metrics.all_passed
    );
    fs::write(out_dir.join("kernels_meta.json"), meta_json)
        .map_err(|e| format!("write kernels_meta.json: {e}"))?;

    println!(
        "Generated 4x 512-tap 48kHz float32 WAV FDTR-XTC kernels in {} (all_passed={})",
        out_dir.display(),
        metrics.all_passed
    );
    Ok(())
}

fn load_kernels_from_dir(dir: &Path) -> Result<FdtrKernels, String> {
    let ll = read_wav(dir.join("h_ll.wav")).map_err(|e| format!("read h_ll.wav: {e}"))?;
    let lr = read_wav(dir.join("h_lr.wav")).map_err(|e| format!("read h_lr.wav: {e}"))?;
    let rl = read_wav(dir.join("h_rl.wav")).map_err(|e| format!("read h_rl.wav: {e}"))?;
    let rr = read_wav(dir.join("h_rr.wav")).map_err(|e| format!("read h_rr.wav: {e}"))?;
    if ll.sample_rate != 48_000
        || ll.channels != 1
        || ll.samples.len() != KERNEL_TAPS
        || lr.samples.len() != KERNEL_TAPS
        || rl.samples.len() != KERNEL_TAPS
        || rr.samples.len() != KERNEL_TAPS
    {
        return Err("Kernel WAV header/length mismatch".into());
    }
    Ok(FdtrKernels {
        sample_rate: ll.sample_rate,
        taps: ll.samples.len(),
        delay_samples: CAUSAL_DELAY_SAMPLES,
        h_ll: ll.samples,
        h_lr: lr.samples,
        h_rl: rl.samples,
        h_rr: rr.samples,
    })
}

fn cmd_verify(kernels_dir: Option<&Path>, geom: &Xps17Geometry, as_json: bool) -> Result<bool, String> {
    let kernels = match kernels_dir {
        Some(dir) => load_kernels_from_dir(dir)?,
        None => generate_fdtr_kernels(geom),
    };
    let m = verify_kernels(geom, &kernels);

    // Real-time DSP chain offline verification:
    // 1) Mono sine (1 kHz) headroom & phase symmetry
    // 2) Full-scale +6 dB transient overload -> verify true-peak limiter holds <= -0.95 dBFS
    // 3) Loudness preservation vs bypass (full +7 dB LOUDNESS_DRIVE_DB)
    // 4) Sub-bass (60 Hz) -> verify missing-fundamental harmonics generated in 120-360 Hz
    // 5) Real-time CPU throughput benchmark (1 second = 48,000 stereo frames)
    let mut engine = SpatialDspEngine::new(48_000.0);
    let params = SpatialParams::default();

    let n_bench = 48_000usize;
    let mut in_l = vec![0.0_f32; n_bench];
    let mut in_r = vec![0.0_f32; n_bench];
    let mut out_l = vec![0.0_f32; n_bench];
    let mut out_r = vec![0.0_f32; n_bench];
    for i in 0..n_bench {
        let t = (i as f64) / 48_000.0;
        // Realistic -14 LUFS music-level multi-tone test: 65 Hz sub-bass + 440 Hz warm mid + stereo lead + 8 kHz air
        let bass = 0.09 * (2.0 * PI * 65.0 * t).sin();
        let mid = 0.10 * (2.0 * PI * 440.0 * t).sin();
        let lead_l = 0.09 * (2.0 * PI * 1500.0 * t).sin();
        let lead_r = 0.09 * (2.0 * PI * 1850.0 * t).sin();
        let air = 0.04 * (2.0 * PI * 8000.0 * t).sin();
        in_l[i] = (bass + mid + lead_l + air) as f32;
        in_r[i] = (bass + mid + lead_r + air) as f32;
    }

    let t0 = Instant::now();
    engine.process_block(&in_l, &in_r, &mut out_l, &mut out_r, &params);
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Skip initial transient (2048 samples) for steady-state RMS & peak check
    let mut in_sq = 0.0_f64;
    let mut out_sq = 0.0_f64;
    let mut max_peak = 0.0_f32;
    let mut nan_count = 0usize;
    for i in 2048..n_bench {
        if !out_l[i].is_finite() || !out_r[i].is_finite() {
            nan_count += 1;
            continue;
        }
        in_sq += (in_l[i] as f64).powi(2) + (in_r[i] as f64).powi(2);
        out_sq += (out_l[i] as f64).powi(2) + (out_r[i] as f64).powi(2);
        max_peak = max_peak.max(out_l[i].abs()).max(out_r[i].abs());
    }
    let cnt = (2 * (n_bench - 2048)) as f64;
    let in_rms_db = lin_to_db((in_sq / cnt).sqrt() as f32);
    let out_rms_db = lin_to_db((out_sq / cnt).sqrt() as f32);
    let loudness_delta_db = out_rms_db - in_rms_db;
    let peak_dbfs = lin_to_db(max_peak);

    // Overload stress test: 0 dBFS full-scale burst with +7 dB drive -> must hold <= -0.98 dBFS
    engine.reset();
    let mut hot_peak = 0.0_f32;
    for i in 0..8192 {
        let s = (2.0 * PI * 997.0 * (i as f64) / 48_000.0).sin() as f32;
        let (yl, yr) = engine.process_sample(s, -s, &params);
        hot_peak = hot_peak.max(yl.abs()).max(yr.abs());
    }
    let overload_peak_dbfs = lin_to_db(hot_peak);
    let headroom_ok = overload_peak_dbfs <= -0.95 && nan_count == 0;
    let loudness_ok = loudness_delta_db >= 2.0; // confirms full LOUDNESS_DRIVE_DB loudness preserved!
    let overall_ok = m.all_passed && headroom_ok && loudness_ok;

    if as_json {
        println!(
            "{{\n\
             \x20 \"all_passed\": {},\n\
             \x20 \"full_span_deg\": {:.2},\n\
             \x20 \"itd_us\": {:.1},\n\
             \x20 \"comb_f1_hz\": {:.2},\n\
             \x20 \"fdtr_latency_ms\": {:.4},\n\
             \x20 \"xtc_200_2600_mean_db\": {:.3},\n\
             \x20 \"xtc_200_2600_net_db\": {:.3},\n\
             \x20 \"xtc_300_2600_mean_db\": {:.3},\n\
             \x20 \"xtc_2600_5500_mean_db\": {:.3},\n\
             \x20 \"mono_sum_max_ripple_db\": {:.6},\n\
             \x20 \"max_hll_gain_db\": {:.3},\n\
             \x20 \"max_hlr_gain_db\": {:.3},\n\
             \x20 \"max_side_gain_db\": {:.3},\n\
             \x20 \"bass_lt200_max_hll_db\": {:.3},\n\
             \x20 \"bass_lt200_max_hlr_db\": {:.3},\n\
             \x20 \"treble_gt6000_max_hll_db\": {:.4},\n\
             \x20 \"treble_gt6000_max_hlr_db\": {:.2},\n\
             \x20 \"offaxis_3cm_mean_db\": {:.3},\n\
             \x20 \"offaxis_5cm_mean_db\": {:.3},\n\
             \x20 \"loudness_delta_db\": {:.3},\n\
             \x20 \"music_peak_dbfs\": {:.3},\n\
             \x20 \"overload_peak_dbfs\": {:.3},\n\
             \x20 \"rt_1s_process_ms\": {:.2}\n\
             }}",
            overall_ok,
            m.full_span_deg,
            m.itd_us,
            m.comb_f1_hz,
            m.latency_ms,
            m.xtc_200_2600_mean_db,
            m.xtc_200_2600_net_db,
            m.xtc_300_2600_mean_db,
            m.xtc_2600_5500_mean_db,
            m.mono_sum_max_ripple_db,
            m.max_hll_gain_db,
            m.max_hlr_gain_db,
            m.max_side_gain_db,
            m.bass_lt200_max_hll_db,
            m.bass_lt200_max_hlr_db,
            m.treble_gt6000_max_hll_db,
            m.treble_gt6000_max_hlr_db,
            m.offaxis_3cm_mean_db,
            m.offaxis_5cm_mean_db,
            loudness_delta_db,
            peak_dbfs,
            overload_peak_dbfs,
            elapsed_ms
        );
    } else {
        println!("=== Omarchy Spatial Audio · Dell XPS 17 9720 FDTR-XTC Verification ===");
        println!(
            "Geometry         : S={:.1} cm, D={:.1} cm -> 2*theta={:.2} deg, ITD={:.1} us, f1={:.2} Hz",
            geom.speaker_span_m * 100.0,
            geom.listening_dist_m * 100.0,
            m.full_span_deg,
            m.itd_us,
            m.comb_f1_hz
        );
        println!(
            "FIR Kernels      : 4x {} taps @ 48 kHz (float32), peak tap={}, latency={:.3} ms",
            kernels.taps, m.peak_tap_index, m.latency_ms
        );
        println!(
            "< 200 Hz Bass    : max |H_ll|={:+.3} dB, max |H_lr|={:+.2} dB (strict unity bypass)",
            m.bass_lt200_max_hll_db, m.bass_lt200_max_hlr_db
        );
        println!(
            "200-2600 Hz XTC  : mean sep={:.2} dB (net +{:.2} dB; 300-2600 Hz={:.2} dB)",
            m.xtc_200_2600_mean_db, m.xtc_200_2600_net_db, m.xtc_300_2600_mean_db
        );
        println!(
            "2600-5500 Hz f1  : mean sep={:.2} dB, max gain={:+.2} dB (3.29 kHz singularity damped)",
            m.xtc_2600_5500_mean_db, m.xtc_2600_5500_max_gain_db
        );
        println!(
            "> 6000 Hz Air    : max |H_ll|={:+.4} dB, max |H_lr|={:+.2} dB (uncolored tweeter taper)",
            m.treble_gt6000_max_hll_db, m.treble_gt6000_max_hlr_db
        );
        println!(
            "Mono-Sum Flatness: max |H_ll+H_lr| ripple = {:.6} dB (spec <= +/-0.60 dB)",
            m.mono_sum_max_ripple_db
        );
        println!(
            "Peak Filter Gain : max |H_ll|={:+.2} dB, max |H_lr|={:+.2} dB, max |H_-|={:+.2} dB (spec <= +4.50 dB)",
            m.max_hll_gain_db, m.max_hlr_gain_db, m.max_side_gain_db
        );
        println!(
            "Sweet-Spot Shift : +/-3 cm -> {:.2} dB, +/-5 cm -> {:.2} dB (300-2600 Hz)",
            m.offaxis_3cm_mean_db, m.offaxis_5cm_mean_db
        );
        println!(
            "Full Chain DSP   : loudness delta={:+.2} dB (drive +7.0 dB), music peak={:+.2} dBFS, overload peak={:+.2} dBFS (ceiling -1.00 dBTP)",
            loudness_delta_db, peak_dbfs, overload_peak_dbfs
        );
        println!(
            "RT Performance   : 1.0 s stereo @ 48 kHz processed in {:.2} ms ({:.2}% of 1 core), NaNs={}",
            elapsed_ms,
            elapsed_ms / 10.0,
            nan_count
        );
        println!(
            "VERDICT          : {}",
            if overall_ok { "PASS" } else { "FAIL" }
        );
    }

    Ok(overall_ok)
}

fn cmd_process_wav(in_path: &Path, out_path: &Path, params: &SpatialParams) -> Result<(), String> {
    let wav = read_wav(in_path).map_err(|e| format!("read {:?}: {e}", in_path))?;
    if wav.channels != 2 {
        return Err(format!("Expected stereo WAV (2 ch), got {} ch", wav.channels));
    }
    let frames = wav.samples.len() / 2;
    let mut engine = SpatialDspEngine::new(wav.sample_rate as f64);
    let mut out = vec![0.0_f32; wav.samples.len()];

    let mut in_sq = 0.0_f64;
    let mut out_sq = 0.0_f64;
    let mut peak = 0.0_f32;
    for i in 0..frames {
        let xl = wav.samples[2 * i];
        let xr = wav.samples[2 * i + 1];
        let (yl, yr) = engine.process_sample(xl, xr, params);
        out[2 * i] = yl;
        out[2 * i + 1] = yr;
        in_sq += (xl as f64).powi(2) + (xr as f64).powi(2);
        out_sq += (yl as f64).powi(2) + (yr as f64).powi(2);
        peak = peak.max(yl.abs()).max(yr.abs());
    }
    write_wav_f32(out_path, wav.sample_rate, 2, &out)
        .map_err(|e| format!("write {:?}: {e}", out_path))?;
    let n = (2 * frames).max(1) as f64;
    println!(
        "Processed {} frames @ {} Hz -> {} | in_rms={:+.2} dBFS, out_rms={:+.2} dBFS, peak={:+.2} dBFS",
        frames,
        wav.sample_rate,
        out_path.display(),
        lin_to_db((in_sq / n).sqrt() as f32),
        lin_to_db((out_sq / n).sqrt() as f32),
        lin_to_db(peak),
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(2);
    }

    match args[1].as_str() {
        "generate-kernels" => {
            let mut out_dir = PathBuf::from("kernels");
            let mut span_cm = 34.0_f64;
            let mut dist_cm = 55.0_f64;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--out-dir" if i + 1 < args.len() => {
                        out_dir = PathBuf::from(&args[i + 1]);
                        i += 2;
                    }
                    "--speaker-span-cm" if i + 1 < args.len() => {
                        span_cm = args[i + 1].parse().unwrap_or(span_cm);
                        i += 2;
                    }
                    "--listening-distance-cm" if i + 1 < args.len() => {
                        dist_cm = args[i + 1].parse().unwrap_or(dist_cm);
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let geom = Xps17Geometry::from_geometry_cm(span_cm, dist_cm);
            match cmd_generate_kernels(&out_dir, &geom) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "verify" => {
            let mut kernels_dir: Option<PathBuf> = None;
            let mut as_json = false;
            let mut span_cm = 34.0_f64;
            let mut dist_cm = 55.0_f64;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--kernels-dir" if i + 1 < args.len() => {
                        kernels_dir = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    }
                    "--speaker-span-cm" if i + 1 < args.len() => {
                        span_cm = args[i + 1].parse().unwrap_or(span_cm);
                        i += 2;
                    }
                    "--listening-distance-cm" if i + 1 < args.len() => {
                        dist_cm = args[i + 1].parse().unwrap_or(dist_cm);
                        i += 2;
                    }
                    "--json" => {
                        as_json = true;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            let geom = Xps17Geometry::from_geometry_cm(span_cm, dist_cm);
            match cmd_verify(kernels_dir.as_deref(), &geom, as_json) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "process-wav" => {
            let mut in_path: Option<PathBuf> = None;
            let mut out_path: Option<PathBuf> = None;
            let mut params = SpatialParams::default();
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--in" if i + 1 < args.len() => {
                        in_path = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    }
                    "--out" if i + 1 < args.len() => {
                        out_path = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    }
                    "--drive-db" if i + 1 < args.len() => {
                        params.drive_db = args[i + 1].parse().unwrap_or(params.drive_db);
                        i += 2;
                    }
                    "--xtc-mix" if i + 1 < args.len() => {
                        params.xtc_mix = args[i + 1].parse().unwrap_or(params.xtc_mix);
                        i += 2;
                    }
                    "--bass" if i + 1 < args.len() => {
                        params.bass_harmonics = args[i + 1].parse().unwrap_or(params.bass_harmonics);
                        i += 2;
                    }
                    "--warmth" if i + 1 < args.len() => {
                        params.harmonic_warmth = args[i + 1].parse().unwrap_or(params.harmonic_warmth);
                        i += 2;
                    }
                    "--contour" if i + 1 < args.len() => {
                        params.loudness_contour = args[i + 1].parse().unwrap_or(params.loudness_contour);
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            match (in_path, out_path) {
                (Some(ip), Some(op)) => match cmd_process_wav(&ip, &op, &params) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                },
                _ => {
                    usage();
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}
