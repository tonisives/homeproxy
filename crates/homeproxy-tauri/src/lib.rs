use homeproxy_core::{ProxyMode, ProxyStatus, VerifyResult};

#[tauri::command]
async fn get_proxy_status() -> Result<ProxyStatus, String> {
    tauri::async_runtime::spawn_blocking(homeproxy_core::status)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_proxy_enabled(enabled: bool) -> Result<ProxyStatus, String> {
    tauri::async_runtime::spawn_blocking(move || homeproxy_core::set_enabled(enabled))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_proxy_mode(mode: ProxyMode) -> Result<ProxyStatus, String> {
    tauri::async_runtime::spawn_blocking(move || homeproxy_core::set_mode(mode))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn verify_proxy() -> Result<VerifyResult, String> {
    tauri::async_runtime::spawn_blocking(homeproxy_core::verify)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_proxy_status,
            set_proxy_enabled,
            set_proxy_mode,
            verify_proxy
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
