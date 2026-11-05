use std::net::IpAddr;

use axum::http::HeaderMap;

/// Resolves the real client address.
///
/// With `trusted_hops == 0` only the socket peer is believed, because `X-Forwarded-For` is
/// trivially forged. With N trusted proxies in front, each appends the address it received
/// the connection from, so the genuine client is the Nth entry from the right; anything an
/// attacker put further left is ignored. If the chain is shorter than expected the proxy's
/// own address is used, which merges users into one bucket rather than trusting bad input.
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted_hops: usize) -> Option<IpAddr> {
    if trusted_hops == 0 {
        return peer;
    }
    let chain: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    if chain.len() < trusted_hops {
        return peer;
    }
    chain[chain.len() - trusted_hops].parse().ok().or(peer)
}

/// Key for rate limiting. IPv6 users routinely control a whole /64, so limiting individual
/// addresses would be meaningless; group by the /64 prefix instead.
pub fn limit_bucket(ip: Option<IpAddr>) -> String {
    match ip {
        None => "unknown".to_owned(),
        Some(IpAddr::V4(v4)) => v4.to_string(),
        Some(IpAddr::V6(v6)) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(xff: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in xff {
            map.append("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn no_trusted_proxies_ignores_the_header() {
        let h = headers(&["6.6.6.6"]);
        assert_eq!(client_ip(&h, Some(ip("10.0.0.1")), 0), Some(ip("10.0.0.1")));
    }

    #[test]
    fn one_proxy_uses_the_rightmost_entry_not_the_forged_left_ones() {
        let h = headers(&["6.6.6.6, 1.2.3.4"]);
        assert_eq!(client_ip(&h, Some(ip("10.0.0.1")), 1), Some(ip("1.2.3.4")));
    }

    #[test]
    fn two_proxies_use_the_second_from_the_right() {
        let h = headers(&["6.6.6.6, 1.2.3.4, 10.0.0.2"]);
        assert_eq!(client_ip(&h, Some(ip("10.0.0.1")), 2), Some(ip("1.2.3.4")));
    }

    #[test]
    fn multiple_header_lines_are_treated_as_one_chain() {
        let h = headers(&["6.6.6.6", "1.2.3.4"]);
        assert_eq!(client_ip(&h, Some(ip("10.0.0.1")), 1), Some(ip("1.2.3.4")));
    }

    #[test]
    fn short_or_garbage_chain_falls_back_to_the_peer() {
        let peer = Some(ip("10.0.0.1"));
        assert_eq!(client_ip(&headers(&["1.2.3.4"]), peer, 2), peer);
        assert_eq!(client_ip(&headers(&["not-an-ip"]), peer, 1), peer);
        assert_eq!(client_ip(&HeaderMap::new(), peer, 1), peer);
    }

    #[test]
    fn ipv6_is_bucketed_by_64_prefix() {
        let a = limit_bucket(Some(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")));
        let b = limit_bucket(Some(ip("2001:db8:1:2:1111:2222:3333:4444")));
        let c = limit_bucket(Some(ip("2001:db8:1:3::1")));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(limit_bucket(Some(ip("1.2.3.4"))), "1.2.3.4");
        assert_eq!(limit_bucket(None), "unknown");
    }
}
