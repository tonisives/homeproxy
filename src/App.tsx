import { invoke } from "@tauri-apps/api/core"
import { useCallback, useEffect, useRef, useState } from "react"

type ProxyMode = "direct" | "nordvpn" | "upstream"

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

type VerifyResult = {
  ok: boolean
  message: string
}

type Sample = {
  pid: number | null
  bytes: number
  time: number
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

let formatRate = (bytesPerSecond: number) => `${formatBytes(bytesPerSecond)}/s`

let normalizeError = (error: unknown) =>
  error instanceof Error ? error.message : String(error)

export let App = () => {
  let [status, setStatus] = useState<ProxyStatus | null>(null)
  let [rate, setRate] = useState(0)
  let [busy, setBusy] = useState(false)
  let [error, setError] = useState("")
  let [verification, setVerification] = useState("")
  let previousSample = useRef<Sample | null>(null)
  let firstRefresh = useRef(true)

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
  }, [])

  let refresh = useCallback(async (showErrors = false) => {
    try {
      let next = await invoke<ProxyStatus>("get_proxy_status")
      applyStatus(next)
      if (showErrors) setError("")
    } catch (refreshError) {
      if (showErrors) setError(normalizeError(refreshError))
    }
  }, [applyStatus])

  useEffect(() => {
    let stopped = false
    let timer: number | undefined

    let poll = async () => {
      await refresh(firstRefresh.current)
      firstRefresh.current = false
      if (!stopped) timer = window.setTimeout(poll, 2500)
    }

    void poll()
    return () => {
      stopped = true
      if (timer) window.clearTimeout(timer)
    }
  }, [refresh])

  let runAction = async (action: () => Promise<ProxyStatus>) => {
    setBusy(true)
    setError("")
    setVerification("")
    try {
      applyStatus(await action())
    } catch (actionError) {
      setError(normalizeError(actionError))
      await refresh()
    } finally {
      setBusy(false)
    }
  }

  let toggleEnabled = () => {
    if (!status) return
    void runAction(() => invoke<ProxyStatus>("set_proxy_enabled", { enabled: !status.enabled }))
  }

  let chooseMode = (mode: ProxyMode) => {
    if (!status || status.mode === mode) return
    void runAction(() => invoke<ProxyStatus>("set_proxy_mode", { mode }))
  }

  let verify = async () => {
    setBusy(true)
    setError("")
    setVerification("")
    try {
      let result = await invoke<VerifyResult>("verify_proxy")
      setVerification(result.message)
      await refresh()
    } catch (verifyError) {
      setError(normalizeError(verifyError))
    } finally {
      setBusy(false)
    }
  }

  let connected = status?.connected ?? false
  let total = (status?.bytesIn ?? 0) + (status?.bytesOut ?? 0)

  return (
    <main>
      <header className="titlebar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <span className="brand-mark" aria-hidden="true">HP</span>
          <div data-tauri-drag-region>
            <h1 data-tauri-drag-region>HomeProxy</h1>
            <p data-tauri-drag-region>{status?.label ?? "Loading service"}</p>
          </div>
        </div>
        <div className={`connection-pill ${connected ? "connected" : "offline"}`}>
          <span className="status-dot" />
          {connected ? "Connected" : status?.enabled ? "Starting" : "Offline"}
        </div>
      </header>

      <section className="hero-panel">
        <div className="route-visual" aria-hidden="true">
          <div className="route-node home-node">HOME</div>
          <div className={`route-line ${connected ? "active" : ""}`}>
            <span />
          </div>
          <div className={`route-node exit-node ${status?.mode === "nordvpn" ? "nord" : "direct"}`}>
            {status?.mode === "nordvpn" ? "NORD" : "ISP"}
          </div>
        </div>

        <div className="hero-copy">
          <span className="eyebrow">Trends egress route</span>
          <h2>{status?.mode === "nordvpn" ? "Home tunnel via NordVPN" : "Direct through home network"}</h2>
          <p>
            {status?.mode === "nordvpn"
              ? "Only this proxy is relayed through Nord's authenticated SOCKS endpoint."
              : "Traffic exits through your home ISP without the Nord relay."}
          </p>
        </div>

        <button
          className={`power-button ${status?.enabled ? "on" : ""}`}
          type="button"
          disabled={!status || busy}
          onClick={toggleEnabled}
          aria-label={status?.enabled ? "Turn proxy off" : "Turn proxy on"}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 2v10M6.3 5.8a8 8 0 1 0 11.4 0" />
          </svg>
          <span>{status?.enabled ? "Turn off" : "Turn on"}</span>
        </button>
      </section>

      <section className="controls-grid">
        <div className="card route-card">
          <div className="card-heading">
            <div>
              <span className="eyebrow">Proxy route</span>
              <h3>Exit mode</h3>
            </div>
            <span className="mode-badge">{status?.mode === "nordvpn" ? "Sweden" : "Home"}</span>
          </div>
          <div className="segmented-control">
            <button
              type="button"
              className={status?.mode === "direct" ? "selected" : ""}
              disabled={!status || busy}
              onClick={() => chooseMode("direct")}
            >
              <strong>Direct</strong>
              <span>Home ISP</span>
            </button>
            <button
              type="button"
              className={status?.mode === "nordvpn" ? "selected" : ""}
              disabled={!status || busy}
              onClick={() => chooseMode("nordvpn")}
            >
              <strong>NordVPN</strong>
              <span>Proxy only</span>
            </button>
          </div>
        </div>

        <div className="card traffic-card">
          <div className="card-heading">
            <div>
              <span className="eyebrow">Managed tunnel</span>
              <h3>Traffic</h3>
            </div>
            <span className="live-rate">{formatRate(rate)}</span>
          </div>
          <div className="traffic-total">{formatBytes(total)}</div>
          <div className="traffic-split">
            <span><i className="down" />Received <strong>{formatBytes(status?.bytesIn ?? 0)}</strong></span>
            <span><i className="up" />Sent <strong>{formatBytes(status?.bytesOut ?? 0)}</strong></span>
          </div>
        </div>
      </section>

      <section className="service-bar">
        <div>
          <span className="service-icon">PID</span>
          <div>
            <strong>{status?.pid ?? "—"}</strong>
            <span>{status?.controllerPath ?? "Controller not found"}</span>
          </div>
        </div>
        <button type="button" className="verify-button" disabled={!connected || busy} onClick={() => void verify()}>
          {busy ? "Working…" : "Verify route"}
        </button>
      </section>

      {(error || verification) && (
        <aside className={error ? "message error" : "message success"} role="status">
          {error || verification}
        </aside>
      )}

      <footer>
        <span>CLI</span>
        <code>homeproxy status</code>

      </footer>
    </main>
  )
}
