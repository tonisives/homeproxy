import { invoke } from "@tauri-apps/api/core"
import { useCallback, useEffect, useRef, useState, type ChangeEvent } from "react"

type ProxyMode = "direct" | "nordvpn" | "upstream"
type ProxyAction = "power" | "mode" | "verify"
type ProxyStatus = {
  label: string
  mode: ProxyMode
  enabled: boolean
  connected: boolean
  relayConnected: boolean
  pid: number | null
  bytesIn: number
  bytesOut: number
  controllerPath: string | null
}
type VerifyResult = { ok: boolean; message: string }
type Sample = { pid: number | null; bytes: number; time: number }

const ROUTES: Record<ProxyMode, { label: string; description: string }> = {
  direct: { label: "Home network", description: "Browser workers use this Mac’s home internet connection." },
  nordvpn: { label: "NordVPN", description: "Proxy traffic uses the configured NordVPN SOCKS endpoint." },
  upstream: { label: "Upstream proxy", description: "Proxy traffic uses your configured upstream SOCKS endpoint." },
}

let formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`
  let units = ["KB", "MB", "GB", "TB"]
  let value = bytes / 1024
  let unit = units[0]
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024
    unit = units[index]
  }
  return `${value.toFixed(value >= 100 ? 0 : value >= 10 ? 1 : 2)} ${unit}`
}

let normalizeError = (error: unknown) => error instanceof Error ? error.message : String(error)

export let App = () => {
  let [status, setStatus] = useState<ProxyStatus | null>(null)
  let [rate, setRate] = useState(0)
  let [action, setAction] = useState<ProxyAction | null>(null)
  let [error, setError] = useState("")
  let [statusError, setStatusError] = useState("")
  let [verification, setVerification] = useState<VerifyResult | null>(null)
  let previousSample = useRef<Sample | null>(null)
  let actionInProgress = useRef(false)
  let statusVersion = useRef(0)

  let applyStatus = useCallback((next: ProxyStatus) => {
    let now = Date.now()
    let bytes = next.bytesIn + next.bytesOut
    let previous = previousSample.current
    if (previous && previous.pid === next.pid && bytes >= previous.bytes) {
      let elapsedSeconds = (now - previous.time) / 1000
      setRate(elapsedSeconds > 0 ? Math.round((bytes - previous.bytes) / elapsedSeconds) : 0)
    } else {
      setRate(0)
    }
    previousSample.current = { pid: next.pid, bytes, time: now }
    setStatus(next)
    setStatusError("")
  }, [])

  let refresh = useCallback(async () => {
    if (actionInProgress.current) return
    let version = ++statusVersion.current
    try {
      let next = await invoke<ProxyStatus>("get_proxy_status")
      if (version === statusVersion.current) applyStatus(next)
    } catch (refreshError) {
      if (version !== statusVersion.current) return
      previousSample.current = null
      setRate(0)
      setStatusError(normalizeError(refreshError))
    }
  }, [applyStatus])

  useEffect(() => {
    let stopped = false
    let timer: number | undefined
    let poll = async () => {
      await refresh()
      if (!stopped) timer = window.setTimeout(poll, 2500)
    }
    void poll()
    return () => {
      stopped = true
      statusVersion.current += 1
      if (timer) window.clearTimeout(timer)
    }
  }, [refresh])

  let runAction = async (kind: ProxyAction, operation: () => Promise<void>) => {
    if (actionInProgress.current) return
    actionInProgress.current = true
    statusVersion.current += 1
    setAction(kind)
    setError("")
    setVerification(null)
    try {
      await operation()
    } catch (actionError) {
      setError(normalizeError(actionError))
    } finally {
      actionInProgress.current = false
      await refresh()
      setAction(null)
    }
  }

  let toggleEnabled = () => {
    if (!status) return
    void runAction("power", async () => {
      applyStatus(await invoke<ProxyStatus>("set_proxy_enabled", { enabled: !status.enabled }))
    })
  }

  let chooseMode = (event: ChangeEvent<HTMLSelectElement>) => {
    let mode = event.target.value as ProxyMode
    if (!status || status.mode === mode) return
    void runAction("mode", async () => {
      applyStatus(await invoke<ProxyStatus>("set_proxy_mode", { mode }))
    })
  }

  let verify = () => {
    void runAction("verify", async () => {
      setVerification(await invoke<VerifyResult>("verify_proxy"))
    })
  }

  let retry = () => { void refresh() }
  let connected = Boolean(status?.connected && !statusError)
  let busy = action !== null
  let unavailable = Boolean(statusError)
  let route = status ? ROUTES[status.mode] : null
  let connectionTitle = unavailable ? "Status unavailable"
    : !status ? "Checking connection…"
    : connected ? "Proxy connected"
    : status.enabled ? "Connecting…" : "Proxy is off"
  let connectionDescription = unavailable ? "The connection status could not be refreshed. Try again to check this Mac."
    : !status ? "Checking the proxy service on this Mac."
    : connected ? "Remote browser workers can use this Mac’s connection."
    : status.enabled ? "The service is running. Waiting for the complete proxy route to respond."
    : "Turn on the proxy to share this Mac’s connection with browser workers."
  let message = error || verification?.message
  let messageIsError = Boolean(error || (verification && !verification.ok))
  let total = (status?.bytesIn ?? 0) + (status?.bytesOut ?? 0)
  let powerLabel = action === "power" ? "Updating…" : status?.enabled ? "Turn off" : "Turn on"

  return (
    <main className="app-window">
      <header className="toolbar" data-tauri-drag-region>
        <h1 data-tauri-drag-region>HomeProxy</h1>
      </header>
      <div className="content-area">
        <div className="content-column">
          <section aria-labelledby="connection-heading">
            <div className="section-heading">
              <h2 id="connection-heading">Connection</h2>
              <span className="secondary">On this Mac</span>
            </div>
            <div className="group">
              <div className="connection-row">
                <div className={`connection-symbol ${connected ? "is-connected" : ""}`} aria-hidden="true">
                  <svg viewBox="0 0 24 24">
                    <path d="M12 3 4 6v6c0 4 4 7 8 9 4-2 8-5 8-9V6l-8-3Z" />
                    <path d="m8 12 3 3 5-6" />
                  </svg>
                </div>
                <div className="connection-copy" role="status">
                  <h3>{connectionTitle}</h3>
                  <p>{connectionDescription}</p>
                </div>
                <button type="button" className={status?.enabled ? "" : "primary-button"}
                  disabled={!status || busy || unavailable} onClick={toggleEnabled}>
                  {powerLabel}
                </button>
              </div>
              <div className="connection-facts">
                <span>Exit route <strong>{route?.label ?? "—"}</strong></span>
                <span>Traffic <strong>{!status || unavailable ? "—" : `${formatBytes(rate)}/s`}</strong></span>
              </div>
              <div className="check-row">
                <span>Test the complete proxy route</span>
                <button type="button" disabled={!status?.enabled || busy || unavailable} onClick={verify}>
                  {action === "verify" ? "Checking…" : "Check connection"}
                </button>
              </div>
            </div>
          </section>
          {unavailable && (
            <aside className="message error-message" role="alert">
              <div><strong>Couldn’t read the proxy status</strong><p>{statusError}</p></div>
              <button type="button" disabled={busy} onClick={retry}>Try again</button>
            </aside>
          )}
          {message && (
            <aside className={`message ${messageIsError ? "error-message" : "success-message"}`}
              role={messageIsError ? "alert" : "status"}>{message}</aside>
          )}
          <section className="details-section" aria-label="Proxy settings and details">
            <details className="disclosure">
              <summary>Route settings <span className="summary-value">{route?.label ?? "—"}</span></summary>
              <div className="disclosure-content">
                <div className="setting-row">
                  <div><label htmlFor="exit-route">Exit route</label><p>{route?.description ?? "Choose where proxy traffic exits."}</p></div>
                  <select id="exit-route" value={status?.mode ?? "direct"} disabled={!status || busy || unavailable} onChange={chooseMode}>
                    <option value="direct">Home network</option>
                    <option value="nordvpn">NordVPN</option>
                    <option value="upstream">Upstream proxy</option>
                  </select>
                </div>
                <p className="footnote">NordVPN and upstream routes require a configured SOCKS endpoint. Changing the route reconnects the proxy.</p>
              </div>
            </details>
            <details className="disclosure">
              <summary>Traffic details <span className="summary-value">{!status || unavailable ? "—" : `${formatBytes(total)} total`}</span></summary>
              <div className="disclosure-content">
                <dl className="traffic-grid">
                  <div><dt>Received</dt><dd>{!status || unavailable ? "—" : formatBytes(status.bytesIn)}</dd></div>
                  <div><dt>Sent</dt><dd>{!status || unavailable ? "—" : formatBytes(status.bytesOut)}</dd></div>
                  <div><dt>Current rate</dt><dd>{!status || unavailable ? "—" : `${formatBytes(rate)}/s`}</dd></div>
                </dl>
                <p className="footnote">Traffic for the current service process. Totals reset when the service restarts.</p>
              </div>
            </details>
            <details className="disclosure">
              <summary>Service details <span className="summary-value">{unavailable ? "Unavailable" : !status ? "Checking…" : status.enabled ? "Running" : "Stopped"}</span></summary>
              <dl className="disclosure-content service-details">
                <div><dt>Service</dt><dd><code>{status?.label ?? "—"}</code></dd></div>
                <div><dt>Process ID</dt><dd>{status?.pid ?? "—"}</dd></div>
                <div><dt>Relay process</dt><dd>{unavailable ? "Unknown" : !status ? "—" : status.relayConnected ? "Running" : "Stopped"}</dd></div>
                <div><dt>Controller</dt><dd><code>{status ? status.controllerPath ?? "Not installed" : "—"}</code></dd></div>
              </dl>
            </details>
          </section>
        </div>
      </div>
      <footer className="statusbar">
        <span className={`connection-state ${connected ? "is-connected" : ""}`}><i aria-hidden="true" />{unavailable ? "Status unavailable" : !status ? "Checking connection" : connected ? "Connected" : status.enabled ? "Connecting" : "Off"}</span>
        <span>{action === "mode" ? "Updating route…" : busy ? "Working…" : "HomeProxy"}</span>
      </footer>
    </main>
  )
}
