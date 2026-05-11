//! Interactive setup for the HTTP/SSE + OAuth deployment.
//!
//! The MCP server itself is just a long-running daemon; what makes it hard to
//! install is the surrounding macOS plumbing — launchd plist, Tailscale Funnel,
//! Zotero local API, paste-friendly OAuth credentials. This module collapses
//! all of that into `zotero-mcp setup` so a user goes from `cargo install` to
//! "Claude.ai is talking to my library" in ~30 seconds.
//!
//! macOS-only by design (launchd). The Linux path would need a systemd unit;
//! deferred until someone asks.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::oauth;

const HTTP_BIND: &str = "127.0.0.1:8765";
const HTTP_PORT: u16 = 8765;
const PLIST_LABEL: &str = "com.zotero-mcp.http";
const LOG_DIR_REL: &str = "Library/Logs/zotero-mcp";

// ---------- setup --------------------------------------------------------

pub async fn run_setup() -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        anyhow::bail!(
            "`zotero-mcp setup` is macOS-only (uses launchd). \
             Configure ZOTERO_MCP_HTTP + ZOTERO_MCP_OAUTH_ISSUER under your own \
             init system and let the server bootstrap oauth.toml on first start."
        );
    }

    println!("zotero-mcp setup\n================\n");

    let hostname = match detect_tailscale_dns_name() {
        Ok(name) => name,
        Err(e) => {
            eprintln!("Could not detect Tailscale Funnel hostname: {e}\n");
            eprintln!("To use this setup helper you need:");
            eprintln!("  1. Tailscale installed and signed in:");
            eprintln!("       https://tailscale.com/download/macos");
            eprintln!("  2. Funnel enabled on your tailnet (admin action, once):");
            eprintln!("       https://login.tailscale.com/admin/settings/features");
            eprintln!("  3. Then re-run `zotero-mcp setup`.");
            anyhow::bail!("Tailscale not available");
        }
    };
    let issuer = format!("https://{hostname}");
    println!("Detected Tailscale Funnel hostname: {hostname}");
    println!("OAuth issuer URL will be:           {issuer}\n");
    if !confirm("Use this hostname? [Y/n] ")? {
        anyhow::bail!("aborted by user");
    }

    let binary_path = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("could not resolve current exe: {e}"))?;
    let plist_path = launchd_plist_path()?;
    let log_dir = home_dir()?.join(LOG_DIR_REL);
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| anyhow::anyhow!("mkdir {}: {e}", log_dir.display()))?;
    let plist_body = render_plist(&binary_path, &issuer, &log_dir);

    println!("Writing launchd plist → {}", plist_path.display());
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist_path, plist_body)?;

    println!("Reloading launchd job…");
    reload_launchd(&plist_path)?;

    println!("Enabling Tailscale Funnel on port {HTTP_PORT}…");
    enable_tailscale_funnel(HTTP_PORT)?;

    println!("Waiting for OAuth credentials to materialize…");
    let oauth_path = oauth::config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve OAuth config path"))?;
    wait_for_file(&oauth_path, Duration::from_secs(10))?;

    let creds = read_oauth(&oauth_path)?;
    print_credentials_block(&creds, &issuer);

    Ok(())
}

// ---------- status -------------------------------------------------------

pub async fn run_status() -> anyhow::Result<()> {
    let mut all_ok = true;

    println!("zotero-mcp status\n=================\n");

    // launchd
    match launchd_job_loaded(PLIST_LABEL) {
        Ok(true) => println!("  [OK]   launchd job {PLIST_LABEL} loaded"),
        Ok(false) => {
            println!("  [FAIL] launchd job {PLIST_LABEL} not loaded");
            println!("         fix: run `zotero-mcp setup`");
            all_ok = false;
        }
        Err(e) => {
            println!("  [FAIL] launchd check errored: {e}");
            all_ok = false;
        }
    }

    // HTTP server
    if tcp_port_listening("127.0.0.1", HTTP_PORT) {
        println!("  [OK]   HTTP server listening on {HTTP_BIND}");
    } else {
        println!("  [FAIL] no listener on {HTTP_BIND}");
        println!("         fix: check ~/Library/Logs/zotero-mcp/http.err.log");
        all_ok = false;
    }

    // Tailscale Funnel
    match tailscale_funnel_active(HTTP_PORT) {
        Ok(true) => println!("  [OK]   Tailscale Funnel is publishing port {HTTP_PORT}"),
        Ok(false) => {
            println!("  [FAIL] Tailscale Funnel is NOT publishing port {HTTP_PORT}");
            println!("         fix: `tailscale funnel --bg {HTTP_PORT}`");
            all_ok = false;
        }
        Err(e) => {
            println!("  [WARN] could not check Tailscale Funnel: {e}");
        }
    }

    // Zotero local API
    if tcp_port_listening("127.0.0.1", 23119) {
        println!("  [OK]   Zotero local API responding on 127.0.0.1:23119");
    } else {
        println!("  [FAIL] no listener on 127.0.0.1:23119 (Zotero closed, or local API disabled)");
        println!("         fix: open Zotero → Preferences → Advanced → \"Allow other applications…\"");
        all_ok = false;
    }

    // oauth.toml
    let oauth_path = oauth::config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve OAuth config path"))?;
    if oauth_path.exists() {
        println!("  [OK]   OAuth config present: {}", oauth_path.display());
    } else {
        println!("  [FAIL] OAuth config missing: {}", oauth_path.display());
        println!("         fix: run `zotero-mcp setup`");
        all_ok = false;
    }

    println!();
    if all_ok {
        println!("All green.");
        Ok(())
    } else {
        anyhow::bail!("one or more checks failed")
    }
}

// ---------- show-credentials --------------------------------------------

pub fn run_show_credentials() -> anyhow::Result<()> {
    let oauth_path = oauth::config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve OAuth config path"))?;
    if !oauth_path.exists() {
        anyhow::bail!(
            "OAuth config not found at {}. Run `zotero-mcp setup` first.",
            oauth_path.display()
        );
    }
    let creds = read_oauth(&oauth_path)?;
    print_credentials_block(&creds, &creds.issuer);
    Ok(())
}

// ---------- helpers ------------------------------------------------------

fn detect_tailscale_dns_name() -> anyhow::Result<String> {
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .map_err(|e| anyhow::anyhow!("running `tailscale status --json`: {e}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "`tailscale status --json` exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_tailscale_dns_name(&output.stdout)
}

/// Pull `Self.DNSName` out of the JSON returned by `tailscale status --json`,
/// strip the trailing dot (FQDN convention), and return e.g.
/// `laptop.stoat-minnow.ts.net`.
fn parse_tailscale_dns_name(stdout: &[u8]) -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct Status {
        #[serde(rename = "Self")]
        self_: SelfNode,
    }
    #[derive(serde::Deserialize)]
    struct SelfNode {
        #[serde(rename = "DNSName")]
        dns_name: String,
    }
    let parsed: Status = serde_json::from_slice(stdout)
        .map_err(|e| anyhow::anyhow!("parse Tailscale status JSON: {e}"))?;
    let name = parsed.self_.dns_name.trim_end_matches('.').to_string();
    if name.is_empty() {
        anyhow::bail!("Tailscale Self.DNSName is empty");
    }
    Ok(name)
}

fn launchd_plist_path() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join("Library/LaunchAgents").join(format!("{PLIST_LABEL}.plist")))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))
}

fn render_plist(binary: &Path, issuer: &str, log_dir: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>

    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string>
        <string>-c</string>
        <string>exec {binary}</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>ZOTERO_MCP_HTTP</key>
        <string>{bind}</string>
        <key>ZOTERO_MCP_OAUTH_ISSUER</key>
        <string>{issuer}</string>
        <key>RUST_LOG</key>
        <string>info,tower_http=debug</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>10</integer>

    <key>StandardOutPath</key>
    <string>{out_log}</string>
    <key>StandardErrorPath</key>
    <string>{err_log}</string>
</dict>
</plist>
"#,
        label = PLIST_LABEL,
        binary = binary.display(),
        bind = HTTP_BIND,
        issuer = issuer,
        out_log = log_dir.join("http.out.log").display(),
        err_log = log_dir.join("http.err.log").display(),
    )
}

fn reload_launchd(plist_path: &Path) -> anyhow::Result<()> {
    let uid = unsafe { libc_geteuid() };
    let domain = format!("gui/{uid}");
    // bootout is fine to fail (job may not be loaded yet); ignore status.
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, &plist_path.display().to_string()])
        .output();
    let out = Command::new("launchctl")
        .args(["bootstrap", &domain, &plist_path.display().to_string()])
        .output()
        .map_err(|e| anyhow::anyhow!("running launchctl bootstrap: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn enable_tailscale_funnel(port: u16) -> anyhow::Result<()> {
    let out = Command::new("tailscale")
        .args(["funnel", "--bg", &port.to_string()])
        .output()
        .map_err(|e| anyhow::anyhow!("running `tailscale funnel`: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Funnel may already be enabled — that's fine.
        if stderr.contains("already") {
            return Ok(());
        }
        anyhow::bail!("tailscale funnel exited {}: {}", out.status, stderr.trim());
    }
    Ok(())
}

fn wait_for_file(path: &Path, timeout: Duration) -> anyhow::Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    anyhow::bail!(
        "timed out after {:?} waiting for {} — check ~/Library/Logs/zotero-mcp/http.err.log",
        timeout,
        path.display()
    )
}

fn read_oauth(path: &Path) -> anyhow::Result<oauth::OAuthConfig> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let cfg: oauth::OAuthConfig = toml::from_str(std::str::from_utf8(&bytes)?)?;
    Ok(cfg)
}

fn print_credentials_block(creds: &oauth::OAuthConfig, issuer: &str) {
    let url = format!("{issuer}/sse");
    println!("\n=== Paste these into Claude.ai → Settings → Connectors → Add custom ===\n");
    println!("  Server URL          {url}");
    println!("  Advanced ▸ Client ID     {}", creds.client_id);
    println!("  Advanced ▸ Client Secret {}", creds.client_secret);
    println!();
}

fn launchd_job_loaded(label: &str) -> anyhow::Result<bool> {
    let uid = unsafe { libc_geteuid() };
    let out = Command::new("launchctl")
        .args(["print", &format!("gui/{uid}/{label}")])
        .output()
        .map_err(|e| anyhow::anyhow!("running launchctl print: {e}"))?;
    Ok(out.status.success())
}

fn tcp_port_listening(host: &str, port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("{host}:{port}")
            .parse()
            .expect("hardcoded host:port"),
        Duration::from_millis(500),
    )
    .is_ok()
}

fn tailscale_funnel_active(port: u16) -> anyhow::Result<bool> {
    let out = Command::new("tailscale")
        .args(["funnel", "status"])
        .output()
        .map_err(|e| anyhow::anyhow!("running `tailscale funnel status`: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "`tailscale funnel status` exited {}",
            out.status
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Output mentions the port number when something is being served on it.
    Ok(stdout.contains(&port.to_string()))
}

fn confirm(prompt: &str) -> anyhow::Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(trimmed.is_empty() || matches!(trimmed.to_ascii_lowercase().as_str(), "y" | "yes"))
}

// libc::geteuid wrapper — we just need the user id for the launchctl domain
// string; bringing in the full libc crate for one call is excessive.
extern "C" {
    fn geteuid() -> u32;
}
unsafe fn libc_geteuid() -> u32 {
    geteuid()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dns_name_with_trailing_dot() {
        let json = br#"{
            "Self": { "DNSName": "laptop.stoat-minnow.ts.net.", "HostName": "Whatever" },
            "MagicDNSSuffix": "stoat-minnow.ts.net"
        }"#;
        let name = parse_tailscale_dns_name(json).unwrap();
        assert_eq!(name, "laptop.stoat-minnow.ts.net");
    }

    #[test]
    fn parses_dns_name_without_trailing_dot() {
        let json = br#"{ "Self": { "DNSName": "machine.tail-net.ts.net", "HostName": "x" } }"#;
        assert_eq!(parse_tailscale_dns_name(json).unwrap(), "machine.tail-net.ts.net");
    }

    #[test]
    fn rejects_empty_dns_name() {
        let json = br#"{ "Self": { "DNSName": "", "HostName": "x" } }"#;
        assert!(parse_tailscale_dns_name(json).is_err());
    }

    #[test]
    fn plist_template_substitutes_paths_and_issuer() {
        let plist = render_plist(
            Path::new("/opt/bin/zotero-mcp"),
            "https://example.test",
            Path::new("/var/log/zotero-mcp"),
        );
        assert!(plist.contains("<string>exec /opt/bin/zotero-mcp</string>"));
        assert!(plist.contains("<string>https://example.test</string>"));
        assert!(plist.contains("<string>/var/log/zotero-mcp/http.out.log</string>"));
        assert!(plist.contains("<string>/var/log/zotero-mcp/http.err.log</string>"));
        assert!(plist.contains("<string>com.zotero-mcp.http</string>"));
    }
}
