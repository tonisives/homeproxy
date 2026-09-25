use homeproxy_core::{ProxyMode, ProxyStatus};
use std::env;
use std::process::ExitCode;

const USAGE: &str = "usage: homeproxy status [--json]
       homeproxy install|on|off|verify|logs
       homeproxy mode [direct|upstream|nordvpn]";

fn print_status(status: &ProxyStatus, json: bool) -> Result<(), String> {
    if json {
        let value = serde_json::to_string_pretty(status).map_err(|error| error.to_string())?;
        println!("{value}");
        return Ok(());
    }

    println!("service={}", status.label);
    println!("mode={}", status.mode.as_str());
    println!("connected={}", status.connected);
    println!(
        "pid={}",
        status
            .pid
            .map_or_else(|| "-".to_owned(), |pid| pid.to_string())
    );
    println!("bytes_in={}", status.bytes_in);
    println!("bytes_out={}", status.bytes_out);
    Ok(())
}

fn parse_mode(value: Option<String>) -> Result<Option<ProxyMode>, String> {
    match value.as_deref() {
        None => Ok(None),
        Some("direct") => Ok(Some(ProxyMode::Direct)),
        Some("upstream") => Ok(Some(ProxyMode::Upstream)),
        Some("nordvpn") => Ok(Some(ProxyMode::Nordvpn)),
        Some(_) => Err(USAGE.to_owned()),
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref().unwrap_or("status") {
        "run" => homeproxy_core::service::run().map_err(|e| e.to_string()),
        "install" => {
            homeproxy_core::service::install(&env::current_exe().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())
        }
        "logs" => {
            let file = homeproxy_core::service::directory()
                .map_err(|e| e.to_string())?
                .join("service.log");
            let result = std::process::Command::new("/usr/bin/tail")
                .arg("-n")
                .arg("80")
                .arg(file)
                .status()
                .map_err(|e| e.to_string())?;
            if result.success() {
                Ok(())
            } else {
                Err("Could not read logs".into())
            }
        }
        "status" => {
            let json = match args.next().as_deref() {
                None => false,
                Some("--json") => true,
                Some(_) => return Err(USAGE.to_owned()),
            };
            print_status(
                &homeproxy_core::status().map_err(|error| error.to_string())?,
                json,
            )
        }
        "on" => print_status(
            &homeproxy_core::set_enabled(true).map_err(|error| error.to_string())?,
            false,
        ),
        "off" => print_status(
            &homeproxy_core::set_enabled(false).map_err(|error| error.to_string())?,
            false,
        ),
        "mode" => match parse_mode(args.next())? {
            Some(mode) => print_status(
                &homeproxy_core::set_mode(mode).map_err(|error| error.to_string())?,
                false,
            ),
            None => {
                let status = homeproxy_core::status().map_err(|error| error.to_string())?;
                println!("{}", status.mode.as_str());
                Ok(())
            }
        },
        "verify" => {
            let result = homeproxy_core::verify().map_err(|error| error.to_string())?;
            println!("{}", result.message);
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
