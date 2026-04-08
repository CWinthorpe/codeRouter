use url::Host;

/// Validate that a base_url is safe for upstream requests.
/// Rejects private/reserved IP ranges to prevent SSRF attacks.
///
/// NOTE: This validation checks the URL string at parse time only.
/// It does NOT protect against DNS rebinding attacks, where a domain
/// initially resolves to a public IP but later resolves to a private IP.
/// For full DNS rebinding protection, resolve the hostname at request
/// time and validate the IP address before each connection.
pub fn validate_base_url(base_url: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(base_url).map_err(|e| format!("Invalid URL '{}': {}", base_url, e))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "base_url must use http or https scheme, got '{}'",
            scheme
        ));
    }

    let host = parsed
        .host()
        .ok_or_else(|| format!("base_url '{}' has no host", base_url))?;

    match host {
        Host::Ipv4(addr) => {
            if is_private_or_reserved_v4(&addr) {
                return Err(format!(
                    "base_url '{}' resolves to a private/reserved IPv4 address ({})",
                    base_url, addr
                ));
            }
        }
        Host::Ipv6(addr) => {
            if is_private_or_reserved_v6(&addr) {
                return Err(format!(
                    "base_url '{}' resolves to a private/reserved IPv6 address ({})",
                    base_url, addr
                ));
            }
        }
        Host::Domain(domain) => {
            let lower = domain.to_lowercase();
            if lower == "localhost"
                || lower == "localhost.localdomain"
                || lower == "ip6-localhost"
                || lower == "ip6-loopback"
                || lower.ends_with(".local")
                || lower.ends_with(".internal")
                || lower.ends_with(".localhost")
            {
                return Err(format!(
                    "base_url '{}' uses a reserved hostname '{}'",
                    base_url, domain
                ));
            }
        }
    }

    Ok(())
}

fn is_private_or_reserved_v4(addr: &std::net::Ipv4Addr) -> bool {
    addr.is_loopback()
        || addr.is_private()
        || addr.is_link_local()
        || addr.is_broadcast()
        || addr.is_documentation()
        || addr.octets()[0] == 0
        || (addr.octets()[0] == 100 && addr.octets()[1] & 0b1100_0000 == 0b0100_0000)
        || (addr.octets()[0] == 198 && addr.octets()[1] == 18)
        || (addr.octets()[0] == 192 && addr.octets()[1] == 0 && addr.octets()[2] == 0)
        || (addr.octets()[0] == 192 && addr.octets()[1] == 0 && addr.octets()[2] == 2)
        || (addr.octets()[0] == 198 && addr.octets()[1] == 51 && addr.octets()[2] == 100)
        || (addr.octets()[0] == 203 && addr.octets()[1] == 0 && addr.octets()[2] == 113)
        || addr.octets()[0] >= 224
}

fn is_private_or_reserved_v6(addr: &std::net::Ipv6Addr) -> bool {
    addr.is_loopback()
        || addr.is_unspecified()
        || addr.segments()[0] == 0xfe80
        || addr.segments()[0] & 0xffc0 == 0xfc00
        || addr
            .to_ipv4_mapped()
            .is_some_and(|v4| is_private_or_reserved_v4(&v4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_public_urls() {
        assert!(validate_base_url("https://api.openai.com/v1").is_ok());
        assert!(validate_base_url("https://api.anthropic.com/v1").is_ok());
        assert!(validate_base_url("http://example.com/v1").is_ok());
        assert!(validate_base_url("https://api.z.ai/v1").is_ok());
    }

    #[test]
    fn test_rejects_loopback() {
        assert!(validate_base_url("http://127.0.0.1:8080/v1").is_err());
        assert!(validate_base_url("http://127.0.0.2/v1").is_err());
        assert!(validate_base_url("http://localhost:8080/v1").is_err());
        assert!(validate_base_url("http://localhost.localdomain/v1").is_err());
    }

    #[test]
    fn test_rejects_private_ranges() {
        assert!(validate_base_url("http://10.0.0.1/v1").is_err());
        assert!(validate_base_url("http://10.255.255.255/v1").is_err());
        assert!(validate_base_url("http://172.16.0.1/v1").is_err());
        assert!(validate_base_url("http://172.31.255.255/v1").is_err());
        assert!(validate_base_url("http://192.168.0.1/v1").is_err());
        assert!(validate_base_url("http://192.168.1.100/v1").is_err());
    }

    #[test]
    fn test_rejects_link_local() {
        assert!(validate_base_url("http://169.254.169.254/v1").is_err());
        assert!(validate_base_url("http://169.254.0.1/v1").is_err());
    }

    #[test]
    fn test_rejects_ipv6_loopback() {
        assert!(validate_base_url("http://[::1]/v1").is_err());
    }

    #[test]
    fn test_rejects_ipv6_private() {
        assert!(validate_base_url("http://[fe80::1]/v1").is_err());
        assert!(validate_base_url("http://[fc00::1]/v1").is_err());
    }

    #[test]
    fn test_rejects_non_http_schemes() {
        assert!(validate_base_url("file:///etc/passwd").is_err());
        assert!(validate_base_url("ftp://example.com/v1").is_err());
        assert!(validate_base_url("gopher://example.com").is_err());
    }

    #[test]
    fn test_rejects_invalid_url() {
        assert!(validate_base_url("not-a-url").is_err());
        assert!(validate_base_url("").is_err());
    }

    #[test]
    fn test_rejects_reserved_hostnames() {
        assert!(validate_base_url("http://foo.local/v1").is_err());
        assert!(validate_base_url("http://bar.internal/v1").is_err());
    }

    #[test]
    fn test_is_private_or_reserved() {
        assert!(is_private_or_reserved_v4(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_or_reserved_v4(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_or_reserved_v4(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_or_reserved_v4(&"192.168.1.1".parse().unwrap()));
        assert!(is_private_or_reserved_v4(
            &"169.254.169.254".parse().unwrap()
        ));
        assert!(is_private_or_reserved_v6(&"::1".parse().unwrap()));
        assert!(is_private_or_reserved_v6(&"fe80::1".parse().unwrap()));

        assert!(!is_private_or_reserved_v4(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_or_reserved_v4(&"1.1.1.1".parse().unwrap()));
        assert!(is_private_or_reserved_v4(&"203.0.113.1".parse().unwrap()));
    }
}
