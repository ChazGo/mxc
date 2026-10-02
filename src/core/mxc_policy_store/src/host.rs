// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Host platform, native system architecture, and approved host symbols
//!
//! The native architecture is the device's, not the process's or the
//! compile target's:
//!
//! - Windows: machine-wide `PROCESSOR_ARCHITECTURE` in
//!   `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`,
//!   read with `%SystemRoot%\System32\reg.exe`.
//! - macOS: `/usr/sbin/sysctl -n hw.optional.arm64` (`1` = Apple silicon even
//!   under Rosetta; the key is absent on Intel Macs), else the machine type.
//! - Linux: the kernel machine type (`uname(2)`).

use crate::errors::{ErrorReason, PolicyCatalogError, Result};
use crate::model::{Architecture, Platform};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Host facts the resolver uses only when the caller omits them.
pub trait HostEnvironment: Send + Sync {
    fn platform(&self) -> Result<Platform>;
    /// The device's native system architecture. Fails when it cannot be
    /// determined; never guesses.
    fn native_architecture(&self) -> Result<Architecture>;
    /// Approved host-known symbols (`source: "host"`) for the current host.
    fn symbol(&self, name: &str) -> Option<String>;
}

/// Maps an OS-reported machine/architecture string to a catalog selector.
pub fn architecture_from_machine(machine: &str) -> Option<Architecture> {
    match crate::text::js_trim(machine).to_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" => Some(Architecture::X64),
        "arm64" | "aarch64" => Some(Architecture::Arm64),
        _ => None,
    }
}

/// Node's `os.platform()` name for the current OS.
fn node_platform_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        "solaris" | "illumos" => "sunos",
        other => other,
    }
}

fn host_platform() -> Result<Platform> {
    match std::env::consts::OS {
        "windows" => Ok(Platform::Windows),
        "macos" => Ok(Platform::Macos),
        "linux" => Ok(Platform::Linux),
        _ => Err(PolicyCatalogError::new(
            ErrorReason::UnsupportedHost,
            format!(
                "host platform '{}' has no catalog selector",
                node_platform_name()
            ),
        )),
    }
}

/// Runs an executable by absolute path; stdout as text, like `execFileSync`.
fn run(file: &str, args: &[&str]) -> std::result::Result<String, String> {
    let mut command = Command::new(file);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|error| {
        let code = match error.kind() {
            std::io::ErrorKind::NotFound => "ENOENT".to_string(),
            std::io::ErrorKind::PermissionDenied => "EACCES".to_string(),
            _ => error.to_string(),
        };
        format!("spawnSync {file} {code}")
    })?;
    if !output.status.success() {
        return Err(format!("Command failed: {file} {}", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Kernel machine type (Node `os.machine()`).
#[cfg(unix)]
fn os_machine() -> std::result::Result<String, String> {
    // SAFETY: `uname` fills a caller-provided, zero-initialized struct.
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut name) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: `machine` is NUL-terminated after a successful `uname`.
    let machine = unsafe { std::ffi::CStr::from_ptr(name.machine.as_ptr()) };
    Ok(machine.to_string_lossy().into_owned())
}

#[cfg(not(unix))]
fn os_machine() -> std::result::Result<String, String> {
    Err("os.machine() is not available on this platform".to_string())
}

/// Extracts the value from `reg query ... /v PROCESSOR_ARCHITECTURE` output
/// (`/PROCESSOR_ARCHITECTURE\s+REG_SZ\s+(\S+)/i`).
fn parse_reg_output(output: &str) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("processor_architecture") {
        let start = from + found;
        let rest = &output[start + "processor_architecture".len()..];
        let trimmed = rest.trim_start_matches(crate::text::js_is_space);
        if trimmed.len() < rest.len()
            && trimmed.len() >= 6
            && trimmed[..6].eq_ignore_ascii_case("reg_sz")
        {
            let after = &trimmed[6..];
            let value = after.trim_start_matches(crate::text::js_is_space);
            if value.len() < after.len() {
                let end = value.find(crate::text::js_is_space).unwrap_or(value.len());
                if end > 0 {
                    return Some(value[..end].to_string());
                }
            }
        }
        from = start + 1;
    }
    None
}

fn detect_native_architecture() -> Result<Architecture> {
    let reported: std::result::Result<Option<String>, String> = match std::env::consts::OS {
        "windows" => {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
            let reg = format!("{}\\System32\\reg.exe", root.trim_end_matches(['\\', '/']));
            run(
                &reg,
                &[
                    "query",
                    "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                    "/v",
                    "PROCESSOR_ARCHITECTURE",
                ],
            )
            .map(|output| parse_reg_output(&output))
        }
        "macos" => {
            let apple_silicon = run("/usr/sbin/sysctl", &["-n", "hw.optional.arm64"])
                .map(|output| output.trim() == "1")
                .unwrap_or(false);
            if apple_silicon {
                Ok(Some("arm64".to_string()))
            } else {
                os_machine().map(Some)
            }
        }
        _ => os_machine().map(Some),
    };
    let reported = reported.map_err(|message| {
        PolicyCatalogError::new(
            ErrorReason::UnsupportedHost,
            format!("native system architecture could not be determined: {message}"),
        )
    })?;
    match reported.as_deref().and_then(architecture_from_machine) {
        Some(architecture) => Ok(architecture),
        None => Err(PolicyCatalogError::new(
            ErrorReason::UnsupportedHost,
            format!(
                "native system architecture '{}' has no catalog selector",
                reported.as_deref().unwrap_or("unknown")
            ),
        )),
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Node `os.homedir()`.
fn home_dir() -> Option<String> {
    let from_env = if cfg!(windows) {
        non_empty_env("USERPROFILE")
    } else {
        non_empty_env("HOME")
    };
    from_env.or_else(|| {
        #[allow(deprecated)]
        std::env::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty())
    })
}

/// Node `os.tmpdir()`.
fn temp_dir() -> Option<String> {
    if cfg!(windows) {
        let mut path = non_empty_env("TEMP")
            .or_else(|| non_empty_env("TMP"))
            .unwrap_or_else(|| {
                format!(
                    "{}\\temp",
                    std::env::var("SystemRoot")
                        .or_else(|_| std::env::var("windir"))
                        .unwrap_or_default()
                )
            });
        let bytes = path.as_bytes();
        if path.len() > 1 && path.ends_with('\\') && !(path.len() == 3 && bytes[1] == b':') {
            path.pop();
        }
        Some(path)
    } else {
        let mut path = non_empty_env("TMPDIR")
            .or_else(|| non_empty_env("TMP"))
            .or_else(|| non_empty_env("TEMP"))
            .unwrap_or_else(|| "/tmp".to_string());
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        Some(path).filter(|p| !p.is_empty())
    }
}

/// The real host. The native architecture is detected once per process, on
/// first need (successful detections are cached).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemHost;

impl HostEnvironment for SystemHost {
    fn platform(&self) -> Result<Platform> {
        host_platform()
    }

    fn native_architecture(&self) -> Result<Architecture> {
        static CACHE: OnceLock<Architecture> = OnceLock::new();
        if let Some(architecture) = CACHE.get() {
            return Ok(*architecture);
        }
        let architecture = detect_native_architecture()?;
        Ok(*CACHE.get_or_init(|| architecture))
    }

    fn symbol(&self, name: &str) -> Option<String> {
        match name {
            "user_home" => home_dir(),
            "temp_dir" => temp_dir(),
            _ => None,
        }
    }
}

/// A host with fixed facts, for tests and callers that already know them.
#[derive(Clone, Debug)]
pub struct FixedHost {
    pub platform: Platform,
    pub architecture: Architecture,
    pub symbols: Vec<(String, String)>,
}

impl FixedHost {
    pub fn new(platform: Platform, architecture: Architecture) -> Self {
        Self {
            platform,
            architecture,
            symbols: Vec::new(),
        }
    }

    pub fn with_symbol(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.symbols.push((name.into(), value.into()));
        self
    }
}

impl HostEnvironment for FixedHost {
    fn platform(&self) -> Result<Platform> {
        Ok(self.platform)
    }

    fn native_architecture(&self) -> Result<Architecture> {
        Ok(self.architecture)
    }

    fn symbol(&self, name: &str) -> Option<String> {
        self.symbols
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_machine_names() {
        assert_eq!(architecture_from_machine("AMD64"), Some(Architecture::X64));
        assert_eq!(architecture_from_machine("x86_64"), Some(Architecture::X64));
        assert_eq!(
            architecture_from_machine("ARM64"),
            Some(Architecture::Arm64)
        );
        assert_eq!(
            architecture_from_machine(" aarch64\n"),
            Some(Architecture::Arm64)
        );
        assert_eq!(architecture_from_machine("riscv64"), None);
        assert_eq!(architecture_from_machine("x86"), None);
    }

    #[test]
    fn parses_reg_query_output() {
        let output =
            "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\...\\Environment\r\n    PROCESSOR_ARCHITECTURE    REG_SZ    ARM64\r\n\r\n";
        assert_eq!(parse_reg_output(output).as_deref(), Some("ARM64"));
        assert_eq!(parse_reg_output("nothing"), None);
    }

    #[test]
    fn system_host_detects_a_supported_architecture_here() {
        // Every CI host (x64/arm64 Windows, Linux, macOS) must be detectable.
        let host = SystemHost;
        assert!(host.platform().is_ok());
        assert!(host.native_architecture().is_ok());
    }
}
