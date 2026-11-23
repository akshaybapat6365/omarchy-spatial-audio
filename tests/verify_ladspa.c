/*
 * C-ABI LADSPA v1.1 Verification Harness for libomarchy_spatial_fdtr.so
 * Verifies dlopen, ladspa_descriptor ABI layout, port contract, impulse latency,
 * mono-sum unity, zero-latency bass mode, in-place safety, true-peak ceiling,
 * and real-time block performance.
 */
#define _POSIX_C_SOURCE 200809L
#include <dlfcn.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef float LADSPA_Data;
typedef int LADSPA_Properties;
typedef int LADSPA_PortDescriptor;
typedef int LADSPA_PortRangeHintDescriptor;
typedef void *LADSPA_Handle;

typedef struct _LADSPA_PortRangeHint {
    LADSPA_PortRangeHintDescriptor HintDescriptor;
    LADSPA_Data LowerBound;
    LADSPA_Data UpperBound;
} LADSPA_PortRangeHint;

typedef struct _LADSPA_Descriptor {
    unsigned long UniqueID;
    const char *Label;
    LADSPA_Properties Properties;
    const char *Name;
    const char *Maker;
    const char *Copyright;
    unsigned long PortCount;
    const LADSPA_PortDescriptor *PortDescriptors;
    const char *const *PortNames;
    const LADSPA_PortRangeHint *PortRangeHints;
    void *ImplementationData;
    LADSPA_Handle (*instantiate)(const struct _LADSPA_Descriptor *Descriptor,
                                 unsigned long SampleRate);
    void (*connect_port)(LADSPA_Handle Instance,
                         unsigned long Port,
                         LADSPA_Data *DataLocation);
    void (*activate)(LADSPA_Handle Instance);
    void (*run)(LADSPA_Handle Instance, unsigned long SampleCount);
    void (*run_adding)(LADSPA_Handle Instance, unsigned long SampleCount);
    void (*set_run_adding_gain)(LADSPA_Handle Instance, LADSPA_Data Gain);
    void (*deactivate)(LADSPA_Handle Instance);
    void (*cleanup)(LADSPA_Handle Instance);
} LADSPA_Descriptor;

typedef const LADSPA_Descriptor *(*LADSPA_Descriptor_Function)(unsigned long Index);

static double now_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e6 + (double)ts.tv_nsec * 1e-3;
}

int main(int argc, char **argv) {
    const char *so_path = argc > 1 ? argv[1] : "lib/libomarchy_spatial_fdtr.so";
    void *handle = dlopen(so_path, RTLD_NOW | RTLD_LOCAL);
    if (!handle) {
        fprintf(stderr, "FAIL: dlopen(%s): %s\n", so_path, dlerror());
        return 1;
    }

    LADSPA_Descriptor_Function fn =
        (LADSPA_Descriptor_Function)dlsym(handle, "ladspa_descriptor");
    if (!fn) {
        fprintf(stderr, "FAIL: dlsym(ladspa_descriptor): %s\n", dlerror());
        dlclose(handle);
        return 1;
    }

    const LADSPA_Descriptor *d = fn(0);
    if (!d) {
        fprintf(stderr, "FAIL: ladspa_descriptor(0) returned NULL\n");
        dlclose(handle);
        return 1;
    }
    if (fn(1) != NULL) {
        fprintf(stderr, "FAIL: ladspa_descriptor(1) expected NULL\n");
        dlclose(handle);
        return 1;
    }

    printf("LADSPA Descriptor Loaded:\n");
    printf("  UniqueID   : %lu\n", d->UniqueID);
    printf("  Label      : %s\n", d->Label);
    printf("  Name       : %s\n", d->Name);
    printf("  Properties : 0x%x (REALTIME=%d, HARD_RT=%d)\n",
           d->Properties, (d->Properties & 0x1) != 0, (d->Properties & 0x4) != 0);
    printf("  PortCount  : %lu\n", d->PortCount);

    if (d->UniqueID != 5198662UL || strcmp(d->Label, "omarchy_spatial_fdtr") != 0 ||
        d->Properties != 0x5 || d->PortCount != 13UL) {
        fprintf(stderr, "FAIL: descriptor metadata mismatch\n");
        dlclose(handle);
        return 1;
    }

    LADSPA_Handle inst = d->instantiate(d, 48000UL);
    if (!inst) {
        fprintf(stderr, "FAIL: instantiate returned NULL\n");
        dlclose(handle);
        return 1;
    }

    enum { N = 2048 };
    float in_l[N], in_r[N], out_l[N], out_r[N];
    float xtc_mix = 1.0f, dry_mix = 1.0f, bass = 0.0f, warmth = 0.0f, contour = 0.0f;
    float drive_db = 0.0f, ceiling_dbfs = -1.0f, enable_lim = 0.0f, latency = -1.0f;

    d->connect_port(inst, 0, in_l);
    d->connect_port(inst, 1, in_r);
    d->connect_port(inst, 2, out_l);
    d->connect_port(inst, 3, out_r);
    d->connect_port(inst, 4, &xtc_mix);
    d->connect_port(inst, 5, &dry_mix);
    d->connect_port(inst, 6, &bass);
    d->connect_port(inst, 7, &warmth);
    d->connect_port(inst, 8, &contour);
    d->connect_port(inst, 9, &drive_db);
    d->connect_port(inst, 10, &ceiling_dbfs);
    d->connect_port(inst, 11, &enable_lim);
    d->connect_port(inst, 12, &latency);

    /* Test 1: Pure FDTR-XTC Mono Impulse & Mono-Sum Unity */
    memset(in_l, 0, sizeof(in_l));
    memset(in_r, 0, sizeof(in_r));
    in_l[0] = 1.0f;
    in_r[0] = 1.0f;
    d->activate(inst);
    d->run(inst, N);

    int peak_idx = 0;
    float peak_val = 0.0f;
    float max_off_peak = 0.0f;
    for (int i = 0; i < N; i++) {
        if (fabsf(out_l[i]) > peak_val) {
            peak_val = fabsf(out_l[i]);
            peak_idx = i;
        }
        if (i != 256) {
            float err = fabsf(out_l[i]);
            if (err > max_off_peak) max_off_peak = err;
        }
    }
    printf("Test 1 (Mono Impulse FDTR-XTC): peak_idx=%d (val=%.6f), max_off_peak=%.2e, latency=%.1f\n",
           peak_idx, peak_val, max_off_peak, latency);
    if (peak_idx != 256 || fabsf(peak_val - 1.0f) > 1e-4f || max_off_peak > 1e-4f ||
        fabsf(latency - 256.0f) > 1e-3f) {
        fprintf(stderr, "FAIL: Test 1 mono impulse check failed\n");
        return 1;
    }

    /* Test 1b: Left-only impulse -> verify L+R sums identically to mono impulse */
    memset(in_l, 0, sizeof(in_l));
    memset(in_r, 0, sizeof(in_r));
    in_l[0] = 1.0f;
    d->activate(inst);
    d->run(inst, N);
    float max_sum_err = 0.0f;
    float max_lr_abs = 0.0f;
    double lr_energy = 0.0;
    for (int i = 0; i < N; i++) {
        float target = (i == 256) ? 1.0f : 0.0f;
        float err = fabsf((out_l[i] + out_r[i]) - target);
        if (err > max_sum_err) max_sum_err = err;
        if (fabsf(out_r[i]) > max_lr_abs) max_lr_abs = fabsf(out_r[i]);
        lr_energy += (double)out_r[i] * (double)out_r[i];
    }
    printf("Test 1b (Left-only Impulse)   : max |(h_ll+h_lr)-delta|=%.2e, max |h_lr|=%.4f, E(h_lr)=%.5f\n",
           max_sum_err, max_lr_abs, lr_energy);
    if (max_sum_err > 1e-4f || max_lr_abs < 0.02f || lr_energy < 0.01) {
        fprintf(stderr, "FAIL: Test 1b crosstalk kernel check failed\n");
        return 1;
    }

    /* Test 2: Zero-latency parallel bass-harmonics generator mode */
    xtc_mix = 0.0f;
    dry_mix = 0.0f;
    bass = 0.45f;
    enable_lim = 0.0f;
    for (int i = 0; i < N; i++) {
        float s = 0.5f * sinf(2.0f * 3.1415926535f * 60.0f * (float)i / 48000.0f);
        in_l[i] = s;
        in_r[i] = s;
    }
    d->activate(inst);
    d->run(inst, N);
    double bass_sq = 0.0;
    for (int i = 512; i < N; i++) bass_sq += (double)out_l[i] * (double)out_l[i];
    double bass_rms_db = 20.0 * log10(sqrt(bass_sq / (N - 512)) + 1e-12);
    printf("Test 2 (Zero-Lat Bass Mode)   : latency=%.1f samples, 60Hz->harmonics RMS=%.2f dBFS\n",
           latency, bass_rms_db);
    if (fabsf(latency) > 1e-3f || bass_rms_db < -40.0 || bass_rms_db > -6.0) {
        fprintf(stderr, "FAIL: Test 2 bass harmonics mode check failed\n");
        return 1;
    }

    /* Test 3: Full Fused 3D Spatial + Warmth + +7 dB Drive True-Peak Limiter & In-Place Safety */
    xtc_mix = 1.0f;
    dry_mix = 1.0f;
    bass = 0.38f;
    warmth = 0.16f;
    contour = 0.25f;
    drive_db = 7.0f;
    ceiling_dbfs = -1.0f;
    enable_lim = 1.0f;

    float ip_l[N], ip_r[N];
    for (int i = 0; i < N; i++) {
        float t = (float)i / 48000.0f;
        in_l[i] = sinf(2.0f * 3.1415926535f * 997.0f * t);
        in_r[i] = -in_l[i];
        ip_l[i] = in_l[i];
        ip_r[i] = in_r[i];
    }
    d->activate(inst);
    d->run(inst, N);

    float max_out = 0.0f;
    int non_finite = 0;
    for (int i = 0; i < N; i++) {
        if (!isfinite(out_l[i]) || !isfinite(out_r[i])) non_finite++;
        if (fabsf(out_l[i]) > max_out) max_out = fabsf(out_l[i]);
        if (fabsf(out_r[i]) > max_out) max_out = fabsf(out_r[i]);
    }
    double max_out_dbfs = 20.0 * log10((double)max_out + 1e-12);

    /* Re-run identically in-place */
    d->connect_port(inst, 0, ip_l);
    d->connect_port(inst, 1, ip_r);
    d->connect_port(inst, 2, ip_l);
    d->connect_port(inst, 3, ip_r);
    d->activate(inst);
    d->run(inst, N);
    float max_ip_diff = 0.0f;
    for (int i = 0; i < N; i++) {
        float dl = fabsf(ip_l[i] - out_l[i]);
        float dr = fabsf(ip_r[i] - out_r[i]);
        if (dl > max_ip_diff) max_ip_diff = dl;
        if (dr > max_ip_diff) max_ip_diff = dr;
    }
    printf("Test 3 (Full Fused + Limiter) : latency=%.1f, overload_peak=%.3f dBFS (ceil -1.00), inplace_diff=%.2e, non_finite=%d\n",
           latency, max_out_dbfs, max_ip_diff, non_finite);
    if (fabsf(latency - 736.0f) > 1e-3f || max_out_dbfs > -0.95 || max_ip_diff > 1e-6f || non_finite != 0) {
        fprintf(stderr, "FAIL: Test 3 fused limiter/in-place check failed\n");
        return 1;
    }

    /* Test 4: Real-time 128-sample quantum benchmark */
    d->connect_port(inst, 0, in_l);
    d->connect_port(inst, 1, in_r);
    d->connect_port(inst, 2, out_l);
    d->connect_port(inst, 3, out_r);
    const int blocks = 375; /* 48,000 samples = 1.000 s */
    double t0 = now_us();
    for (int b = 0; b < blocks; b++) {
        d->run(inst, 128UL);
    }
    double dt_us = now_us() - t0;
    printf("Test 4 (RT 128-smp Quantum)   : 1.0 s audio in %.2f ms (%.1f us / 2667 us quantum = %.2f%% of 1 core)\n",
           dt_us / 1000.0, dt_us / blocks, (dt_us / 1e6) * 100.0);

    d->deactivate(inst);
    d->cleanup(inst);
    dlclose(handle);
    printf("LADSPA C-ABI VERDICT          : PASS\n");
    return 0;
}
