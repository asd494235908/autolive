use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};

const AUTH_CREDENTIAL_SERVICE: &str = "autolive.desktop";
const MAX_DEVICE_ID_LENGTH: usize = 64;
const MAX_REFRESH_TOKEN_LENGTH: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
pub struct DeviceCredentialRequestDto {
    pub device_id: String,
}

#[derive(Debug, Deserialize)]
pub struct StoreRefreshTokenRequestDto {
    pub device_id: String,
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct AuthCredentialErrorDto {
    pub code: &'static str,
    pub message: &'static str,
}

fn invalid_request(message: &'static str) -> AuthCredentialErrorDto {
    AuthCredentialErrorDto {
        code: "auth_credential_invalid_request",
        message,
    }
}

fn keyring_error(code: &'static str, message: &'static str) -> AuthCredentialErrorDto {
    AuthCredentialErrorDto { code, message }
}

fn credential_account(device_id: &str) -> Result<String, AuthCredentialErrorDto> {
    if device_id.is_empty()
        || device_id.len() > MAX_DEVICE_ID_LENGTH
        || !device_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'_' || value == b'-')
    {
        return Err(invalid_request("设备标识格式无效"));
    }

    Ok(format!("refresh-token-{device_id}"))
}

fn validate_refresh_token(refresh_token: &str) -> Result<(), AuthCredentialErrorDto> {
    if refresh_token.is_empty()
        || refresh_token.len() > MAX_REFRESH_TOKEN_LENGTH
        || refresh_token.contains('\0')
    {
        return Err(invalid_request("刷新令牌格式无效"));
    }

    Ok(())
}

fn credential_entry(device_id: &str) -> Result<Entry, AuthCredentialErrorDto> {
    let account = credential_account(device_id)?;
    Entry::new(AUTH_CREDENTIAL_SERVICE, &account).map_err(|_| {
        keyring_error(
            "auth_credential_unavailable",
            "系统钥匙串不可用，请检查当前操作系统的凭据服务",
        )
    })
}

#[tauri::command]
pub fn store_refresh_token(
    request: StoreRefreshTokenRequestDto,
) -> Result<(), AuthCredentialErrorDto> {
    validate_refresh_token(&request.refresh_token)?;
    let entry = credential_entry(&request.device_id)?;
    entry
        .set_password(&request.refresh_token)
        .map_err(|_| keyring_error("auth_credential_write_failed", "刷新令牌写入系统钥匙串失败"))
}

#[tauri::command]
pub fn load_refresh_token(
    request: DeviceCredentialRequestDto,
) -> Result<Option<String>, AuthCredentialErrorDto> {
    let entry = credential_entry(&request.device_id)?;
    match entry.get_password() {
        Ok(refresh_token) => {
            validate_refresh_token(&refresh_token)?;
            Ok(Some(refresh_token))
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err(keyring_error(
            "auth_credential_read_failed",
            "刷新令牌读取系统钥匙串失败",
        )),
    }
}

#[tauri::command]
pub fn delete_refresh_token(
    request: DeviceCredentialRequestDto,
) -> Result<(), AuthCredentialErrorDto> {
    let entry = credential_entry(&request.device_id)?;
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err(keyring_error(
            "auth_credential_delete_failed",
            "刷新令牌从系统钥匙串删除失败",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::credential_account;

    #[test]
    fn credential_account_is_stable_and_namespaced_by_device() {
        assert_eq!(
            credential_account("device-abc_123").expect("valid device id"),
            "refresh-token-device-abc_123"
        );
    }

    #[test]
    fn credential_account_rejects_values_that_could_escape_the_account_namespace() {
        assert!(credential_account("../other-device").is_err());
        assert!(credential_account("device with spaces").is_err());
        assert!(credential_account("").is_err());
    }
}
