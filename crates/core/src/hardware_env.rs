//! Hardware Environment Detection & SafeLoad Guard
//!
//! Cross-platform hardware profiling with `cfg` flags:
//! - Apple Silicon: Unified memory, Accelerate framework, SIMD
//! - Windows/Linux + GPU: CUDA or Vulkan detection
//! - CPU-only: Force extra-small models, aggressive parallelism

use sysinfo::System;

// ────────────────────────────────────────────────────────
// Constants
// ────────────────────────────────────────────────────────

/// SafeLoad buffer — always reserve 4 GB for OS and other processes
const SAFE_LOAD_BUFFER_GB: f64 = 4.0;

/// KV cache overhead estimate in GB
const KV_CACHE_OVERHEAD_GB: f64 = 1.5;

/// Maximum RAM percentage a model may consume (80%)
const MAX_RAM_USAGE_PCT: f64 = 0.80;

// ────────────────────────────────────────────────────────
// Platform Detection
// ────────────────────────────────────────────────────────

/// Detected compute platform
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Platform {
    /// Apple Silicon with unified memory (M1-M5)
    AppleSilicon {
        chip: String,
        unified_memory_gb: u64,
        gpu_cores: u32,
        perf_cores: u32,
        efficiency_cores: u32,
    },
    /// Discrete GPU with CUDA support (NVIDIA)
    CudaGpu {
        device_name: String,
        vram_gb: f64,
        compute_capability: String,
    },
    /// Discrete GPU with Vulkan support (AMD/Intel/NVIDIA fallback)
    VulkanGpu {
        device_name: String,
        vram_gb: f64,
    },
    /// CPU-only — no GPU acceleration available
    CpuOnly {
        cpu_name: String,
        cores: usize,
        threads: usize,
    },
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::AppleSilicon { chip, unified_memory_gb, gpu_cores, .. } => {
                write!(f, "Apple Silicon {chip} ({unified_memory_gb} GB unified, {gpu_cores} GPU cores)")
            }
            Platform::CudaGpu { device_name, vram_gb, compute_capability } => {
                write!(f, "CUDA GPU: {device_name} ({vram_gb:.1} GB VRAM, CC {compute_capability})")
            }
            Platform::VulkanGpu { device_name, vram_gb } => {
                write!(f, "Vulkan GPU: {device_name} ({vram_gb:.1} GB VRAM)")
            }
            Platform::CpuOnly { cpu_name, cores, threads } => {
                write!(f, "CPU Only: {cpu_name} ({cores}C/{threads}T)")
            }
        }
    }
}

/// Hardware performance tier — determines model selection strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum PerformanceTier {
    /// <8 GB usable — 1B-3B models only
    ExtraSmall,
    /// 8-12 GB — up to 7B models
    Small,
    /// 12-20 GB — up to 14B models
    Medium,
    /// 20+ GB — 14B+ models, dual-model orchestration
    HighEnd,
}

impl std::fmt::Display for PerformanceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PerformanceTier::ExtraSmall => write!(f, "Extra-Small (<8 GB)"),
            PerformanceTier::Small => write!(f, "Small (8-12 GB)"),
            PerformanceTier::Medium => write!(f, "Medium (12-20 GB)"),
            PerformanceTier::HighEnd => write!(f, "High-End (20+ GB)"),
        }
    }
}

/// Recommended model configuration based on detected hardware
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelRecommendation {
    pub dev_model: &'static str,
    pub audit_model: &'static str,
    pub router_model: &'static str,
    pub max_context_tokens: u32,
}

impl PerformanceTier {
    /// Get recommended models for this tier
    pub fn recommended_models(&self) -> ModelRecommendation {
        match self {
            PerformanceTier::HighEnd => ModelRecommendation {
                dev_model: "qwen2.5-coder:14b-q8_0",
                audit_model: "deepseek-r1:14b",
                router_model: "qwen2.5:7b",
                max_context_tokens: 32768,
            },
            PerformanceTier::Medium => ModelRecommendation {
                dev_model: "qwen2.5-coder:7b",
                audit_model: "deepseek-r1:7b",
                router_model: "llama3.2:1b",
                max_context_tokens: 16384,
            },
            PerformanceTier::Small => ModelRecommendation {
                dev_model: "qwen2.5-coder:3b",
                audit_model: "phi-4:mini",
                router_model: "llama3.2:1b",
                max_context_tokens: 8192,
            },
            PerformanceTier::ExtraSmall => ModelRecommendation {
                dev_model: "llama3.2:3b",
                audit_model: "phi-4:mini",
                router_model: "llama3.2:1b",
                max_context_tokens: 4096,
            },
        }
    }
}

// ────────────────────────────────────────────────────────
// Model Weight Calculation
// ────────────────────────────────────────────────────────

/// Precise model RAM requirement
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelWeight {
    pub name: String,
    pub params_billions: f64,
    pub quant_bits: f64,
    pub model_weight_gb: f64,
    pub kv_cache_gb: f64,
    pub total_required_gb: f64,
}

impl ModelWeight {
    /// Required_RAM = (Params_B * Quant_Bits / 8) + KV_Cache_Buffer
    pub fn calculate(name: &str, params_b: f64, quant: f64) -> Self {
        let model_weight_gb = params_b * quant / 8.0;
        let total = model_weight_gb + KV_CACHE_OVERHEAD_GB;
        Self {
            name: name.to_string(),
            params_billions: params_b,
            quant_bits: quant,
            model_weight_gb,
            kv_cache_gb: KV_CACHE_OVERHEAD_GB,
            total_required_gb: total,
        }
    }

    /// Estimate from model name (parse params + quantization from tag)
    pub fn estimate(name: &str) -> Self {
        let params = extract_param_count(name).unwrap_or(7.0);
        let qbits = detect_quant_bits(name);
        Self::calculate(name, params, qbits)
    }
}

impl std::fmt::Display for ModelWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {:.1}B @ Q{:.0} = {:.1} GB + {:.1} GB KV = {:.1} GB",
            self.name, self.params_billions, self.quant_bits,
            self.model_weight_gb, self.kv_cache_gb, self.total_required_gb
        )
    }
}

// ────────────────────────────────────────────────────────
// SafeLoad Guard
// ────────────────────────────────────────────────────────

/// Result of SafeLoad check
#[derive(Debug, Clone)]
pub enum SafeLoadResult {
    /// Model is safe to load
    Safe {
        model: String,
        required_gb: f64,
        available_gb: f64,
    },
    /// Model fits but leaves little headroom
    Warning {
        model: String,
        required_gb: f64,
        available_gb: f64,
        message: String,
    },
    /// Model MUST NOT be loaded — would cause heavy swap
    Blocked {
        model: String,
        required_gb: f64,
        available_gb: f64,
        suggestion: ModelWeight,
    },
}

impl std::fmt::Display for SafeLoadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafeLoadResult::Safe { model, required_gb, available_gb } => {
                write!(f, "[SAFE] {model}: {required_gb:.1} GB required, {available_gb:.1} GB available")
            }
            SafeLoadResult::Warning { model, message, .. } => {
                write!(f, "[WARN] {model}: {message}")
            }
            SafeLoadResult::Blocked { model, required_gb, available_gb, suggestion } => {
                write!(
                    f,
                    "[BLOCKED] {model}: needs {required_gb:.1} GB but only {available_gb:.1} GB available.\n  Suggestion: {suggestion}"
                )
            }
        }
    }
}

// ────────────────────────────────────────────────────────
// Hardware Environment (main struct)
// ────────────────────────────────────────────────────────

/// Full hardware environment profile
pub struct HardwareEnv {
    sys: System,
    pub platform: Platform,
    pub tier: PerformanceTier,
    pub total_ram_gb: f64,
    pub available_ram_gb: f64,
    pub os: String,
    pub arch: String,
}

impl HardwareEnv {
    /// Detect hardware environment at startup
    pub fn detect() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();

        let total_ram_gb = sys.total_memory() as f64 / 1_073_741_824.0;
        let used_ram_gb = sys.used_memory() as f64 / 1_073_741_824.0;
        let available_ram_gb = total_ram_gb - used_ram_gb;

        let cpu_name = sys.cpus().first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown".into());
        let core_count = sys.cpus().len();

        let platform = detect_platform(&cpu_name, core_count, total_ram_gb);
        let tier = classify_tier(&platform, available_ram_gb);

        let os = std::env::consts::OS.to_string();
        let arch = std::env::consts::ARCH.to_string();

        Self {
            sys,
            platform,
            tier,
            total_ram_gb,
            available_ram_gb,
            os,
            arch,
        }
    }

    /// Refresh system metrics
    pub fn refresh(&mut self) {
        self.sys.refresh_memory();
        self.sys.refresh_cpu_all();

        let total = self.sys.total_memory() as f64 / 1_073_741_824.0;
        let used = self.sys.used_memory() as f64 / 1_073_741_824.0;
        self.total_ram_gb = total;
        self.available_ram_gb = total - used;
        self.tier = classify_tier(&self.platform, self.available_ram_gb);
    }

    /// CPU usage percentage
    pub fn cpu_usage(&self) -> f32 {
        self.sys.global_cpu_usage()
    }

    /// SafeLoad check: can this model be loaded without crashing?
    /// Rule: (ModelWeight + 4GB buffer) > Available RAM → BLOCKED
    pub fn safe_load(&mut self, model_name: &str) -> SafeLoadResult {
        self.refresh();
        let weight = ModelWeight::estimate(model_name);
        let required = weight.total_required_gb + SAFE_LOAD_BUFFER_GB;

        if required > self.available_ram_gb {
            // Find a quantized version that fits
            let suggestion = find_fitting_alternative(
                model_name,
                self.available_ram_gb - SAFE_LOAD_BUFFER_GB,
            );
            SafeLoadResult::Blocked {
                model: model_name.to_string(),
                required_gb: required,
                available_gb: self.available_ram_gb,
                suggestion,
            }
        } else if required > self.available_ram_gb * MAX_RAM_USAGE_PCT {
            SafeLoadResult::Warning {
                model: model_name.to_string(),
                required_gb: required,
                available_gb: self.available_ram_gb,
                message: format!(
                    "Will use {:.0}% of available RAM — may cause slowdowns",
                    (required / self.available_ram_gb) * 100.0
                ),
            }
        } else {
            SafeLoadResult::Safe {
                model: model_name.to_string(),
                required_gb: required,
                available_gb: self.available_ram_gb,
            }
        }
    }

    /// Filter models to only those that pass SafeLoad
    pub fn filter_loadable(&mut self, models: &[String]) -> Vec<(String, ModelWeight)> {
        self.refresh();
        let budget = self.available_ram_gb - SAFE_LOAD_BUFFER_GB;

        models.iter().filter_map(|name| {
            let weight = ModelWeight::estimate(name);
            if weight.total_required_gb <= budget {
                Some((name.clone(), weight))
            } else {
                None
            }
        }).collect()
    }

    /// Get recommended parallel thread count based on platform
    pub fn recommended_threads(&self) -> usize {
        match &self.platform {
            Platform::CpuOnly { threads, .. } => {
                // CPU-only: aggressive parallelism — use all threads
                *threads
            }
            Platform::AppleSilicon { perf_cores, efficiency_cores, .. } => {
                // Apple Silicon: use perf cores for inference, efficiency for I/O
                (*perf_cores + *efficiency_cores) as usize
            }
            _ => {
                // GPU present: conservative CPU threading — GPU does heavy lifting
                (self.sys.cpus().len() / 2).max(4)
            }
        }
    }

    /// Full status display for TUI
    pub fn status_report(&mut self) -> String {
        self.refresh();
        let rec = self.tier.recommended_models();
        format!(
            "── Hardware Environment ──\n\
             Platform:  {platform}\n\
             Tier:      {tier}\n\
             OS:        {os} ({arch})\n\
             RAM:       {used:.1} / {total:.1} GB ({avail:.1} GB free)\n\
             CPU:       {cpu:.1}%\n\
             Threads:   {threads} recommended\n\n\
             ── Recommended Models ──\n\
             Dev:       {dev}\n\
             Audit:     {audit}\n\
             Router:    {router}\n\
             Context:   {ctx} tokens",
            platform = self.platform,
            tier = self.tier,
            os = self.os,
            arch = self.arch,
            used = self.total_ram_gb - self.available_ram_gb,
            total = self.total_ram_gb,
            avail = self.available_ram_gb,
            cpu = self.cpu_usage(),
            threads = self.recommended_threads(),
            dev = rec.dev_model,
            audit = rec.audit_model,
            router = rec.router_model,
            ctx = rec.max_context_tokens,
        )
    }
}

// ────────────────────────────────────────────────────────
// Platform-specific detection (cfg gated)
// ────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn detect_platform(cpu_name: &str, core_count: usize, total_ram_gb: f64) -> Platform {
    let name_lower = cpu_name.to_lowercase();

    // Detect Apple Silicon by CPU brand string
    if name_lower.contains("apple") {
        let chip = if name_lower.contains("m5") {
            "M5"
        } else if name_lower.contains("m4") {
            "M4"
        } else if name_lower.contains("m3") {
            "M3"
        } else if name_lower.contains("m2") {
            "M2"
        } else if name_lower.contains("m1") {
            "M1"
        } else {
            "Apple Silicon"
        };

        // Estimate GPU cores and core layout from total core count
        // Apple Silicon typically: P-cores + E-cores, GPU cores vary
        let (perf, eff, gpu) = estimate_apple_silicon_layout(chip, core_count);

        Platform::AppleSilicon {
            chip: chip.to_string(),
            unified_memory_gb: total_ram_gb as u64,
            gpu_cores: gpu,
            perf_cores: perf,
            efficiency_cores: eff,
        }
    } else {
        // Intel Mac fallback
        Platform::CpuOnly {
            cpu_name: cpu_name.to_string(),
            cores: core_count,
            threads: core_count,
        }
    }
}

#[cfg(target_os = "macos")]
fn estimate_apple_silicon_layout(chip: &str, total_cores: usize) -> (u32, u32, u32) {
    // (performance_cores, efficiency_cores, gpu_cores)
    // Based on known Apple Silicon configurations
    match chip {
        "M5" => (6, 4, 10),     // Estimated M5 base
        "M4" => (4, 6, 10),
        "M3" => (4, 4, 10),
        "M2" => (4, 4, 8),
        "M1" => (4, 4, 7),
        _ => {
            let perf = (total_cores / 2) as u32;
            let eff = (total_cores - total_cores / 2) as u32;
            (perf, eff, 8)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_platform(cpu_name: &str, core_count: usize, _total_ram_gb: f64) -> Platform {
    // Try CUDA first (NVIDIA)
    if let Some(cuda) = detect_cuda() {
        return cuda;
    }

    // Try Vulkan (AMD/Intel/NVIDIA fallback)
    if let Some(vulkan) = detect_vulkan() {
        return vulkan;
    }

    // CPU-only fallback
    Platform::CpuOnly {
        cpu_name: cpu_name.to_string(),
        cores: core_count,
        threads: core_count * 2, // Assume hyperthreading
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_cuda() -> Option<Platform> {
    use libloading::{Library, Symbol};

    // Try to load NVIDIA Management Library
    let lib_name = if cfg!(target_os = "windows") {
        "nvml.dll"
    } else {
        "libnvidia-ml.so.1"
    };

    let lib = unsafe { Library::new(lib_name) }.ok()?;

    // nvmlInit_v2
    let init: Symbol<unsafe extern "C" fn() -> i32> =
        unsafe { lib.get(b"nvmlInit_v2") }.ok()?;
    if unsafe { init() } != 0 {
        return None;
    }

    // nvmlDeviceGetCount_v2
    let get_count: Symbol<unsafe extern "C" fn(*mut u32) -> i32> =
        unsafe { lib.get(b"nvmlDeviceGetCount_v2") }.ok()?;
    let mut count: u32 = 0;
    if unsafe { get_count(&mut count) } != 0 || count == 0 {
        return None;
    }

    // Get first device handle
    let get_handle: Symbol<unsafe extern "C" fn(u32, *mut u64) -> i32> =
        unsafe { lib.get(b"nvmlDeviceGetHandleByIndex_v2") }.ok()?;
    let mut handle: u64 = 0;
    if unsafe { get_handle(0, &mut handle) } != 0 {
        return None;
    }

    // Get device name
    let get_name: Symbol<unsafe extern "C" fn(u64, *mut u8, u32) -> i32> =
        unsafe { lib.get(b"nvmlDeviceGetName") }.ok()?;
    let mut name_buf = [0u8; 256];
    if unsafe { get_name(handle, name_buf.as_mut_ptr(), 256) } != 0 {
        return None;
    }
    let device_name = String::from_utf8_lossy(
        &name_buf[..name_buf.iter().position(|&b| b == 0).unwrap_or(256)]
    ).to_string();

    // Get memory info
    #[repr(C)]
    struct NvmlMemory { total: u64, free: u64, used: u64 }
    let get_mem: Symbol<unsafe extern "C" fn(u64, *mut NvmlMemory) -> i32> =
        unsafe { lib.get(b"nvmlDeviceGetMemoryInfo") }.ok()?;
    let mut mem = NvmlMemory { total: 0, free: 0, used: 0 };
    let vram_gb = if unsafe { get_mem(handle, &mut mem) } == 0 {
        mem.total as f64 / 1_073_741_824.0
    } else {
        0.0
    };

    // Shutdown
    let shutdown: Symbol<unsafe extern "C" fn() -> i32> =
        unsafe { lib.get(b"nvmlShutdown") }.ok()?;
    unsafe { shutdown() };

    Some(Platform::CudaGpu {
        device_name,
        vram_gb,
        compute_capability: "unknown".into(), // Would need additional NVML calls
    })
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_vulkan() -> Option<Platform> {
    use libloading::{Library, Symbol};

    let lib_name = if cfg!(target_os = "windows") {
        "vulkan-1.dll"
    } else {
        "libvulkan.so.1"
    };

    // Just check if Vulkan is loadable — full enumeration is complex
    let _lib = unsafe { Library::new(lib_name) }.ok()?;

    // Vulkan is available but we can't easily get device details without
    // pulling in the full Vulkan headers. Return a generic detection.
    Some(Platform::VulkanGpu {
        device_name: "Vulkan-capable GPU detected".into(),
        vram_gb: 0.0, // Would need VkPhysicalDeviceMemoryProperties
    })
}

// Fallback for other platforms
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn detect_platform(cpu_name: &str, core_count: usize, _total_ram_gb: f64) -> Platform {
    Platform::CpuOnly {
        cpu_name: cpu_name.to_string(),
        cores: core_count,
        threads: core_count,
    }
}

// ────────────────────────────────────────────────────────
// Performance Tier Classification
// ────────────────────────────────────────────────────────

fn classify_tier(platform: &Platform, available_ram_gb: f64) -> PerformanceTier {
    let usable = available_ram_gb - SAFE_LOAD_BUFFER_GB;

    // GPU presence boosts the tier (VRAM offloads model layers)
    let gpu_bonus = match platform {
        Platform::CudaGpu { vram_gb, .. } => *vram_gb,
        Platform::AppleSilicon { .. } => 0.0, // Unified memory — already counted
        Platform::VulkanGpu { vram_gb, .. } => *vram_gb * 0.7, // Less efficient than CUDA
        Platform::CpuOnly { .. } => 0.0,
    };

    let effective = usable + gpu_bonus;

    if effective >= 20.0 {
        PerformanceTier::HighEnd
    } else if effective >= 12.0 {
        PerformanceTier::Medium
    } else if effective >= 8.0 {
        PerformanceTier::Small
    } else {
        PerformanceTier::ExtraSmall
    }
}

// ────────────────────────────────────────────────────────
// Utility Functions
// ────────────────────────────────────────────────────────

/// Detect quantization bits from model name tag
pub fn detect_quant_bits(tag: &str) -> f64 {
    let lower = tag.to_lowercase();
    if lower.contains("q2") { 2.0 }
    else if lower.contains("q3") { 3.0 }
    else if lower.contains("q4") { 4.0 }
    else if lower.contains("q5") { 5.0 }
    else if lower.contains("q6") { 6.0 }
    else if lower.contains("q8") { 8.0 }
    else if lower.contains("fp16") || lower.contains("f16") { 16.0 }
    else if lower.contains("fp32") || lower.contains("f32") { 32.0 }
    else { 4.0 } // Default: most Ollama models are Q4
}

/// Extract parameter count from model name (e.g., "qwen2.5:7b" -> 7.0)
pub fn extract_param_count(name: &str) -> Option<f64> {
    for sep in &[":", "-", "_"] {
        for part in name.split(sep) {
            let part = part.trim().to_lowercase();
            if part.ends_with('b') {
                if let Ok(n) = part[..part.len() - 1].parse::<f64>() {
                    if n > 0.0 && n < 1000.0 {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Find a smaller quantized alternative that fits in the given RAM budget
fn find_fitting_alternative(model_name: &str, budget_gb: f64) -> ModelWeight {
    let params = extract_param_count(model_name).unwrap_or(7.0);

    // Try progressively smaller quantizations
    for qbits in &[4.0, 3.0, 2.0] {
        let w = ModelWeight::calculate(model_name, params, *qbits);
        if w.total_required_gb <= budget_gb {
            return w;
        }
    }

    // Try smaller param counts at Q4
    for smaller_params in &[7.0, 3.0, 1.0] {
        if *smaller_params < params {
            let alt_name = format!("alt:{smaller_params:.0}b-q4");
            let w = ModelWeight::calculate(&alt_name, *smaller_params, 4.0);
            if w.total_required_gb <= budget_gb {
                return w;
            }
        }
    }

    // Last resort: smallest possible
    ModelWeight::calculate("llama3.2:1b", 1.0, 4.0)
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_quant_bits() {
        assert_eq!(detect_quant_bits("model:7b-q4_k_m"), 4.0);
        assert_eq!(detect_quant_bits("model:7b-q8_0"), 8.0);
        assert_eq!(detect_quant_bits("model:7b-fp16"), 16.0);
        assert_eq!(detect_quant_bits("model:7b"), 4.0); // default
    }

    #[test]
    fn test_extract_param_count() {
        assert_eq!(extract_param_count("qwen2.5:7b"), Some(7.0));
        assert_eq!(extract_param_count("deepseek-r1:14b"), Some(14.0));
        assert_eq!(extract_param_count("llama3.2:1b"), Some(1.0));
        assert_eq!(extract_param_count("mistral-small-4"), None);
    }

    #[test]
    fn test_model_weight_calculation() {
        let w = ModelWeight::calculate("test:7b-q4", 7.0, 4.0);
        assert!((w.total_required_gb - 5.0).abs() < 0.1); // 3.5 + 1.5

        let w = ModelWeight::calculate("test:14b-q8", 14.0, 8.0);
        assert!((w.total_required_gb - 15.5).abs() < 0.1); // 14.0 + 1.5
    }

    #[test]
    fn test_model_weight_estimate() {
        let w = ModelWeight::estimate("qwen2.5:7b-q4_k_m");
        assert!((w.params_billions - 7.0).abs() < 0.1);
        assert!((w.quant_bits - 4.0).abs() < 0.1);
    }

    #[test]
    fn test_performance_tier_classification() {
        let cpu_only = Platform::CpuOnly {
            cpu_name: "test".into(), cores: 4, threads: 8,
        };
        assert_eq!(classify_tier(&cpu_only, 6.0), PerformanceTier::ExtraSmall);
        assert_eq!(classify_tier(&cpu_only, 14.0), PerformanceTier::Small);
        assert_eq!(classify_tier(&cpu_only, 18.0), PerformanceTier::Medium);
        assert_eq!(classify_tier(&cpu_only, 26.0), PerformanceTier::HighEnd);
    }

    #[test]
    fn test_gpu_bonus_classification() {
        let cuda = Platform::CudaGpu {
            device_name: "RTX 4090".into(),
            vram_gb: 24.0,
            compute_capability: "8.9".into(),
        };
        // 10 GB available + 24 GB VRAM = 34 effective → HighEnd
        assert_eq!(classify_tier(&cuda, 10.0), PerformanceTier::HighEnd);
    }

    #[test]
    fn test_recommended_models_per_tier() {
        let rec = PerformanceTier::HighEnd.recommended_models();
        assert!(rec.dev_model.contains("14b"));
        assert!(rec.audit_model.contains("14b"));

        let rec = PerformanceTier::ExtraSmall.recommended_models();
        assert!(rec.dev_model.contains("3b"));
    }

    #[test]
    fn test_find_fitting_alternative() {
        let alt = find_fitting_alternative("huge-model:70b", 6.0);
        assert!(alt.total_required_gb <= 6.0);
    }

    #[test]
    fn test_hardware_env_detect() {
        let env = HardwareEnv::detect();
        assert!(env.total_ram_gb > 0.0);
        assert!(!env.os.is_empty());
        assert!(!env.arch.is_empty());
    }

    #[test]
    fn test_safe_load_small_model() {
        let mut env = HardwareEnv::detect();
        let result = env.safe_load("llama3.2:1b");
        // 1B model should always be safe on any dev machine
        assert!(matches!(result, SafeLoadResult::Safe { .. } | SafeLoadResult::Warning { .. }));
    }
}
