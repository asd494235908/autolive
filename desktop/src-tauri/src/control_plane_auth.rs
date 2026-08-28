use keyring::{Entry, Error as KeyringError};
use reqwest::blocking::{Client, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::Read;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{State, WebviewWindow};

const AUTH_CREDENTIAL_SERVICE: &str = "autolive.desktop";
const DEVICE_ID_ACCOUNT: &str = "device-id";
const DEVELOPMENT_CONTROL_PLANE_BASE_URL: &str = "http://127.0.0.1:18090";
const TEST_CONTROL_PLANE_BASE_URL: &str = "http://101.96.208.132:9090";
const MAX_DEVICE_ID_LENGTH: usize = 64;
const MAX_USERNAME_LENGTH: usize = 64;
const MAX_PASSWORD_LENGTH: usize = 256;
const MAX_TOKEN_LENGTH: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_PENDING_LOGOUT_TOKENS: usize = 8;
const MAX_REQUEST_ID_LENGTH: usize = 128;

#[derive(Clone, Default)]
pub struct ControlPlaneAuthState {
    operation: Arc<Mutex<()>>,
}

impl ControlPlaneAuthState {
    fn run_serialized<T>(
        &self,
        task: impl FnOnce() -> Result<T, ControlPlaneAuthErrorDto>,
    ) -> Result<T, ControlPlaneAuthErrorDto> {
        // ponytail: 一个桌面进程只有一个认证所有者；需要多账号并行时再拆成按设备锁。
        let _guard = self.operation.lock().map_err(|_| {
            auth_error(
                "auth_state_unavailable",
                "本机认证状态暂时不可用，请重启应用后重试",
                500,
            )
        })?;
        task()
    }
}

#[derive(Debug, Deserialize)]
pub struct DeviceSessionRequestDto {
    pub device_id: String,
}

#[derive(Debug, Deserialize)]
pub struct StableDeviceIdRequestDto {
    pub migration_device_id: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginSessionRequestDto {
    pub device_id: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct ControlPlaneAuthErrorDto {
    pub code: String,
    pub message: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UserSummaryDto {
    pub id: String,
    pub username: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ControlPlaneAccessSessionDto {
    pub access_token: String,
    pub expires_at: String,
    pub audience: String,
    pub user: Option<UserSummaryDto>,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogoutSessionResultDto {
    pub warning: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionTokensDto {
    access_token: String,
    refresh_token: String,
    expires_at: String,
    audience: String,
}

#[derive(Debug, Deserialize)]
struct LoginResponseDto {
    tokens: SessionTokensDto,
    user: UserSummaryDto,
}

#[derive(Debug, Deserialize)]
struct RefreshResponseDto {
    tokens: SessionTokensDto,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDto {
    code: String,
    message: String,
    request_id: Option<String>,
}

#[derive(Serialize)]
struct LoginRequestBody<'a> {
    username: &'a str,
    password: &'a str,
    product: &'static str,
}

#[derive(Serialize)]
struct RefreshRequestBody<'a> {
    refresh_token: &'a str,
}

fn auth_error(code: &str, message: &str, status: u16) -> ControlPlaneAuthErrorDto {
    ControlPlaneAuthErrorDto {
        code: code.to_owned(),
        message: message.to_owned(),
        status,
        request_id: None,
    }
}

fn api_error(error: ApiErrorDto, status: u16) -> ControlPlaneAuthErrorDto {
    let request_id = error
        .request_id
        .filter(|value| !value.is_empty() && value.len() <= MAX_REQUEST_ID_LENGTH);
    ControlPlaneAuthErrorDto {
        code: error.code,
        message: error.message,
        status,
        request_id,
    }
}

fn invalid_request(message: &str) -> ControlPlaneAuthErrorDto {
    auth_error("auth_credential_invalid_request", message, 400)
}

fn ensure_main_window(label: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    if label == "main" {
        Ok(())
    } else {
        Err(auth_error(
            "auth_command_forbidden",
            "当前窗口无权访问登录凭据",
            403,
        ))
    }
}

fn validate_device_id(device_id: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    if device_id.is_empty()
        || device_id.len() > MAX_DEVICE_ID_LENGTH
        || !device_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'_' || value == b'-')
    {
        return Err(invalid_request("设备标识格式无效"));
    }
    Ok(())
}

fn validate_login(request: &LoginSessionRequestDto) -> Result<(), ControlPlaneAuthErrorDto> {
    validate_device_id(&request.device_id)?;
    let username_length = request.username.chars().count();
    let password_length = request.password.chars().count();
    if !(3..=MAX_USERNAME_LENGTH).contains(&username_length) || request.username.contains('\0') {
        return Err(invalid_request("账号格式无效"));
    }
    if !(8..=MAX_PASSWORD_LENGTH).contains(&password_length)
        || request.password.len() > MAX_PASSWORD_LENGTH
        || request.password.contains('\0')
    {
        return Err(invalid_request("密码格式无效"));
    }
    Ok(())
}

fn refresh_account(device_id: &str) -> Result<String, ControlPlaneAuthErrorDto> {
    validate_device_id(device_id)?;
    Ok(format!("refresh-token-{device_id}"))
}

fn get_or_create_device_id(migration_device_id: &str) -> Result<String, ControlPlaneAuthErrorDto> {
    validate_device_id(migration_device_id)?;
    let entry = credential_entry(DEVICE_ID_ACCOUNT)?;
    match entry.get_password() {
        Ok(device_id) if validate_device_id(&device_id).is_ok() => Ok(device_id),
        Ok(_) => {
            delete_account(DEVICE_ID_ACCOUNT)?;
            store_device_id(migration_device_id)
        }
        Err(KeyringError::NoEntry) => store_device_id(migration_device_id),
        Err(_) => Err(auth_error(
            "auth_credential_read_failed",
            "稳定设备标识读取失败",
            503,
        )),
    }
}

fn store_device_id(device_id: &str) -> Result<String, ControlPlaneAuthErrorDto> {
    credential_entry(DEVICE_ID_ACCOUNT)?
        .set_password(device_id)
        .map_err(|_| {
            auth_error(
                "auth_credential_write_failed",
                "稳定设备标识写入系统钥匙串失败",
                503,
            )
        })?;
    Ok(device_id.to_owned())
}

fn pending_logout_account(device_id: &str) -> Result<String, ControlPlaneAuthErrorDto> {
    validate_device_id(device_id)?;
    Ok(format!("pending-logout-{device_id}"))
}

fn credential_entry(account: &str) -> Result<Entry, ControlPlaneAuthErrorDto> {
    Entry::new(AUTH_CREDENTIAL_SERVICE, account).map_err(|_| {
        auth_error(
            "auth_credential_unavailable",
            "系统钥匙串不可用，请检查当前操作系统的凭据服务",
            503,
        )
    })
}

fn load_refresh_token(device_id: &str) -> Result<Option<String>, ControlPlaneAuthErrorDto> {
    let account = refresh_account(device_id)?;
    match credential_entry(&account)?.get_password() {
        Ok(token) if is_valid_token(&token) => Ok(Some(token)),
        Ok(_) => {
            let _ = delete_account(&account);
            Err(auth_error(
                "auth_credential_invalid",
                "系统钥匙串中的会话凭据无效",
                401,
            ))
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err(auth_error(
            "auth_credential_read_failed",
            "会话凭据读取失败",
            503,
        )),
    }
}

fn store_refresh_token(device_id: &str, token: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    if !is_valid_token(token) {
        return Err(auth_error(
            "auth_credential_invalid",
            "服务端返回了无效的会话凭据",
            502,
        ));
    }
    let account = refresh_account(device_id)?;
    credential_entry(&account)?
        .set_password(token)
        .map_err(|_| {
            auth_error(
                "auth_credential_write_failed",
                "会话凭据写入系统钥匙串失败",
                503,
            )
        })
}

fn delete_account(account: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    match credential_entry(account)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err(auth_error(
            "auth_credential_delete_failed",
            "凭据从系统钥匙串删除失败",
            503,
        )),
    }
}

fn delete_refresh_token(device_id: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    delete_account(&refresh_account(device_id)?)
}

fn load_pending_logout_tokens(device_id: &str) -> Result<Vec<String>, ControlPlaneAuthErrorDto> {
    let account = pending_logout_account(device_id)?;
    match credential_entry(&account)?.get_password() {
        Ok(serialized) => {
            let tokens = serde_json::from_str::<Vec<String>>(&serialized)
                .unwrap_or_else(|_| vec![serialized]);
            if tokens.len() > MAX_PENDING_LOGOUT_TOKENS
                || tokens.iter().any(|token| !is_valid_token(token))
            {
                let _ = delete_account(&account);
                return Ok(Vec::new());
            }
            Ok(tokens)
        }
        Err(KeyringError::NoEntry) => Ok(Vec::new()),
        Err(_) => Err(auth_error(
            "auth_credential_read_failed",
            "待撤销会话凭据读取失败",
            503,
        )),
    }
}

fn store_pending_logout_tokens(
    device_id: &str,
    tokens: &[String],
) -> Result<(), ControlPlaneAuthErrorDto> {
    if tokens.is_empty()
        || tokens.len() > MAX_PENDING_LOGOUT_TOKENS
        || tokens.iter().any(|token| !is_valid_token(token))
    {
        return Err(auth_error(
            "auth_credential_invalid",
            "待撤销会话凭据无效",
            500,
        ));
    }
    let serialized = serde_json::to_string(tokens)
        .map_err(|_| auth_error("auth_credential_invalid", "待撤销会话凭据序列化失败", 500))?;
    credential_entry(&pending_logout_account(device_id)?)?
        .set_password(&serialized)
        .map_err(|_| {
            auth_error(
                "auth_credential_write_failed",
                "待撤销会话凭据保存失败",
                503,
            )
        })
}

fn append_pending_logout_token(
    device_id: &str,
    token: String,
) -> Result<(), ControlPlaneAuthErrorDto> {
    let mut tokens = load_pending_logout_tokens(device_id)?;
    if !tokens.contains(&token) {
        tokens.push(token);
    }
    store_pending_logout_tokens(device_id, &tokens)
}

fn delete_pending_logout_token(device_id: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    delete_account(&pending_logout_account(device_id)?)
}

fn clear_legacy_credentials(device_id: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    validate_device_id(device_id)?;
    let mut first_error = None;
    for prefix in ["login-password", "activation-code"] {
        if let Err(error) = delete_account(&format!("{prefix}-{device_id}")) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn is_valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN_LENGTH && !token.contains('\0')
}

fn control_plane_environment_allows_loopback_http(
    _environment: Option<&str>,
    debug_assertions: bool,
) -> bool {
    debug_assertions
}

fn control_plane_environment_allows_test_http(environment: Option<&str>) -> bool {
    environment == Some("test")
}

fn resolve_control_plane_base_url(
    configured: Option<&str>,
    allow_loopback_http: bool,
    allow_test_http: bool,
) -> Result<reqwest::Url, ControlPlaneAuthErrorDto> {
    let candidate = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| allow_loopback_http.then_some(DEVELOPMENT_CONTROL_PLANE_BASE_URL))
        .ok_or_else(|| {
            auth_error(
                "control_plane_config_missing",
                "生产桌面端构建缺少 VITE_CONTROL_PLANE_BASE_URL",
                500,
            )
        })?;
    let url = reqwest::Url::parse(candidate).map_err(|_| {
        auth_error(
            "control_plane_config_invalid",
            "控制面地址必须是绝对 URL",
            500,
        )
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(auth_error(
            "control_plane_config_invalid",
            "控制面地址不能包含凭据、查询参数或片段",
            500,
        ));
    }
    let loopback_http = allow_loopback_http
        && url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    let test_http = allow_test_http
        && url.scheme() == "http"
        && url.origin().ascii_serialization() == TEST_CONTROL_PLANE_BASE_URL
        && url.path() == "/";
    if url.scheme() != "https" && !loopback_http && !test_http {
        return Err(auth_error(
            "control_plane_config_invalid",
            "控制面地址必须使用 HTTPS；开发明文 HTTP 仅允许回环地址，测试明文 HTTP 仅允许固定测试地址",
            500,
        ));
    }
    Ok(url)
}

fn control_plane_url(path: &str) -> Result<reqwest::Url, ControlPlaneAuthErrorDto> {
    let environment = option_env!("VITE_CONTROL_PLANE_ENV");
    let allow_loopback_http =
        control_plane_environment_allows_loopback_http(environment, cfg!(debug_assertions));
    resolve_control_plane_base_url(
        option_env!("VITE_CONTROL_PLANE_BASE_URL"),
        allow_loopback_http,
        control_plane_environment_allows_test_http(environment),
    )?
    .join(path)
    .map_err(|_| auth_error("control_plane_config_invalid", "控制面请求地址无效", 500))
}

fn http_client() -> Result<Client, ControlPlaneAuthErrorDto> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| auth_error("network_error", "无法初始化控制面连接", 0))
}

fn read_response(mut response: Response) -> Result<Vec<u8>, ControlPlaneAuthErrorDto> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(auth_error(
            "control_plane_response_too_large",
            "控制面响应超过 1 MiB 限制",
            502,
        ));
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| auth_error("network_error", "读取控制面响应失败", 0))?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(auth_error(
            "control_plane_response_too_large",
            "控制面响应超过 1 MiB 限制",
            502,
        ));
    }
    Ok(body)
}

fn post_json<B: Serialize, R: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<R, ControlPlaneAuthErrorDto> {
    let response = http_client()?
        .post(control_plane_url(path)?)
        .header("Accept", "application/json")
        .json(body)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                auth_error("control_plane_timeout", "控制面请求超时", 408)
            } else {
                auth_error("network_error", "网络异常，暂时无法连接到控制面", 0)
            }
        })?;
    let status = response.status();
    let response_body = read_response(response)?;
    if !status.is_success() {
        if let Ok(error) = serde_json::from_slice::<ApiErrorDto>(&response_body) {
            return Err(api_error(error, status.as_u16()));
        }
        return Err(auth_error(
            "http_error",
            "控制面返回了无法识别的错误",
            status.as_u16(),
        ));
    }
    serde_json::from_slice(&response_body).map_err(|_| {
        auth_error(
            "control_plane_response_invalid",
            "控制面返回了无效响应",
            502,
        )
    })
}

fn validate_desktop_tokens(tokens: &SessionTokensDto) -> Result<(), ControlPlaneAuthErrorDto> {
    if tokens.audience != "desktop"
        || !is_valid_token(&tokens.access_token)
        || !is_valid_token(&tokens.refresh_token)
        || tokens.expires_at.is_empty()
        || tokens.expires_at.len() > 64
    {
        return Err(auth_error(
            "control_plane_response_invalid",
            "控制面返回了无效的桌面会话",
            502,
        ));
    }
    Ok(())
}

fn access_session(
    tokens: SessionTokensDto,
    user: Option<UserSummaryDto>,
    warning: Option<String>,
) -> ControlPlaneAccessSessionDto {
    ControlPlaneAccessSessionDto {
        access_token: tokens.access_token,
        expires_at: tokens.expires_at,
        audience: tokens.audience,
        user,
        warning,
    }
}

fn revoke_remote_token(token: &str) -> Result<(), ControlPlaneAuthErrorDto> {
    post_json::<_, serde_json::Value>(
        "/api/v1/auth/logout",
        &RefreshRequestBody {
            refresh_token: token,
        },
    )
    .map(|_| ())
}

fn login(
    request: LoginSessionRequestDto,
) -> Result<ControlPlaneAccessSessionDto, ControlPlaneAuthErrorDto> {
    validate_login(&request)?;
    let warning = clear_legacy_credentials(&request.device_id)
        .err()
        .map(|_| "历史登录密码清理失败；本次登录仍可继续，请稍后检查系统钥匙串。".to_owned());
    let response: LoginResponseDto = post_json(
        "/api/v1/client/auth/login",
        &LoginRequestBody {
            username: &request.username,
            password: &request.password,
            product: "autolive",
        },
    )?;
    if let Err(error) = validate_desktop_tokens(&response.tokens) {
        if is_valid_token(&response.tokens.refresh_token) {
            let _ = revoke_remote_token(&response.tokens.refresh_token);
        }
        return Err(error);
    }
    if response.user.role != "user" {
        let _ = revoke_remote_token(&response.tokens.refresh_token);
        return Err(auth_error("unauthenticated", "账号或密码错误", 401));
    }
    if let Err(error) = store_refresh_token(&request.device_id, &response.tokens.refresh_token) {
        let _ = revoke_remote_token(&response.tokens.refresh_token);
        return Err(error);
    }
    Ok(access_session(
        response.tokens,
        Some(response.user),
        warning,
    ))
}

fn refresh(
    device_id: &str,
) -> Result<Option<ControlPlaneAccessSessionDto>, ControlPlaneAuthErrorDto> {
    let warning = clear_legacy_credentials(device_id)
        .err()
        .map(|_| "历史登录密码清理失败；会话刷新未受影响。".to_owned());
    let Some(refresh_token) = load_refresh_token(device_id)? else {
        return Ok(None);
    };
    let response: RefreshResponseDto = match post_json(
        "/api/v1/auth/refresh",
        &RefreshRequestBody {
            refresh_token: &refresh_token,
        },
    ) {
        Ok(response) => response,
        Err(error) => {
            if error.status == 401 {
                let _ = delete_refresh_token(device_id);
            }
            return Err(error);
        }
    };
    if let Err(error) = validate_desktop_tokens(&response.tokens) {
        let _ = delete_refresh_token(device_id);
        if is_valid_token(&response.tokens.refresh_token) {
            let _ = revoke_remote_token(&response.tokens.refresh_token);
        }
        return Err(error);
    }
    if let Err(error) = store_refresh_token(device_id, &response.tokens.refresh_token) {
        let _ = delete_refresh_token(device_id);
        let _ = revoke_remote_token(&response.tokens.refresh_token);
        return Err(error);
    }
    Ok(Some(access_session(response.tokens, None, warning)))
}

fn logout(device_id: &str) -> Result<LogoutSessionResultDto, ControlPlaneAuthErrorDto> {
    validate_device_id(device_id)?;
    let token = load_refresh_token(device_id);
    let mut warnings = Vec::new();
    match token {
        Ok(Some(token)) => {
            if revoke_remote_token(&token).is_err() {
                if append_pending_logout_token(device_id, token).is_ok() {
                    warnings
                        .push("本机已退出，远端会话撤销尚未确认；下次启动将自动重试。".to_owned());
                } else {
                    warnings
                        .push("本机已退出，但远端会话撤销未确认且待重试凭据保存失败。".to_owned());
                }
            }
        }
        Ok(None) => {}
        Err(_) => warnings.push("本机已退出，但无法读取远端会话凭据以确认撤销。".to_owned()),
    }
    if delete_refresh_token(device_id).is_err() {
        warnings.push("本机内存已退出，但系统钥匙串中的当前会话凭据清理失败。".to_owned());
    }
    Ok(LogoutSessionResultDto {
        warning: (!warnings.is_empty()).then(|| warnings.join(" ")),
    })
}

fn retry_pending_logout(device_id: &str) -> Result<Option<String>, ControlPlaneAuthErrorDto> {
    let mut tokens = load_pending_logout_tokens(device_id)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let token = tokens.remove(0);
    if revoke_remote_token(&token).is_err() {
        tokens.insert(0, token);
        store_pending_logout_tokens(device_id, &tokens)?;
        return Ok(Some(
            "上次退出的远端会话仍未确认撤销；已保留安全重试凭据。".to_owned(),
        ));
    }
    if tokens.is_empty() {
        delete_pending_logout_token(device_id)?;
        Ok(None)
    } else {
        store_pending_logout_tokens(device_id, &tokens)?;
        Ok(Some(
            "已撤销一条历史远端会话，其余待撤销会话将在后续启动继续重试。".to_owned(),
        ))
    }
}

async fn run_blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, ControlPlaneAuthErrorDto> + Send + 'static,
) -> Result<T, ControlPlaneAuthErrorDto> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| auth_error("auth_task_failed", "登录任务异常结束", 500))?
}

#[tauri::command]
pub async fn get_or_create_control_plane_device_id(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: StableDeviceIdRequestDto,
) -> Result<String, ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || {
        state.run_serialized(|| get_or_create_device_id(&request.migration_device_id))
    })
    .await
}

#[tauri::command]
pub async fn login_control_plane_session(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: LoginSessionRequestDto,
) -> Result<ControlPlaneAccessSessionDto, ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || state.run_serialized(|| login(request))).await
}

#[tauri::command]
pub async fn restore_control_plane_session(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: DeviceSessionRequestDto,
) -> Result<Option<ControlPlaneAccessSessionDto>, ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || state.run_serialized(|| refresh(&request.device_id))).await
}

#[tauri::command]
pub async fn refresh_control_plane_session(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: DeviceSessionRequestDto,
) -> Result<Option<ControlPlaneAccessSessionDto>, ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || state.run_serialized(|| refresh(&request.device_id))).await
}

#[tauri::command]
pub async fn logout_control_plane_session(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: DeviceSessionRequestDto,
) -> Result<LogoutSessionResultDto, ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || state.run_serialized(|| logout(&request.device_id))).await
}

#[tauri::command]
pub async fn retry_pending_control_plane_logout(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: DeviceSessionRequestDto,
) -> Result<Option<String>, ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || state.run_serialized(|| retry_pending_logout(&request.device_id))).await
}

#[tauri::command]
pub async fn clear_legacy_auth_credentials(
    window: WebviewWindow,
    state: State<'_, ControlPlaneAuthState>,
    request: DeviceSessionRequestDto,
) -> Result<(), ControlPlaneAuthErrorDto> {
    ensure_main_window(window.label())?;
    let state = state.inner().clone();
    run_blocking(move || state.run_serialized(|| clear_legacy_credentials(&request.device_id)))
        .await
}

#[cfg(test)]
mod tests {
    use super::{
        access_session, api_error, control_plane_environment_allows_loopback_http,
        control_plane_environment_allows_test_http, ensure_main_window, refresh_account,
        resolve_control_plane_base_url, validate_login, ApiErrorDto, ControlPlaneAuthState,
        LoginSessionRequestDto, SessionTokensDto,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn auth_commands_are_restricted_to_the_main_window() {
        assert!(ensure_main_window("main").is_ok());
        assert!(ensure_main_window("final-effect").is_err());
    }

    #[test]
    fn refresh_credentials_are_namespaced_by_validated_device_id() {
        assert_eq!(
            refresh_account("device-abc_123").expect("valid device id"),
            "refresh-token-device-abc_123"
        );
        assert!(refresh_account("../other-device").is_err());
    }

    #[test]
    fn production_control_plane_requires_https_and_rejects_url_secrets() {
        assert!(
            resolve_control_plane_base_url(Some("https://control.example.com"), false, false)
                .is_ok()
        );
        assert!(
            resolve_control_plane_base_url(Some("http://127.0.0.1:18090"), true, false).is_ok()
        );
        assert!(
            resolve_control_plane_base_url(Some("http://localhost:18090"), true, false).is_ok()
        );
        assert!(
            resolve_control_plane_base_url(Some("http://101.96.208.132:9090"), true, false)
                .is_err()
        );
        assert!(
            resolve_control_plane_base_url(Some("http://control.example.com"), false, false)
                .is_err()
        );
        assert!(resolve_control_plane_base_url(
            Some("https://user@control.example.com"),
            false,
            false
        )
        .is_err());
        assert!(resolve_control_plane_base_url(None, false, false).is_err());
        assert!(resolve_control_plane_base_url(None, true, false).is_ok());
    }

    #[test]
    fn release_test_environment_allows_only_the_fixed_remote_http_origin() {
        let allow_loopback = control_plane_environment_allows_loopback_http(Some("test"), false);
        let allow_test_http = control_plane_environment_allows_test_http(Some("test"));
        assert!(!allow_loopback);
        assert!(resolve_control_plane_base_url(
            Some("http://127.0.0.1:18090"),
            allow_loopback,
            allow_test_http,
        )
        .is_err());
        assert!(resolve_control_plane_base_url(
            Some("http://101.96.208.132:9090"),
            allow_loopback,
            allow_test_http,
        )
        .is_ok());
        assert!(resolve_control_plane_base_url(
            Some("http://101.96.208.132:9090/api"),
            allow_loopback,
            allow_test_http,
        )
        .is_err());
        assert!(resolve_control_plane_base_url(
            Some("http://101.96.208.133:9090"),
            allow_loopback,
            allow_test_http,
        )
        .is_err());

        let production = control_plane_environment_allows_loopback_http(Some("production"), false);
        let production_test_http = control_plane_environment_allows_test_http(Some("production"));
        assert!(!production);
        assert!(!production_test_http);
        assert!(resolve_control_plane_base_url(
            Some("http://127.0.0.1:18090"),
            production,
            production_test_http,
        )
        .is_err());
        assert!(resolve_control_plane_base_url(
            Some("https://control.example.com"),
            production,
            production_test_http,
        )
        .is_ok());
    }

    #[test]
    fn webview_session_projection_never_serializes_the_refresh_token() {
        let session = access_session(
            SessionTokensDto {
                access_token: "access-secret".to_owned(),
                refresh_token: "refresh-secret".to_owned(),
                expires_at: "2026-08-26T00:00:00Z".to_owned(),
                audience: "desktop".to_owned(),
            },
            None,
            None,
        );
        let serialized = serde_json::to_string(&session).expect("session should serialize");

        assert!(serialized.contains("access-secret"));
        assert!(!serialized.contains("refresh-secret"));
        assert!(!serialized.contains("refresh_token"));
    }

    #[test]
    fn login_password_is_bounded_by_characters_and_utf8_bytes() {
        let request = |password: String| LoginSessionRequestDto {
            device_id: "device-abc".to_owned(),
            username: "alice".to_owned(),
            password,
        };

        assert!(validate_login(&request("a".repeat(256))).is_ok());
        assert!(validate_login(&request("密".repeat(100))).is_err());
        assert!(validate_login(&request("short".to_owned())).is_err());
    }

    #[test]
    fn credential_operations_are_serialized() {
        let state = ControlPlaneAuthState::default();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let state = state.clone();
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                state
                    .run_serialized(|| {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum.fetch_max(current, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(20));
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .expect("serialized operation");
            }));
        }

        barrier.wait();
        for worker in workers {
            worker.join().expect("worker should finish");
        }
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn server_request_id_is_preserved_without_accepting_unbounded_values() {
        let error = api_error(
            ApiErrorDto {
                code: "RATE_LIMITED".to_owned(),
                message: "too many requests".to_owned(),
                request_id: Some("request-123".to_owned()),
            },
            429,
        );
        assert_eq!(error.request_id.as_deref(), Some("request-123"));

        let oversized = api_error(
            ApiErrorDto {
                code: "RATE_LIMITED".to_owned(),
                message: "too many requests".to_owned(),
                request_id: Some("x".repeat(129)),
            },
            429,
        );
        assert!(oversized.request_id.is_none());
    }
}
