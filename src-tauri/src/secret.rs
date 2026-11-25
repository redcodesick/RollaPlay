use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{Connection, OptionalExtension};

const SETTING_KEY: &str = "media_server_access_token";
const TOKEN_ENTROPY: &[u8] = b"WindowsMediaStation.LocalHttpMediaServer.Token.v1";
const LG_KEY_ENTROPY: &[u8] = b"RollaPlay.LgWebOS.ClientKey.v1";

pub fn lg_key_setting_key(address: &str) -> String {
    format!("lg_webos_client_key:{address}")
}

/// Reads a DPAPI-protected application setting. The value is never returned in
/// diagnostics or logs.
pub fn load_protected_setting(
    connection: &Connection,
    key: &str,
    entropy: &[u8],
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT value FROM application_settings WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "Could not read a protected application setting".to_string())?
        .map(|stored| unprotect(&stored, entropy))
        .transpose()
}

/// Writes a DPAPI-protected application setting using an UPSERT so re-pairing
/// replaces the old client key cleanly.
pub fn save_protected_setting(
    connection: &Connection,
    key: &str,
    value: &str,
    entropy: &[u8],
) -> Result<(), String> {
    let stored = protect(value.as_bytes(), entropy)?;
    connection
        .execute(
            "INSERT INTO application_settings(key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, &stored],
        )
        .map_err(|_| "Could not save a protected application setting".to_string())?;
    Ok(())
}

pub fn load_or_create_token(connection: &Connection) -> Result<String, String> {
    if let Some(stored) = load_protected_setting(connection, SETTING_KEY, TOKEN_ENTROPY)? {
        return Ok(stored);
    }

    let mut raw = [0_u8; 32];
    OsRng.fill_bytes(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    save_protected_setting(connection, SETTING_KEY, &token, TOKEN_ENTROPY)?;
    Ok(token)
}

pub fn load_lg_client_key(
    connection: &Connection,
    address: &str,
) -> Result<Option<String>, String> {
    load_protected_setting(connection, &lg_key_setting_key(address), LG_KEY_ENTROPY)
}

pub fn save_lg_client_key(
    connection: &Connection,
    address: &str,
    key: &str,
) -> Result<(), String> {
    save_protected_setting(connection, &lg_key_setting_key(address), key, LG_KEY_ENTROPY)
}

#[cfg(windows)]
fn protect(token: &[u8], entropy: &[u8]) -> Result<String, String> {
    let protected = windows_dpapi::encrypt_data(token, windows_dpapi::Scope::User, Some(entropy))
        .map_err(|_| "Windows could not protect a local secret".to_string())?;
    Ok(format!("dpapi-user-v1:{}", URL_SAFE_NO_PAD.encode(protected)))
}

#[cfg(not(windows))]
fn protect(token: &[u8], _entropy: &[u8]) -> Result<String, String> {
    Ok(format!("local-dev-v1:{}", URL_SAFE_NO_PAD.encode(token)))
}

#[cfg(windows)]
fn unprotect(stored: &str, entropy: &[u8]) -> Result<String, String> {
    let encoded = stored
        .strip_prefix("dpapi-user-v1:")
        .ok_or_else(|| "Stored secret is not protected for this Windows user".to_string())?;
    let protected = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Stored secret is corrupt".to_string())?;
    let raw = windows_dpapi::decrypt_data(&protected, windows_dpapi::Scope::User, Some(entropy))
        .map_err(|_| "Windows could not decrypt the local secret".to_string())?;
    String::from_utf8(raw).map_err(|_| "Stored secret is corrupt".to_string())
}

#[cfg(not(windows))]
fn unprotect(stored: &str, _entropy: &[u8]) -> Result<String, String> {
    let encoded = stored
        .strip_prefix("local-dev-v1:")
        .ok_or_else(|| "Stored secret uses a different platform protector".to_string())?;
    let raw = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Stored secret is corrupt".to_string())?;
    String::from_utf8(raw).map_err(|_| "Stored secret is corrupt".to_string())
}

#[cfg(test)]
mod tests {
    use super::{load_lg_client_key, load_or_create_token, save_lg_client_key};
    use rusqlite::Connection;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute(
                "CREATE TABLE application_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
                [],
            )
            .unwrap();
        connection
    }

    #[test]
    fn token_is_random_persisted_and_reused_without_plaintext_storage() {
        let connection = database();
        let first = load_or_create_token(&connection).unwrap();
        let second = load_or_create_token(&connection).unwrap();
        let stored = connection
            .query_row("SELECT value FROM application_settings", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 43);
        assert_ne!(stored, first);
    }

    #[test]
    fn lg_client_key_is_persisted_per_address_and_replaced_on_repair() {
        let connection = database();
        assert_eq!(load_lg_client_key(&connection, "192.168.1.20").unwrap(), None);
        save_lg_client_key(&connection, "192.168.1.20", "first-key").unwrap();
        save_lg_client_key(&connection, "192.168.1.21", "other-key").unwrap();
        save_lg_client_key(&connection, "192.168.1.20", "second-key").unwrap();
        assert_eq!(
            load_lg_client_key(&connection, "192.168.1.20").unwrap().as_deref(),
            Some("second-key")
        );
        assert_eq!(
            load_lg_client_key(&connection, "192.168.1.21").unwrap().as_deref(),
            Some("other-key")
        );
        let stored = connection
            .query_row(
                "SELECT value FROM application_settings WHERE key = ?1",
                ["lg_webos_client_key:192.168.1.20"],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert!(!stored.contains("second-key"));
    }
}
