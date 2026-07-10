use serde_json::{Map, Value};
use url::Url;

pub const REDACTED: &str = "[REDACTED]";

/// Recursively removes secret-bearing fields from arbitrary diagnostics JSON.
#[must_use]
pub fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut sanitized = Map::with_capacity(fields.len());
            for (key, value) in fields {
                if is_sensitive_key(key) {
                    sanitized.insert(key.clone(), Value::String(REDACTED.to_owned()));
                } else {
                    sanitized.insert(key.clone(), sanitize_value(value));
                }
            }
            Value::Object(sanitized)
        }
        Value::Array(values) => Value::Array(values.iter().map(sanitize_value).collect()),
        Value::String(value) => Value::String(sanitize_text(value)),
        primitive => primitive.clone(),
    }
}

/// Best-effort sanitation for free-form log messages. Structured fields remain
/// the preferred input because their keys can be redacted without ambiguity.
#[must_use]
pub fn sanitize_text(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().collect();
    let mut output = Vec::with_capacity(words.len());
    let mut redact_next = false;
    for word in words {
        if redact_next {
            output.push(REDACTED.to_owned());
            redact_next = false;
            continue;
        }
        let normalized = normalize_key(word.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && character != '='
        }));
        if normalized == "bearer" || normalized.ends_with("authorizationbearer") {
            output.push(word.to_owned());
            redact_next = true;
            continue;
        }
        if let Some((key, _)) = word.split_once('=')
            && is_sensitive_key(key)
        {
            output.push(format!("{key}={REDACTED}"));
            continue;
        }
        output.push(sanitize_url_word(word));
    }
    output.join(" ")
}

fn sanitize_url_word(word: &str) -> String {
    let trimmed = word.trim_matches(|character: char| {
        matches!(character, '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';')
    });
    let Ok(mut url) = Url::parse(trimmed) else {
        return word.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https" | "postgres" | "postgresql") {
        return word.to_owned();
    }
    let had_credentials = !url.username().is_empty() || url.password().is_some();
    if had_credentials {
        let _result = url.set_username(REDACTED);
        let _result = url.set_password(Some(REDACTED));
    }
    let retained: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| !is_sensitive_key(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if url.query().is_some() {
        url.set_query(None);
        if !retained.is_empty() {
            let mut query = url.query_pairs_mut();
            query.extend_pairs(retained);
        }
    }
    url.to_string()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = normalize_key(key);
    [
        "password",
        "passphrase",
        "token",
        "accesstoken",
        "refreshtoken",
        "authorization",
        "credential",
        "turncredential",
        "secret",
        "privatekey",
        "databaseurl",
        "apikey",
    ]
    .iter()
    .any(|sensitive| key == *sensitive || key.ends_with(sensitive))
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recursively_redacts_every_secret_class() {
        let input = json!({
            "access_token": "token-value",
            "nested": { "TURN_CREDENTIAL": "turn-value", "rtt": 12 },
            "database_url": "postgresql://user:db-password@host/db",
            "message": "Authorization Bearer bearer-value password=plain"
        });
        let rendered = sanitize_value(&input).to_string();
        for secret in [
            "token-value",
            "turn-value",
            "db-password",
            "bearer-value",
            "plain",
        ] {
            assert!(!rendered.contains(secret), "leaked {secret}");
        }
        assert!(rendered.contains(REDACTED));
        assert!(rendered.contains("12"));
    }
}
