//! Bounded, single-instance public ingress limits. Caddy overwrites the trusted
//! client IP header; public clients cannot reach the backend listener directly.
use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Request, State},
    http::{Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_CLIENTS: usize = 4096;

struct Bucket {
    tokens: f64,
    capacity: f64,
    per_second: f64,
    updated: Instant,
}
impl Bucket {
    fn new(capacity: f64, per_minute: f64, now: Instant) -> Self {
        Self {
            tokens: capacity,
            capacity,
            per_second: per_minute / 60.0,
            updated: now,
        }
    }
    fn take(&mut self, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now.duration_since(self.updated).as_secs_f64() * self.per_second)
            .min(self.capacity);
        self.updated = now;
        if self.tokens < 1.0 {
            false
        } else {
            self.tokens -= 1.0;
            true
        }
    }
}

struct Client {
    reads: Bucket,
    writes: Bucket,
    bootstrap: Bucket,
    rooms: Bucket,
    seen: Instant,
}
impl Client {
    fn new(now: Instant) -> Self {
        Self {
            reads: Bucket::new(100.0, 600.0, now),
            writes: Bucket::new(30.0, 120.0, now),
            bootstrap: Bucket::new(15.0, 60.0, now),
            rooms: Bucket::new(3.0, 10.0 / 60.0, now),
            seen: now,
        }
    }
}
struct Limits {
    clients: HashMap<IpAddr, Client>,
    total: Bucket,
    bootstrap: Bucket,
    rooms: Bucket,
}
impl Limits {
    fn new(now: Instant) -> Self {
        Self {
            clients: HashMap::new(),
            total: Bucket::new(300.0, 3000.0, now),
            bootstrap: Bucket::new(100.0, 300.0, now),
            rooms: Bucket::new(10.0, 50.0 / 60.0, now),
        }
    }
    fn allow(&mut self, ip: IpAddr, method: &Method, path: &str, now: Instant) -> bool {
        if !self.clients.contains_key(&ip) && self.clients.len() >= MAX_CLIENTS {
            self.clients
                .retain(|_, client| now.duration_since(client.seen) < Duration::from_secs(3600));
            if self.clients.len() >= MAX_CLIENTS {
                return false;
            }
        }
        let client = self.clients.entry(ip).or_insert_with(|| Client::new(now));
        client.seen = now;
        if method == Method::POST
            && path == "/rooms"
            && (!client.rooms.take(now) || !self.rooms.take(now))
        {
            return false;
        }
        if path == "/bootstrap" && (!client.bootstrap.take(now) || !self.bootstrap.take(now)) {
            return false;
        }
        let bucket = if method == Method::GET {
            &mut client.reads
        } else {
            &mut client.writes
        };
        bucket.take(now) && self.total.take(now)
    }
}
struct Guard {
    limits: Mutex<Limits>,
    trust_header: bool,
}

pub fn protect(app: Router, security: &crate::security::Security) -> Router {
    if !security.public_rate_limits {
        return app;
    }
    let guard = Arc::new(Guard {
        limits: Mutex::new(Limits::new(Instant::now())),
        trust_header: security.trust_client_ip_header,
    });
    app.layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(guard, check))
}

fn client_ip(request: &Request, trust_header: bool) -> Option<IpAddr> {
    if trust_header {
        request
            .headers()
            .get("x-sanscue-client-ip")?
            .to_str()
            .ok()?
            .parse()
            .ok()
    } else {
        Some(
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|peer| peer.0.ip())
                .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        )
    }
}

async fn check(State(guard): State<Arc<Guard>>, request: Request, next: Next) -> Response {
    // Container probes use real loopback connections without proxy headers.
    if matches!(request.uri().path(), "/healthz" | "/readyz")
        && request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|peer| peer.0.ip().is_loopback())
    {
        return next.run(request).await;
    }
    let Some(ip) = client_ip(&request, guard.trust_header) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code":"invalid_client_address"})),
        )
            .into_response();
    };
    let allowed = guard
        .limits
        .lock()
        .map(|mut limits| limits.allow(ip, request.method(), request.uri().path(), Instant::now()))
        .unwrap_or(false);
    if !allowed {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                (header::RETRY_AFTER, "60"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            Json(json!({"code":"rate_limited","message":"Try again later"})),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn room_floods_are_limited_across_browser_cookies_and_refill() {
        let now = Instant::now();
        let mut limits = Limits::new(now);
        let ip = "1.2.3.4".parse().unwrap();
        for _ in 0..3 {
            assert!(limits.allow(ip, &Method::POST, "/rooms", now));
        }
        assert!(!limits.allow(ip, &Method::POST, "/rooms", now));
        assert!(limits.allow(ip, &Method::GET, "/rooms/a/state", now));
        assert!(limits.allow(ip, &Method::POST, "/rooms", now + Duration::from_secs(361)));
    }
    #[test]
    fn distributed_room_floods_have_a_global_bound() {
        let now = Instant::now();
        let mut limits = Limits::new(now);
        for n in 1..=10 {
            assert!(limits.allow(
                IpAddr::V4(Ipv4Addr::new(1, 2, 3, n)),
                &Method::POST,
                "/rooms",
                now
            ));
        }
        assert!(!limits.allow("9.8.7.6".parse().unwrap(), &Method::POST, "/rooms", now));
    }
    #[test]
    fn client_tracking_has_a_fixed_memory_bound() {
        let now = Instant::now();
        let mut limits = Limits::new(now);
        for n in 0..MAX_CLIENTS {
            limits
                .clients
                .insert(IpAddr::V4(Ipv4Addr::from(n as u32)), Client::new(now));
        }
        assert!(!limits.allow("9.8.7.6".parse().unwrap(), &Method::GET, "/bootstrap", now));
        assert_eq!(limits.clients.len(), MAX_CLIENTS);
        assert!(limits.allow(
            "9.8.7.6".parse().unwrap(),
            &Method::GET,
            "/bootstrap",
            now + Duration::from_secs(3601)
        ));
    }
    #[test]
    fn proxy_headers_are_trusted_only_when_explicitly_configured() {
        let mut request = Request::builder()
            .header("x-sanscue-client-ip", "9.8.7.6")
            .body(axum::body::Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo("1.2.3.4:1234".parse::<SocketAddr>().unwrap()));
        assert_eq!(client_ip(&request, false), Some("1.2.3.4".parse().unwrap()));
        assert_eq!(client_ip(&request, true), Some("9.8.7.6".parse().unwrap()));
        request.headers_mut().remove("x-sanscue-client-ip");
        assert_eq!(client_ip(&request, true), None);
    }
}
