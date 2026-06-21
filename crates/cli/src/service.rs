//! OS service management for the auto-sync loop.
//!
//! A single supervised service (systemd user unit on Linux, launchd agent on
//! macOS) runs `brim autosync run` and keeps it alive across crashes and
//! reboots. We deliberately do NOT ship a second PID-file daemon — the OS
//! supervisor is the only background mechanism.
//
// ponytail: shells out to systemctl/launchctl instead of linking a service
// library. Upgrade to a crate only if we outgrow the two init systems.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

const SERVICE_NAME: &str = "brim-autosync";

/// Outcome of an install/uninstall attempt, so callers can print sensible hints.
pub enum ServiceOutcome {
    Done,
    /// No supported init system was found; auto-start was skipped (not an error).
    Unsupported,
}

/// Path to the currently running `brim` binary, for embedding in the unit file.
fn brim_exe() -> Result<PathBuf> {
    std::env::current_exe().context("cannot determine path to the brim binary")
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    fn unit_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
        Ok(PathBuf::from(home)
            .join(".config/systemd/user")
            .join(format!("{SERVICE_NAME}.service")))
    }

    fn unit_contents(exe: &str) -> String {
        format!(
            "[Unit]\n\
             Description=brim usage auto-sync\n\
             After=network-online.target\n\n\
             [Service]\n\
             Type=simple\n\
             ExecStart={exe} autosync run\n\
             Restart=on-failure\n\
             RestartSec=30\n\n\
             [Install]\n\
             WantedBy=default.target\n"
        )
    }

    fn systemctl(args: &[&str]) -> Result<bool> {
        let status = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .status()
            .context("failed to run systemctl")?;
        Ok(status.success())
    }

    fn has_systemd() -> bool {
        Command::new("systemctl")
            .arg("--user")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn install() -> Result<ServiceOutcome> {
        if !has_systemd() {
            return Ok(ServiceOutcome::Unsupported);
        }
        let path = unit_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let exe = brim_exe()?;
        std::fs::write(&path, unit_contents(&exe.to_string_lossy()))
            .with_context(|| format!("write unit file {}", path.display()))?;

        systemctl(&["daemon-reload"]).ok();
        // enable --now both enables at boot and starts immediately.
        if !systemctl(&["enable", "--now", SERVICE_NAME])? {
            return Err(anyhow!("systemctl enable --now {SERVICE_NAME} failed"));
        }
        Ok(ServiceOutcome::Done)
    }

    pub fn uninstall() -> Result<ServiceOutcome> {
        if !has_systemd() {
            return Ok(ServiceOutcome::Unsupported);
        }
        systemctl(&["disable", "--now", SERVICE_NAME]).ok();
        let path = unit_path()?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        systemctl(&["daemon-reload"]).ok();
        Ok(ServiceOutcome::Done)
    }

    pub fn is_active() -> Option<bool> {
        if !has_systemd() {
            return None;
        }
        let out = Command::new("systemctl")
            .args(["--user", "is-active", SERVICE_NAME])
            .output()
            .ok()?;
        Some(out.status.success())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const LABEL: &str = "dev.brim.autosync";

    fn plist_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
        Ok(PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn plist_contents(exe: &str) -> String {
        let exe = xml_escape(exe);
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key><string>{LABEL}</string>\n\
             \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{exe}</string>\n\t\t<string>autosync</string>\n\t\t<string>run</string>\n\t</array>\n\
             \t<key>RunAtLoad</key><true/>\n\
             \t<key>KeepAlive</key><true/>\n\
             </dict>\n</plist>\n"
        )
    }

    fn domain() -> String {
        // ponytail: `id -u` avoids pulling in libc just for getuid().
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        format!("gui/{uid}")
    }

    pub fn install() -> Result<ServiceOutcome> {
        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let exe = brim_exe()?;
        std::fs::write(&path, plist_contents(&exe.to_string_lossy()))
            .with_context(|| format!("write plist {}", path.display()))?;

        // bootout first so a re-install reloads cleanly; ignore its failure.
        Command::new("launchctl")
            .args(["bootout", &domain(), &path.to_string_lossy()])
            .status()
            .ok();
        let ok = Command::new("launchctl")
            .args(["bootstrap", &domain(), &path.to_string_lossy()])
            .status()
            .context("failed to run launchctl bootstrap")?
            .success();
        if !ok {
            return Err(anyhow!("launchctl bootstrap failed"));
        }
        Ok(ServiceOutcome::Done)
    }

    pub fn uninstall() -> Result<ServiceOutcome> {
        let path = plist_path()?;
        Command::new("launchctl")
            .args(["bootout", &domain(), &path.to_string_lossy()])
            .status()
            .ok();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(ServiceOutcome::Done)
    }

    pub fn is_active() -> Option<bool> {
        let out = Command::new("launchctl")
            .args(["print", &format!("{}/{LABEL}", domain())])
            .output()
            .ok()?;
        Some(out.status.success())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    // Registered Scheduled Task name. Backslash would create a folder; keep flat.
    const TASK_NAME: &str = "brim-autosync";

    fn schtasks(args: &[&str]) -> Result<bool> {
        let status = Command::new("schtasks")
            .args(args)
            .status()
            .context("failed to run schtasks (Windows Task Scheduler)")?;
        Ok(status.success())
    }

    pub fn install() -> Result<ServiceOutcome> {
        let exe = brim_exe()?;
        // /sc onlogon keeps it alive across reboots without admin rights; the
        // run loop itself supervises crashes via its internal retry.
        let run = format!("\"{}\" autosync run", exe.to_string_lossy());
        // ponytail: /ru %USERNAME% scopes the task to the interactive user so it
        // can read the per-user keyring; without it some SKUs default to SYSTEM,
        // which can't see DPAPI-protected credentials. %USERNAME% is expanded by
        // schtasks itself. Upgrade to an explicit SID only if a SKU misbehaves.
        let ok = schtasks(&[
            "/create",
            "/tn",
            TASK_NAME,
            "/tr",
            &run,
            "/sc",
            "onlogon",
            "/ru",
            "%USERNAME%",
            "/f",
        ])?;
        if !ok {
            return Err(anyhow!("schtasks /create {TASK_NAME} failed"));
        }
        // Start it now so the first sync doesn't wait for the next logon.
        schtasks(&["/run", "/tn", TASK_NAME]).ok();
        Ok(ServiceOutcome::Done)
    }

    pub fn uninstall() -> Result<ServiceOutcome> {
        schtasks(&["/end", "/tn", TASK_NAME]).ok();
        schtasks(&["/delete", "/tn", TASK_NAME, "/f"]).ok();
        Ok(ServiceOutcome::Done)
    }

    pub fn is_active() -> Option<bool> {
        // `/query` exits non-zero when the task isn't registered.
        let out = Command::new("schtasks")
            .args(["/query", "/tn", TASK_NAME])
            .output()
            .ok()?;
        if !out.status.success() {
            return Some(false);
        }
        // Registered; report Running vs not from the status column.
        let text = String::from_utf8_lossy(&out.stdout);
        Some(text.contains("Running"))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub fn install() -> Result<ServiceOutcome> {
        Ok(ServiceOutcome::Unsupported)
    }
    pub fn uninstall() -> Result<ServiceOutcome> {
        Ok(ServiceOutcome::Unsupported)
    }
    pub fn is_active() -> Option<bool> {
        None
    }
}

/// Install and enable the auto-sync service. Idempotent.
pub fn install() -> Result<ServiceOutcome> {
    platform::install()
}

/// Disable and remove the auto-sync service. Idempotent.
pub fn uninstall() -> Result<ServiceOutcome> {
    platform::uninstall()
}

/// Whether the service is currently active. `None` if no init system is available.
pub fn is_active() -> Option<bool> {
    platform::is_active()
}
