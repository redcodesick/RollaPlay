use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{Connection, OptionalExtension};

const SETTING_KEY: &str = "media_server_access_token";
const TOKEN_ENTROPY: &[u8] = b"WindowsMediaStation.LocalHttpMediaServer.Token.v1";

pub fn load_or_create_token(connection: &Connection) -> Result<String, String> {
    if let Some(stored) = connection
        .query_row("SELECT value FROM application_settings WHERE key = ?1", [SETTING_KEY], |row| row.get::<_, String>(0))
        .optional()
        .map_err(|_| "Could not read the local media access token".to_string())?
    {
        return unprotect(&stored);
    }

    let mut raw = [0_u8; 32];
    OsRng.fill_bytes(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    let stored = protect(token.as_bytes())?;
    connection
        .execute(
            "INSERT INTO application_settings(key, value) VALUES (?1, ?2)",
            [SETTING_KEY, &stored],
        )
        .map_err(|_| "Could not persist the local media access token".to_string())?;
    Ok(token)
}

#[cfg(windows)]
fn protect(token: &[u8]) -> Result<String, String> {
    let protected = windows_dpapi::encrypt_data(token, windows_dpapi::Scope::User, Some(TOKEN_ENTROPY))
        .map_err(|_| "Windows could not protect the local media access token".to_string())?;
    Ok(format!("dpapi-user-v1:{}", URL_SAFE_NO_PAD.encode(protected)))
}

#[cfg(not(windows))]
fn protect(token: &[u8]) -> Result<String, String> {
    Ok(format!("local-dev-v1:{}", URL_SAFE_NO_PAD.encode(token)))
}

#[cfg(windows)]
fn unprotect(stored: &str) -> Result<String, String> {
    let encoded = stored
        .strip_prefix("dpapi-user-v1:")
        .ok_or_else(|| "Stored media token is not protected for this Windows user".to_string())?;
    let protected = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| "Stored media token is corrupt".to_string())?;
    let raw = windows_dpapi::decrypt_data(&protected, windows_dpapi::Scope::User, Some(TOKEN_ENTROPY))
        .map_err(|_| "Windows could not decrypt the local media access token".to_string())?;
    String::from_utf8(raw).map_err(|_| "Stored media token is corrupt".to_string())
}

#[cfg(not(windows))]
fn unprotect(stored: &str) -> Result<String, String> {
    let encoded = stored
        .strip_prefix("local-dev-v1:")
        .ok_or_else(|| "Stored media token uses a different platform protector".to_string())?;
    let raw = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| "Stored media token is corrupt".to_string())?;
    String::from_utf8(raw).map_err(|_| "Stored media token is corrupt".to_string())
}

#[cfg(test)]
mod tests {
    use super::load_or_create_token;
    use rusqlite::Connection;

    #[test]
    fn token_is_random_persisted_and_reused_without_plaintext_storage() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute("CREATE TABLE application_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL)", []).unwrap();
        let first = load_or_create_token(&connection).unwrap();
        let second = load_or_create_token(&connection).unwrap();
        let stored = connection.query_row("SELECT value FROM application_settings", [], |row| row.get::<_, String>(0)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 43);
        assert_ne!(stored, first);
    }
}
