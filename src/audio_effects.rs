use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};
use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Ui, Vec2};

pub const AUDIO_SAMPLE_RATE: f32 = 48000.0;

// ============================================================================
// 1. BIQUAD FILTER & 4-BAND PARAMETRIC EQ (Robert Bristow-Johnson Cookbook)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FilterType {
    LowPass,
    HighPass,
    LowShelf,
    HighShelf,
    Peaking,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BiquadFilter {
    pub filter_type: FilterType,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub enabled: bool,

    // Коэффициенты прямого преобразования (Direct Form II Transposed)
    #[serde(skip)]
    b0: f32,
    #[serde(skip)]
    b1: f32,
    #[serde(skip)]
    b2: f32,
    #[serde(skip)]
    a1: f32,
    #[serde(skip)]
    a2: f32,

    // Внутреннее состояние задержки для левого и правого каналов
    #[serde(skip)]
    s1_l: f32,
    #[serde(skip)]
    s2_l: f32,
    #[serde(skip)]
    s1_r: f32,
    #[serde(skip)]
    s2_r: f32,
}

impl BiquadFilter {
    pub fn new(filter_type: FilterType, freq_hz: f32, gain_db: f32, q: f32) -> Self {
        let mut filter = Self {
            filter_type,
            freq_hz: freq_hz.clamp(20.0, 20000.0),
            gain_db: gain_db.clamp(-24.0, 24.0),
            q: q.clamp(0.1, 10.0),
            enabled: true,
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            s1_l: 0.0,
            s2_l: 0.0,
            s1_r: 0.0,
            s2_r: 0.0,
        };
        filter.recalculate_coefficients(AUDIO_SAMPLE_RATE);
        filter
    }

    pub fn recalculate_coefficients(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        let omega = 2.0 * std::f32::consts::PI * (self.freq_hz / sr).clamp(0.0001, 0.499);
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * self.q.max(0.01));
        let a = 10.0f32.powf(self.gain_db / 40.0); // sqrt(A)

        let (b0, b1, b2, a0, a1, a2) = match self.filter_type {
            FilterType::LowPass => {
                let b1 = 1.0 - cos_omega;
                let b0 = b1 * 0.5;
                let b2 = b0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_omega;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::HighPass => {
                let b1 = -(1.0 + cos_omega);
                let b0 = (1.0 + cos_omega) * 0.5;
                let b2 = b0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_omega;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::Peaking => {
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cos_omega;
                let b2 = 1.0 - alpha * a;
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cos_omega;
                let a2 = 1.0 - alpha / a;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::LowShelf => {
                let sqrt_a = a.sqrt();
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos_omega + 2.0 * sqrt_a * alpha);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_omega);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_omega - 2.0 * sqrt_a * alpha);
                let a0 = (a + 1.0) + (a - 1.0) * cos_omega + 2.0 * sqrt_a * alpha;
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_omega);
                let a2 = (a + 1.0) + (a - 1.0) * cos_omega - 2.0 * sqrt_a * alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::HighShelf => {
                let sqrt_a = a.sqrt();
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos_omega + 2.0 * sqrt_a * alpha);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_omega);
                let b2 = a * ((a + 1.0) + (a - 1.0) * cos_omega - 2.0 * sqrt_a * alpha);
                let a0 = (a + 1.0) - (a - 1.0) * cos_omega + 2.0 * sqrt_a * alpha;
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_omega);
                let a2 = (a + 1.0) - (a - 1.0) * cos_omega - 2.0 * sqrt_a * alpha;
                (b0, b1, b2, a0, a1, a2)
            }
        };

        let inv_a0 = 1.0 / a0.max(1e-7);
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    #[inline]
    pub fn process_sample(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if !self.enabled {
            return (in_l, in_r);
        }

        // Left channel: Direct Form II Transposed
        let out_l = self.b0 * in_l + self.s1_l;
        self.s1_l = self.b1 * in_l - self.a1 * out_l + self.s2_l;
        self.s2_l = self.b2 * in_l - self.a2 * out_l;

        // Right channel: Direct Form II Transposed
        let out_r = self.b0 * in_r + self.s1_r;
        self.s1_r = self.b1 * in_r - self.a1 * out_r + self.s2_r;
        self.s2_r = self.b2 * in_r - self.a2 * out_r;

        (out_l, out_r)
    }

    /// Амплитудный отклик (Magnitude response в дБ) на частоте `f_hz`
    pub fn magnitude_response_db(&self, f_hz: f32, sample_rate: f32) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let omega = 2.0 * std::f32::consts::PI * (f_hz / sample_rate);
        let cos1 = omega.cos();
        let sin1 = omega.sin();
        let cos2 = (2.0 * omega).cos();
        let sin2 = (2.0 * omega).sin();

        let num_re = self.b0 + self.b1 * cos1 + self.b2 * cos2;
        let num_im = -self.b1 * sin1 - self.b2 * sin2;
        let den_re = 1.0 + self.a1 * cos1 + self.a2 * cos2;
        let den_im = -self.a1 * sin1 - self.a2 * sin2;

        let num_mag2 = num_re * num_re + num_im * num_im;
        let den_mag2 = den_re * den_re + den_im * den_im;

        if den_mag2 <= 1e-12 {
            return 0.0;
        }
        let mag2 = (num_mag2 / den_mag2).max(1e-12);
        10.0 * mag2.log10()
    }
}

/// 4-полосный студийный параметрический эквалайзер DaVinci Fairlight
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParametricEq {
    pub enabled: bool,
    pub bands: [BiquadFilter; 4],
}

impl Default for ParametricEq {
    fn default() -> Self {
        Self {
            enabled: true,
            bands: [
                BiquadFilter::new(FilterType::LowShelf, 80.0, 0.0, 0.71),
                BiquadFilter::new(FilterType::Peaking, 350.0, 0.0, 1.0),
                BiquadFilter::new(FilterType::Peaking, 2400.0, 0.0, 1.0),
                BiquadFilter::new(FilterType::HighShelf, 8000.0, 0.0, 0.71),
            ],
        }
    }
}

impl ParametricEq {
    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        if !self.enabled {
            return;
        }
        for chunk in buffer.chunks_exact_mut(2) {
            let mut l = chunk[0];
            let mut r = chunk[1];
            for band in &mut self.bands {
                let (nl, nr) = band.process_sample(l, r);
                l = nl;
                r = nr;
            }
            chunk[0] = l;
            chunk[1] = r;
        }
    }

    pub fn total_magnitude_response_db(&self, f_hz: f32) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let mut sum = 0.0;
        for band in &self.bands {
            sum += band.magnitude_response_db(f_hz, AUDIO_SAMPLE_RATE);
        }
        sum
    }
}

// ============================================================================
// 2. STUDIO DYNAMICS & COMPRESSOR / LIMITER
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynamicsCompressor {
    pub enabled: bool,
    pub threshold_db: f32, // -60.0 .. 0.0 dB
    pub ratio: f32,        // 1.0 .. 20.0
    pub attack_ms: f32,    // 0.1 .. 100.0 ms
    pub release_ms: f32,   // 10.0 .. 1000.0 ms
    pub makeup_gain_db: f32, // 0.0 .. 24.0 dB

    #[serde(skip)]
    envelope: f32,
    #[serde(skip)]
    pub current_gr_db: f32, // Live Gain Reduction (0.0 to -24.0 dB)
}

impl Default for DynamicsCompressor {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_db: -18.0,
            ratio: 3.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_gain_db: 2.0,
            envelope: 0.0,
            current_gr_db: 0.0,
        }
    }
}

impl DynamicsCompressor {
    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        if !self.enabled {
            self.current_gr_db = 0.0;
            return;
        }

        let attack_coeff = (-1.0 / (self.attack_ms.max(0.1) * 0.001 * AUDIO_SAMPLE_RATE)).exp();
        let release_coeff = (-1.0 / (self.release_ms.max(1.0) * 0.001 * AUDIO_SAMPLE_RATE)).exp();
        let makeup_linear = 10.0f32.powf(self.makeup_gain_db / 20.0);
        let ratio_inv = 1.0 / self.ratio.max(1.0);

        let mut max_gr = 0.0f32;

        for chunk in buffer.chunks_exact_mut(2) {
            let in_l = chunk[0];
            let in_r = chunk[1];
            let peak = in_l.abs().max(in_r.abs()).max(1e-6);

            // Ballistic peak envelope follower
            if peak > self.envelope {
                self.envelope = attack_coeff * self.envelope + (1.0 - attack_coeff) * peak;
            } else {
                self.envelope = release_coeff * self.envelope + (1.0 - release_coeff) * peak;
            }

            let env_db = 20.0 * self.envelope.log10();
            let gr_db = if env_db > self.threshold_db {
                (env_db - self.threshold_db) * (ratio_inv - 1.0)
            } else {
                0.0
            };

            if gr_db < max_gr {
                max_gr = gr_db;
            }

            let gain_mult = 10.0f32.powf(gr_db / 20.0) * makeup_linear;
            chunk[0] = (in_l * gain_mult).clamp(-1.0, 1.0);
            chunk[1] = (in_r * gain_mult).clamp(-1.0, 1.0);
        }

        self.current_gr_db = max_gr;
    }
}

// ============================================================================
// 3. STUDIO REVERB / DELAY (Freeverb comb & allpass topology)
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StudioReverb {
    pub enabled: bool,
    pub room_size: f32, // 0.0 .. 1.0
    pub damping: f32,   // 0.0 .. 1.0
    pub wet_mix: f32,   // 0.0 .. 1.0
    pub dry_mix: f32,   // 0.0 .. 1.0

    #[serde(skip)]
    comb_buffers: Vec<Vec<f32>>,
    #[serde(skip)]
    comb_idx: Vec<usize>,
    #[serde(skip)]
    comb_filter_store: Vec<f32>,
}

impl Default for StudioReverb {
    fn default() -> Self {
        let comb_lens = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        let mut buffers = Vec::new();
        let mut indices = Vec::new();
        let mut stores = Vec::new();
        for &len in &comb_lens {
            buffers.push(vec![0.0f32; len]);
            indices.push(0);
            stores.push(0.0);
        }

        Self {
            enabled: false,
            room_size: 0.5,
            damping: 0.3,
            wet_mix: 0.25,
            dry_mix: 0.8,
            comb_buffers: buffers,
            comb_idx: indices,
            comb_filter_store: stores,
        }
    }
}

impl StudioReverb {
    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        if !self.enabled || self.comb_buffers.is_empty() {
            return;
        }

        let feedback = (self.room_size * 0.28 + 0.7).clamp(0.0, 0.98);
        let damp1 = self.damping * 0.4;
        let damp2 = 1.0 - damp1;
        let wet = self.wet_mix;
        let dry = self.dry_mix;

        for chunk in buffer.chunks_exact_mut(2) {
            let mono_in = (chunk[0] + chunk[1]) * 0.5;
            let mut rev_out = 0.0f32;

            for i in 0..self.comb_buffers.len() {
                let buf = &mut self.comb_buffers[i];
                let idx = self.comb_idx[i];
                let output = buf[idx];
                self.comb_filter_store[i] = output * damp2 + self.comb_filter_store[i] * damp1;
                buf[idx] = mono_in + self.comb_filter_store[i] * feedback;
                self.comb_idx[i] = (idx + 1) % buf.len();
                rev_out += output;
            }

            rev_out *= 0.125;
            chunk[0] = (chunk[0] * dry + rev_out * wet).clamp(-1.0, 1.0);
            chunk[1] = (chunk[1] * dry + rev_out * wet).clamp(-1.0, 1.0);
        }
    }
}

// ============================================================================
// 4. LINUX VST2 PLUGIN HOST C-ABI SPECIFICATION & DYNAMIC LOADER
// ============================================================================

/// Стандартный C-ABI VST 2.4 AEffect
#[repr(C)]
pub struct AEffect {
    pub magic: i32, // 'VstP' = 0x56737450
    pub dispatcher: extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize,
    pub process: extern "C" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32),
    pub set_parameter: extern "C" fn(*mut AEffect, i32, f32),
    pub get_parameter: extern "C" fn(*mut AEffect, i32) -> f32,
    pub num_programs: i32,
    pub num_params: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub flags: i32,
    pub resvd1: isize,
    pub resvd2: isize,
    pub initial_delay: i32,
    pub real_qualities: i32,
    pub off_qualities: i32,
    pub io_ratio: f32,
    pub object: *mut c_void,
    pub user: *mut c_void,
    pub unique_id: i32,
    pub version: i32,
    pub process_replacing: extern "C" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32),
    pub process_double_replacing: Option<unsafe extern "C" fn(*mut AEffect, *mut *mut f64, *mut *mut f64, i32)>,
}

pub const VST_MAGIC: i32 = 0x56737450;
pub const EFF_OPEN: i32 = 0;
pub const EFF_CLOSE: i32 = 1;
pub const EFF_SET_SAMPLE_RATE: i32 = 10;
pub const EFF_SET_BLOCK_SIZE: i32 = 11;
pub const EFF_MAINS_CHANGED: i32 = 12;
pub const EFF_GET_PARAM_NAME: i32 = 8;
pub const EFF_GET_PARAM_DISPLAY: i32 = 7;
pub const EFF_GET_EFFECT_NAME: i32 = 45;
pub const EFF_GET_VENDOR_STRING: i32 = 47;
pub const EFF_GET_PRODUCT_STRING: i32 = 48;

/// Host Callback для VST плагина
extern "C" fn host_audio_master(
    _effect: *mut AEffect,
    opcode: i32,
    _index: i32,
    _value: isize,
    _ptr: *mut c_void,
    _opt: f32,
) -> isize {
    // 1 = audioMasterVersion
    if opcode == 1 {
        return 2400; // VST 2.4
    }
    0
}

#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub name: String,
    pub vendor: String,
    pub path: PathBuf,
    pub is_vst3: bool,
}

pub struct VstParameterInfo {
    pub index: i32,
    pub name: String,
    pub display: String,
    pub value: f32,
}

pub struct VstHostInstance {
    pub path: PathBuf,
    pub name: String,
    pub enabled: bool,
    lib_handle: *mut c_void,
    effect: *mut AEffect,
    pub num_params: i32,
    left_in: Vec<f32>,
    right_in: Vec<f32>,
    left_out: Vec<f32>,
    right_out: Vec<f32>,
}

unsafe impl Send for VstHostInstance {}
unsafe impl Sync for VstHostInstance {}

impl VstHostInstance {
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;

        unsafe {
            let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                return Err(format!("dlopen failed: {}", CStr::from_ptr(libc::dlerror()).to_string_lossy()));
            }

            // Поиск точки входа VSTPluginMain или main
            let mut entry_sym = libc::dlsym(handle, b"VSTPluginMain\0".as_ptr() as *const _);
            if entry_sym.is_null() {
                entry_sym = libc::dlsym(handle, b"main\0".as_ptr() as *const _);
            }

            if entry_sym.is_null() {
                libc::dlclose(handle);
                return Err("No VST entry point found (VSTPluginMain or main)".to_string());
            }

            type VstEntryFn = extern "C" fn(extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize) -> *mut AEffect;
            let entry_fn: VstEntryFn = std::mem::transmute(entry_sym);
            let effect = entry_fn(host_audio_master);

            if effect.is_null() || (*effect).magic != VST_MAGIC {
                libc::dlclose(handle);
                return Err("Invalid VST effect magic or null instance".to_string());
            }

            // Инициализация плагина
            ((*effect).dispatcher)(effect, EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
            ((*effect).dispatcher)(effect, EFF_SET_SAMPLE_RATE, 0, 0, std::ptr::null_mut(), AUDIO_SAMPLE_RATE);
            ((*effect).dispatcher)(effect, EFF_SET_BLOCK_SIZE, 0, 1024, std::ptr::null_mut(), 0.0);
            ((*effect).dispatcher)(effect, EFF_MAINS_CHANGED, 0, 1, std::ptr::null_mut(), 0.0); // Resume audio

            let mut name_buf = [0u8; 64];
            ((*effect).dispatcher)(effect, EFF_GET_EFFECT_NAME, 0, 0, name_buf.as_mut_ptr() as *mut c_void, 0.0);
            let name_str = CStr::from_ptr(name_buf.as_ptr() as *const _).to_string_lossy().into_owned();
            let final_name = if name_str.trim().is_empty() {
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("VST Plugin").to_string()
            } else {
                name_str
            };

            let num_params = (*effect).num_params;

            Ok(Self {
                path: path.to_path_buf(),
                name: final_name,
                enabled: true,
                lib_handle: handle,
                effect,
                num_params,
                left_in: vec![0.0f32; 2048],
                right_in: vec![0.0f32; 2048],
                left_out: vec![0.0f32; 2048],
                right_out: vec![0.0f32; 2048],
            })
        }
    }

    pub fn get_parameter(&self, index: i32) -> f32 {
        if self.effect.is_null() || index < 0 || index >= self.num_params {
            return 0.0;
        }
        unsafe { ((*self.effect).get_parameter)(self.effect, index) }
    }

    pub fn set_parameter(&self, index: i32, val: f32) {
        if self.effect.is_null() || index < 0 || index >= self.num_params {
            return;
        }
        unsafe { ((*self.effect).set_parameter)(self.effect, index, val.clamp(0.0, 1.0)) };
    }

    pub fn get_parameter_info(&self, index: i32) -> VstParameterInfo {
        let val = self.get_parameter(index);
        let mut name_buf = [0u8; 32];
        let mut disp_buf = [0u8; 32];

        unsafe {
            if !self.effect.is_null() {
                ((*self.effect).dispatcher)(self.effect, EFF_GET_PARAM_NAME, index, 0, name_buf.as_mut_ptr() as *mut c_void, 0.0);
                ((*self.effect).dispatcher)(self.effect, EFF_GET_PARAM_DISPLAY, index, 0, disp_buf.as_mut_ptr() as *mut c_void, 0.0);
            }
            let name = CStr::from_ptr(name_buf.as_ptr() as *const _).to_string_lossy().into_owned();
            let disp = CStr::from_ptr(disp_buf.as_ptr() as *const _).to_string_lossy().into_owned();
            VstParameterInfo {
                index,
                name: if name.trim().is_empty() { format!("Param {}", index) } else { name },
                display: disp,
                value: val,
            }
        }
    }

    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        if !self.enabled || self.effect.is_null() {
            return;
        }
        let frames = buffer.len() / 2;
        if frames == 0 {
            return;
        }

        if self.left_in.len() < frames {
            self.left_in.resize(frames, 0.0);
            self.right_in.resize(frames, 0.0);
            self.left_out.resize(frames, 0.0);
            self.right_out.resize(frames, 0.0);
        }

        for (i, chunk) in buffer.chunks_exact(2).enumerate() {
            self.left_in[i] = chunk[0];
            self.right_in[i] = chunk[1];
        }

        unsafe {
            let mut in_ptrs = [self.left_in.as_mut_ptr(), self.right_in.as_mut_ptr()];
            let mut out_ptrs = [self.left_out.as_mut_ptr(), self.right_out.as_mut_ptr()];

            ((*self.effect).process_replacing)(
                self.effect,
                in_ptrs.as_mut_ptr(),
                out_ptrs.as_mut_ptr(),
                frames as i32,
            );
        }

        for (i, chunk) in buffer.chunks_exact_mut(2).enumerate() {
            chunk[0] = self.left_out[i].clamp(-1.0, 1.0);
            chunk[1] = self.right_out[i].clamp(-1.0, 1.0);
        }
    }
}

impl Drop for VstHostInstance {
    fn drop(&mut self) {
        unsafe {
            if !self.effect.is_null() {
                ((*self.effect).dispatcher)(self.effect, EFF_MAINS_CHANGED, 0, 0, std::ptr::null_mut(), 0.0);
                ((*self.effect).dispatcher)(self.effect, EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
            }
            if !self.lib_handle.is_null() {
                libc::dlclose(self.lib_handle);
            }
        }
    }
}

/// Сканер установленных в Linux VST плагинов
pub fn scan_system_vst_plugins() -> Vec<DiscoveredPlugin> {
    let mut plugins = Vec::new();
    let mut search_dirs = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        let home_p = PathBuf::from(home);
        search_dirs.push(home_p.join(".vst"));
        search_dirs.push(home_p.join(".vst3"));
    }
    search_dirs.push(PathBuf::from("/usr/lib/vst"));
    search_dirs.push(PathBuf::from("/usr/lib/lxvst"));
    search_dirs.push(PathBuf::from("/usr/lib/vst3"));
    search_dirs.push(PathBuf::from("/usr/local/lib/vst"));
    search_dirs.push(PathBuf::from("/usr/local/lib/vst3"));

    for dir in search_dirs {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let is_so = p.extension().map(|e| e == "so").unwrap_or(false);
                let is_vst3 = p.extension().map(|e| e == "vst3").unwrap_or(false);

                if is_so || is_vst3 {
                    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Plugin").to_string();
                    plugins.push(DiscoveredPlugin {
                        name,
                        vendor: "Linux Audio".to_string(),
                        path: p,
                        is_vst3,
                    });
                }
            }
        }
    }

    plugins
}

// ============================================================================
// 5. TRACK & MASTER AUDIO FX RACK
// ============================================================================

pub struct TrackAudioRack {
    pub track_index: usize,
    pub volume_db: f32, // -60.0 .. +12.0 dB
    pub pan: f32,       // -1.0 .. +1.0
    pub is_muted: bool,
    pub is_solo: bool,
    pub eq: ParametricEq,
    pub dynamics: DynamicsCompressor,
    pub reverb: StudioReverb,
    pub vst: Option<VstHostInstance>,
    pub peak_l: f32,
    pub peak_r: f32,
}

impl TrackAudioRack {
    pub fn new(track_index: usize) -> Self {
        Self {
            track_index,
            volume_db: 0.0,
            pan: 0.0,
            is_muted: false,
            is_solo: false,
            eq: ParametricEq::default(),
            dynamics: DynamicsCompressor::default(),
            reverb: StudioReverb::default(),
            vst: None,
            peak_l: 0.0,
            peak_r: 0.0,
        }
    }

    pub fn process(&mut self, buffer: &mut [f32]) {
        if self.is_muted {
            buffer.fill(0.0);
            self.peak_l = 0.0;
            self.peak_r = 0.0;
            return;
        }

        self.eq.process_interleaved(buffer);
        self.dynamics.process_interleaved(buffer);
        self.reverb.process_interleaved(buffer);

        if let Some(ref mut vst) = self.vst {
            vst.process_interleaved(buffer);
        }

        let vol_mult = 10.0f32.powf(self.volume_db / 20.0);
        let pan_l = ((1.0 - self.pan) * 0.5).clamp(0.0, 1.0);
        let pan_r = ((1.0 + self.pan) * 0.5).clamp(0.0, 1.0);

        let mut max_l = 0.0f32;
        let mut max_r = 0.0f32;

        for chunk in buffer.chunks_exact_mut(2) {
            let l = chunk[0] * vol_mult * pan_l;
            let r = chunk[1] * vol_mult * pan_r;
            chunk[0] = l.clamp(-1.0, 1.0);
            chunk[1] = r.clamp(-1.0, 1.0);

            if l.abs() > max_l {
                max_l = l.abs();
            }
            if r.abs() > max_r {
                max_r = r.abs();
            }
        }

        self.peak_l = max_l;
        self.peak_r = max_r;
    }
}

pub struct MasterAudioRack {
    pub volume_db: f32,
    pub eq: ParametricEq,
    pub dynamics: DynamicsCompressor,
    pub reverb: StudioReverb,
    pub vst: Option<VstHostInstance>,
    pub peak_l: f32,
    pub peak_r: f32,
}

impl Default for MasterAudioRack {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            eq: ParametricEq::default(),
            dynamics: DynamicsCompressor {
                threshold_db: -2.0,
                ratio: 10.0,
                attack_ms: 1.0,
                release_ms: 50.0,
                makeup_gain_db: 0.0,
                ..Default::default()
            },
            reverb: StudioReverb::default(),
            vst: None,
            peak_l: 0.0,
            peak_r: 0.0,
        }
    }
}

impl MasterAudioRack {
    pub fn process(&mut self, buffer: &mut [f32]) {
        self.eq.process_interleaved(buffer);
        self.dynamics.process_interleaved(buffer);
        self.reverb.process_interleaved(buffer);

        if let Some(ref mut vst) = self.vst {
            vst.process_interleaved(buffer);
        }

        let vol_mult = 10.0f32.powf(self.volume_db / 20.0);
        let mut max_l = 0.0f32;
        let mut max_r = 0.0f32;

        for chunk in buffer.chunks_exact_mut(2) {
            let l = (chunk[0] * vol_mult).clamp(-1.0, 1.0);
            let r = (chunk[1] * vol_mult).clamp(-1.0, 1.0);
            chunk[0] = l;
            chunk[1] = r;

            if l.abs() > max_l {
                max_l = l.abs();
            }
            if r.abs() > max_r {
                max_r = r.abs();
            }
        }

        self.peak_l = max_l;
        self.peak_r = max_r;
    }
}

// ============================================================================
// 6. AUDIO WORKSTATION & FAIRLIGHT MIXER STATE
// ============================================================================

pub struct FairlightWorkstationState {
    pub track_racks: Vec<TrackAudioRack>,
    pub master_rack: MasterAudioRack,
    pub discovered_vsts: Vec<DiscoveredPlugin>,
    pub selected_channel: Option<usize>, // None = Master, Some(idx) = Track A1, A2...
    pub is_vst_modal_open: bool,
    pub scan_performed: bool,
}

impl FairlightWorkstationState {
    pub fn new() -> Self {
        let plugins = scan_system_vst_plugins();
        let mut racks = Vec::new();
        for i in 0..8 {
            racks.push(TrackAudioRack::new(i));
        }

        Self {
            track_racks: racks,
            master_rack: MasterAudioRack::default(),
            discovered_vsts: plugins,
            selected_channel: None,
            is_vst_modal_open: false,
            scan_performed: true,
        }
    }

    pub fn ensure_track_count(&mut self, count: usize) {
        while self.track_racks.len() < count {
            let idx = self.track_racks.len();
            self.track_racks.push(TrackAudioRack::new(idx));
        }
    }

    pub fn rescan_plugins(&mut self) {
        self.discovered_vsts = scan_system_vst_plugins();
        self.scan_performed = true;
    }

    /// Отрисовка микшерного пульта (Mixer Console)
    pub fn render_mixer_strip(
        ui: &mut Ui,
        title: &str,
        color: Color32,
        vol_db: &mut f32,
        pan: &mut f32,
        is_muted: &mut bool,
        is_solo: &mut bool,
        eq_active: bool,
        dyn_active: bool,
        peak_l: f32,
        peak_r: f32,
        gr_db: f32,
        is_selected: bool,
    ) -> bool {
        let mut clicked = false;
        let strip_width = 86.0;

        let frame = egui::Frame::new()
            .fill(if is_selected { Color32::from_rgb(0x22, 0x25, 0x30) } else { Color32::from_rgb(0x16, 0x18, 0x1e) })
            .stroke(Stroke::new(
                if is_selected { 1.5 } else { 1.0 },
                if is_selected { Color32::from_rgb(0xff, 0x88, 0x00) } else { Color32::from_rgb(0x28, 0x2b, 0x34) },
            ))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(6, 6));

        frame.show(ui, |ui| {
            ui.set_width(strip_width);
            ui.vertical_centered(|ui| {
                // Заголовок дорожки
                let hdr_btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new(title)
                            .size(11.5)
                            .strong()
                            .color(color),
                    )
                    .fill(Color32::from_rgb(0x1e, 0x20, 0x28))
                    .min_size(Vec2::new(strip_width - 8.0, 22.0)),
                );
                if hdr_btn.clicked() {
                    clicked = true;
                }

                ui.add_space(3.0);

                // Индикаторы EQ / DYN
                ui.horizontal(|ui| {
                    let eq_lbl = egui::RichText::new("EQ")
                        .size(9.0)
                        .color(if eq_active { Color32::from_rgb(0x40, 0xd0, 0x80) } else { Color32::from_rgb(0x55, 0x5b, 0x6a) });
                    ui.label(eq_lbl);

                    let dyn_lbl = egui::RichText::new("DYN")
                        .size(9.0)
                        .color(if dyn_active { Color32::from_rgb(0xff, 0xaa, 0x30) } else { Color32::from_rgb(0x55, 0x5b, 0x6a) });
                    ui.label(dyn_lbl);
                });

                // Gain reduction мини-индикатор
                if gr_db < -0.1 {
                    let gr_bar_h = (-gr_db * 2.0).clamp(2.0, 20.0);
                    let (gr_rect, _) = ui.allocate_exact_size(Vec2::new(strip_width - 12.0, 3.0), egui::Sense::hover());
                    ui.painter().rect_filled(gr_rect, CornerRadius::same(1), Color32::from_rgb(0x30, 0x15, 0x15));
                    let active_gr = Rect::from_min_size(gr_rect.min, Vec2::new(gr_bar_h, 3.0));
                    ui.painter().rect_filled(active_gr, CornerRadius::same(1), Color32::from_rgb(0xff, 0x44, 0x44));
                }

                ui.add_space(4.0);

                // Панорама
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Pan").size(9.5).color(Color32::from_rgb(0x8c, 0x92, 0xa4)));
                    ui.add(egui::Slider::new(pan, -1.0..=1.0).show_value(false));
                });

                ui.add_space(6.0);

                // Mute / Solo кнопки
                ui.horizontal(|ui| {
                    let m_btn = egui::Button::new(
                        egui::RichText::new("M")
                            .size(10.0)
                            .strong()
                            .color(if *is_muted { Color32::WHITE } else { Color32::from_rgb(0xa0, 0xa5, 0xb4) }),
                    )
                    .fill(if *is_muted { Color32::from_rgb(0xd0, 0x30, 0x30) } else { Color32::from_rgb(0x22, 0x25, 0x2e) })
                    .min_size(Vec2::new(32.0, 20.0));

                    if ui.add(m_btn).clicked() {
                        *is_muted = !*is_muted;
                    }

                    let s_btn = egui::Button::new(
                        egui::RichText::new("S")
                            .size(10.0)
                            .strong()
                            .color(if *is_solo { Color32::BLACK } else { Color32::from_rgb(0xa0, 0xa5, 0xb4) }),
                    )
                    .fill(if *is_solo { Color32::from_rgb(0xf0, 0xb0, 0x20) } else { Color32::from_rgb(0x22, 0x25, 0x2e) })
                    .min_size(Vec2::new(32.0, 20.0));

                    if ui.add(s_btn).clicked() {
                        *is_solo = !*is_solo;
                    }
                });

                ui.add_space(6.0);

                // Вертикальный фейдер и стерео VU-метр
                ui.horizontal(|ui| {
                    // Фейдер громкости
                    ui.add(
                        egui::Slider::new(vol_db, -60.0..=12.0)
                            .vertical()
                            .show_value(false),
                    );

                    // VU метр
                    let vu_height = 140.0;
                    let vu_width = 12.0;
                    let (meter_rect, _) = ui.allocate_exact_size(Vec2::new(vu_width, vu_height), egui::Sense::hover());

                    // Фон индикатора
                    ui.painter().rect_filled(meter_rect, CornerRadius::same(1), Color32::from_rgb(0x10, 0x11, 0x14));

                    let l_h = (peak_l * vu_height).clamp(0.0, vu_height);
                    let r_h = (peak_r * vu_height).clamp(0.0, vu_height);

                    let half_w = (vu_width - 1.0) * 0.5;
                    let l_rect = Rect::from_min_max(
                        Pos2::new(meter_rect.min.x, meter_rect.max.y - l_h),
                        Pos2::new(meter_rect.min.x + half_w, meter_rect.max.y),
                    );
                    let r_rect = Rect::from_min_max(
                        Pos2::new(meter_rect.min.x + half_w + 1.0, meter_rect.max.y - r_h),
                        Pos2::new(meter_rect.max.x, meter_rect.max.y),
                    );

                    let vu_color_l = if peak_l > 0.95 { Color32::RED } else if peak_l > 0.75 { Color32::YELLOW } else { Color32::GREEN };
                    let vu_color_r = if peak_r > 0.95 { Color32::RED } else if peak_r > 0.75 { Color32::YELLOW } else { Color32::GREEN };

                    ui.painter().rect_filled(l_rect, CornerRadius::ZERO, vu_color_l);
                    ui.painter().rect_filled(r_rect, CornerRadius::ZERO, vu_color_r);
                });

                ui.add_space(4.0);

                // Значение dB
                let db_text = if *vol_db <= -59.5 {
                    "-inf dB".to_string()
                } else {
                    format!("{:+.1} dB", *vol_db)
                };
                ui.label(egui::RichText::new(db_text).size(9.0).color(Color32::from_rgb(0xaa, 0xb0, 0xc0)));
            });
        });

        clicked
    }

    /// Интерактивный визуализатор параметрического эквалайзера DaVinci Fairlight
    pub fn render_eq_visualizer(ui: &mut Ui, eq: &mut ParametricEq) {
        let (graph_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 160.0), egui::Sense::hover());
        ui.painter().rect_filled(graph_rect, CornerRadius::same(4), Color32::from_rgb(0x12, 0x13, 0x17));
        ui.painter().rect_stroke(graph_rect, CornerRadius::same(4), Stroke::new(1.0, Color32::from_rgb(0x24, 0x27, 0x30)), egui::StrokeKind::Inside);

        let center_y = graph_rect.center().y;
        // 0 dB линия
        ui.painter().line_segment(
            [Pos2::new(graph_rect.min.x, center_y), Pos2::new(graph_rect.max.x, center_y)],
            Stroke::new(1.0, Color32::from_rgb(0x30, 0x35, 0x42)),
        );

        let min_f = 20.0f32;
        let max_f = 20000.0f32;
        let log_min = min_f.log10();
        let log_max = max_f.log10();
        let log_range = log_max - log_min;

        let steps = 120;
        let mut points = Vec::with_capacity(steps);
        for i in 0..steps {
            let t = i as f32 / (steps - 1) as f32;
            let f = 10.0f32.powf(log_min + t * log_range);
            let db = eq.total_magnitude_response_db(f);
            let y_norm = (db / 24.0).clamp(-1.0, 1.0);
            let px = graph_rect.min.x + t * graph_rect.width();
            let py = center_y - y_norm * (graph_rect.height() * 0.45);
            points.push(Pos2::new(px, py));
        }

        // Отрисовка кривой АЧХ
        for w in points.windows(2) {
            ui.painter().line_segment([w[0], w[1]], Stroke::new(2.0, Color32::from_rgb(0x40, 0xd0, 0x90)));
        }

        // Ручки управления для 4 полос
        let band_colors = [
            Color32::from_rgb(0x40, 0xa0, 0xff),
            Color32::from_rgb(0x40, 0xd0, 0x90),
            Color32::from_rgb(0xf0, 0xb0, 0x20),
            Color32::from_rgb(0xff, 0x60, 0x60),
        ];

        for (i, band) in eq.bands.iter().enumerate() {
            let t = (band.freq_hz.log10() - log_min) / log_range;
            let px = graph_rect.min.x + t * graph_rect.width();
            let y_norm = (band.gain_db / 24.0).clamp(-1.0, 1.0);
            let py = center_y - y_norm * (graph_rect.height() * 0.45);

            ui.painter().circle_filled(Pos2::new(px, py), 5.0, band_colors[i]);
            ui.painter().circle_stroke(Pos2::new(px, py), 5.0, Stroke::new(1.0, Color32::WHITE));
        }

        ui.add_space(6.0);

        // Ручки настройки частот и усиления
        ui.horizontal(|ui| {
            for (i, band) in eq.bands.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.set_width(110.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(format!("Band {}", i + 1)).size(10.5).color(band_colors[i]).strong());
                        ui.add(egui::Slider::new(&mut band.freq_hz, 20.0..=20000.0).logarithmic(true).text("Hz"));
                        ui.add(egui::Slider::new(&mut band.gain_db, -24.0..=24.0).text("dB"));
                        ui.add(egui::Slider::new(&mut band.q, 0.1..=10.0).text("Q"));
                        band.recalculate_coefficients(AUDIO_SAMPLE_RATE);
                    });
                });
            }
        });
    }

    /// Интерактивный блок VST плагинов
    pub fn render_vst_section(&mut self, ui: &mut Ui, target_rack_name: &str) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("🔌 VST Plugins Rack")
                            .size(12.5)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🔄 Rescan VST Plugins").clicked() {
                            self.rescan_plugins();
                        }
                    });
                });

                ui.separator();

                ui.label(
                    egui::RichText::new(format!("Target: {}", target_rack_name))
                        .size(11.0)
                        .color(Color32::from_rgb(0xa0, 0xa5, 0xb8)),
                );

                if self.discovered_vsts.is_empty() {
                    ui.label(
                        egui::RichText::new("В системе не найдены установленные Linux VST плагины (~/.vst, /usr/lib/vst, /usr/lib/vst3)")
                            .size(11.0)
                            .color(Color32::from_rgb(0x80, 0x85, 0x94)),
                    );
                } else {
                    egui::ComboBox::from_label("Load Plugin")
                        .selected_text("Select VST Plugin...")
                        .show_ui(ui, |ui| {
                            for p in &self.discovered_vsts {
                                let label = format!("{} ({})", p.name, if p.is_vst3 { "VST3" } else { "VST2" });
                                if ui.button(label).clicked() {
                                    if let Ok(inst) = VstHostInstance::load_from_path(&p.path) {
                                        if let Some(ch) = self.selected_channel {
                                            if let Some(rack) = self.track_racks.get_mut(ch) {
                                                rack.vst = Some(inst);
                                            }
                                        } else {
                                            self.master_rack.vst = Some(inst);
                                        }
                                    }
                                }
                            }
                        });
                }

                // Текущий загруженный плагин
                let cur_vst = if let Some(ch) = self.selected_channel {
                    self.track_racks.get_mut(ch).and_then(|r| r.vst.as_mut())
                } else {
                    self.master_rack.vst.as_mut()
                };

                if let Some(vst) = cur_vst {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("Active: {}", vst.name)).strong().color(Color32::from_rgb(0x40, 0xd0, 0x80)));
                        ui.checkbox(&mut vst.enabled, "Enabled");
                    });

                    // Параметры VST плагина
                    let num_p = vst.num_params.min(16);
                    for i in 0..num_p {
                        let info = vst.get_parameter_info(i);
                        let mut val = info.value;
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&info.name).size(10.5));
                            if ui.add(egui::Slider::new(&mut val, 0.0..=1.0)).changed() {
                                vst.set_parameter(i, val);
                            }
                            if !info.display.is_empty() {
                                ui.label(egui::RichText::new(&info.display).size(9.5).color(Color32::GRAY));
                            }
                        });
                    }
                }
            });
        });
    }

    /// Главный рендер панели Fairlight Workstation
    pub fn render_workstation(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🎚 DaVinci Fairlight Audio Console & VST Host")
                        .size(14.0)
                        .strong()
                        .color(Color32::WHITE),
                );
            });

            ui.add_space(6.0);

            // Микшерные линейки дорожек
            egui::ScrollArea::horizontal()
                .id_salt("fairlight_strips_scroll")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut selected_to_set = None;

                        for (i, rack) in self.track_racks.iter_mut().enumerate() {
                            let title = format!("A{}", i + 1);
                            let is_sel = self.selected_channel == Some(i);
                            let clicked = Self::render_mixer_strip(
                                ui,
                                &title,
                                Color32::from_rgb(0x35, 0x84, 0xe4),
                                &mut rack.volume_db,
                                &mut rack.pan,
                                &mut rack.is_muted,
                                &mut rack.is_solo,
                                rack.eq.enabled,
                                rack.dynamics.enabled,
                                rack.peak_l,
                                rack.peak_r,
                                rack.dynamics.current_gr_db,
                                is_sel,
                            );
                            if clicked {
                                selected_to_set = Some(Some(i));
                            }
                            ui.add_space(4.0);
                        }

                        // Мастер линейка (Main)
                        let is_master_sel = self.selected_channel.is_none();
                        let mut dummy_muted = false;
                        let mut dummy_solo = false;
                        let mut dummy_pan = 0.0;
                        let clicked_master = Self::render_mixer_strip(
                            ui,
                            "MAIN",
                            Color32::from_rgb(0xff, 0x78, 0x00),
                            &mut self.master_rack.volume_db,
                            &mut dummy_pan,
                            &mut dummy_muted,
                            &mut dummy_solo,
                            self.master_rack.eq.enabled,
                            self.master_rack.dynamics.enabled,
                            self.master_rack.peak_l,
                            self.master_rack.peak_r,
                            self.master_rack.dynamics.current_gr_db,
                            is_master_sel,
                        );
                        if clicked_master {
                            selected_to_set = Some(None);
                        }

                        if let Some(target) = selected_to_set {
                            self.selected_channel = target;
                        }
                    });
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            // Нижняя секция: Детальный редактор EQ и VST рэк для выбранного канала
            let selected_name = match self.selected_channel {
                Some(idx) => format!("Track A{}", idx + 1),
                None => "Master Bus (Main)".to_string(),
            };

            ui.label(
                egui::RichText::new(format!("Channel Inspector: {}", selected_name))
                    .size(13.0)
                    .strong()
                    .color(Color32::WHITE),
            );

            ui.columns(2, |cols| {
                // Левая колонка: 4-полосный EQ
                cols[0].vertical(|ui| {
                    ui.label(egui::RichText::new("4-Band Parametric EQ").strong().color(Color32::from_rgb(0x40, 0xd0, 0x90)));
                    if let Some(ch) = self.selected_channel {
                        if let Some(rack) = self.track_racks.get_mut(ch) {
                            Self::render_eq_visualizer(ui, &mut rack.eq);
                        }
                    } else {
                        Self::render_eq_visualizer(ui, &mut self.master_rack.eq);
                    }
                });

                // Правая колонка: VST плагины
                cols[1].vertical(|ui| {
                    self.render_vst_section(ui, &selected_name);
                });
            });
        });
    }
}

// ============================================================================
// 7. UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_biquad_coefficients_no_nan() {
        let mut filter = BiquadFilter::new(FilterType::Peaking, 1000.0, 6.0, 1.41);
        assert!(!filter.b0.is_nan());
        assert!(!filter.b1.is_nan());
        assert!(!filter.b2.is_nan());
        assert!(!filter.a1.is_nan());
        assert!(!filter.a2.is_nan());

        let (out_l, out_r) = filter.process_sample(0.5, -0.5);
        assert!(!out_l.is_nan());
        assert!(!out_r.is_nan());
    }

    #[test]
    fn test_parametric_eq_magnitude_response() {
        let mut eq = ParametricEq::default();
        eq.bands[1].gain_db = 12.0; // Boost 350 Hz by 12 dB
        eq.bands[1].recalculate_coefficients(AUDIO_SAMPLE_RATE);

        let resp_at_center = eq.total_magnitude_response_db(350.0);
        assert!(resp_at_center > 6.0, "Boost at center frequency must be positive");

        let mut buffer = vec![0.1f32; 512];
        eq.process_interleaved(&mut buffer);
        for s in buffer {
            assert!(!s.is_nan());
        }
    }

    #[test]
    fn test_dynamics_compressor() {
        let mut comp = DynamicsCompressor::default();
        comp.threshold_db = -20.0;
        comp.ratio = 4.0;

        let mut buffer = vec![0.8f32; 1024]; // High level signal
        comp.process_interleaved(&mut buffer);

        assert!(comp.current_gr_db < 0.0, "Compressor must engage on hot input");
    }

    #[test]
    fn test_vst_scanner_runs_without_panic() {
        let plugins = scan_system_vst_plugins();
        // Scanner runs cleanly without throwing or crashing
        let _ = plugins.len();
    }
}
