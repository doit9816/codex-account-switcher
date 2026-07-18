use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub(crate) const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxySettings {
    pub(crate) enabled: bool,
    pub(crate) url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeProxyTestResult {
    status: String,
    http_status: Option<u16>,
    proxy_url: String,
    elapsed_ms: u64,
    message: String,
}

pub(crate) async fn test_proxy_settings(
    enabled: bool,
    url: String,
) -> Result<ProbeProxyTestResult, String> {
    let normalized = normalize_proxy_url(&url)?;
    if enabled && normalized.is_empty() {
        return Err("启用代理时必须填写代理地址".to_string());
    }
    let proxy = ProxySettings {
        enabled,
        url: normalized,
    };
    let client = build_probe_client_with_timeout(&proxy, Duration::from_secs(15))?;
    let started = Instant::now();
    let response = client
        .get(CHATGPT_USAGE_URL)
        .send()
        .await
        .map_err(|error| format!("代理连通测试失败: {error}"))?;
    let http_status = response.status().as_u16();
    let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let proxy_url = if proxy.enabled {
        proxy.url
    } else {
        String::new()
    };
    Ok(ProbeProxyTestResult {
        status: "ok".to_string(),
        http_status: Some(http_status),
        proxy_url,
        elapsed_ms,
        message: format!("代理连通测试成功，HTTP {http_status}"),
    })
}

pub(crate) fn build_probe_client(proxy: &ProxySettings) -> Result<reqwest::Client, String> {
    build_probe_client_with_timeout(proxy, DEFAULT_PROBE_TIMEOUT)
}

fn build_probe_client_with_timeout(
    proxy: &ProxySettings,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(DEFAULT_PROBE_CONNECT_TIMEOUT);
    if !timeout.is_zero() {
        builder = builder.timeout(timeout);
    }
    if proxy.enabled {
        let proxy_url = normalize_proxy_url(&proxy.url)?;
        if proxy_url.is_empty() {
            return Err("proxy is enabled but proxy url is empty".to_string());
        }
        builder = builder.proxy(reqwest::Proxy::all(proxy_url).map_err(crate::display_err)?);
    }
    builder.build().map_err(crate::display_err)
}

pub(crate) fn normalize_proxy_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let mut normalized = if trimmed.contains("://") {
        trimmed.to_string()
    } else if bare_proxy_port(trimmed) == Some(1080) {
        format!("socks5h://{trimmed}")
    } else {
        format!("http://{trimmed}")
    };
    if normalized.to_ascii_lowercase().starts_with("socks5://") {
        normalized.replace_range(0.."socks5://".len(), "socks5h://");
    }
    url::Url::parse(&normalized).map_err(|_| "代理地址格式不正确".to_string())?;
    let lower = normalized.to_ascii_lowercase();
    if !(lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("socks5://")
        || lower.starts_with("socks5h://"))
    {
        return Err("代理地址仅支持 http、https、socks5、socks5h".to_string());
    }
    Ok(normalized)
}

fn bare_proxy_port(value: &str) -> Option<u16> {
    url::Url::parse(&format!("http://{value}"))
        .ok()
        .and_then(|url| url.port_or_known_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_validates_probe_proxy_urls() {
        assert_eq!(
            normalize_proxy_url("127.0.0.1:7890").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_url(" socks5://127.0.0.1:7890 ").unwrap(),
            "socks5h://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_url("127.0.0.1:1080").unwrap(),
            "socks5h://127.0.0.1:1080"
        );
        assert_eq!(
            normalize_proxy_url("socks5h://127.0.0.1:1080").unwrap(),
            "socks5h://127.0.0.1:1080"
        );
        assert!(normalize_proxy_url("ftp://127.0.0.1:21").is_err());
        assert_eq!(normalize_proxy_url("").unwrap(), "");
    }

    #[test]
    fn builds_probe_client_with_proxy_settings() {
        assert!(build_probe_client(&ProxySettings::default()).is_ok());
        assert!(build_probe_client(&ProxySettings {
            enabled: true,
            url: "http://127.0.0.1:7890".to_string(),
        })
        .is_ok());
        assert!(build_probe_client(&ProxySettings {
            enabled: true,
            url: "socks5://127.0.0.1:1080".to_string(),
        })
        .is_ok());
        assert!(build_probe_client(&ProxySettings {
            enabled: true,
            url: "".to_string(),
        })
        .is_err());
    }
}
