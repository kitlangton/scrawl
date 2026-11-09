//! URL parsing shared by local dev-page registration and legacy bundle URLs.

/// The parts of an `http(s)` URL routing needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebUrl {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl WebUrl {
    /// `scheme://host[:port]`, the port left out when it's the scheme's default (as browsers send
    /// `Origin`).
    pub fn origin(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == default {
            format!("{}://{host}", self.scheme)
        } else {
            format!("{}://{host}:{}", self.scheme, self.port)
        }
    }

    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }
}

/// Parses `http://` and `https://` URLs (and bare origins). Hosts are lowercased; IPv6 hosts
/// lose their brackets.
pub fn parse_url(url: &str) -> Option<WebUrl> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        (host.to_owned(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port.parse().ok()?)),
            None => (authority.to_owned(), None),
        }
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
    {
        return None;
    }
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some(WebUrl {
        scheme,
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// `localhost`, `*.localhost`, `127.0.0.0/8`, `::1`, and `0.0.0.0` (which browsers send to this
/// machine).
pub fn is_loopback_host(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host == "::1" || host == "0.0.0.0" {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_parse_to_origins() {
        let u = parse_url("http://localhost:5173/a/b?c#d").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("localhost", 5173));
        assert_eq!(u.origin(), "http://localhost:5173");
        assert!(u.is_loopback());
        assert_eq!(
            parse_url("HTTPS://User:pw@App.Example.com/x")
                .unwrap()
                .origin(),
            "https://app.example.com"
        );
        assert_eq!(parse_url("http://example.com").unwrap().port, 80);
        let v6 = parse_url("http://[::1]:3000/").unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("::1", 3000));
        assert_eq!(v6.origin(), "http://[::1]:3000");
        assert!(v6.is_loopback());
        assert!(parse_url("http://127.0.0.1:8080").unwrap().is_loopback());
        assert!(
            parse_url("http://my-app.localhost:1355/")
                .unwrap()
                .is_loopback()
        );
        assert!(
            !parse_url("http://studio.example.ts.net:5173/")
                .unwrap()
                .is_loopback()
        );
        assert!(
            !parse_url("http://localhost.evil.com/")
                .unwrap()
                .is_loopback()
        );
        assert!(parse_url("file:///tmp/x.html").is_none());
        assert!(parse_url("chrome://newtab").is_none());
        assert!(parse_url("http://localhost:notaport/").is_none());
        assert!(parse_url("http:///nohost").is_none());
    }

}
