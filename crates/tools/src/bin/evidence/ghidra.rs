//! Talking to GhidraMCP's HTTP API directly.
//!
//! Not through the MCP bridge: this is plain GET against the same server the
//! `ghidra` MCP client talks to, so the export works whether or not that
//! client is connected, and so a 1500-function walk is not 6000 MCP
//! round-trips through a stdio transport.

use std::io::Read;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

/// 127.0.0.1 first, then the WSL default gateway (Windows host under NAT).
///
/// The order matters and is not ours to choose: it is the order
/// `~/.local/bin/ghidra-mcp-bridge-win` uses, and a session that has the
/// bridge running has already proved which of the two answers. Probing the
/// gateway first would hang for the connect timeout on every machine where
/// Ghidra is local.
pub fn discover_host(explicit: Option<&str>, port: u32) -> Option<String> {
    // An empty `--host ""` probed in the Python (empty string is falsy), so it
    // probes here too rather than producing the URL `/methods?...`.
    if let Some(e) = explicit.filter(|s| !s.is_empty()) {
        return Some(e.trim_end_matches('/').to_string());
    }
    let mut candidates = vec!["127.0.0.1".to_string()];
    if let Some(gw) = default_gateway() {
        candidates.push(gw);
    }
    for host in candidates {
        let base = format!("http://{host}:{port}");
        let agent = agent(Duration::from_secs(4));
        // A listening-but-unhappy server (a 404, a wrong app on the port) is
        // treated exactly like nothing listening and we move on, which is what
        // the Python did: `urllib.error.HTTPError` is a subclass of `URLError`
        // and its `except` caught both. The probe asks for one method because
        // an empty body is the other way a wrong server fails this.
        if let Ok(body) = fetch(&agent, &format!("{base}/methods"), &[("offset", "0"), ("limit", "1")]) {
            if !body.trim().is_empty() {
                return Some(base);
            }
        }
    }
    None
}

fn default_gateway() -> Option<String> {
    // No timeout: std has none for `Command`, and the Python's 5 seconds was
    // never the thing that hung - `ip route` reads a kernel table.
    let out = Command::new("ip").args(["route", "show", "default"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let parts: Vec<&str> = text.split_whitespace().collect();
    let at = parts.iter().position(|p| *p == "via")?;
    parts.get(at + 1).map(|s| (*s).to_string())
}

fn agent(timeout: Duration) -> ureq::Agent {
    // Both, and `timeout_connect` is the one that matters. ureq 2 defaults
    // `timeout_connect` to 30 seconds and documents it as taking precedence
    // over the blunt `timeout`, so an agent built with only `.timeout(4)`
    // spends thirty seconds on the connect to a host that is not there. That
    // is exactly the case the probe is for: measured against the Python, which
    // passes one `timeout=` to urllib and covers connect with it, the probe
    // for a missing server took 30s here and 4s there until this line existed.
    ureq::AgentBuilder::new().timeout_connect(timeout).timeout(timeout).build()
}

fn fetch(agent: &ureq::Agent, url: &str, params: &[(&str, &str)]) -> Result<String> {
    let mut req = agent.get(url);
    for (k, v) in params {
        req = req.query(k, v);
    }
    let resp = match req.call() {
        Ok(r) => r,
        // Spell a status error the way Python's HTTPError did, because this
        // text is written into the exported file and read by a human later.
        Err(ureq::Error::Status(code, r)) => {
            anyhow::bail!("HTTP Error {code}: {}", r.status_text())
        }
        Err(e) => return Err(e).context("request failed"),
    };
    // Deliberately not `into_string()`: that caps the body at 10 MB and errors
    // above it. The whole reason --max-body-bytes exists is that this program
    // contains "functions" whose disassembly runs to 34.8 million lines, and a
    // hard 10 MB cap would turn one of those into a request failure recorded
    // permanently in the tree rather than a clipped section.
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf).context("reading the response body")?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// A GhidraMCP server, plus the counters the run reports at the end.
pub struct Ghidra {
    host: String,
    agent: ureq::Agent,
    pub calls: AtomicUsize,
    pub failures: AtomicUsize,
}

impl Ghidra {
    pub fn new(host: String, timeout: Duration) -> Ghidra {
        Ghidra {
            host,
            agent: agent(timeout),
            calls: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
        }
    }

    /// A failed request answers with its own error text rather than failing the
    /// run: one unreachable function out of 1500 should not lose the other
    /// 1499. The cost is that the text lands in the exported file, and a
    /// resumed run will not re-fetch it - see the port notes.
    pub fn get(&self, path: &str, params: &[(&str, &str)]) -> String {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match fetch(&self.agent, &format!("{}/{}", self.host, path), params) {
            Ok(body) => body,
            Err(e) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
                format!("[evidence: request failed: {}]", chain(&e))
            }
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    pub fn failures(&self) -> usize {
        self.failures.load(Ordering::Relaxed)
    }
}

/// `anyhow`'s `{:#}` with `: ` between the causes, so the recorded text reads
/// like the single-line exception string the Python embedded.
fn chain(e: &anyhow::Error) -> String {
    e.chain().map(ToString::to_string).collect::<Vec<_>>().join(": ")
}
