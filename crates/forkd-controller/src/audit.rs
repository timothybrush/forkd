//! Append-only audit log for daemon requests.
//!
//! One line of JSON per request. Fields:
//!   {ts: RFC3339, method, path, status, latency_us, remote, ua}
//!
//! Writes go through a single `Mutex<BufWriter<File>>` so concurrent
//! requests serialize on the lock for the line write only; the bulk
//! of request handling stays parallel.
//!
//! Designed to be tailed by an external log shipper (vector, fluentbit).
//! No rotation in-process — operators should plug in logrotate or run
//! the daemon under a journal that handles size caps.
use anyhow::{Context, Result};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use parking_lot::Mutex;
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct AuditSink {
    inner: Arc<AuditInner>,
}

struct AuditInner {
    writer: Mutex<BufWriter<File>>,
    path: PathBuf,
}

impl AuditSink {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create audit log parent {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open audit log {}", path.display()))?;
        Ok(Self {
            inner: Arc::new(AuditInner {
                writer: Mutex::new(BufWriter::new(file)),
                path,
            }),
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.inner.path
    }

    pub fn write(&self, line: serde_json::Value) {
        let mut w = self.inner.writer.lock();
        if let Err(e) = writeln!(w, "{line}") {
            tracing::warn!(error=%e, "audit write failed");
            return;
        }
        if let Err(e) = w.flush() {
            tracing::warn!(error=%e, "audit flush failed");
        }
    }
}

/// axum middleware that emits one audit line per request after the
/// handler returns. Captures method, path, status, wall-clock latency.
pub async fn audit_layer(sink: AuditSink, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let ua = req
        .headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let start = Instant::now();
    let resp = next.run(req).await;
    let latency_us = start.elapsed().as_micros();
    let status = resp.status().as_u16();
    let line = json!({
        "ts": now_rfc3339(),
        "method": method.as_str(),
        "path": path,
        "status": status,
        "latency_us": latency_us as u64,
        "ua": ua,
    });
    sink.write(line);
    resp
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Minimal RFC3339-ish without pulling in chrono. Seconds-precision
    // is plenty for audit purposes; the latency_us field captures the
    // sub-second cost of each handler.
    format_unix_seconds_as_rfc3339(secs)
}

fn format_unix_seconds_as_rfc3339(secs: u64) -> String {
    // 1970-01-01T00:00:00Z + secs. Algorithm from Howard Hinnant's
    // "civil from days" (public domain). Avoids the chrono dependency.
    let z = secs as i64;
    let days = z.div_euclid(86_400);
    let sec_of_day = z.rem_euclid(86_400);
    let hour = (sec_of_day / 3600) as u32;
    let minute = ((sec_of_day % 3600) / 60) as u32;
    let second = (sec_of_day % 60) as u32;

    let z2 = days + 719_468;
    let era = z2.div_euclid(146_097);
    let doe = z2.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (y + if m <= 2 { 1 } else { 0 }) as i32;

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_format_epoch_zero() {
        assert_eq!(format_unix_seconds_as_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn rfc3339_format_known_timestamp() {
        // 2024-01-01T00:00:00Z == 1704067200
        assert_eq!(
            format_unix_seconds_as_rfc3339(1_704_067_200),
            "2024-01-01T00:00:00Z"
        );
    }

    #[test]
    fn audit_sink_writes_and_persists() {
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path().join("audit.log");
        let sink = AuditSink::open(&path).unwrap();
        sink.write(json!({"a": 1}));
        sink.write(json!({"a": 2}));
        let contents = std::fs::read_to_string(&path).unwrap();
        let mut lines = contents.lines();
        assert!(lines.next().unwrap().contains("\"a\":1"));
        assert!(lines.next().unwrap().contains("\"a\":2"));
        assert!(lines.next().is_none());
    }
}
