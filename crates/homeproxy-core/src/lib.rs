pub mod service;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use thiserror::Error;

const PROXY_LABEL: &str = service::LABEL;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("could not determine the current user id")]
    MissingUserId,
    #[error(transparent)]
    Service(#[from] service::ServiceError),
    #[error("{program} failed: {message}")]
    CommandFailed { program: String, message: String },
    #[error("could not execute {program}: {source}")]
    CommandIo {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    Direct,
    Nordvpn,
    Upstream,
}

impl ProxyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Nordvpn => "nordvpn",
            Self::Upstream => "upstream",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub label: &'static str,
    pub mode: ProxyMode,
    pub enabled: bool,
    pub connected: bool,
    pub relay_connected: bool,
    pub pid: Option<u32>,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub controller_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Default)]
struct JobState {
    running: bool,
    pid: Option<u32>,
}

pub fn status() -> Result<ProxyStatus, ProxyError> {
    let mode = selected_mode();
    let proxy = launchd_state(PROXY_LABEL)?;

    let (bytes_in, bytes_out) = proxy.pid.map_or((0, 0), traffic_for_process_tree);
    let relay_connected = proxy.running;

    Ok(ProxyStatus {
        label: PROXY_LABEL,
        mode,
        enabled: proxy.running,
        connected: proxy.running && service::verify().is_ok(),
        relay_connected,
        pid: proxy.pid,
        bytes_in,
        bytes_out,
        controller_path: controller_path(),
    })
}

pub fn set_enabled(enabled: bool) -> Result<ProxyStatus, ProxyError> {
    service::set_enabled(enabled)?;
    status()
}

pub fn set_mode(mode: ProxyMode) -> Result<ProxyStatus, ProxyError> {
    service::set_mode(mode.as_str())?;
    status()
}

pub fn verify() -> Result<VerifyResult, ProxyError> {
    service::verify()?;
    Ok(VerifyResult {
        ok: true,
        message: "End-to-end proxy check passed".into(),
    })
}
fn selected_mode() -> ProxyMode {
    match service::load().ok().map(|c| c.mode).as_deref() {
        Some("nordvpn") => ProxyMode::Nordvpn,
        Some("upstream") => ProxyMode::Upstream,
        _ => ProxyMode::Direct,
    }
}

fn launchd_state(label: &str) -> Result<JobState, ProxyError> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/{label}");
    let output = command_output("/bin/launchctl", &["print", &target])?;
    if !output.status.success() {
        return Ok(JobState::default());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let running = text.lines().any(|line| line.trim() == "state = running");
    let pid = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("pid = ")
            .and_then(|value| value.parse::<u32>().ok())
    });
    Ok(JobState { running, pid })
}

fn current_uid() -> Result<String, ProxyError> {
    let output = command_output("/usr/bin/id", &["-u"])?;
    if !output.status.success() {
        return Err(command_failure("/usr/bin/id", &output));
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if uid.is_empty() {
        Err(ProxyError::MissingUserId)
    } else {
        Ok(uid)
    }
}

fn traffic_for_process_tree(root_pid: u32) -> (u64, u64) {
    let pids = process_tree_pids(root_pid);
    let mut args = vec![
        "-P".to_owned(),
        "-L".to_owned(),
        "1".to_owned(),
        "-J".to_owned(),
        "bytes_in,bytes_out".to_owned(),
    ];
    for pid in pids {
        args.push("-p".to_owned());
        args.push(pid.to_string());
    }

    let Ok(output) = Command::new("/usr/bin/nettop").args(&args).output() else {
        return (0, 0);
    };
    if !output.status.success() {
        return (0, 0);
    }

    parse_nettop_traffic(&output.stdout)
}

fn process_tree_pids(root_pid: u32) -> Vec<u32> {
    let mut pids = vec![root_pid];
    let Ok(output) = command_output("/bin/ps", &["-axo", "pid=,ppid="]) else {
        return pids;
    };
    if !output.status.success() {
        return pids;
    }

    let processes: Vec<(u32, u32)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let pid = columns.next()?.parse::<u32>().ok()?;
            let parent_pid = columns.next()?.parse::<u32>().ok()?;
            Some((pid, parent_pid))
        })
        .collect();

    let mut index = 0;
    while let Some(parent_pid) = pids.get(index).copied() {
        for (pid, process_parent_pid) in &processes {
            if *process_parent_pid == parent_pid && !pids.contains(pid) {
                pids.push(*pid);
            }
        }
        index += 1;
    }
    pids
}

fn parse_nettop_traffic(output: &[u8]) -> (u64, u64) {
    String::from_utf8_lossy(output)
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split(',');
            let _process = columns.next()?;
            let bytes_in = columns.next()?.parse::<u64>().ok()?;
            let bytes_out = columns.next()?.parse::<u64>().ok()?;
            Some((bytes_in, bytes_out))
        })
        .fold((0, 0), |(total_in, total_out), (bytes_in, bytes_out)| {
            (
                total_in.saturating_add(bytes_in),
                total_out.saturating_add(bytes_out),
            )
        })
}

fn controller_path() -> Option<PathBuf> {
    service::directory()
        .ok()
        .map(|p| p.join("homeproxy"))
        .filter(|p| p.is_file())
}

fn command_output(program: &str, args: &[&str]) -> Result<Output, ProxyError> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|source| ProxyError::CommandIo {
            program: program.to_owned(),
            source,
        })
}

fn command_failure(program: impl AsRef<Path>, output: &Output) -> ProxyError {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    ProxyError::CommandFailed {
        program: program.as_ref().display().to_string(),
        message: if stderr.is_empty() { stdout } else { stderr },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_mode_serializes_for_frontend() {
        let serialized = serde_json::to_string(&ProxyMode::Nordvpn);
        assert!(matches!(serialized.as_deref(), Ok("\"nordvpn\"")));
    }

    #[test]
    fn parses_all_selected_nettop_process_rows() {
        let output = b",bytes_in,bytes_out,\nproxy.100,17,23,\nssh.101,7018,10038,\n";
        assert_eq!(parse_nettop_traffic(output), (7035, 10061));
    }
}
