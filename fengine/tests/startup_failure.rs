use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

#[test]
fn missing_fll_library_fails_closed_with_endpoint_diagnostic() {
    let state_directory = temporary_state_directory();
    let missing_library = state_directory.join("missing-framelean-fll");
    assert_startup_failure(
        &state_directory,
        &missing_library,
        "could not load FLL dynamic library",
    );
}

#[test]
fn incompatible_fll_abi_fails_closed_with_version_diagnostic() {
    let state_directory = temporary_state_directory();
    let fake_library = compile_incompatible_library(&state_directory);
    assert_startup_failure(
        &state_directory,
        &fake_library,
        "FLL ABI v1.0 negotiation failed",
    );
}

fn temporary_state_directory() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("framelean-work-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn compile_incompatible_library(directory: &Path) -> std::path::PathBuf {
    let source = directory.join("fake_fll.rs");
    fs::write(
        &source,
        r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn framelean_fll_get_api(
    _major: u16,
    _minor: u16,
    _out_api: *mut *const std::ffi::c_void,
) -> i32 {
    73
}
"#,
    )
    .unwrap();
    let library = directory.join(if cfg!(target_os = "macos") {
        "libframelean_fake_fll.dylib"
    } else if cfg!(target_os = "windows") {
        "framelean_fake_fll.dll"
    } else {
        "libframelean_fake_fll.so"
    });
    let output = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-type=cdylib",
            "--crate-name=framelean_fake_fll",
            "-o",
        ])
        .arg(&library)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "failed to build fake FLL library: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    library
}

fn assert_startup_failure(state_directory: &Path, library: &Path, expected_error: &str) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_framelean-engine"))
        .args(["serve", "--state-dir"])
        .arg(state_directory)
        .env("FRAMELEAN_FLL_LIBRARY", library)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let endpoint_path = state_directory.join("endpoint.json");
    let deadline = Instant::now() + Duration::from_secs(10);
    let failure = loop {
        if let Ok(bytes) = fs::read(&endpoint_path) {
            if let Ok(endpoint) = serde_json::from_slice::<Value>(&bytes) {
                if endpoint["state"] == "failed" {
                    break endpoint;
                }
            }
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("Work exited before publishing its startup error: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Work did not publish a failure endpoint");
        }
        thread::sleep(Duration::from_millis(20));
    };

    assert!(
        failure["error"]
            .as_str()
            .unwrap_or_default()
            .contains(expected_error),
        "unexpected startup error: {}",
        failure["error"].as_str().unwrap_or_default()
    );
    assert!(!child.wait().unwrap().success());
    fs::remove_dir_all(state_directory).unwrap();
}
