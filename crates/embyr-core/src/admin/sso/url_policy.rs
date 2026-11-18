//! Outbound URL / IP policy (ADR-086 decision 3), pure half.
//!
//! Applies to every server-initiated fetch whose target an account Owner
//! influences: discovery, JWKS, token endpoint. `embyr-server` resolves the
//! host, calls [`check_resolved`] and connects to the checked address.
//! `allow_private` (`EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true`: dev, compose, tests
//! against a loopback mock IdP) lifts the scheme and address rules only; the
//! URL shape rules always apply.

use std::net::{IpAddr, Ipv4Addr};

const MAX_URL_LEN: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundRejection {
    /// Not an absolute `scheme://host[:port][/path]` URL, or contains characters we refuse.
    Malformed,
    /// Scheme is not `https` (or `http` under `allow_private`).
    NotHttps,
    /// `user:pass@` in the authority.
    Userinfo,
    /// Host (or a resolved address) is loopback/private/link-local/etc.
    ForbiddenAddress,
    /// Host resolved to no address.
    Unresolvable,
}

impl OutboundRejection {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Malformed => "url_malformed",
            Self::NotHttps => "url_not_https",
            Self::Userinfo => "url_userinfo",
            Self::ForbiddenAddress => "url_forbidden_address",
            Self::Unresolvable => "url_unresolvable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundUrl {
    pub https: bool,
    /// Lower-cased host name, or the IP literal without brackets.
    pub host: String,
    pub port: u16,
    /// Set when `host` is an IP literal (no DNS needed).
    pub ip_literal: Option<IpAddr>,
    /// Always starts with `/`; includes the query string.
    pub path_and_query: String,
}

/// Addresses a customer DSN must not point at: loopback, private, link-local,
/// unspecified, unique-local and IPv4-mapped forms of those. (Moved here from
/// `dsn_policy`; behaviour unchanged.)
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

/// [`is_forbidden_ip`] plus ranges that are never a public IdP: `0.0.0.0/8`,
/// CGNAT `100.64.0.0/10` (some clouds expose metadata there), broadcast,
/// multicast. ponytail: no NAT64/6to4 unwrapping; add if egress ever goes through one.
pub fn is_forbidden_outbound_ip(ip: IpAddr) -> bool {
    if is_forbidden_ip(ip) {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => is_extra_forbidden_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(mapped) => is_extra_forbidden_v4(mapped),
            None => v6.is_multicast(),
        },
    }
}

fn is_extra_forbidden_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 0 || (o[0] == 100 && o[1] & 0xc0 == 64) || v4.is_broadcast() || v4.is_multicast()
}

/// Parse and vet an outbound URL before any DNS or network activity.
pub fn parse_outbound_url(url: &str, allow_private: bool) -> Result<OutboundUrl, OutboundRejection> {
    use OutboundRejection::*;
    if url.len() > MAX_URL_LEN || !url.bytes().all(|b| b.is_ascii_graphic()) || url.contains(['\\', '#']) {
        return Err(Malformed);
    }
    let (scheme, rest) = url.split_once("://").ok_or(Malformed)?;
    let https = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" if allow_private => false,
        _ => return Err(NotHttps),
    };
    let (authority, path_and_query) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    if authority.contains('@') {
        return Err(Userinfo);
    }
    let default_port = if https { 443 } else { 80 };
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, after) = inner.split_once(']').ok_or(Malformed)?;
        let port = match after {
            "" => default_port,
            _ => parse_port(after.strip_prefix(':').ok_or(Malformed)?)?,
        };
        (host, port)
    } else {
        match authority.split_once(':') {
            None => (authority, default_port),
            Some((h, p)) => (h, parse_port(p)?),
        }
    };
    let ip_literal = host.parse::<IpAddr>().ok();
    if ip_literal.is_none()
        && (host.is_empty()
            || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    {
        return Err(Malformed);
    }
    // A bracketed host must be an IPv6 literal; an unbracketed one must not be.
    if authority.starts_with('[') != matches!(ip_literal, Some(IpAddr::V6(_))) {
        return Err(Malformed);
    }
    if let Some(ip) = ip_literal {
        if !allow_private && is_forbidden_outbound_ip(ip) {
            return Err(ForbiddenAddress);
        }
    }
    Ok(OutboundUrl {
        https,
        host: host.to_ascii_lowercase(),
        port,
        ip_literal,
        path_and_query,
    })
}

fn parse_port(p: &str) -> Result<u16, OutboundRejection> {
    if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
        return Err(OutboundRejection::Malformed);
    }
    match p.parse::<u16>() {
        Ok(n) if n != 0 => Ok(n),
        _ => Err(OutboundRejection::Malformed),
    }
}

/// Vet the addresses a host resolved to: at least one, and every one allowed
/// (a mixed public/private answer is refused, so the connect cannot land on the private one).
pub fn check_resolved(addrs: &[IpAddr], allow_private: bool) -> Result<(), OutboundRejection> {
    if addrs.is_empty() {
        return Err(OutboundRejection::Unresolvable);
    }
    if !allow_private && addrs.iter().any(|ip| is_forbidden_outbound_ip(*ip)) {
        return Err(OutboundRejection::ForbiddenAddress);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use OutboundRejection::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn dsn_predicate_unchanged() {
        for bad in [
            "127.0.0.1", "127.8.8.8", "169.254.169.254", "10.1.2.3", "172.16.0.1", "172.31.255.1",
            "192.168.1.1", "0.0.0.0", "::1", "::", "fe80::1", "fd00::1", "fc00::1",
            "::ffff:127.0.0.1", "::ffff:169.254.169.254",
        ] {
            assert!(is_forbidden_ip(ip(bad)), "{bad}");
        }
        for ok in ["8.8.8.8", "172.32.0.1", "192.169.0.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(!is_forbidden_ip(ip(ok)), "{ok}");
        }
    }

    #[test]
    fn outbound_predicate_is_a_superset() {
        for bad in [
            "0.1.2.3", "100.64.0.1", "100.100.100.200", "100.127.255.255", "255.255.255.255",
            "224.0.0.1", "239.1.1.1", "ff02::1", "::ffff:100.64.0.1", "::ffff:224.0.0.1",
            "127.0.0.1", "10.0.0.1", "::1",
        ] {
            assert!(is_forbidden_outbound_ip(ip(bad)), "{bad}");
        }
        for ok in ["8.8.8.8", "100.63.255.255", "100.128.0.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(!is_forbidden_outbound_ip(ip(ok)), "{ok}");
        }
    }

    #[test]
    fn accepts_plain_https_urls() {
        let u = parse_outbound_url("https://Accounts.Example.com/.well-known/openid-configuration", false).unwrap();
        assert_eq!(
            u,
            OutboundUrl {
                https: true,
                host: "accounts.example.com".into(),
                port: 443,
                ip_literal: None,
                path_and_query: "/.well-known/openid-configuration".into(),
            }
        );
        let u = parse_outbound_url("HTTPS://idp.example.com:8443", false).unwrap();
        assert_eq!((u.port, u.path_and_query.as_str()), (8443, "/"));
        let u = parse_outbound_url("https://idp.example.com?x=1", false).unwrap();
        assert_eq!(u.path_and_query, "/?x=1");
        let u = parse_outbound_url("https://8.8.8.8/jwks", false).unwrap();
        assert_eq!(u.ip_literal, Some(ip("8.8.8.8")));
        let u = parse_outbound_url("https://[2606:4700::1111]:444/x", false).unwrap();
        assert_eq!((u.ip_literal, u.port), (Some(ip("2606:4700::1111")), 444));
    }

    #[test]
    fn scheme_rules() {
        for bad in ["http://idp.example.com/", "ftp://idp.example.com/", "file:///etc/passwd", "gopher://x/"] {
            assert_eq!(parse_outbound_url(bad, false), Err(NotHttps), "{bad}");
        }
        assert_eq!(parse_outbound_url("idp.example.com/x", false), Err(Malformed));
        // dev flag allows http, still nothing else
        assert!(!parse_outbound_url("http://127.0.0.1:9000/x", true).unwrap().https);
        assert_eq!(parse_outbound_url("ftp://127.0.0.1/", true), Err(NotHttps));
    }

    #[test]
    fn userinfo_and_shape_rules() {
        assert_eq!(parse_outbound_url("https://user:pw@idp.example.com/", false), Err(Userinfo));
        assert_eq!(parse_outbound_url("https://idp.example.com@evil.com/", false), Err(Userinfo));
        assert_eq!(parse_outbound_url("https://evil.com\\@idp.example.com/", false), Err(Malformed));
        for bad in [
            "https://", "https:///x", "https://idp.example.com:/x", "https://idp.example.com:0/",
            "https://idp.example.com:65536/", "https://idp.example.com:80a/",
            "https://idp example.com/", "https://idp.example.com/a b", "https://idp.example.com/#frag",
            "https://idp.example.com/\n", "https://idp_example.com/", "https://[::1/x",
            "https://[::1]x/", "https://[example.com]/", "https://::1/x", "https://\u{e9}.example.com/",
        ] {
            assert_eq!(parse_outbound_url(bad, true), Err(Malformed), "{bad:?}");
        }
        let long = format!("https://idp.example.com/{}", "a".repeat(2100));
        assert_eq!(parse_outbound_url(&long, false), Err(Malformed));
    }

    #[test]
    fn ip_literal_targets_refused_unless_dev_flag() {
        for bad in [
            "https://127.0.0.1/", "https://169.254.169.254/latest/meta-data", "https://10.0.0.5/",
            "https://[::1]/", "https://[::ffff:127.0.0.1]/", "https://100.100.100.200/", "https://0.0.0.0/",
        ] {
            assert_eq!(parse_outbound_url(bad, false), Err(ForbiddenAddress), "{bad}");
            assert!(parse_outbound_url(bad, true).is_ok(), "{bad} with dev flag");
        }
    }

    #[test]
    fn host_names_are_left_to_resolution() {
        // localhost / numeric shorthands are names to the parser; check_resolved catches them
        for name in ["localhost", "localhost.", "127.1", "2130706433", "0x7f.1"] {
            assert!(parse_outbound_url(&format!("https://{name}/"), false).is_ok(), "{name}");
        }
    }

    #[test]
    fn resolved_address_rules() {
        assert_eq!(check_resolved(&[], false), Err(Unresolvable));
        assert_eq!(check_resolved(&[], true), Err(Unresolvable));
        assert_eq!(check_resolved(&[ip("8.8.8.8"), ip("2606:4700::1111")], false), Ok(()));
        assert_eq!(check_resolved(&[ip("127.0.0.1")], false), Err(ForbiddenAddress));
        // mixed answer (DNS rebinding style) is refused as a whole
        assert_eq!(check_resolved(&[ip("8.8.8.8"), ip("10.0.0.1")], false), Err(ForbiddenAddress));
        assert_eq!(check_resolved(&[ip("::ffff:169.254.169.254")], false), Err(ForbiddenAddress));
        assert_eq!(check_resolved(&[ip("127.0.0.1")], true), Ok(()));
    }

    #[test]
    fn rejection_reasons_are_distinct_labels() {
        let all = [Malformed, NotHttps, Userinfo, ForbiddenAddress, Unresolvable];
        let set: std::collections::HashSet<_> = all.iter().map(|r| r.reason()).collect();
        assert_eq!(set.len(), all.len());
    }
}
