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

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::SocketAddr;

/// Requests allowed per client per window.
const MAX_REQUESTS: u32 = 10;
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

        if buckets.len() >= MAX_TRACKED && !buckets.contains_key(&ip) {
            return true;
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
/// In production the service sits behind Traefik, so the peer address is the
/// proxy and every request would share one bucket. Prefer the left-most
/// `X-Forwarded-For` entry, which the proxy sets; this assumes the service is
/// not reachable directly, which is also what the deployment intends.
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .unwrap_or_else(|| peer.ip())
}

pub async fn rate_limit<B>(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(limiter): State<RateLimiter>,
    request: Request<B>,
    next: Next<B>,
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
    fn prefers_forwarded_header_over_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.7, 10.0.0.1".parse().unwrap());
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();

        assert_eq!(
            client_ip(&headers, peer),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_ip(&HeaderMap::new(), peer),
            "10.0.0.1".parse::<IpAddr>().unwrap()
        );
    }
}
