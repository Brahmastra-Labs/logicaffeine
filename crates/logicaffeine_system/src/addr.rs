//! Address normalization — libp2p multiaddr → WebSocket URL.
//!
//! The compiled path dials peers with libp2p multiaddrs
//! (`/ip4/127.0.0.1/tcp/9944`); the interpreter rides the thin WS relay, which
//! speaks `ws://`. To keep one surface across both runtimes a program can write
//! `Connect to "/ip4/127.0.0.1/tcp/9944"` and have the interpreter accept it —
//! this maps the multiaddr to the relay's `ws://host:port`. Raw `ws://`/`wss://`
//! URLs pass through unchanged, so either form works.
//!
//! Pure string logic, no libp2p, no target gating — it compiles into the native
//! binary and the wasm bundle alike, so the interpreter normalizes addresses the
//! same way on both.

/// Normalize a peer address to a WebSocket URL the relay can dial.
///
/// Accepts:
/// - a libp2p multiaddr: `/ip4/H/tcp/P`, `/ip6/H/tcp/P`, `/dns4/H/tcp/P`
///   (also `/dns`, `/dns6`, `/dnsaddr`), with an optional `/ws` or `/wss`
///   transport suffix (default `ws`) and an optional trailing `/p2p/<id>`
///   (ignored — the relay addresses by host:port, not peer id);
/// - a raw `ws://…` or `wss://…` URL, returned unchanged.
///
/// IPv6 literals are bracketed (`/ip6/::1/tcp/9944` → `ws://[::1]:9944`).
///
/// Returns `Err` with a human message on anything it cannot map.
pub fn multiaddr_to_ws_url(addr: &str) -> Result<String, String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("address is empty".to_string());
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("ws://") || lower.starts_with("wss://") {
        return Ok(trimmed.to_string());
    }

    if !trimmed.starts_with('/') {
        return Err(format!("not a ws:// URL or multiaddr: {trimmed}"));
    }

    let segs: Vec<&str> = trimmed.split('/').skip(1).collect();
    let mut host: Option<(String, bool)> = None; // (host, is_ipv6_literal)
    let mut port: Option<String> = None;
    let mut scheme = "ws";

    let mut i = 0;
    while i < segs.len() {
        match segs[i] {
            "ip4" | "dns" | "dns4" | "dns6" | "dnsaddr" => {
                let v = segs.get(i + 1).filter(|s| !s.is_empty());
                let v = v.ok_or_else(|| {
                    format!("multiaddr '{trimmed}' has no host after /{}/", segs[i])
                })?;
                host = Some((v.to_string(), false));
                i += 2;
            }
            "ip6" => {
                let v = segs
                    .get(i + 1)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("multiaddr '{trimmed}' has no host after /ip6/"))?;
                host = Some((v.to_string(), true));
                i += 2;
            }
            "tcp" => {
                let v = segs
                    .get(i + 1)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("multiaddr '{trimmed}' has no port after /tcp/"))?;
                if v.parse::<u16>().is_err() {
                    return Err(format!(
                        "multiaddr '{trimmed}' has a non-numeric port '{v}'"
                    ));
                }
                port = Some(v.to_string());
                i += 2;
            }
            "ws" => {
                scheme = "ws";
                i += 1;
            }
            "wss" | "tls" => {
                scheme = "wss";
                i += 1;
            }
            "p2p" | "p2p-circuit" => {
                // The peer-id (and circuit-relay marker) carry no host:port; the
                // relay does not use them. Skip the marker and any value.
                i += if segs[i] == "p2p" { 2 } else { 1 };
            }
            other => {
                return Err(format!(
                    "multiaddr '{trimmed}' has unsupported protocol '/{other}'"
                ));
            }
        }
    }

    let (host, is_v6) =
        host.ok_or_else(|| format!("multiaddr '{trimmed}' has no host component"))?;
    let port = port.ok_or_else(|| format!("multiaddr '{trimmed}' has no /tcp/<port>"))?;
    let host = if is_v6 { format!("[{host}]") } else { host };
    Ok(format!("{scheme}://{host}:{port}"))
}

/// The canonical relay topic for a peer address — its stable identity on the
/// relay, computed identically by both endpoints.
///
/// A multiaddr or `ws://` URL normalizes to its `ws://` form (so
/// `/ip4/127.0.0.1/tcp/8000` and `ws://127.0.0.1:8000` name the same peer); any
/// other string (a bare name like `"alice"`) is used verbatim (trimmed). This is
/// what `Send … to <peer>` publishes on and what `Listen at "<me>"` subscribes
/// to, so a sender and a receiver that write the same address agree on the topic.
pub fn canonical_topic(addr: &str) -> String {
    multiaddr_to_ws_url(addr).unwrap_or_else(|_| addr.trim().to_string())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/addr/tests.rs"]
mod tests;
