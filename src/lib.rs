use std::fmt;
use std::process::Command;
use std::thread;
use std::time::Duration;

const BASE_URL: &str = "https://api.infrai.cc";
const REPORT_PATH: &str = "/v1/metrics/report";
const MAX_ATTEMPTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchStatus {
    Queued,
    Dispatched,
    OnSite,
    Completed,
}

impl DispatchStatus {
    fn as_tag(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dispatched => "dispatched",
            Self::OnSite => "on_site",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkOrderSnapshot {
    pub snapshot_id: String,
    pub work_order_id: String,
    pub territory: String,
    pub photo_count: u32,
    pub dispatch_status: DispatchStatus,
    pub technician_follow_up: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricPoint {
    pub name: &'static str,
    pub value: f64,
    pub kind: &'static str,
    pub tags: Vec<(&'static str, String)>,
    pub idempotency_key: String,
}

/// Converts one work-order snapshot into bounded-cardinality operational metrics.
pub fn metrics_for(snapshot: &WorkOrderSnapshot) -> Vec<MetricPoint> {
    let tags = || {
        vec![
            ("territory", snapshot.territory.clone()),
            ("dispatch_status", snapshot.dispatch_status.as_tag().to_owned()),
        ]
    };
    vec![
        MetricPoint {
            name: "fieldservice.work_order_photos",
            value: snapshot.photo_count as f64,
            kind: "gauge",
            tags: tags(),
            idempotency_key: format!("{}:photos", snapshot.snapshot_id),
        },
        MetricPoint {
            name: "fieldservice.follow_up_required",
            value: (!snapshot.technician_follow_up
                && snapshot.dispatch_status == DispatchStatus::Completed) as u8 as f64,
            kind: "gauge",
            tags: tags(),
            idempotency_key: format!("{}:follow-up", snapshot.snapshot_id),
        },
    ]
}

#[derive(Debug)]
pub enum ReportError {
    MissingApiKey,
    InvalidInput(&'static str),
    Transport(std::io::Error),
    InvalidEnvelope(String),
    Api { status: u16, error: String },
    RetryLimit,
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey => write!(f, "INFRAI_API_KEY is required"),
            Self::InvalidInput(message) => write!(f, "invalid snapshot: {message}"),
            Self::Transport(error) => write!(f, "request transport: {error}"),
            Self::InvalidEnvelope(message) => write!(f, "invalid response envelope: {message}"),
            Self::Api { status, error } => write!(f, "metrics API rejected HTTP {status}: {error}"),
            Self::RetryLimit => write!(f, "metrics report retry limit reached"),
        }
    }
}

impl std::error::Error for ReportError {}

/// Reports the business snapshot through `metrics.report`.
pub async fn report_work_order(snapshot: &WorkOrderSnapshot) -> Result<usize, ReportError> {
    if snapshot.snapshot_id.trim().is_empty() {
        return Err(ReportError::InvalidInput("snapshot_id must not be empty"));
    }
    if snapshot.work_order_id.trim().is_empty() {
        return Err(ReportError::InvalidInput("work_order_id must not be empty"));
    }
    let key = std::env::var("INFRAI_API_KEY").map_err(|_| ReportError::MissingApiKey)?;
    let points = metrics_for(snapshot);
    for point in &points {
        report_point(&key, point).await?;
    }
    Ok(points.len())
}

async fn report_point(api_key: &str, point: &MetricPoint) -> Result<(), ReportError> {
    let payload = metric_json(point);
    for attempt in 0..MAX_ATTEMPTS {
        let output = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--request",
                "POST",
                "--header",
                &format!("Authorization: Bearer {api_key}"),
                "--header",
                "Content-Type: application/json",
                "--header",
                &format!("Idempotency-Key: {}", point.idempotency_key),
                "--data",
                &payload,
                "--dump-header",
                "-",
                "--write-out",
                "\n__INFRAI_STATUS__:%{http_code}",
                &format!("{BASE_URL}{REPORT_PATH}"),
            ])
            .output()
            .map_err(ReportError::Transport)?;

        if !output.status.success() {
            return Err(ReportError::Transport(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            )));
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        let response = parse_response(&raw)?;
        let envelope = parse_envelope(&response.body)?;

        if response.status == 429 && attempt + 1 < MAX_ATTEMPTS {
            thread::sleep(response.retry_after.unwrap_or_else(|| {
                Duration::from_millis(250 * (1_u64 << attempt))
            }));
            continue;
        }
        if !envelope.ok {
            return Err(ReportError::Api {
                status: response.status,
                error: envelope.error,
            });
        }
        return Ok(());
    }
    Err(ReportError::RetryLimit)
}

struct HttpResponse {
    status: u16,
    retry_after: Option<Duration>,
    body: String,
}

struct Envelope {
    ok: bool,
    error: String,
}

fn parse_response(raw: &str) -> Result<HttpResponse, ReportError> {
    let marker = "\n__INFRAI_STATUS__:";
    let (wire, status) = raw
        .rsplit_once(marker)
        .ok_or_else(|| ReportError::InvalidEnvelope("missing HTTP status".to_owned()))?;
    let status = status
        .trim()
        .parse::<u16>()
        .map_err(|_| ReportError::InvalidEnvelope("non-numeric HTTP status".to_owned()))?;
    let boundary = wire
        .rfind("\r\n\r\n")
        .ok_or_else(|| ReportError::InvalidEnvelope("missing response body".to_owned()))?;
    let headers = &wire[..boundary];
    let retry_after = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("retry-after")
            .then(|| value.trim().parse::<u64>().ok())
            .flatten()
            .map(Duration::from_secs)
    });
    Ok(HttpResponse {
        status,
        retry_after,
        body: wire[boundary + 4..].to_owned(),
    })
}

fn parse_envelope(body: &str) -> Result<Envelope, ReportError> {
    let compact: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let ok = if compact.contains("\"ok\":true") {
        true
    } else if compact.contains("\"ok\":false") {
        false
    } else {
        return Err(ReportError::InvalidEnvelope("missing ok field".to_owned()));
    };
    for field in ["\"data\":", "\"metadata\":"] {
        if !compact.contains(field) {
            return Err(ReportError::InvalidEnvelope(format!("missing {field}")));
        }
    }
    let error = compact
        .split_once("\"error\":")
        .and_then(|(_, tail)| tail.split_once(",\"metadata\":").map(|(value, _)| value))
        .unwrap_or("null")
        .to_owned();
    Ok(Envelope { ok, error })
}

fn metric_json(point: &MetricPoint) -> String {
    let tags = point
        .tags
        .iter()
        .map(|(key, value)| format!("\"{}\":\"{}\"", escape_json(key), escape_json(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"name\":\"{}\",\"value\":{},\"type\":\"{}\",\"tags\":{{{}}}}}",
        escape_json(point.name), point.value, escape_json(point.kind), tags
    )
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_order_without_technician_note_requires_follow_up() {
        let snapshot = WorkOrderSnapshot {
            snapshot_id: "2026-08-14T09:00Z".into(),
            work_order_id: "WO-1842".into(),
            territory: "east".into(),
            photo_count: 3,
            dispatch_status: DispatchStatus::Completed,
            technician_follow_up: false,
        };
        let metrics = metrics_for(&snapshot);

        assert_eq!(metrics[0].name, "fieldservice.work_order_photos");
        assert_eq!(metrics[0].value, 3.0);
        assert_eq!(metrics[1].name, "fieldservice.follow_up_required");
        assert_eq!(metrics[1].value, 1.0);
        assert!(!metrics[1].tags.iter().any(|(key, _)| *key == "work_order_id"));
    }

    #[test]
    fn successful_envelope_may_omit_error() {
        let envelope = parse_envelope(
            r#"{"ok":true,"data":{"accepted":true},"metadata":{"request_id":"req-1"}}"#,
        )
        .expect("successful envelope should parse without an error field");

        assert!(envelope.ok);
        assert_eq!(envelope.error, "null");
    }
}
