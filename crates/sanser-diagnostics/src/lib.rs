//! Bounded diagnostics collection and sanitized support exports.

mod export;
mod history;
mod log_buffer;
mod metrics;
mod redact;

pub use export::{DiagnosticsExport, ExportError, export_json, export_text};
pub use history::{DiagnosticsHistory, HistoryConfig, HistoryConfigError};
pub use log_buffer::{BoundedLogBuffer, LogBufferConfig, LogBufferError, LogEvent, LogLevel};
pub use metrics::{LatencyBreakdown, ResourceMetrics, SessionMetrics};
pub use redact::{REDACTED, sanitize_text, sanitize_value};
