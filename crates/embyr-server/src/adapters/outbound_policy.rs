//! Outbound fetch policy for admin SSO (ADR-086 decision 3), IO half: resolve the host and vet
//! every address with the pure rules in `embyr_core::admin::sso::url_policy`.
//!
//! The caller connects to one of the returned addresses (reqwest `ClientBuilder::resolve`), so a
//! hostile resolver cannot answer public here and private at connect time. `http` and private
//! addresses are allowed only under `EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true`, as for DSNs.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use embyr_core::admin::sso::url_policy::{
    check_resolved, parse_outbound_url, OutboundRejection, OutboundUrl,
};

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A URL that passed the policy plus the vetted addresses to connect to.
#[derive(Debug)]
pub struct CheckedTarget {
    pub url: OutboundUrl,
    pub addrs: Vec<SocketAddr>,
}

/// Parse, resolve and vet `url` under the process-wide dev flag.
pub async fn resolve_checked(url: &str) -> Result<CheckedTarget, OutboundRejection> {
    resolve_checked_with(url, super::dsn_policy::allow_private()).await
}

pub async fn resolve_checked_with(
    url: &str,
    allow_private: bool,
) -> Result<CheckedTarget, OutboundRejection> {
    let parsed = parse_outbound_url(url, allow_private)?;
    let addrs: Vec<SocketAddr> = match parsed.ip_literal {
        Some(ip) => vec![SocketAddr::new(ip, parsed.port)],
        None => tokio::time::timeout(
            RESOLVE_TIMEOUT,
            tokio::net::lookup_host((parsed.host.as_str(), parsed.port)),
        )
        .await
        .map_err(|_| OutboundRejection::Unresolvable)?
        .map_err(|_| OutboundRejection::Unresolvable)?
        .collect(),
    };
    let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
    check_resolved(&ips, allow_private)?;
    Ok(CheckedTarget { url: parsed, addrs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn public_ip_literal_passes_and_keeps_the_port() {
        let t = resolve_checked_with("https://8.8.8.8:8443/jwks", false).await.unwrap();
        assert_eq!(t.addrs, vec!["8.8.8.8:8443".parse().unwrap()]);
        assert_eq!(t.url.path_and_query, "/jwks");
    }

    #[tokio::test]
    async fn private_targets_are_refused_unless_the_dev_flag_is_on() {
        for url in ["https://127.0.0.1/", "https://169.254.169.254/", "https://[::1]/", "https://localhost/"] {
            assert_eq!(
                resolve_checked_with(url, false).await.err(),
                Some(OutboundRejection::ForbiddenAddress),
                "{url}"
            );
        }
        let t = resolve_checked_with("http://127.0.0.1:9000/x", true).await.unwrap();
        assert_eq!(t.addrs, vec!["127.0.0.1:9000".parse().unwrap()]);
        assert!(resolve_checked_with("http://127.0.0.1:9000/x", false).await.is_err());
    }

    #[tokio::test]
    async fn shape_rules_apply_before_any_lookup() {
        assert_eq!(
            resolve_checked_with("https://u:p@8.8.8.8/", true).await.err(),
            Some(OutboundRejection::Userinfo)
        );
        assert_eq!(
            resolve_checked_with("ftp://8.8.8.8/", true).await.err(),
            Some(OutboundRejection::NotHttps)
        );
    }

    #[tokio::test]
    async fn unresolvable_host_is_refused() {
        assert_eq!(
            resolve_checked_with("https://does-not-exist.invalid/", false).await.err(),
            Some(OutboundRejection::Unresolvable)
        );
    }
}
