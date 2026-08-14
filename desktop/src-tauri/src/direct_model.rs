#[cfg(test)]
mod tests {
    use super::{
        DelegatedAccessCredential, DirectChatMessage, DirectChatRequest, DirectLeaseDescriptor,
        DirectLeaseSession, DirectModelError,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, SystemTime};

    fn descriptor() -> DirectLeaseDescriptor {
        DirectLeaseDescriptor {
            lease_id: "lease_001".to_owned(),
            provider: "openai-compatible".to_owned(),
            model: "rewrite-model".to_owned(),
            status: "active".to_owned(),
            proxy_mode: "direct_lease".to_owned(),
            direct_base_url: "https://api.example.com/v1".to_owned(),
            expires_at: SystemTime::now() + Duration::from_secs(300),
        }
    }

    #[test]
    fn direct_call_requires_a_supplier_delegated_credential() {
        let result = DirectLeaseSession::new(descriptor(), None, SystemTime::now());
        assert_eq!(result, Err(DirectModelError::DelegatedCredentialRequired));
    }

    #[test]
    fn direct_call_rejects_expired_lease_and_credential() {
        let now = SystemTime::now();
        let mut expired = descriptor();
        expired.expires_at = now - Duration::from_secs(1);
        let credential = DelegatedAccessCredential {
            access_token: "delegated-token".to_owned(),
            expires_at: now + Duration::from_secs(60),
        };
        assert_eq!(
            DirectLeaseSession::new(expired, Some(credential), now),
            Err(DirectModelError::LeaseExpired)
        );
    }

    #[test]
    fn direct_session_uses_the_lease_url() {
        let now = SystemTime::now();
        let credential = DelegatedAccessCredential {
            access_token: "delegated-token".to_owned(),
            expires_at: now + Duration::from_secs(60),
        };
        let session = DirectLeaseSession::new(descriptor(), Some(credential), now)
            .expect("valid delegated lease should create a session");
        assert_eq!(
            session.chat_completions_url(),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn direct_session_rejects_public_plain_http() {
        let mut lease = descriptor();
        lease.direct_base_url = "http://api.example.com/v1".to_owned();
        let credential = DelegatedAccessCredential {
            access_token: "delegated-token".to_owned(),
            expires_at: SystemTime::now() + Duration::from_secs(60),
        };
        assert_eq!(
            DirectLeaseSession::new(lease, Some(credential), SystemTime::now()),
            Err(DirectModelError::InvalidLeaseUrl)
        );
    }

    #[test]
    fn direct_chat_request_rejects_invalid_message_before_network() {
        let request = DirectChatRequest {
            messages: vec![DirectChatMessage {
                role: "tool".to_owned(),
                content: "bad".to_owned(),
            }],
            temperature: None,
            max_tokens: None,
        };
        assert!(matches!(
            super::validate_chat_request(&request),
            Err(DirectModelError::InvalidRequest(_))
        ));
    }

    #[test]
    fn direct_chat_completions_posts_openai_compatible_request_and_returns_summary() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("local test listener should bind");
        let address = listener
            .local_addr()
            .expect("listener address should exist");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4 * 1024];
            loop {
                let size = stream.read(&mut chunk).expect("request should be readable");
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..size]);
                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| position + 4);
                let body_length = header_end.and_then(|end| {
                    String::from_utf8_lossy(&request[..end])
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then_some(value)
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                });
                if let (Some(end), Some(body_length)) = (header_end, body_length) {
                    if request.len() >= end + body_length {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            assert!(request.contains("post /v1/chat/completions http/1.1"));
            assert!(request.contains("authorization: bearer delegated-token"));
            assert!(request.contains("\"model\":\"rewrite-model\""));
            let body = r#"{"model":"rewrite-model","choices":[{"message":{"content":"改写后的话术"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":5,"total_tokens":13}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be writable");
        });

        let now = SystemTime::now();
        let mut lease = descriptor();
        lease.direct_base_url = format!("http://{address}/v1");
        lease.expires_at = now + Duration::from_secs(60);
        let session = DirectLeaseSession::new(
            lease,
            Some(DelegatedAccessCredential {
                access_token: "delegated-token".to_owned(),
                expires_at: now + Duration::from_secs(30),
            }),
            now,
        )
        .expect("local delegated lease should be valid");
        let result = session
            .chat_completions(
                &DirectChatRequest {
                    messages: vec![DirectChatMessage {
                        role: "user".to_owned(),
                        content: "请改写".to_owned(),
                    }],
                    temperature: Some(0.4),
                    max_tokens: Some(64),
                },
                Duration::from_secs(3),
                now,
            )
            .expect("OpenAI-compatible response should be parsed");
        server.join().expect("test server should finish");
        assert_eq!(result.text, "改写后的话术");
        assert_eq!(result.model, "rewrite-model");
        assert_eq!(result.usage.expect("usage should exist").total_tokens, 13);
    }
}
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedAccessCredential {
    pub access_token: String,
    pub expires_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectLeaseDescriptor {
    pub lease_id: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub proxy_mode: String,
    pub direct_base_url: String,
    pub expires_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectLeaseSession {
    descriptor: DirectLeaseDescriptor,
    credential: DelegatedAccessCredential,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectModelError {
    DelegatedCredentialRequired,
    LeaseExpired,
    LeaseUnavailable,
    InvalidLeaseUrl,
    InvalidRequest(String),
    RequestFailed(String),
    SupplierRejected { status: u16 },
    InvalidResponse(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectChatRequest {
    pub messages: Vec<DirectChatMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectChatResult {
    pub text: String,
    pub model: String,
    pub finish_reason: Option<String>,
    pub usage: Option<DirectUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: Option<DirectUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAIChatRequest {
    model: String,
    messages: Vec<DirectChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

impl DirectLeaseSession {
    pub fn new(
        descriptor: DirectLeaseDescriptor,
        credential: Option<DelegatedAccessCredential>,
        now: SystemTime,
    ) -> Result<Self, DirectModelError> {
        if descriptor.status != "active" || descriptor.proxy_mode != "direct_lease" {
            return Err(DirectModelError::LeaseUnavailable);
        }
        validate_base_url(&descriptor.direct_base_url)?;
        if descriptor.expires_at <= now {
            return Err(DirectModelError::LeaseExpired);
        }
        let credential = credential.ok_or(DirectModelError::DelegatedCredentialRequired)?;
        if credential.access_token.trim().is_empty() {
            return Err(DirectModelError::DelegatedCredentialRequired);
        }
        if credential.expires_at <= now || credential.expires_at > descriptor.expires_at {
            return Err(DirectModelError::LeaseExpired);
        }
        Ok(Self {
            descriptor,
            credential,
        })
    }

    #[must_use]
    pub fn chat_completions_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.descriptor.direct_base_url.trim_end_matches('/')
        )
    }

    #[must_use]
    pub fn bearer_token(&self) -> &str {
        &self.credential.access_token
    }

    pub fn chat_completions(
        &self,
        request: &DirectChatRequest,
        timeout: Duration,
        now: SystemTime,
    ) -> Result<DirectChatResult, DirectModelError> {
        self.ensure_live(now)?;
        validate_chat_request(request)?;
        if timeout.is_zero() {
            return Err(DirectModelError::InvalidRequest(
                "模型请求超时必须大于 0".to_owned(),
            ));
        }
        let remaining = self
            .descriptor
            .expires_at
            .duration_since(now)
            .map_err(|_| DirectModelError::LeaseExpired)?;
        let client = Client::builder()
            .no_proxy()
            .timeout(timeout.min(remaining))
            .build()
            .map_err(|error| DirectModelError::RequestFailed(error.to_string()))?;
        let response = client
            .post(self.chat_completions_url())
            .header(AUTHORIZATION, format!("Bearer {}", self.bearer_token()))
            .header(CONTENT_TYPE, "application/json")
            .json(&OpenAIChatRequest {
                model: self.descriptor.model.clone(),
                messages: request.messages.clone(),
                temperature: request.temperature,
                max_tokens: request.max_tokens,
            })
            .send()
            .map_err(|error| DirectModelError::RequestFailed(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(DirectModelError::SupplierRejected {
                status: status.as_u16(),
            });
        }
        let payload = response
            .json::<OpenAIChatResponse>()
            .map_err(|error| DirectModelError::InvalidResponse(error.to_string()))?;
        let choice = payload
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| DirectModelError::InvalidResponse("响应缺少 choices".to_owned()))?;
        let text = choice
            .message
            .content
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DirectModelError::InvalidResponse("响应缺少模型文本".to_owned()))?;
        Ok(DirectChatResult {
            text,
            model: payload.model,
            finish_reason: choice.finish_reason,
            usage: payload.usage,
        })
    }

    fn ensure_live(&self, now: SystemTime) -> Result<(), DirectModelError> {
        if self.descriptor.expires_at <= now || self.credential.expires_at <= now {
            return Err(DirectModelError::LeaseExpired);
        }
        Ok(())
    }
}

fn validate_base_url(value: &str) -> Result<(), DirectModelError> {
    let value = value.trim();
    let parsed = reqwest::Url::parse(value).map_err(|_| DirectModelError::InvalidLeaseUrl)?;
    if parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path().is_empty()
        || (parsed.scheme() != "https"
            && !(parsed.scheme() == "http"
                && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))))
    {
        return Err(DirectModelError::InvalidLeaseUrl);
    }
    Ok(())
}

fn validate_chat_request(request: &DirectChatRequest) -> Result<(), DirectModelError> {
    if request.messages.is_empty() || request.messages.len() > 64 {
        return Err(DirectModelError::InvalidRequest(
            "messages 数量必须在 1 到 64 之间".to_owned(),
        ));
    }
    for message in &request.messages {
        if !matches!(message.role.as_str(), "system" | "user" | "assistant")
            || message.content.trim().is_empty()
            || message.content.chars().count() > 32_000
        {
            return Err(DirectModelError::InvalidRequest(
                "消息角色或内容不符合限制".to_owned(),
            ));
        }
    }
    if request
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        || request
            .max_tokens
            .is_some_and(|value| !(1..=16_384).contains(&value))
    {
        return Err(DirectModelError::InvalidRequest(
            "temperature 或 max_tokens 超出范围".to_owned(),
        ));
    }
    Ok(())
}
