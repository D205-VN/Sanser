use crate::{SessionMetrics, sanitize_value};
use sanser_core::{NATIVE_PROTOCOL_NAME, PRODUCT_NAME, PROTOCOL_VERSION, VERSION};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

const MAX_EXPORT_SAMPLES: usize = 10_000;

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticsExport {
    pub product: &'static str,
    pub version: &'static str,
    pub protocol_version: u8,
    pub native_protocol: &'static str,
    pub generated_at_us: u64,
    pub metadata: Value,
    pub samples: Vec<SessionMetrics>,
}

impl DiagnosticsExport {
    /// Creates a sanitized diagnostics export from metadata and metric samples.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::TooManySamples`] when `samples` exceeds the
    /// maximum supported export size.
    pub fn new(
        generated_at_us: u64,
        metadata: &Value,
        samples: Vec<SessionMetrics>,
    ) -> Result<Self, ExportError> {
        if samples.len() > MAX_EXPORT_SAMPLES {
            return Err(ExportError::TooManySamples(samples.len()));
        }
        Ok(Self {
            product: PRODUCT_NAME,
            version: VERSION,
            protocol_version: PROTOCOL_VERSION,
            native_protocol: NATIVE_PROTOCOL_NAME,
            generated_at_us,
            metadata: sanitize_value(metadata),
            samples,
        })
    }
}

/// Serializes a diagnostics export as sanitized, pretty-printed JSON.
///
/// # Errors
///
/// Returns [`ExportError::Serialization`] when the export cannot be converted
/// to or encoded from a JSON value.
pub fn export_json(export: &DiagnosticsExport) -> Result<String, ExportError> {
    let serialized = serde_json::to_value(export).map_err(|_| ExportError::Serialization)?;
    serde_json::to_string_pretty(&sanitize_value(&serialized))
        .map_err(|_| ExportError::Serialization)
}

/// Serializes a diagnostics export as sanitized, human-readable text.
///
/// # Errors
///
/// Returns [`ExportError::Serialization`] when the metadata cannot be encoded
/// as JSON.
pub fn export_text(export: &DiagnosticsExport) -> Result<String, ExportError> {
    let mut output = format!(
        "{} {}\nProtocol v{} {}\nGenerated: {}us\nSamples: {}\n",
        export.product,
        export.version,
        export.protocol_version,
        export.native_protocol,
        export.generated_at_us,
        export.samples.len()
    );
    let metadata = serde_json::to_string_pretty(&sanitize_value(&export.metadata))
        .map_err(|_| ExportError::Serialization)?;
    output.push_str("Metadata:\n");
    output.push_str(&metadata);
    output.push('\n');
    Ok(output)
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ExportError {
    #[error("diagnostics export has {0} samples, exceeding 10000")]
    TooManySamples(usize),
    #[error("diagnostics serialization failed")]
    Serialization,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn export_contains_identity_but_no_secret() {
        let export = DiagnosticsExport::new(
            12,
            &json!({"token": "never-export", "route": "direct"}),
            vec![SessionMetrics::default()],
        )
        .unwrap_or_else(|error| panic!("export setup failed: {error}"));
        let json = export_json(&export).unwrap_or_else(|error| panic!("export failed: {error}"));
        assert!(json.contains("Sanser"));
        assert!(json.contains("2.0.7"));
        assert!(!json.contains("never-export"));
    }
}
