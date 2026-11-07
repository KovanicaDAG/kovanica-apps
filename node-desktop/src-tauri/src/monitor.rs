//! Read-only loopback monitor.
//!
//! Wire shapes were read from KovanicaDAG/kovanica
//! `74045cf3e27a70569c32af35aec24ada5dd9ff79`, not from kovanica-sdk.
//!
//! `GET /health` flattens `HealthSnapshot` beside `status` and `peer_count`
//! (`crates/kovanica-rpc/src/lib.rs` health_check, `health.rs`).
//! `kovanica_syncStatus` uses camelCase (`committedHeight`, `peerCount`,
//! `networkTag`, peer `peerId` / `reportedTipRound`) in
//! `crates/kovanica-rpc/src/chain_methods.rs`.
//!
//! A missing or wrong-typed field is unknown. It is never shown as zero.
//! An explicit numeric zero stays zero. `catchingUp: false` is not a claim
//! that the node has finished syncing.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
pub const BASE_POLL: Duration = Duration::from_secs(2);
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Value,
    Unknown,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub kind: FieldKind,
    pub value: Option<String>,
}

impl Field {
    pub fn value(text: impl Into<String>) -> Self {
        Self {
            kind: FieldKind::Value,
            value: Some(text.into()),
        }
    }

    pub fn unknown() -> Self {
        Self {
            kind: FieldKind::Unknown,
            value: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            kind: FieldKind::Unavailable,
            value: None,
        }
    }

    fn from_u64(value: Option<u64>, failed: bool) -> Self {
        match (failed, value) {
            (true, _) => Self::unavailable(),
            (false, Some(number)) => Self::value(number.to_string()),
            (false, None) => Self::unknown(),
        }
    }

    fn from_bool(value: Option<bool>, failed: bool) -> Self {
        match (failed, value) {
            (true, _) => Self::unavailable(),
            (false, Some(flag)) => Self::value(if flag { "true" } else { "false" }),
            (false, None) => Self::unknown(),
        }
    }

    fn from_str(value: Option<&str>, failed: bool) -> Self {
        match (failed, value) {
            (true, _) => Self::unavailable(),
            (false, Some(text)) if !text.is_empty() => Self::value(text),
            (false, _) => Self::unknown(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthView {
    pub http_status: Field,
    pub status: Field,
    pub healthy: Field,
    pub peer_count: Field,
    pub execution_state: Field,
    pub consensus_state: Field,
    pub last_error: Field,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    pub committed_height: Field,
    pub local_tip_round: Field,
    pub catching_up: Field,
    pub peer_count: Field,
    pub network_tag: Field,
    pub peer_ids: Field,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RpcReach {
    Unknown,
    Reachable,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorView {
    pub rpc: RpcReach,
    pub health: HealthView,
    pub sync: SyncView,
    pub stale: bool,
    pub last_success_age_ms: Option<u64>,
    pub detail: Option<String>,
}

impl MonitorView {
    pub fn unknown() -> Self {
        Self {
            rpc: RpcReach::Unknown,
            health: HealthView::blank(false),
            sync: SyncView::blank(false),
            stale: false,
            last_success_age_ms: None,
            detail: None,
        }
    }
}

impl HealthView {
    pub fn unavailable() -> Self {
        Self::blank(true)
    }

    fn blank(failed: bool) -> Self {
        let field = if failed {
            Field::unavailable()
        } else {
            Field::unknown()
        };
        Self {
            http_status: field.clone(),
            status: field.clone(),
            healthy: field.clone(),
            peer_count: field.clone(),
            execution_state: field.clone(),
            consensus_state: field.clone(),
            last_error: field,
        }
    }
}

impl SyncView {
    pub fn unavailable() -> Self {
        Self::blank(true)
    }

    fn blank(failed: bool) -> Self {
        let field = if failed {
            Field::unavailable()
        } else {
            Field::unknown()
        };
        Self {
            committed_height: field.clone(),
            local_tip_round: field.clone(),
            catching_up: field.clone(),
            peer_count: field.clone(),
            network_tag: field.clone(),
            peer_ids: field,
        }
    }
}

/// `true` when a new HTTP read may start.
pub fn poll_due(paused: bool, in_flight: bool, now: Instant, next_allowed: Instant) -> bool {
    !paused && !in_flight && now >= next_allowed
}

/// A response belongs to the generation that started it.
pub fn accept_generation(started: u64, current: u64) -> bool {
    started == current
}

/// Failure backoff: 2s, 4s, 8s, … capped at 30s. Zero failures uses the base interval.
pub fn next_wait(failures: u32) -> Duration {
    if failures == 0 {
        return BASE_POLL;
    }
    let shift = failures.saturating_sub(1).min(4);
    BASE_POLL.saturating_mul(1_u32 << shift).min(MAX_BACKOFF)
}

pub fn parse_health(http_status: u16, body: &str) -> HealthView {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return HealthView::blank(true);
    };
    let Some(object) = value.as_object() else {
        return HealthView::blank(true);
    };
    HealthView {
        http_status: Field::value(http_status.to_string()),
        status: match object.get("status") {
            Some(Value::String(text)) if text == "ok" || text == "degraded" => Field::value(text),
            Some(_) => Field::unknown(),
            None => Field::unknown(),
        },
        healthy: Field::from_bool(object.get("healthy").and_then(Value::as_bool), false),
        peer_count: Field::from_u64(json_u64(object.get("peer_count")), false),
        execution_state: component_state(object.get("execution")),
        consensus_state: component_state(object.get("consensus")),
        last_error: match object.get("last_error") {
            Some(Value::String(text)) if !text.is_empty() => Field::value(text),
            Some(Value::Null) | None => Field::unknown(),
            Some(_) => Field::unknown(),
        },
    }
}

pub fn parse_sync_envelope(body: &str) -> SyncView {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return SyncView::blank(true);
    };
    let Some(object) = value.as_object() else {
        return SyncView::blank(true);
    };
    if matches!(object.get("error"), Some(value) if !value.is_null()) {
        return SyncView::blank(true);
    }
    let Some(result) = object.get("result") else {
        return SyncView::blank(true);
    };
    let Some(result) = result.as_object() else {
        return SyncView::blank(true);
    };
    SyncView {
        committed_height: Field::from_u64(json_u64(result.get("committedHeight")), false),
        local_tip_round: Field::from_u64(json_u64(result.get("localTipRound")), false),
        catching_up: Field::from_bool(result.get("catchingUp").and_then(Value::as_bool), false),
        peer_count: Field::from_u64(json_u64(result.get("peerCount")), false),
        network_tag: Field::from_str(result.get("networkTag").and_then(Value::as_str), false),
        peer_ids: peer_ids(result.get("peers")),
    }
}

fn component_state(value: Option<&Value>) -> Field {
    let Some(object) = value.and_then(Value::as_object) else {
        return Field::unknown();
    };
    match object.get("state").and_then(Value::as_str) {
        Some(state @ ("starting" | "running" | "stopped" | "failed")) => Field::value(state),
        Some(_) | None => Field::unknown(),
    }
}

fn peer_ids(value: Option<&Value>) -> Field {
    let Some(list) = value.and_then(Value::as_array) else {
        return Field::unknown();
    };
    let mut ids = Vec::new();
    for item in list {
        let Some(peer_id) = item.get("peerId").and_then(Value::as_str) else {
            return Field::unknown();
        };
        if peer_id.is_empty() {
            return Field::unknown();
        }
        ids.push(peer_id.to_string());
    }
    Field::value(if ids.is_empty() {
        "(none)".to_string()
    } else {
        ids.join(", ")
    })
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number.as_u64(),
        _ => None,
    }
}

#[derive(Debug)]
pub struct PollGate {
    pub paused: bool,
    pub in_flight: bool,
    pub generation: u64,
    pub failures: u32,
    pub next_allowed: Instant,
    pub last_success: Option<Instant>,
    pub last: MonitorView,
}

impl PollGate {
    pub fn new(now: Instant) -> Self {
        Self {
            paused: false,
            in_flight: false,
            generation: 1,
            failures: 0,
            next_allowed: now,
            last_success: None,
            last: MonitorView::unknown(),
        }
    }

    pub fn begin(&mut self, now: Instant) -> Option<u64> {
        if !poll_due(self.paused, self.in_flight, now, self.next_allowed) {
            return None;
        }
        self.in_flight = true;
        Some(self.generation)
    }

    pub fn finish_ok(&mut self, started: u64, now: Instant, view: MonitorView) {
        self.in_flight = false;
        if !accept_generation(started, self.generation) {
            return;
        }
        self.failures = 0;
        self.next_allowed = now + next_wait(0);
        self.last_success = Some(now);
        self.last = view;
        self.last.stale = false;
        self.last.last_success_age_ms = Some(0);
    }

    pub fn finish_err(&mut self, started: u64, now: Instant, detail: String) {
        self.in_flight = false;
        if !accept_generation(started, self.generation) {
            return;
        }
        self.failures = self.failures.saturating_add(1);
        self.next_allowed = now + next_wait(self.failures);
        self.last.rpc = RpcReach::Unreachable;
        self.last.stale = self.last_success.is_some();
        self.last.last_success_age_ms = self
            .last_success
            .map(|then| now.saturating_duration_since(then).as_millis() as u64);
        self.last.detail = Some(detail);
        if self.last_success.is_none() {
            self.last.health = HealthView::blank(true);
            self.last.sync = SyncView::blank(true);
        }
    }

    pub fn bump_generation(&mut self, now: Instant) {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = false;
        self.failures = 0;
        self.next_allowed = now;
        self.last_success = None;
        self.last = MonitorView::unknown();
    }
}

pub fn endpoint_urls(rpc_http: &str) -> (String, String) {
    let base = rpc_http.trim_end_matches('/');
    (format!("{base}/health"), format!("{base}/rpc"))
}

pub async fn fetch_monitor(rpc_http: &str) -> Result<MonitorView, String> {
    let (health_url, rpc_url) = endpoint_urls(rpc_http);
    let client = reqwest::Client::builder()
        .timeout(HEALTH_TIMEOUT)
        .connect_timeout(HEALTH_TIMEOUT)
        .build()
        .map_err(|err| err.to_string())?;
    let health_response = client
        .get(&health_url)
        .send()
        .await
        .map_err(|err| format!("health request failed: {err}"))?;
    let health_status = health_response.status().as_u16();
    let health_body = health_response
        .text()
        .await
        .map_err(|err| format!("health body failed: {err}"))?;
    let sync_response = client
        .post(&rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "kovanica_syncStatus",
            "params": []
        }))
        .send()
        .await
        .map_err(|err| format!("sync request failed: {err}"))?;
    let sync_body = sync_response
        .text()
        .await
        .map_err(|err| format!("sync body failed: {err}"))?;
    let mut view = MonitorView {
        rpc: RpcReach::Reachable,
        health: parse_health(health_status, &health_body),
        sync: parse_sync_envelope(&sync_body),
        stale: false,
        last_success_age_ms: Some(0),
        detail: None,
    };
    if health_status != 200 && health_status != 503 {
        view.health = HealthView::blank(true);
        view.detail = Some(format!("health HTTP {health_status}"));
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEALTH_OK: &str = r#"{
      "status": "ok",
      "peer_count": 0,
      "healthy": true,
      "execution": {"state": "running", "executed_subdags": 1, "last_executed_round": 4, "seconds_since_last_execution": 2},
      "consensus": {"state": "running", "committed_leaders": 1, "last_committed_round": 3, "seconds_since_last_commit": 2, "commit_stall_secs": 120}
    }"#;

    const SYNC_OK: &str = r#"{
      "jsonrpc": "2.0",
      "id": 1,
      "result": {
        "localTipRound": 40,
        "committedHeight": 12,
        "localHeight": 12,
        "catchingUp": false,
        "peerCount": 2,
        "peers": [{"peerId": "12D3KooWexample", "reportedTipRound": 39}],
        "networkTag": "1337-abcdef0123456789"
      }
    }"#;

    #[test]
    fn health_zero_peer_count_is_zero_and_missing_is_unknown() {
        let parsed = parse_health(200, HEALTH_OK);
        assert_eq!(parsed.status, Field::value("ok"));
        assert_eq!(parsed.peer_count, Field::value("0"));
        assert_eq!(parsed.execution_state, Field::value("running"));
        let missing = parse_health(
            200,
            r#"{"status":"degraded","healthy":false,"execution":{"state":"failed"},"consensus":{"state":"stopped"}}"#,
        );
        assert_eq!(missing.status, Field::value("degraded"));
        assert_eq!(missing.peer_count, Field::unknown());
        assert_eq!(missing.peer_count.value, None);
    }

    #[test]
    fn sync_uses_camel_case_wire_shape() {
        let parsed = parse_sync_envelope(SYNC_OK);
        assert_eq!(parsed.committed_height, Field::value("12"));
        assert_eq!(parsed.local_tip_round, Field::value("40"));
        assert_eq!(parsed.catching_up, Field::value("false"));
        assert_eq!(parsed.peer_count, Field::value("2"));
        assert_eq!(parsed.network_tag, Field::value("1337-abcdef0123456789"));
        assert_eq!(parsed.peer_ids, Field::value("12D3KooWexample"));
    }

    #[test]
    fn malformed_missing_and_wrong_types_stay_unknown_or_unavailable() {
        assert_eq!(
            parse_sync_envelope("not-json").committed_height,
            Field::unavailable()
        );
        let empty = parse_sync_envelope(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        assert_eq!(empty.committed_height, Field::unknown());
        assert_eq!(empty.catching_up, Field::unknown());
        assert_eq!(empty.peer_count, Field::unknown());
        assert_ne!(empty.peer_count.value.as_deref(), Some("0"));
        let wrong = parse_sync_envelope(
            r#"{"jsonrpc":"2.0","id":1,"result":{"committedHeight":"0x10","catchingUp":"no","peerCount":1.5,"peers":[{"addr":"1"}]}}"#,
        );
        assert_eq!(wrong.committed_height, Field::unknown());
        assert_eq!(wrong.catching_up, Field::unknown());
        assert_eq!(wrong.peer_count, Field::unknown());
        assert_eq!(wrong.peer_ids, Field::unknown());
        let errored = parse_sync_envelope(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"nope"}}"#,
        );
        assert_eq!(errored.committed_height, Field::unavailable());
        assert_eq!(parse_health(200, "[]").status, Field::unavailable());
        assert_eq!(parse_health(503, r#"{"status":"degraded","healthy":false,"peer_count":3,"execution":{"state":"running"},"consensus":{"state":"running"}}"#).status, Field::value("degraded"));
    }

    #[test]
    fn explicit_zero_height_is_zero() {
        let parsed = parse_sync_envelope(
            r#"{"jsonrpc":"2.0","id":1,"result":{"committedHeight":0,"catchingUp":true,"peerCount":0,"peers":[]}}"#,
        );
        assert_eq!(parsed.committed_height, Field::value("0"));
        assert_eq!(parsed.peer_count, Field::value("0"));
        assert_eq!(parsed.catching_up, Field::value("true"));
        assert_eq!(parsed.peer_ids, Field::value("(none)"));
    }

    #[test]
    fn backoff_generation_and_overlap_rules() {
        assert_eq!(next_wait(0), Duration::from_secs(2));
        assert_eq!(next_wait(1), Duration::from_secs(2));
        assert_eq!(next_wait(2), Duration::from_secs(4));
        assert_eq!(next_wait(3), Duration::from_secs(8));
        assert_eq!(next_wait(8), Duration::from_secs(30));
        let now = Instant::now();
        assert!(!poll_due(true, false, now, now));
        assert!(!poll_due(false, true, now, now));
        assert!(poll_due(false, false, now, now));
        assert!(!poll_due(false, false, now, now + Duration::from_secs(5)));
        assert!(accept_generation(3, 3));
        assert!(!accept_generation(3, 4));

        let mut gate = PollGate::new(now);
        let started = gate.begin(now).unwrap();
        assert!(gate.begin(now).is_none(), "in-flight poll must not overlap");
        gate.finish_err(started, now, "down".into());
        assert_eq!(gate.last.rpc, RpcReach::Unreachable);
        assert!(!gate.last.stale);
        let again = gate.begin(now + next_wait(1)).unwrap();
        gate.finish_ok(again, now + next_wait(1), MonitorView::unknown());
        gate.last.rpc = RpcReach::Reachable;
        let stale_start = gate.begin(now + next_wait(1) + next_wait(0)).unwrap();
        gate.bump_generation(now);
        gate.finish_ok(stale_start, now, {
            let mut view = MonitorView::unknown();
            view.rpc = RpcReach::Reachable;
            view.sync.committed_height = Field::value("99");
            view
        });
        assert_eq!(gate.last.sync.committed_height, Field::unknown());
        assert_ne!(gate.last.sync.committed_height, Field::value("99"));
    }
}
