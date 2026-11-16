//! Audit #65 (SSRF) + X10 (TLS): policy for any customer DSN an operator/admin submits. The server
//! connects to that host, so reject targets inside our own network and plaintext transports, unless
//! `EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true` (docker-compose, dev, tests).

use std::net::IpAddr;
use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgSslMode};

#[derive(Debug, PartialEq, Eq)]
pub enum DsnRejection {
    /// Not a parseable `postgres(ql)://` URL.
    Invalid,
    /// `sslmode` is not `require`, `verify-ca` or `verify-full`.
    TlsRequired,
    /// Host (or any address it resolves to) is loopback/link-local/private/unspecified, or a socket path.
    PrivateHost,
    /// Host did not resolve.
    Unresolvable,
}

impl DsnRejection {
    pub fn detail(&self) -> &'static str {
        match self {
            DsnRejection::Invalid => "invalid_dsn",
            DsnRejection::TlsRequired => "dsn_tls_required",
            DsnRejection::PrivateHost => "dsn_host_not_allowed",
            DsnRejection::Unresolvable => "dsn_host_unresolvable",
        }
    }
}

pub fn allow_private() -> bool {
    std::env::var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS").is_ok_and(|v| v.eq_ignore_ascii_case("true"))
}

/// Pure: true for addresses a customer DSN must not point at.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(mapped));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || first & 0xfe00 == 0xfc00 // unique-local fc00::/7
                || first & 0xffc0 == 0xfe80 // link-local fe80::/10
        }
    }
}

/// Pure: sslmode acceptable for a customer connection.
pub fn tls_acceptable(mode: PgSslMode) -> bool {
    matches!(mode, PgSslMode::Require | PgSslMode::VerifyCa | PgSslMode::VerifyFull)
}

/// Validate `dsn`. Scheme/shape is always checked; TLS and host rules are lifted by the dev flag.
pub async fn check_dsn(dsn: &str) -> Result<(), DsnRejection> {
    // sqlx's URL parser ignores the scheme, so check it ourselves.
    let lower = dsn.get(..13).unwrap_or(dsn).to_ascii_lowercase();
    if !(lower.starts_with("postgres://") || lower.starts_with("postgresql://")) {
        return Err(DsnRejection::Invalid);
    }
    let opts = PgConnectOptions::from_str(dsn).map_err(|_| DsnRejection::Invalid)?;
    if allow_private() {
        return Ok(());
    }
    if !tls_acceptable(opts.get_ssl_mode()) {
        return Err(DsnRejection::TlsRequired);
    }
    if opts.get_socket().is_some() {
        return Err(DsnRejection::PrivateHost);
    }
    let addrs = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((opts.get_host().trim_matches(|c| c == '[' || c == ']'), opts.get_port())),
    )
    .await
    .map_err(|_| DsnRejection::Unresolvable)?
    .map_err(|_| DsnRejection::Unresolvable)?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if is_forbidden_ip(addr.ip()) {
            return Err(DsnRejection::PrivateHost);
        }
    }
    // ponytail: DNS-rebinding ceiling. We resolve here, but sqlx resolves again when it connects, so a
    // hostile resolver can answer public now and private then. Pin by connecting to the checked IP
    // (hostaddr=) if this ever matters.
    if any {
        Ok(())
    } else {
        Err(DsnRejection::Unresolvable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_addresses() {
        for bad in [
            "127.0.0.1", "127.8.8.8", "169.254.169.254", "10.1.2.3", "172.16.0.1", "172.31.255.1",
            "192.168.1.1", "0.0.0.0", "::1", "::", "fe80::1", "fd00::1", "fc00::1", "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
        ] {
            assert!(is_forbidden_ip(bad.parse().unwrap()), "{bad} must be forbidden");
        }
        for ok in ["8.8.8.8", "172.32.0.1", "192.169.0.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(!is_forbidden_ip(ok.parse().unwrap()), "{ok} must be allowed");
        }
    }

    #[test]
    fn tls_modes() {
        assert!(!tls_acceptable(PgSslMode::Disable));
        assert!(!tls_acceptable(PgSslMode::Allow));
        assert!(!tls_acceptable(PgSslMode::Prefer));
        assert!(tls_acceptable(PgSslMode::Require));
        assert!(tls_acceptable(PgSslMode::VerifyCa));
        assert!(tls_acceptable(PgSslMode::VerifyFull));
    }

    // Flag-unset behaviour; IP-literal hosts need no DNS. (Unit-test binary never sets the flag.)
    #[tokio::test]
    async fn check_dsn_rejections() {
        assert_eq!(check_dsn("mysql://u@8.8.8.8/db").await, Err(DsnRejection::Invalid));
        assert_eq!(check_dsn("postgres://u@8.8.8.8/db").await, Err(DsnRejection::TlsRequired));
        assert_eq!(
            check_dsn("postgres://u@8.8.8.8/db?sslmode=prefer").await,
            Err(DsnRejection::TlsRequired)
        );
        for host in ["169.254.169.254", "127.0.0.1", "10.0.0.5", "[::1]"] {
            assert_eq!(
                check_dsn(&format!("postgres://u@{host}/db?sslmode=require")).await,
                Err(DsnRejection::PrivateHost),
                "{host}"
            );
        }
        // hostaddr overrides the displayed host
        assert_eq!(
            check_dsn("postgres://u@8.8.8.8/db?sslmode=require&hostaddr=127.0.0.1").await,
            Err(DsnRejection::PrivateHost)
        );
        assert_eq!(check_dsn("postgres://u@8.8.8.8/db?sslmode=verify-full").await, Ok(()));
    }
}
