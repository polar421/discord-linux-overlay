//! Discord application credentials.
//!
//! Discord's local RPC cannot be used without a registered Discord application:
//! the `AUTHORIZE` command issues an OAuth2 code that has to be exchanged for an
//! access token with the application's client id *and* client secret.
//!
//! This project therefore asks the user to create their own (free) Discord
//! application once. The secret is a credential the user owns; it is kept in a
//! separate `0600` file, is never logged, never serialised to the settings UI
//! and never committed to git.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::write_private_json;
use crate::error::Result;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordCredentials {
    pub client_id: String,
    pub client_secret: String,
    /// Only needed if one was supplied during the authorization request.
    pub redirect_uri: Option<String>,
}

impl DiscordCredentials {
    pub fn is_configured(&self) -> bool {
        !self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }

    pub fn client_id(&self) -> &str {
        self.client_id.trim()
    }

    pub fn client_secret(&self) -> &str {
        self.client_secret.trim()
    }

    pub fn redirect_uri(&self) -> Option<&str> {
        self.redirect_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    /// What the settings window is allowed to see.
    pub fn public_view(&self) -> CredentialsView {
        CredentialsView {
            client_id: self.client_id().to_string(),
            masked_client_id: mask(self.client_id()),
            has_secret: !self.client_secret().is_empty(),
            redirect_uri: self.redirect_uri().map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsView {
    pub client_id: String,
    pub masked_client_id: String,
    pub has_secret: bool,
    pub redirect_uri: Option<String>,
}

fn mask(value: &str) -> String {
    let value = value.trim();
    if value.len() <= 8 {
        return "*".repeat(value.len());
    }
    format!("{}…{}", &value[..4], &value[value.len() - 4..])
}

#[derive(Debug, Clone)]
pub struct CredentialsStore {
    path: PathBuf,
}

impl CredentialsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> DiscordCredentials {
        match fs::read_to_string(&self.path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => DiscordCredentials::default(),
        }
    }

    pub fn save(&self, credentials: &DiscordCredentials) -> Result<()> {
        write_private_json(&self.path, &serde_json::to_value(credentials)?)
    }

    pub fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dlo-credentials-{}-{label}-{nanos}/credentials.json",
            std::process::id()
        ))
    }

    #[test]
    fn configured_requires_both_parts() {
        let mut credentials = DiscordCredentials::default();
        assert!(!credentials.is_configured());

        credentials.client_id = "123456".into();
        assert!(!credentials.is_configured());

        credentials.client_secret = "secret".into();
        assert!(credentials.is_configured());

        credentials.client_id = "   ".into();
        assert!(!credentials.is_configured());
    }

    #[test]
    fn public_view_never_contains_the_secret() {
        let credentials = DiscordCredentials {
            client_id: "1234567890123456789".into(),
            client_secret: "super-secret-value".into(),
            redirect_uri: Some("http://localhost".into()),
        };

        let view = credentials.public_view();
        assert_eq!(view.client_id, "1234567890123456789");
        assert_eq!(view.masked_client_id, "1234…6789");
        assert!(view.has_secret);
        assert_eq!(view.redirect_uri.as_deref(), Some("http://localhost"));

        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("super-secret"));
    }

    #[test]
    fn short_ids_are_fully_masked() {
        let credentials = DiscordCredentials {
            client_id: "123".into(),
            client_secret: "s".into(),
            redirect_uri: None,
        };
        assert_eq!(credentials.public_view().masked_client_id, "***");
    }

    #[test]
    fn credentials_round_trip_with_private_permissions() {
        let path = temp_path("round-trip");
        let store = CredentialsStore::new(&path);
        assert!(!store.load().is_configured());

        let credentials = DiscordCredentials {
            client_id: "999".into(),
            client_secret: "shh".into(),
            redirect_uri: None,
        };
        store.save(&credentials).unwrap();

        assert_eq!(store.load(), credentials);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        store.clear().unwrap();
        assert!(!path.exists());
        // Clearing twice is not an error.
        store.clear().unwrap();

        fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
