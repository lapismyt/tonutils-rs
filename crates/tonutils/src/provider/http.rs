use std::{sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose};
use num_bigint::BigUint;
use reqwest::{
    Client, Method, RequestBuilder, StatusCode, Url,
    header::{HeaderName, HeaderValue},
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    tl::TlInt256 as Int256,
    tvm::{Cell, deserialize_boc},
};

use super::error::{ProviderError, ProviderResult};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Auth {
    Header(&'static str),
    Bearer,
}

#[derive(Clone)]
pub(crate) struct HttpTransport {
    client: Client,
    base_url: Url,
    header_name: Option<HeaderName>,
    header_value: Option<HeaderValue>,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("base_url", &self.base_url)
            .field("authenticated", &self.header_value.is_some())
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    pub(crate) fn new(
        endpoint: &str,
        api_key: Option<String>,
        auth: Auth,
        timeout: Option<Duration>,
        client: Option<Client>,
    ) -> ProviderResult<Self> {
        let base_url = Url::parse(endpoint)
            .map_err(|error| ProviderError::Url(format!("{endpoint}: {error}")))?;
        let client = match (client, timeout) {
            (Some(client), _) => client,
            (None, Some(timeout)) => Client::builder()
                .timeout(timeout)
                .build()
                .map_err(ProviderError::Http)?,
            (None, None) => Client::new(),
        };
        let (header_name, header_value) = match (auth, api_key) {
            (_, None) => (None, None),
            (Auth::Bearer, Some(key)) => (
                Some(HeaderName::from_static("authorization")),
                Some(
                    HeaderValue::from_str(&format!("Bearer {key}")).map_err(|error| {
                        ProviderError::Url(format!("invalid bearer token: {error}"))
                    })?,
                ),
            ),
            (Auth::Header(name), Some(key)) => (
                Some(HeaderName::from_static(name)),
                Some(HeaderValue::from_str(&key).map_err(|error| {
                    ProviderError::Url(format!("invalid {name} header value: {error}"))
                })?),
            ),
        };
        Ok(Self {
            client,
            base_url,
            header_name,
            header_value,
        })
    }

    pub(crate) fn url(&self, path: &str) -> ProviderResult<Url> {
        let joined = format!(
            "{}/{}",
            self.base_url.as_str().trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        Url::parse(&joined).map_err(|error| ProviderError::Url(format!("{joined}: {error}")))
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> ProviderResult<RequestBuilder> {
        let mut request = self.client.request(method, self.url(path)?);
        if let (Some(name), Some(value)) = (&self.header_name, &self.header_value) {
            request = request.header(name, value);
        }
        Ok(request)
    }

    pub(crate) async fn send_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        provider: &'static str,
    ) -> ProviderResult<T> {
        let bytes = self.send(request, provider).await?;
        serde_json::from_slice(&bytes).map_err(|source| ProviderError::Decode { provider, source })
    }

    pub(crate) async fn send_json_value(
        &self,
        request: RequestBuilder,
        provider: &'static str,
    ) -> ProviderResult<Value> {
        self.send_json(request, provider).await
    }

    pub(crate) async fn send_empty(
        &self,
        request: RequestBuilder,
        provider: &'static str,
    ) -> ProviderResult<()> {
        let _ = self.send(request, provider).await?;
        Ok(())
    }

    async fn send(
        &self,
        request: RequestBuilder,
        provider: &'static str,
    ) -> ProviderResult<Vec<u8>> {
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(api_error(provider, status, &bytes));
        }
        Ok(bytes.to_vec())
    }
}

pub(crate) fn api_error(provider: &'static str, status: StatusCode, body: &[u8]) -> ProviderError {
    ProviderError::Api {
        provider,
        status: status.as_u16(),
        message: error_message(body),
    }
}

pub(crate) fn error_message(body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        for key in ["error", "message", "detail"] {
            if let Some(message) = value.get(key) {
                if let Some(message) = message.as_str() {
                    return message.to_owned();
                }
                if !message.is_null() {
                    return message.to_string();
                }
            }
        }
    }
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "<empty response>".to_owned()
    } else {
        trimmed.chars().take(512).collect()
    }
}

pub(crate) fn decode_boc_value(value: &str) -> Result<Arc<Cell>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty BoC value".to_owned());
    }
    let bytes = decode_bytes(value)?;
    deserialize_boc(&bytes).map_err(|error| format!("BoC decode failed: {error}"))
}

pub(crate) fn decode_bytes(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim();
    let hex_value = value.strip_prefix("0x").unwrap_or(value);
    if hex_value.len().is_multiple_of(2)
        && !hex_value.is_empty()
        && hex_value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return hex::decode(hex_value).map_err(|error| format!("hex decode failed: {error}"));
    }
    general_purpose::STANDARD
        .decode(value)
        .or_else(|_| general_purpose::URL_SAFE.decode(value))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(value))
        .map_err(|error| format!("base64 decode failed: {error}"))
}

pub(crate) fn parse_int256(value: &str) -> Result<Int256, String> {
    let bytes = decode_bytes(value)?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(Int256(hash))
}

pub(crate) fn optional_int256(value: Option<&str>) -> Result<Option<Int256>, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(parse_int256)
        .transpose()
}

pub(crate) fn parse_biguint(value: &str) -> Result<BigUint, String> {
    let value = value.trim();
    if let Some(value) = value.strip_prefix("0x") {
        BigUint::parse_bytes(value.as_bytes(), 16)
            .ok_or_else(|| format!("invalid hex integer: {value}"))
    } else {
        BigUint::parse_bytes(value.as_bytes(), 10)
            .ok_or_else(|| format!("invalid decimal integer: {value}"))
    }
}

pub(crate) fn required_str<'a>(
    value: &'a Value,
    field: &str,
    provider: &'static str,
) -> ProviderResult<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::InvalidResponse {
            provider,
            message: format!("missing string field `{field}`"),
        })
}

pub(crate) fn optional_str<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

pub(crate) fn required_u64(
    value: &Value,
    field: &str,
    provider: &'static str,
) -> ProviderResult<u64> {
    value_u64(value.get(field), field, provider)
}

pub(crate) fn value_u64(
    value: Option<&Value>,
    field: &str,
    provider: &'static str,
) -> ProviderResult<u64> {
    let Some(value) = value else {
        return Err(ProviderError::InvalidResponse {
            provider,
            message: format!("missing field `{field}`"),
        });
    };
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    if let Some(value) = value.as_str() {
        return value
            .parse::<u64>()
            .map_err(|error| ProviderError::InvalidResponse {
                provider,
                message: format!("invalid u64 field `{field}`: {error}"),
            });
    }
    Err(ProviderError::InvalidResponse {
        provider,
        message: format!("field `{field}` is not a u64"),
    })
}

pub(crate) fn required_i64(
    value: &Value,
    field: &str,
    provider: &'static str,
) -> ProviderResult<i64> {
    let Some(value) = value.get(field) else {
        return Err(ProviderError::InvalidResponse {
            provider,
            message: format!("missing field `{field}`"),
        });
    };
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    if let Some(value) = value.as_str() {
        return value
            .parse::<i64>()
            .map_err(|error| ProviderError::InvalidResponse {
                provider,
                message: format!("invalid i64 field `{field}`: {error}"),
            });
    }
    Err(ProviderError::InvalidResponse {
        provider,
        message: format!("field `{field}` is not an i64"),
    })
}
