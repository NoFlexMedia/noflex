// SPDX-License-Identifier: AGPL-3.0-only
//
// `host-net`: host-performed HTTP egress for socketless WASI guests. Every
// destination address is vetted at connect time by the shared client's DNS
// resolver (or, for IP-literal URLs, before the request is built), so DNS
// cannot move a request across the network trust boundary after
// validation. Proxies are never used: a proxy would connect on the guest's
// behalf to an address the host never vetted.

use std::{
    error::Error as StdError,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, OnceLock},
    time::Duration,
};

use futures::StreamExt as _;
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    header::{HeaderMap, HeaderName, HeaderValue},
};
use url::Host;
use wasmtime::component::{Accessor, HasSelf};

use crate::{
    PluginCtx,
    bindings::metadata_only::noflex::plugin::{host_net, types},
};

const MAX_REQUEST_BODY: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BODY: usize = 16 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
const DNS_TIMEOUT: Duration = Duration::from_secs(10);
const ALLOWED_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];
const STRIPPED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "keep-alive",
    "upgrade",
];

/// A destination the address policy refuses. Raised by the resolver inside
/// reqwest's connector, so it is recovered from the error's source chain.
#[derive(Debug)]
struct ForbiddenDestination {
    host: String,
    ip: IpAddr,
}

impl fmt::Display for ForbiddenDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "destination `{}` resolves to forbidden address {}",
            self.host, self.ip
        )
    }
}

impl StdError for ForbiddenDestination {}

/// Resolver for the shared clients: system DNS under [`DNS_TIMEOUT`], then
/// the forbidden-address policy unless private networks are allowed. A mixed
/// answer is refused outright rather than filtered, so a rebinding name
/// cannot steer the connector onto its private half.
struct GuardedResolver {
    allow_private: bool,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let lookup =
                tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((host.as_str(), 0)))
                    .await
                    .map_err(|_| format!("resolving `{host}` timed out after {DNS_TIMEOUT:?}"))??;
            let addrs = vet_addresses(&host, lookup.collect(), allow_private)?;
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Apply the address policy to one resolution. Errors when the answer is
/// empty or (without `allow_private`) contains any forbidden address.
fn vet_addresses(
    host: &str,
    addrs: Vec<SocketAddr>,
    allow_private: bool,
) -> Result<Vec<SocketAddr>, Box<dyn StdError + Send + Sync>> {
    if addrs.is_empty() {
        return Err(format!("destination `{host}` resolved to no addresses").into());
    }
    if !allow_private && let Some(addr) = addrs.iter().find(|addr| forbidden_ip(addr.ip())) {
        return Err(Box::new(ForbiddenDestination {
            host: host.to_string(),
            ip: addr.ip(),
        }));
    }
    Ok(addrs)
}

/// One pooled client per address policy, built on first use. Pooling is
/// safe across plugins: the host allowlist is checked per request before
/// any connection is chosen, and the policies never share a pool.
fn shared_client(allow_private: bool) -> Result<&'static reqwest::Client, String> {
    static PUBLIC: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    static PRIVATE: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let cell = if allow_private { &PRIVATE } else { &PUBLIC };
    cell.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(GuardedResolver { allow_private }))
            .build()
            .map_err(|e| format!("build HTTP client: {e}"))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Match `host` (a domain, or an IP literal without brackets) against the
/// manifest allowlist: exact case-insensitive entries, `*.suffix` for
/// subdomains of `suffix` (never the apex), or a bare `*`. Bracketed IPv6
/// entries are accepted for convenience.
fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    allowlist.iter().any(|entry| {
        let entry = entry.to_ascii_lowercase();
        if entry == "*" {
            return true;
        }
        if let Some(suffix) = entry.strip_prefix("*.") {
            return host.len() > suffix.len()
                && host.ends_with(&suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.';
        }
        let entry = entry
            .strip_prefix('[')
            .and_then(|e| e.strip_suffix(']'))
            .unwrap_or(&entry);
        host == entry
    })
}

fn parse_method(method: &str) -> Option<reqwest::Method> {
    let upper = method.to_ascii_uppercase();
    if !ALLOWED_METHODS.contains(&upper.as_str()) {
        return None;
    }
    reqwest::Method::from_bytes(upper.as_bytes()).ok()
}

/// Addresses a plugin without `http.private-network` may not reach:
/// loopback, private, link-local, CGNAT, benchmarking, reserved and other
/// special-purpose ranges, plus every IPv6 form that embeds such an IPv4
/// address (mapped, compatible, NAT64, 6to4).
fn forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => forbidden_v4(ip),
        IpAddr::V6(ip) => forbidden_v6(ip),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        // 0.0.0.0/8 "this network" (includes unspecified).
        || a == 0
        // 100.64.0.0/10 shared address space (CGNAT).
        || (a == 100 && (b & 0xc0) == 64)
        // 192.0.0.0/24 IETF protocol assignments.
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 benchmarking.
        || (a == 198 && (b & 0xfe) == 18)
        // 240.0.0.0/4 reserved.
        || a >= 240
}

fn forbidden_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
        // fec0::/10 deprecated site-local.
        || (seg[0] & 0xffc0) == 0xfec0
        // 64:ff9b:1::/48 local-use NAT64; the embedded IPv4 position depends
        // on the operator's prefix length, so the whole range is refused.
        || (seg[0] == 0x64 && seg[1] == 0xff9b && seg[2] == 1)
    {
        return true;
    }
    let embedded = |hi: u16, lo: u16| Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo));
    // ::ffff:a.b.c.d mapped and ::a.b.c.d compatible.
    if let Some(v4) = ip.to_ipv4() {
        return forbidden_v4(v4);
    }
    // 64:ff9b::/96 well-known NAT64.
    if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return forbidden_v4(embedded(seg[6], seg[7]));
    }
    // 2002::/16 6to4: the relay's IPv4 sits in bits 16..48.
    if seg[0] == 0x2002 {
        return forbidden_v4(embedded(seg[1], seg[2]));
    }
    false
}

/// The deny reason when an IP-literal destination is refused by policy.
/// reqwest skips the resolver for literals, so they are vetted here.
fn literal_denied(host: &Host<&str>, allow_private: bool) -> Option<String> {
    let ip = match *host {
        Host::Ipv4(ip) => IpAddr::V4(ip),
        Host::Ipv6(ip) => IpAddr::V6(ip),
        Host::Domain(_) => return None,
    };
    (!allow_private && forbidden_ip(ip)).then(|| format!("destination {ip} is a forbidden address"))
}

/// Find a policy refusal inside a reqwest error's source chain.
fn forbidden_cause(error: &reqwest::Error) -> Option<&ForbiddenDestination> {
    let mut source = error.source();
    while let Some(err) = source {
        if let Some(forbidden) = err.downcast_ref::<ForbiddenDestination>() {
            return Some(forbidden);
        }
        source = err.source();
    }
    None
}

impl host_net::Host for PluginCtx {}

impl host_net::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn request(
        accessor: &Accessor<PluginCtx, Self>,
        req: host_net::HttpRequest,
    ) -> wasmtime::Result<Result<host_net::HttpResponse, types::PluginError>> {
        use types::PluginError;

        let (plugin_id, capabilities, allowlist) = accessor.with(|mut access| {
            let ctx = access.get();
            (
                ctx.plugin_id.clone(),
                ctx.declared_capabilities.clone(),
                ctx.http_egress_hosts.clone(),
            )
        });
        let audit = |status: Option<u16>| {
            crate::audit::http_egress(&plugin_id.0, &req.method, &req.url, status);
        };
        let deny = |e: PluginError| {
            audit(Some(0));
            Ok(Err(e))
        };

        if !capabilities.iter().any(|c| c == "http.egress") {
            return deny(PluginError::CapabilityDenied(
                "host-net.request requires the `http.egress` capability".into(),
            ));
        }
        let url = match reqwest::Url::parse(&req.url) {
            Ok(url) => url,
            Err(e) => return deny(PluginError::BadInput(format!("invalid URL: {e}"))),
        };
        if !matches!(url.scheme(), "http" | "https") {
            return deny(PluginError::BadInput(format!(
                "only http/https URLs are allowed, got `{}`",
                url.scheme()
            )));
        }
        let Some(host) = url.host() else {
            return deny(PluginError::BadInput("URL has no host".into()));
        };
        // `host_str` brackets IPv6 literals; match on the bare form.
        let host_name = match host {
            Host::Domain(domain) => domain.to_string(),
            Host::Ipv4(ip) => ip.to_string(),
            Host::Ipv6(ip) => ip.to_string(),
        };
        if !host_allowed(&host_name, &allowlist) {
            return deny(PluginError::CapabilityDenied(format!(
                "host `{host_name}` is not in the manifest's http_egress_hosts"
            )));
        }
        let private_network = capabilities.iter().any(|c| c == "http.private-network");
        if let Some(reason) = literal_denied(&host, private_network) {
            return deny(PluginError::CapabilityDenied(reason));
        }
        let Some(method) = parse_method(&req.method) else {
            return deny(PluginError::BadInput(format!(
                "unsupported method `{}` (expected one of: {})",
                req.method,
                ALLOWED_METHODS.join(", ")
            )));
        };
        if req
            .body
            .as_ref()
            .is_some_and(|body| body.len() > MAX_REQUEST_BODY)
        {
            return deny(PluginError::BadInput(format!(
                "request body exceeds the {} MiB cap",
                MAX_REQUEST_BODY / (1024 * 1024)
            )));
        }

        let mut headers = HeaderMap::new();
        for (name, value) in &req.headers {
            if STRIPPED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                continue;
            }
            let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
                return deny(PluginError::BadInput(format!(
                    "invalid header name `{name}`"
                )));
            };
            let Ok(value) = HeaderValue::from_str(value) else {
                return deny(PluginError::BadInput(format!(
                    "invalid value for header `{name}`"
                )));
            };
            headers.append(name, value);
        }

        let client = match shared_client(private_network) {
            Ok(client) => client,
            Err(e) => return deny(PluginError::Internal(e)),
        };
        let timeout = req
            .timeout_ms
            .map(|ms| Duration::from_millis(u64::from(ms)).min(MAX_TIMEOUT))
            .unwrap_or(DEFAULT_TIMEOUT);
        let mut request = client
            .request(method, url)
            .headers(headers)
            .timeout(timeout);
        if let Some(body) = req.body.clone() {
            request = request.body(body);
        }

        let resp = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                if let Some(forbidden) = forbidden_cause(&e) {
                    return deny(PluginError::CapabilityDenied(forbidden.to_string()));
                }
                audit(None);
                return Ok(Err(PluginError::Unavailable(format!(
                    "request failed: {e}"
                ))));
            }
        };
        let status = resp.status().as_u16();
        let response_headers = resp
            .headers()
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string()))
            .collect();

        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    audit(Some(status));
                    return Ok(Err(PluginError::Internal(format!(
                        "reading response body: {e}"
                    ))));
                }
            };
            if body.len() + chunk.len() > MAX_RESPONSE_BODY {
                audit(Some(status));
                return Ok(Err(PluginError::Internal(format!(
                    "response body exceeds the {} MiB cap",
                    MAX_RESPONSE_BODY / (1024 * 1024)
                ))));
            }
            body.extend_from_slice(&chunk);
        }

        audit(Some(status));
        Ok(Ok(host_net::HttpResponse {
            status,
            headers: response_headers,
            body,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn wildcard_hosts_do_not_change_address_policy() {
        assert!(host_allowed("anything.example", &allowlist(&["*"])));
        assert!(forbidden_ip(ip("127.0.0.1")));
        assert!(forbidden_ip(ip("10.2.3.4")));
        assert!(forbidden_ip(ip("169.254.1.1")));
        assert!(forbidden_ip(ip("ff02::1")));
        assert!(forbidden_ip(ip("::")));
        assert!(!forbidden_ip(ip("8.8.8.8")));
        assert!(!forbidden_ip(ip("2606:4700::1111")));
    }

    #[test]
    fn special_purpose_ipv4_ranges_are_forbidden() {
        for addr in [
            "100.64.0.1",
            "100.127.255.254",
            "0.1.2.3",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.255",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(forbidden_ip(ip(addr)), "{addr} should be forbidden");
        }
        for addr in ["100.63.255.255", "100.128.0.0", "192.0.1.1", "198.20.0.1"] {
            assert!(!forbidden_ip(ip(addr)), "{addr} should be allowed");
        }
    }

    #[test]
    fn ipv6_forms_embedding_private_ipv4_are_forbidden() {
        // NAT64 well-known prefix: embedded v4 decides.
        assert!(forbidden_ip(ip("64:ff9b::7f00:1")));
        assert!(forbidden_ip(ip("64:ff9b::a00:1")));
        assert!(!forbidden_ip(ip("64:ff9b::808:808")));
        // Local-use NAT64 is refused wholesale.
        assert!(forbidden_ip(ip("64:ff9b:1::808:808")));
        // 6to4: the relay address in bits 16..48 decides.
        assert!(forbidden_ip(ip("2002:c0a8:101::1")));
        assert!(forbidden_ip(ip("2002:7f00:1::1")));
        assert!(!forbidden_ip(ip("2002:808:808::1")));
        // IPv4-mapped and IPv4-compatible.
        assert!(forbidden_ip(ip("::ffff:127.0.0.1")));
        assert!(forbidden_ip(ip("::ffff:100.64.0.1")));
        assert!(forbidden_ip(ip("::10.0.0.1")));
        assert!(forbidden_ip(ip("::127.0.0.1")));
        assert!(!forbidden_ip(ip("::ffff:8.8.8.8")));
        // Site-local.
        assert!(forbidden_ip(ip("fec0::1")));
        assert!(forbidden_ip(ip("feff::1")));
    }

    #[test]
    fn mixed_public_private_answers_are_refused() {
        let addrs = vec![
            "8.8.8.8:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap(),
        ];
        let err = vet_addresses("rebind.example", addrs, false).unwrap_err();
        assert!(err.downcast_ref::<ForbiddenDestination>().is_some());
    }

    #[test]
    fn private_network_capability_is_separate() {
        let addrs: Vec<SocketAddr> = vec!["192.168.1.10:80".parse().unwrap()];
        assert!(vet_addresses("nas.example", addrs.clone(), false).is_err());
        assert_eq!(
            vet_addresses("nas.example", addrs.clone(), true).unwrap(),
            addrs
        );
        assert!(vet_addresses("empty.example", Vec::new(), true).is_err());
    }

    #[test]
    fn ip_literal_urls_match_the_allowlist_without_brackets() {
        let url = reqwest::Url::parse("http://[::1]:8080/hook").unwrap();
        let host = url.host().unwrap();
        assert!(matches!(host, Host::Ipv6(_)));
        assert!(host_allowed("::1", &allowlist(&["::1"])));
        assert!(host_allowed("::1", &allowlist(&["[::1]"])));
        assert!(literal_denied(&host, false).is_some());
        assert!(literal_denied(&host, true).is_none());

        let url = reqwest::Url::parse("http://8.8.8.8/").unwrap();
        assert!(literal_denied(&url.host().unwrap(), false).is_none());
        let url = reqwest::Url::parse("http://hooks.example/").unwrap();
        assert!(literal_denied(&url.host().unwrap(), false).is_none());
    }

    #[tokio::test]
    async fn resolver_refusal_is_recoverable_from_reqwest_errors() {
        let client = shared_client(false).expect("client builds");
        let err = client
            .get("http://localhost:9/")
            .send()
            .await
            .expect_err("loopback is forbidden without http.private-network");
        let forbidden = forbidden_cause(&err).expect("policy refusal in the source chain");
        assert!(forbidden.ip.is_loopback());
    }

    #[test]
    fn wildcard_matches_subdomains_but_not_apex() {
        let list = allowlist(&["*.example.com"]);
        assert!(host_allowed("hooks.example.com", &list));
        assert!(!host_allowed("example.com", &list));
        assert!(!host_allowed("example.com.evil.net", &list));
    }

    #[test]
    fn method_gate() {
        for method in ALLOWED_METHODS {
            assert!(parse_method(method).is_some());
            assert!(parse_method(&method.to_ascii_lowercase()).is_some());
        }
        assert!(parse_method("TRACE").is_none());
    }
}
