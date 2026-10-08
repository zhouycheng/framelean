use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use framelean_core::{MemoryBytes, ObservationStatus, Observed, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sysinfo::System;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CpuInfo {
    pub model: Observed<String>,
    pub architecture: String,
    pub physical_cores: Observed<u32>,
    pub logical_cores: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GpuInfo {
    pub name: String,
    pub kind: Observed<String>,
    pub memory: Observed<MemoryBytes>,
    pub driver: Observed<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NativeMediaFrameworkInfo {
    pub name: String,
    pub status: Observed<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentSnapshot {
    pub observed_at_unix_ms: u64,
    pub operating_system: Observed<String>,
    pub os_version: Observed<String>,
    pub device_model: Observed<String>,
    pub cpu: CpuInfo,
    pub total_memory: MemoryBytes,
    pub gpus: Observed<Vec<GpuInfo>>,
    pub native_media_frameworks: Vec<NativeMediaFrameworkInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResourceSample {
    pub sampled_at_unix_ms: u64,
    pub cpu_usage_basis_points: Observed<u16>,
    pub used_memory: Observed<MemoryBytes>,
    pub gpu_usage_basis_points: Observed<u16>,
    pub used_gpu_memory: Observed<MemoryBytes>,
    pub temperature_millidegrees_celsius: Observed<u64>,
    pub power_milliwatts: Observed<u64>,
}

pub trait EnvironmentSnapshotProvider: Send + Sync {
    fn snapshot(&self) -> Result<EnvironmentSnapshot>;
}

pub trait ResourceMonitor: Send + Sync {
    fn sample(&self) -> Result<ResourceSample>;
}

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub interval: Duration,
    pub channel_capacity: usize,
}

impl MonitorConfig {
    pub fn new(interval: Duration, channel_capacity: usize) -> Result<Self> {
        if interval.is_zero() {
            return Err(framelean_core::EngineError::invalid_argument(
                "monitor interval must be greater than zero",
            ));
        }
        if channel_capacity == 0 {
            return Err(framelean_core::EngineError::invalid_argument(
                "monitor channel capacity must be greater than zero",
            ));
        }
        Ok(Self {
            interval,
            channel_capacity,
        })
    }
}

pub struct MonitorSession {
    receiver: mpsc::Receiver<ResourceSample>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl MonitorSession {
    pub fn start(monitor: Arc<dyn ResourceMonitor>, config: MonitorConfig) -> Self {
        let (sender, receiver) = mpsc::sync_channel(config.channel_capacity);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::spawn(move || {
            while !worker_cancelled.load(Ordering::Acquire) {
                if let Ok(sample) = monitor.sample() {
                    let _ = sender.try_send(sample);
                }
                thread::sleep(config.interval);
            }
        });
        Self {
            receiver,
            cancelled,
            worker: Some(worker),
        }
    }

    pub fn receiver(&self) -> &mpsc::Receiver<ResourceSample> {
        &self.receiver
    }

    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for MonitorSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub struct SystemEnvironment {
    system: Mutex<System>,
}

impl SystemEnvironment {
    pub fn new() -> Self {
        Self {
            system: Mutex::new(System::new_all()),
        }
    }
}

impl Default for SystemEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvironmentSnapshotProvider for SystemEnvironment {
    fn snapshot(&self) -> Result<EnvironmentSnapshot> {
        let mut system = self.system.lock().map_err(|_| {
            framelean_core::EngineError::new(
                framelean_core::ErrorKind::Environment,
                "environment system lock is poisoned",
            )
        })?;
        system.refresh_all();
        let cpu_model = system.cpus().first().map(|cpu| cpu.brand().to_owned());
        let frameworks = native_frameworks();
        Ok(EnvironmentSnapshot {
            observed_at_unix_ms: unix_ms(),
            operating_system: observed_option(System::name(), "sysinfo"),
            os_version: observed_option(System::os_version(), "sysinfo"),
            device_model: Observed::with_status(
                ObservationStatus::NotProbed,
                "device model requires a platform adapter",
            ),
            cpu: CpuInfo {
                model: observed_option(cpu_model, "sysinfo"),
                architecture: std::env::consts::ARCH.to_owned(),
                physical_cores: observed_option(
                    System::physical_core_count().map(|value| value as u32),
                    "sysinfo",
                ),
                logical_cores: system.cpus().len() as u32,
            },
            total_memory: MemoryBytes::new(system.total_memory()),
            gpus: Observed::with_status(
                ObservationStatus::NotProbed,
                "GPU inventory requires a platform adapter",
            ),
            native_media_frameworks: frameworks,
        })
    }
}

impl ResourceMonitor for SystemEnvironment {
    fn sample(&self) -> Result<ResourceSample> {
        let mut system = self.system.lock().map_err(|_| {
            framelean_core::EngineError::new(
                framelean_core::ErrorKind::Environment,
                "resource monitor system lock is poisoned",
            )
        })?;
        system.refresh_cpu_usage();
        system.refresh_memory();
        let cpu_basis_points = (system.global_cpu_usage().clamp(0.0, 100.0) * 100.0) as u16;
        Ok(ResourceSample {
            sampled_at_unix_ms: unix_ms(),
            cpu_usage_basis_points: Observed::detected(cpu_basis_points, "sysinfo"),
            used_memory: Observed::detected(MemoryBytes::new(system.used_memory()), "sysinfo"),
            gpu_usage_basis_points: unsupported("GPU utilization requires a platform adapter"),
            used_gpu_memory: unsupported("GPU memory requires a platform adapter"),
            temperature_millidegrees_celsius: unsupported(
                "temperature is not reliably available on this platform adapter",
            ),
            power_milliwatts: unsupported(
                "power is not reliably available on this platform adapter",
            ),
        })
    }
}

fn observed_option<T>(value: Option<T>, source: &str) -> Observed<T> {
    value.map_or_else(
        || Observed::with_status(ObservationStatus::NotProbed, "value unavailable"),
        |value| Observed::detected(value, source),
    )
}

fn unsupported<T>(reason: &str) -> Observed<T> {
    Observed::with_status(ObservationStatus::Unsupported, reason)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn native_frameworks() -> Vec<NativeMediaFrameworkInfo> {
    let name = match std::env::consts::OS {
        "macos" => "videotoolbox",
        "windows" => "media_foundation",
        "linux" => "vaapi",
        _ => "platform_media_framework",
    };
    vec![NativeMediaFrameworkInfo {
        name: name.to_owned(),
        status: Observed::with_status(
            ObservationStatus::NotProbed,
            "framework initialization is handled by a platform backend adapter",
        ),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_snapshot_does_not_contain_ffmpeg_state() {
        let snapshot = SystemEnvironment::new().snapshot().unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("ffmpeg"));
        assert!(snapshot.cpu.logical_cores > 0);
    }

    #[test]
    fn monitor_config_rejects_unbounded_zero_capacity() {
        assert!(MonitorConfig::new(Duration::from_millis(1), 0).is_err());
    }
}
