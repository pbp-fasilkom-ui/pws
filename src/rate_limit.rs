//! Small fixed-window rate limiter for credential endpoints.
//!
//! There was previously no limit anywhere, so `/api/login` and `/api/register`
//! could be attacked at full speed. Implemented in-process rather than pulling
//! in a dependency; it is per-instance and resets on restart, which is
//! sufficient to make online guessing impractical without pretending to be a
//! distributed quota.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::SocketAddr;

/// Requests allowed per client per window.
///
/// Deliberately generous. The key is a client address, and a university lab
/// sits behind one NAT -- at a strict setting a whole class starting at once
/// locks itself out. Credential guessing needs thousands of attempts to be
/// worth anything, so this still removes the attack while leaving normal shared
/// egress far below the ceiling.
const MAX_REQUESTS: u32 = 60;
/// Length of the window.
const WINDOW: Duration = Duration::from_secs(60);
/// Upper bound on tracked clients, so the map cannot grow without limit.
const MAX_TRACKED: usize = 10_000;

#[derive(Clone, Default)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<IpAddr, (Instant, u32)>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a hit and reports whether the caller is over its allowance.
    fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut buckets = match self.buckets.lock() {
            Ok(buckets) => buckets,
            // A poisoned lock should not lock everyone out of logging in.
            Err(poisoned) => poisoned.into_inner(),
        };

        buckets.retain(|_, (started, _)| now.duration_since(*started) < WINDOW);

        // Fail CLOSED when the table is full. Returning "allowed" here let an
        // attacker switch the limiter off -- for themselves and for everyone
        // else -- with roughly MAX_TRACKED cheap requests carrying distinct
        // keys.
        if buckets.len() >= MAX_TRACKED && !buckets.contains_key(&ip) {
            tracing::warn!("Rate limiter table is full; rejecting unseen clients");
            return false;
        }

        let entry = buckets.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= WINDOW {
            *entry = (now, 0);
        }
        entry.1 += 1;

        entry.1 <= MAX_REQUESTS
    }
}

/// Client address for rate-limiting purposes.
///
/// The service sits behind Traefik, so the peer address is the proxy and every
/// request would otherwise share a single bucket.
///
/// Takes the RIGHT-most `X-Forwarded-For` entry, not the left-most. Traefik
/// *appends* the address it observed to any header the client already sent, so
/// a request arriving with `X-Forwarded-For: 1.2.3.4` reaches this service as
/// `1.2.3.4, <real client>`. The left-most entry is therefore entirely
/// attacker-controlled: rotating it defeated the limiter outright and could
/// also flood the tracking table. The right-most entry is the one Traefik
/// itself appended, so it is the last value a client cannot forge.
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit(',').next())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .unwrap_or_else(|| peer.ip())
}

pub async fn rate_limit(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(limiter): State<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    let ip = client_ip(request.headers(), peer);

    if !limiter.check(ip) {
        tracing::warn!(%ip, "Rate limit exceeded");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests, please try again later",
        )
            .into_response();
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_blocks() {
        let limiter = RateLimiter::new();
        let ip: IpAddr = "203.0.113.5".parse().unwrap();

        for _ in 0..MAX_REQUESTS {
            assert!(limiter.check(ip));
        }
        assert!(!limiter.check(ip));
    }

    #[test]
    fn tracks_clients_separately() {
        let limiter = RateLimiter::new();
        let a: IpAddr = "203.0.113.5".parse().unwrap();
        let b: IpAddr = "203.0.113.6".parse().unwrap();

        for _ in 0..MAX_REQUESTS {
            assert!(limiter.check(a));
        }
        assert!(!limiter.check(a));
        assert!(limiter.check(b));
    }

    #[test]
    fn uses_the_proxy_appended_forwarded_entry_not_the_client_supplied_one() {
        // Traefik appends what it saw, so the last entry is trustworthy and
        // everything to its left was supplied by the caller.
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 203.0.113.9".parse().unwrap());
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();

        assert_eq!(
            client_ip(&headers, peer),
            "203.0.113.9".parse::<IpAddr>().unwrap(),
            "a spoofed left-most entry must not become the bucket key"
        );
    }

    #[test]
    fn falls_back_to_the_peer_without_a_forwarded_header() {
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        assert_eq!(
            client_ip(&HeaderMap::new(), peer),
            "10.0.0.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn a_spoofed_header_cannot_reset_the_bucket() {
        let limiter = RateLimiter::new();
        let mut headers = HeaderMap::new();
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();

        // Same real client, rotating the forgeable prefix on every request.
        for i in 0..(MAX_REQUESTS + 5) {
            headers.insert(
                "x-forwarded-for",
                format!("9.9.9.{i}, 203.0.113.9").parse().unwrap(),
            );
            let ip = client_ip(&headers, peer);
            let allowed = limiter.check(ip);
            if i >= MAX_REQUESTS {
                assert!(!allowed, "request {i} should have been limited");
            }
        }
    }

    #[test]
    fn a_full_table_fails_closed() {
        let limiter = RateLimiter::new();
        for i in 0..MAX_TRACKED {
            let ip: IpAddr = format!("10.{}.{}.{}", i / 65536, (i / 256) % 256, i % 256)
                .parse()
                .unwrap();
            limiter.check(ip);
        }
        // An unseen key must now be rejected, not waved through.
        let fresh: IpAddr = "203.0.113.200".parse().unwrap();
        assert!(!limiter.check(fresh));
    }
}
