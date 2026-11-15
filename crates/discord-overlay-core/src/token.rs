//! OAuth2 code exchange for Discord.
//!
//! Discord's `AUTHORIZE` RPC command hands back a one-time authorization code.
//! Exchanging that code for an access token requires the application's client
//! secret, which is why this project needs the user's own Discord application
//! instead of shipping a shared one.
//!
//! The exchange runs in the app process (never in a web view), uses TLS and is
//! only reachable with the `http` feature enabled.

use serde_json::Value;

use crate::client::{now_millis, Token, TokenExchange};
use crate::error::{Error, Result};

const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";

#[derive(Debug, Clone)]
pub struct HttpTokenExchange {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: Option<String>,
}

impl HttpTokenExchange {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: Option<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri,
        }
    }
}

impl TokenExchange for HttpTokenExchange {
    fn exchange(&self, code: &str) -> Result<Token> {
        let mut fields = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("client_id", self.client_id.clone()),
            ("client_secret", self.client_secret.clone()),
        ];

        if let Some(redirect_uri) = self.redirect_uri.as_deref() {
            if !redirect_uri.trim().is_empty() {
                fields.push(("redirect_uri", redirect_uri.to_string()));
            }
        }

        let body = form_encode(&fields);

        let response = ureq::post(TOKEN_URL)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .set("Accept", "application/json")
            .send_string(&body);

        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(Error::TokenExchange(format!(
                    "Discord rejected the authorization (HTTP {status}): {}",
                    summarize_discord_error(&body)
                )));
            }
            Err(error) => {
                return Err(Error::TokenExchange(format!(
                    "could not reach discord.com: {error}"
                )));
            }
        };

        let payload: Value = response
            .into_json()
            .map_err(|error| Error::TokenExchange(format!("invalid token response: {error}")))?;

        let access_token = payload
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::TokenExchange("token response had no access_token".to_string()))?
            .to_string();

        let expires_at = payload
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| now_millis() + seconds.saturating_mul(1000));

        Ok(Token {
            access_token,
            expires_at,
        })
    }
}

/// Discord returns `{"error": "...", "error_description": "..."}` on failure.
/// Only the safe, human-readable description is surfaced.
fn summarize_discord_error(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        let description = value
            .get("error_description")
            .and_then(Value::as_str)
            .or_else(|| value.get("error").and_then(Value::as_str));
        if let Some(description) = description {
            return description.to_string();
        }
    }
    body.chars().take(200).collect()
}

fn form_encode(fields: &[(&str, String)]) -> String {
    let mut encoded = String::new();
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            encoded.push('&');
        }
        encoded.push_str(&percent_encode(key));
        encoded.push('=');
        encoded.push_str(&percent_encode(value));
    }
    encoded
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn form_encoding_escapes_reserved_characters() {
        assert_eq!(percent_encode("abcXYZ019-._~"), "abcXYZ019-._~");
        assert_eq!(percent_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(percent_encode("ü"), "%C3%BC");
    }

    #[test]
    fn form_body_contains_the_grant() {
        let body = form_encode(&[
            ("grant_type", "authorization_code".to_string()),
            ("code", "abc".to_string()),
            ("client_id", "1".to_string()),
        ]);
        assert_eq!(body, "grant_type=authorization_code&code=abc&client_id=1");
    }

    #[test]
    fn discord_error_bodies_are_reduced_to_the_description() {
        let body = json!({
            "error": "invalid_grant",
            "error_description": "Invalid \"code\" in request."
        })
        .to_string();
        assert_eq!(
            summarize_discord_error(&body),
            "Invalid \"code\" in request."
        );
    }

    #[test]
    fn non_json_errors_are_truncated() {
        let message = summarize_discord_error(&"x".repeat(500));
        assert_eq!(message.len(), 200);
    }
}
