//! Spec-exact LADSPA v1.1 C-ABI export for PipeWire `filter-chain` (`type = ladspa`).
//!
//! Hard real-time safe (`LADSPA_PROPERTY_REALTIME | LADSPA_PROPERTY_HARD_RT_CAPABLE`):
//!   - Zero heap allocations in `activate()`, `run()`, or `deactivate()`
//!   - Zero locks or syscalls in audio callback
//!   - Safe for in-place buffers (`out_l == in_l` / `out_r == in_r`)

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::ptr;

use crate::dsp::{SpatialDspEngine, SpatialParams};

pub type LadspaData = f32;
pub type LadspaProperties = c_int;
pub type LadspaPortDescriptor = c_int;
pub type LadspaPortRangeHintDescriptor = c_int;
pub type LadspaHandle = *mut c_void;

const LADSPA_PROPERTY_REALTIME: LadspaProperties = 0x1;
const LADSPA_PROPERTY_HARD_RT_CAPABLE: LadspaProperties = 0x4;

const LADSPA_PORT_INPUT: LadspaPortDescriptor = 0x1;
const LADSPA_PORT_OUTPUT: LadspaPortDescriptor = 0x2;
const LADSPA_PORT_CONTROL: LadspaPortDescriptor = 0x4;
const LADSPA_PORT_AUDIO: LadspaPortDescriptor = 0x8;

const LADSPA_HINT_BOUNDED_BELOW: LadspaPortRangeHintDescriptor = 0x1;
const LADSPA_HINT_BOUNDED_ABOVE: LadspaPortRangeHintDescriptor = 0x2;
const LADSPA_HINT_TOGGLED: LadspaPortRangeHintDescriptor = 0x4;
const LADSPA_HINT_DEFAULT_LOW: LadspaPortRangeHintDescriptor = 0x80;
const LADSPA_HINT_DEFAULT_MIDDLE: LadspaPortRangeHintDescriptor = 0xC0;
const LADSPA_HINT_DEFAULT_HIGH: LadspaPortRangeHintDescriptor = 0x100;
const LADSPA_HINT_DEFAULT_0: LadspaPortRangeHintDescriptor = 0x200;
const LADSPA_HINT_DEFAULT_1: LadspaPortRangeHintDescriptor = 0x240;

const PORT_COUNT: usize = 13;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LadspaPortRangeHint {
    pub hint_descriptor: LadspaPortRangeHintDescriptor,
    pub lower_bound: LadspaData,
    pub upper_bound: LadspaData,
}

#[repr(C)]
pub struct LadspaDescriptor {
    pub unique_id: c_ulong,
    pub label: *const c_char,
    pub properties: LadspaProperties,
    pub name: *const c_char,
    pub maker: *const c_char,
    pub copyright: *const c_char,
    pub port_count: c_ulong,
    pub port_descriptors: *const LadspaPortDescriptor,
    pub port_names: *const *const c_char,
    pub port_range_hints: *const LadspaPortRangeHint,
    pub implementation_data: *mut c_void,
    pub instantiate: Option<unsafe extern "C" fn(*const LadspaDescriptor, c_ulong) -> LadspaHandle>,
    pub connect_port: Option<unsafe extern "C" fn(LadspaHandle, c_ulong, *mut LadspaData)>,
    pub activate: Option<unsafe extern "C" fn(LadspaHandle)>,
    pub run: Option<unsafe extern "C" fn(LadspaHandle, c_ulong)>,
    pub run_adding: Option<unsafe extern "C" fn(LadspaHandle, c_ulong)>,
    pub set_run_adding_gain: Option<unsafe extern "C" fn(LadspaHandle, LadspaData)>,
    pub deactivate: Option<unsafe extern "C" fn(LadspaHandle)>,
    pub cleanup: Option<unsafe extern "C" fn(LadspaHandle)>,
}

// Descriptor is read-only static metadata after compile time.
unsafe impl Sync for LadspaDescriptor {}

struct SyncPortNames([*const c_char; PORT_COUNT]);
unsafe impl Sync for SyncPortNames {}

static PORT_DESCRIPTORS: [LadspaPortDescriptor; PORT_COUNT] = [
    LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO,    // 0: In L
    LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO,    // 1: In R
    LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO,   // 2: Out L
    LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO,   // 3: Out R
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 4: XTC Mix
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 5: Dry Mix
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 6: Bass Harmonics
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 7: Harmonic Warmth
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 8: Loudness Contour
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 9: Drive (dB)
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 10: Ceiling (dBFS)
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // 11: Enable Limiter
    LADSPA_PORT_OUTPUT | LADSPA_PORT_CONTROL, // 12: latency
];

static PORT_NAMES: SyncPortNames = SyncPortNames([
    c"In L".as_ptr(),
    c"In R".as_ptr(),
    c"Out L".as_ptr(),
    c"Out R".as_ptr(),
    c"XTC Mix".as_ptr(),
    c"Dry Mix".as_ptr(),
    c"Bass Harmonics".as_ptr(),
    c"Harmonic Warmth".as_ptr(),
    c"Loudness Contour".as_ptr(),
    c"Drive (dB)".as_ptr(),
    c"Ceiling (dBFS)".as_ptr(),
    c"Enable Limiter".as_ptr(),
    c"latency".as_ptr(),
]);

static PORT_RANGE_HINTS: [LadspaPortRangeHint; PORT_COUNT] = [
    LadspaPortRangeHint { hint_descriptor: 0, lower_bound: 0.0, upper_bound: 0.0 },
    LadspaPortRangeHint { hint_descriptor: 0, lower_bound: 0.0, upper_bound: 0.0 },
    LadspaPortRangeHint { hint_descriptor: 0, lower_bound: 0.0, upper_bound: 0.0 },
    LadspaPortRangeHint { hint_descriptor: 0, lower_bound: 0.0, upper_bound: 0.0 },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_1,
        lower_bound: 0.0,
        upper_bound: 1.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_1,
        lower_bound: 0.0,
        upper_bound: 1.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_LOW,
        lower_bound: 0.0,
        upper_bound: 1.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_LOW,
        lower_bound: 0.0,
        upper_bound: 1.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_LOW,
        lower_bound: 0.0,
        upper_bound: 1.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_MIDDLE,
        lower_bound: 0.0,
        upper_bound: 12.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_HIGH,
        lower_bound: -9.0,
        upper_bound: -0.5,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_TOGGLED | LADSPA_HINT_DEFAULT_1,
        lower_bound: 0.0,
        upper_bound: 1.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_0,
        lower_bound: 0.0,
        upper_bound: 1024.0,
    },
];

struct PluginInstance {
    engine: SpatialDspEngine,
    ports: [*mut LadspaData; PORT_COUNT],
}

unsafe extern "C" fn instantiate(
    _descriptor: *const LadspaDescriptor,
    sample_rate: c_ulong,
) -> LadspaHandle {
    let instance = Box::new(PluginInstance {
        engine: SpatialDspEngine::new(sample_rate as f64),
        ports: [ptr::null_mut(); PORT_COUNT],
    });
    Box::into_raw(instance) as LadspaHandle
}

unsafe extern "C" fn connect_port(
    instance: LadspaHandle,
    port: c_ulong,
    data_location: *mut LadspaData,
) {
    if instance.is_null() || (port as usize) >= PORT_COUNT {
        return;
    }
    let inst = &mut *(instance as *mut PluginInstance);
    inst.ports[port as usize] = data_location;
}

unsafe extern "C" fn activate(instance: LadspaHandle) {
    if instance.is_null() {
        return;
    }
    let inst = &mut *(instance as *mut PluginInstance);
    inst.engine.reset();
}

#[inline]
unsafe fn read_ctrl(ptr: *mut LadspaData, default: f32) -> f32 {
    if ptr.is_null() {
        default
    } else {
        let v = *ptr;
        if v.is_finite() { v } else { default }
    }
}

unsafe extern "C" fn run(instance: LadspaHandle, sample_count: c_ulong) {
    if instance.is_null() {
        return;
    }
    let inst = &mut *(instance as *mut PluginInstance);
    let in_l = inst.ports[0];
    let in_r = inst.ports[1];
    let out_l = inst.ports[2];
    let out_r = inst.ports[3];
    if in_l.is_null() || in_r.is_null() || out_l.is_null() || out_r.is_null() {
        return;
    }

    let params = SpatialParams {
        xtc_mix: read_ctrl(inst.ports[4], 1.0).clamp(0.0, 1.0),
        dry_mix: read_ctrl(inst.ports[5], 1.0).clamp(0.0, 1.0),
        bass_harmonics: read_ctrl(inst.ports[6], 0.38).clamp(0.0, 1.0),
        harmonic_warmth: read_ctrl(inst.ports[7], 0.16).clamp(0.0, 1.0),
        loudness_contour: read_ctrl(inst.ports[8], 0.25).clamp(0.0, 1.0),
        drive_db: read_ctrl(inst.ports[9], 7.0).clamp(0.0, 12.0),
        ceiling_dbfs: read_ctrl(inst.ports[10], -1.0).clamp(-9.0, -0.5),
        enable_limiter: read_ctrl(inst.ports[11], 1.0) >= 0.5,
        // External PipeWire graph applies 180 Hz LSP HP when used in stereo mode;
        // when used standalone/7.1 with limiter active, protect drivers inside plugin too.
        enable_driver_hp: read_ctrl(inst.ports[11], 1.0) >= 0.5,
    };

    if !inst.ports[12].is_null() {
        *inst.ports[12] = inst.engine.latency_samples(&params) as f32;
    }

    let n = sample_count as usize;
    for i in 0..n {
        let xl = *in_l.add(i);
        let xr = *in_r.add(i);
        let (yl, yr) = inst.engine.process_sample(xl, xr, &params);
        *out_l.add(i) = yl;
        *out_r.add(i) = yr;
    }
}

unsafe extern "C" fn deactivate(_instance: LadspaHandle) {}

unsafe extern "C" fn cleanup(instance: LadspaHandle) {
    if instance.is_null() {
        return;
    }
    drop(Box::from_raw(instance as *mut PluginInstance));
}

static DESCRIPTOR: LadspaDescriptor = LadspaDescriptor {
    unique_id: 5_198_662, // 0x004F5346 ("OSF" = Omarchy Spatial FDTR)
    label: c"omarchy_spatial_fdtr".as_ptr(),
    properties: LADSPA_PROPERTY_REALTIME | LADSPA_PROPERTY_HARD_RT_CAPABLE,
    name: c"Omarchy XPS 17 9720 Spatial FDTR-XTC & Warmth".as_ptr(),
    maker: c"Omarchy Spatial Audio (Dell XPS 17 9720)".as_ptr(),
    copyright: c"MIT".as_ptr(),
    port_count: PORT_COUNT as c_ulong,
    port_descriptors: PORT_DESCRIPTORS.as_ptr(),
    port_names: PORT_NAMES.0.as_ptr(),
    port_range_hints: PORT_RANGE_HINTS.as_ptr(),
    implementation_data: ptr::null_mut(),
    instantiate: Some(instantiate),
    connect_port: Some(connect_port),
    activate: Some(activate),
    run: Some(run),
    run_adding: None,
    set_run_adding_gain: None,
    deactivate: Some(deactivate),
    cleanup: Some(cleanup),
};

#[no_mangle]
pub extern "C" fn ladspa_descriptor(index: c_ulong) -> *const LadspaDescriptor {
    if index == 0 {
        &DESCRIPTOR as *const LadspaDescriptor
    } else {
        ptr::null()
    }
}
