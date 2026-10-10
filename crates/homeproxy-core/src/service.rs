use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::{
    env, fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;

pub const LABEL: &str = "dev.homeproxy.connector";
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("HomeProxy I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid HomeProxy configuration: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} failed")]
    Command(&'static str),
}
type Result<T> = std::result::Result<T, ServiceError>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub gateway: String,
    pub ssh_port: u16,
    pub user: String,
    pub identity_file: PathBuf,
    pub known_hosts_file: PathBuf,
    pub remote_port: u16,
    pub verify_port: u16,
    pub relay_port: u16,
    pub mode: String,
    pub upstream: Option<Upstream>,
}
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

pub fn directory() -> Result<PathBuf> {
    env::var_os("HOMEPROXY_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config/homeproxy")))
        .ok_or(ServiceError::Invalid("HOME is unavailable"))
}
pub fn load() -> Result<Config> {
    let file = directory()?.join("config.json");
    if fs::metadata(&file)?.permissions().mode() & 0o077 != 0 {
        return Err(ServiceError::Invalid("config.json must have mode 600"));
    }
    let config: Config = serde_json::from_slice(&fs::read(file)?)?;
    validate(&config)?;
    Ok(config)
}
fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
}
pub fn validate(c: &Config) -> Result<()> {
    if !safe_name(&c.gateway) || !safe_name(&c.user) {
        return Err(ServiceError::Invalid("Invalid gateway or SSH user"));
    }
    if [c.ssh_port, c.remote_port, c.verify_port, c.relay_port].contains(&0)
        || c.verify_port == c.relay_port
    {
        return Err(ServiceError::Invalid("Invalid ports"));
    }
    if !["direct", "upstream", "nordvpn"].contains(&c.mode.as_str()) {
        return Err(ServiceError::Invalid("Invalid mode"));
    }
    if !c.identity_file.is_absolute() || !c.known_hosts_file.is_absolute() {
        return Err(ServiceError::Invalid("SSH file paths must be absolute"));
    }
    if c.mode != "direct" && c.upstream.is_none() {
        return Err(ServiceError::Invalid(
            "Upstream credentials are required for this mode",
        ));
    }
    if let Some(u) = &c.upstream {
        if !safe_name(&u.host)
            || u.port == 0
            || u.username.is_empty()
            || u.username.len() > 255
            || u.password.is_empty()
            || u.password.len() > 255
        {
            return Err(ServiceError::Invalid("Invalid upstream configuration"));
        }
    }
    Ok(())
}
pub fn save(c: &Config) -> Result<()> {
    validate(c)?;
    let dir = directory()?;
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let tmp = dir.join("config.json.pending");
    let mut options = fs::OpenOptions::new();
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(c)?)?;
    file.sync_all()?;
    fs::rename(tmp, dir.join("config.json"))?;
    Ok(())
}
pub fn ssh_args(c: &Config) -> Vec<String> {
    vec![
        "-F".into(),
        "/dev/null".into(),
        "-NT".into(),
        "-p".into(),
        c.ssh_port.to_string(),
        "-i".into(),
        c.identity_file.display().to_string(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", c.known_hosts_file.display()),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-R".into(),
        if c.mode == "direct" {
            format!("127.0.0.1:{}", c.remote_port)
        } else {
            format!("127.0.0.1:{}:127.0.0.1:{}", c.remote_port, c.relay_port)
        },
        "-L".into(),
        format!("127.0.0.1:{}:127.0.0.1:{}", c.verify_port, c.remote_port),
        format!("{}@{}", c.user, c.gateway),
    ]
}
struct SleepAssertion(Child);
impl Drop for SleepAssertion {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Default)]
struct RouteHealth {
    failures: u8,
}
impl RouteHealth {
    fn observe(&mut self, healthy: bool) -> bool {
        self.failures = if healthy {
            0
        } else {
            self.failures.saturating_add(1)
        };
        self.failures >= 3
    }
}

fn check_socks_route(port: u16, timeout: Duration) -> Result<()> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(&[5, 1, 0])?;
    let mut reply = [0; 2];
    stream.read_exact(&mut reply)?;
    if reply != [5, 0] {
        return Err(ServiceError::Invalid("SOCKS route greeting rejected"));
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let c = load()?;
    if c.mode != "direct" {
        let upstream = c
            .upstream
            .as_ref()
            .ok_or(ServiceError::Invalid("Missing upstream"))?
            .clone();
        let listener = TcpListener::bind(("127.0.0.1", c.relay_port))?;
        thread::spawn(move || {
            for incoming in listener.incoming().flatten() {
                let settings = upstream.clone();
                thread::spawn(move || {
                    let _ = relay(incoming, &settings);
                });
            }
        });
    }
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, stopped.clone())?;
    }
    // The production route depends on this Mac while the connector is enabled.
    let _awake = SleepAssertion(
        Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
            .spawn()?,
    );
    let mut child = Command::new("/usr/bin/ssh").args(ssh_args(&c)).spawn()?;
    let mut health = RouteHealth::default();
    let mut next_check = Instant::now() + Duration::from_secs(15);
    loop {
        if stopped.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(ServiceError::Command("SSH tunnel"))
            };
        }
        if Instant::now() >= next_check {
            let healthy = check_socks_route(c.verify_port, Duration::from_secs(5)).is_ok();
            next_check = Instant::now() + Duration::from_secs(15);
            if health.observe(healthy) {
                // SSH keepalives do not prove that forwarded SOCKS requests work.
                eprintln!("SOCKS route failed three health checks; restarting connector");
                let _ = child.kill();
                let _ = child.wait();
                return Err(ServiceError::Command("SOCKS route health"));
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}
fn relay(mut client: TcpStream, u: &Upstream) -> Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(15)))?;
    client.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting)?;
    if greeting[0] != 5 {
        return Err(ServiceError::Invalid("Invalid SOCKS version"));
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    client.read_exact(&mut methods)?;
    if !methods.contains(&0) {
        client.write_all(&[5, 255])?;
        return Err(ServiceError::Invalid("SOCKS authentication unavailable"));
    }
    let mut upstream = TcpStream::connect((u.host.as_str(), u.port))?;
    upstream.set_read_timeout(Some(Duration::from_secs(15)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(15)))?;
    upstream.write_all(&[5, 1, 2])?;
    upstream.read_exact(&mut greeting)?;
    if greeting != [5, 2] {
        return Err(ServiceError::Invalid("Upstream authentication unavailable"));
    }
    let mut auth = vec![1, u.username.len() as u8];
    auth.extend(u.username.as_bytes());
    auth.push(u.password.len() as u8);
    auth.extend(u.password.as_bytes());
    upstream.write_all(&auth)?;
    upstream.read_exact(&mut greeting)?;
    if greeting != [1, 0] {
        return Err(ServiceError::Invalid("Upstream authentication failed"));
    }
    client.write_all(&[5, 0])?;
    client.set_read_timeout(None)?;
    client.set_write_timeout(None)?;
    upstream.set_read_timeout(None)?;
    upstream.set_write_timeout(None)?;
    let mut outgoing = upstream.try_clone()?;
    let mut source = client.try_clone()?;
    let copy = thread::spawn(move || {
        let _ = std::io::copy(&mut source, &mut outgoing);
        let _ = outgoing.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(std::net::Shutdown::Write);
    let _ = copy.join();
    Ok(())
}
fn uid() -> Result<String> {
    let result = Command::new("/usr/bin/id").arg("-u").output()?;
    if !result.status.success() {
        return Err(ServiceError::Command("id"));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().into())
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn plist() -> Result<PathBuf> {
    Ok(
        PathBuf::from(env::var_os("HOME").ok_or(ServiceError::Invalid("HOME unavailable"))?)
            .join(format!("Library/LaunchAgents/{LABEL}.plist")),
    )
}
pub fn install(executable: &Path) -> Result<()> {
    let _ = load()?;
    let dir = directory()?;
    let installed = dir.join("homeproxy");
    if executable != installed {
        fs::copy(executable, &installed)?;
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o700))?;
    }
    let target = plist()?;
    fs::create_dir_all(
        target
            .parent()
            .ok_or(ServiceError::Invalid("Invalid plist path"))?,
    )?;
    fs::write(target,format!("<?xml version=\"1.0\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string><string>run</string></array><key>EnvironmentVariables</key><dict><key>HOMEPROXY_CONFIG_DIR</key><string>{}</string></dict><key>KeepAlive</key><true/><key>RunAtLoad</key><true/><key>ThrottleInterval</key><integer>10</integer><key>StandardErrorPath</key><string>{}</string></dict></plist>",xml(&installed.display().to_string()),xml(&dir.display().to_string()),xml(&dir.join("service.log").display().to_string())))?;
    set_enabled(true)
}
pub fn set_enabled(enabled: bool) -> Result<()> {
    let domain = format!("gui/{}", uid()?);
    let target = format!("{domain}/{LABEL}");
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &target])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if enabled {
        // launchd can finish bootout asynchronously while the old worker exits.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Command::new("/bin/launchctl")
            .args(["print", &target])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success()
        {
            if Instant::now() >= deadline {
                return Err(ServiceError::Command("launchctl bootout completion"));
            }
            thread::sleep(Duration::from_millis(200));
        }
        let path = plist()?;
        for attempt in 0..3 {
            if Command::new("/bin/launchctl")
                .arg("bootstrap")
                .arg(&domain)
                .arg(&path)
                .status()?
                .success()
            {
                return Ok(());
            }
            if attempt < 2 {
                thread::sleep(Duration::from_secs(1));
            }
        }
        return Err(ServiceError::Command("launchctl bootstrap"));
    }
    Ok(())
}
pub fn set_mode(mode: &str) -> Result<()> {
    let mut c = load()?;
    c.mode = mode.into();
    save(&c)?;
    set_enabled(true)
}
pub fn verify() -> Result<()> {
    let c = load()?;
    if !Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--silent",
            "--max-time",
            "15",
            "--proxy",
            &format!("socks5h://127.0.0.1:{}", c.verify_port),
            "https://example.com/",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success()
    {
        return Err(ServiceError::Command("End-to-end proxy check"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_consecutive_route_failures_before_restarting() {
        let mut health = RouteHealth::default();
        assert!(!health.observe(false));
        assert!(!health.observe(false));
        assert!(!health.observe(true));
        assert!(!health.observe(false));
        assert!(!health.observe(false));
        assert!(health.observe(false));
    }

    #[test]
    fn checks_the_socks_greeting_and_bounds_a_stalled_route() -> Result<()> {
        for healthy in [true, false] {
            let listener = TcpListener::bind(("127.0.0.1", 0))?;
            let port = listener.local_addr()?.port();
            let server = thread::spawn(move || -> Result<()> {
                let (mut stream, _) = listener.accept()?;
                let mut hello = [0; 3];
                stream.read_exact(&mut hello)?;
                assert_eq!(hello, [5, 1, 0]);
                if healthy {
                    stream.write_all(&[5])?;
                    stream.write_all(&[0])?;
                } else {
                    thread::sleep(Duration::from_millis(100));
                }
                Ok(())
            });
            assert_eq!(
                check_socks_route(port, Duration::from_millis(25)).is_ok(),
                healthy
            );
            server
                .join()
                .map_err(|_| ServiceError::Invalid("fixture failed"))??;
        }
        Ok(())
    }

    fn config() -> Config {
        Config {
            gateway: "gateway.example.com".into(),
            ssh_port: 2222,
            user: "proxy".into(),
            identity_file: "/tmp/key".into(),
            known_hosts_file: "/tmp/known_hosts".into(),
            remote_port: 18080,
            verify_port: 18083,
            relay_port: 18082,
            mode: "direct".into(),
            upstream: None,
        }
    }
    #[test]
    fn pins_host_keys_and_keeps_listeners_private() {
        let c = config();
        assert!(validate(&c).is_ok());
        let args = ssh_args(&c);
        assert!(args.iter().any(|v| v == "StrictHostKeyChecking=yes"));
        assert!(args.iter().any(|v| v == "127.0.0.1:18080"));
        assert!(args.iter().any(|v| v == "127.0.0.1:18083:127.0.0.1:18080"));
        assert!(!args.iter().any(|v| v == "accept-new"));
    }
    #[test]
    fn rejects_options_and_missing_upstream_without_falling_back() {
        let mut c = config();
        c.gateway = "-oProxyCommand=bad".into();
        assert!(validate(&c).is_err());
        c.gateway = "gateway.example.com".into();
        c.mode = "upstream".into();
        assert!(validate(&c).is_err());
        c.upstream = Some(Upstream {
            host: "proxy.example.com".into(),
            port: 1080,
            username: "fixture-user".into(),
            password: "fixture-password".into(),
        });
        assert!(validate(&c).is_ok());
        assert!(ssh_args(&c).iter().all(|v| !v.contains("fixture-password")));
        assert!(ssh_args(&c)
            .iter()
            .any(|v| v == "127.0.0.1:18080:127.0.0.1:18082"));
    }
    #[test]
    fn escapes_launchd_values() {
        assert_eq!(xml("a&<\"'>"), "a&amp;&lt;&quot;&apos;&gt;");
    }
    #[test]
    fn relays_authenticated_socks_without_exposing_credentials_to_client(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let upstream = TcpListener::bind("127.0.0.1:0")?;
        let upstream_port = upstream.local_addr()?.port();
        let fake = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = upstream.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            let mut hello = [0u8; 3];
            stream.read_exact(&mut hello)?;
            assert_eq!(hello, [5, 1, 2]);
            stream.write_all(&[5, 2])?;
            let mut auth = [0u8; 5];
            stream.read_exact(&mut auth)?;
            assert_eq!(auth, [1, 1, b'u', 1, b'p']);
            stream.write_all(&[1, 0])?;
            let mut request = [0u8; 4];
            stream.read_exact(&mut request)?;
            assert_eq!(&request, b"test");
            stream.write_all(b"okay")?;
            Ok(())
        });
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let bridge = thread::spawn(move || -> Result<()> {
            let (stream, _) = listener.accept()?;
            relay(
                stream,
                &Upstream {
                    host: "127.0.0.1".into(),
                    port: upstream_port,
                    username: "u".into(),
                    password: "p".into(),
                },
            )
        });
        let mut client = TcpStream::connect(("127.0.0.1", port))?;
        client.set_read_timeout(Some(Duration::from_secs(3)))?;
        client.write_all(&[5, 1, 0])?;
        let mut hello = [0u8; 2];
        client.read_exact(&mut hello)?;
        assert_eq!(hello, [5, 0]);
        client.write_all(b"test")?;
        client.shutdown(std::net::Shutdown::Write)?;
        let mut response = [0u8; 4];
        client.read_exact(&mut response)?;
        assert_eq!(&response, b"okay");
        assert!(fake.join().is_ok_and(|r| r.is_ok()));
        assert!(bridge.join().is_ok_and(|r| r.is_ok()));
        Ok(())
    }
}
