use std::{
    env,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use sqlx::postgres::PgConnectOptions;
use tracing_subscriber::EnvFilter;

// Deliberately no Debug implementation: connection options contain credentials.
pub struct Config {
    pub database: PgConnectOptions,
    pub address: SocketAddr,
    pub internal_address: SocketAddr,
    pub log_filter: EnvFilter,
}

impl Config {
    pub fn from_env() -> Result<Self, &'static str> {
        Self::parse(|key| match env::var(key) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err("configuration must be UTF-8"),
        })
    }

    fn parse(
        mut get: impl FnMut(&str) -> Result<Option<String>, &'static str>,
    ) -> Result<Self, &'static str> {
        let url = get("DATABASE_URL")?.ok_or("DATABASE_URL is required")?;
        if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
            return Err("DATABASE_URL must be a PostgreSQL URL");
        }
        let database = PgConnectOptions::from_str(&url).map_err(|_| "invalid DATABASE_URL")?;
        let host: IpAddr = get("HTTP_HOST")?
            .unwrap_or_else(|| "127.0.0.1".into())
            .parse()
            .map_err(|_| "HTTP_HOST must be an IP address")?;
        let port: u16 = get("HTTP_PORT")?
            .unwrap_or_else(|| "3000".into())
            .parse()
            .map_err(|_| "HTTP_PORT must be between 1 and 65535")?;
        if port == 0 {
            return Err("HTTP_PORT must be between 1 and 65535");
        }
        let internal_host: IpAddr = get("SESSIONS_INTERNAL_HOST")?
            .unwrap_or_else(|| "127.0.0.1".into())
            .parse()
            .map_err(|_| "invalid internal host")?;
        let internal_port: u16 = get("SESSIONS_INTERNAL_PORT")?
            .unwrap_or_else(|| "3001".into())
            .parse()
            .map_err(|_| "invalid internal port")?;
        if internal_port == 0 {
            return Err("invalid internal port");
        }
        let log_filter = EnvFilter::try_new(get("RUST_LOG")?.unwrap_or_else(|| "info".into()))
            .map_err(|_| "invalid RUST_LOG")?;
        Ok(Self {
            database,
            address: SocketAddr::new(host, port),
            internal_address: SocketAddr::new(internal_host, internal_port),
            log_filter,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(values: &[(&str, &str)]) -> Result<Config, &'static str> {
        Config::parse(|key| {
            Ok(values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string()))
        })
    }
    #[test]
    fn defaults_and_required_database() {
        assert!(config(&[]).is_err());
        let c = config(&[("DATABASE_URL", "postgres://localhost/sessions")]).unwrap();
        assert_eq!(c.address.to_string(), "127.0.0.1:3000");
        assert_eq!(c.internal_address.to_string(), "127.0.0.1:3001");
        assert_eq!(c.log_filter.to_string(), "info");
    }
    #[test]
    fn explicit_configuration_overrides_defaults() {
        let c = config(&[
            ("DATABASE_URL", "postgres://localhost/sessions"),
            ("HTTP_HOST", "0.0.0.0"),
            ("HTTP_PORT", "4000"),
            ("RUST_LOG", "warn"),
        ])
        .unwrap();
        assert_eq!(c.address.to_string(), "0.0.0.0:4000");
        assert_eq!(c.log_filter.to_string(), "warn");
    }

    #[test]
    fn invalid_configuration_is_rejected_without_secrets() {
        for (key, value) in [
            ("DATABASE_URL", "secret"),
            ("HTTP_HOST", "invalid"),
            ("HTTP_PORT", "0"),
            ("HTTP_PORT", "65536"),
            ("SESSIONS_INTERNAL_HOST", "not-an-ip"),
            ("SESSIONS_INTERNAL_PORT", "0"),
            ("RUST_LOG", "info[invalid"),
        ] {
            let result = config(&[
                (key, value),
                ("DATABASE_URL", "postgres://localhost/sessions"),
            ]);
            assert!(result.is_err(), "accepted invalid {key}");
        }
    }
}
