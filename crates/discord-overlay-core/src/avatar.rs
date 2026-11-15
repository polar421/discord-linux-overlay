//! Discord CDN avatar URLs.
//!
//! The overlay never proxies or caches avatars itself: the web view loads them
//! straight from Discord's CDN, which keeps idle CPU and disk usage at zero and
//! guarantees a fresh image when a user changes their avatar.

/// Size bucket accepted by the Discord CDN.
pub fn clamp_avatar_size(size: u32) -> u32 {
    size.clamp(16, 1024)
}

/// Index of the default avatar for a snowflake id.
///
/// Discord's documented formula for migrated accounts is `(id >> 22) % 6`;
/// legacy discriminators map to `discriminator % 5`. Snowflakes are numeric,
/// so anything else (test fixtures, future formats) falls back to the first
/// default avatar instead of panicking.
pub fn default_avatar_index(user_id: &str) -> u8 {
    match user_id.parse::<u64>() {
        Ok(id) => ((id >> 22) % 6) as u8,
        Err(_) => 0,
    }
}

/// Build a CDN URL for a user's avatar, or one of Discord's default avatars
/// when the user has never uploaded one.
pub fn avatar_url(user_id: &str, avatar_hash: Option<&str>, size: u32) -> String {
    let size = clamp_avatar_size(size);

    match avatar_hash {
        Some(hash) if !hash.trim().is_empty() => {
            // Animated avatars are stored as `a_<hash>` and only served as GIF.
            let extension = if hash.starts_with("a_") { "gif" } else { "png" };
            format!("https://cdn.discordapp.com/avatars/{user_id}/{hash}.{extension}?size={size}")
        }
        _ => format!(
            "https://cdn.discordapp.com/embed/avatars/{}.png",
            default_avatar_index(user_id)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_use_png_and_requested_size() {
        assert_eq!(
            avatar_url("123", Some("abc"), 64),
            "https://cdn.discordapp.com/avatars/123/abc.png?size=64"
        );
    }

    #[test]
    fn animated_avatars_use_gif() {
        assert_eq!(
            avatar_url("123", Some("a_abc"), 512),
            "https://cdn.discordapp.com/avatars/123/a_abc.gif?size=512"
        );
    }

    #[test]
    fn missing_avatar_uses_the_default_bucket() {
        // 80351110224678912 >> 22 = 19157375095 ; 19157375095 % 6 = 5
        let url = avatar_url("80351110224678912", None, 32);
        assert_eq!(url, "https://cdn.discordapp.com/embed/avatars/5.png");

        // Empty hashes are treated as "no avatar".
        assert_eq!(
            avatar_url("1", Some("   "), 32),
            "https://cdn.discordapp.com/embed/avatars/0.png"
        );
    }

    #[test]
    fn non_numeric_ids_do_not_panic() {
        assert_eq!(default_avatar_index("not-a-snowflake"), 0);
    }

    #[test]
    fn avatar_size_is_clamped_to_cdn_limits() {
        assert_eq!(clamp_avatar_size(1), 16);
        assert_eq!(clamp_avatar_size(8192), 1024);
        assert_eq!(clamp_avatar_size(64), 64);
        assert!(avatar_url("1", Some("h"), 0).ends_with("size=16"));
    }
}
