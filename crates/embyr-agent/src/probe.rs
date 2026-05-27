//! StartupProbe — validates Postgres connectivity and TLS cert before port binding.
//!
//! SECURITY: db_dsn is NEVER passed to any tracing macro — not in info!, not in
//! error!, not in format strings within tracing calls. sqlx error messages do not
//! include the DSN by default, but we map all errors to generic messages to be safe.

use std::time::Duration;

use tracing::info;

/// Hard-gated startup checks that must pass before the gRPC listener is bound.
pub struct StartupProbe<'a> {
    /// NEVER log this value — it contains the database password.
    db_dsn: &'a str,
    cert_path: &'a str,
}

impl<'a> StartupProbe<'a> {
    /// Construct the probe with the DSN and TLS cert path.
    pub fn new(db_dsn: &'a str, cert_path: &'a str) -> Self {
        Self { db_dsn, cert_path }
    }

    /// Run all probe gates in sequence.
    ///
    /// Returns `Ok(())` only if all gates pass. Any failure returns a
    /// human-readable `Err` string that is safe to log (no DSN).
    pub async fn run(&self) -> Result<(), String> {
        self.probe_postgres().await?;
        self.probe_cert()?;
        Ok(())
    }

    /// Gate 1: SELECT 1 must succeed within 5s.
    /// Gate 2: LISTEN/NOTIFY round-trip must complete within 5s.
    ///
    /// SECURITY: All error strings are generic — db_dsn is never interpolated.
    async fn probe_postgres(&self) -> Result<(), String> {
        // Gate 1a: connect and run SELECT 1.
        let pool = tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::PgPool::connect(self.db_dsn),
        )
        .await
        .map_err(|_| "startup probe: Postgres connection timed out after 5s".to_string())?
        .map_err(|_| "startup probe: Postgres connection failed".to_string())?;

        tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::query("SELECT 1").execute(&pool),
        )
        .await
        .map_err(|_| "startup probe: SELECT 1 timed out".to_string())?
        .map_err(|_| "startup probe: SELECT 1 failed".to_string())?;

        // Gate 1b: LISTEN/NOTIFY round-trip.
        let mut listener = tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::postgres::PgListener::connect(self.db_dsn),
        )
        .await
        .map_err(|_| "startup probe: PgListener connect timed out".to_string())?
        .map_err(|_| "startup probe: PgListener connect failed".to_string())?;

        listener
            .listen("embyr_probe")
            .await
            .map_err(|_| "startup probe: LISTEN failed".to_string())?;

        sqlx::query("SELECT pg_notify('embyr_probe', 'ok')")
            .execute(&pool)
            .await
            .map_err(|_| "startup probe: NOTIFY failed".to_string())?;

        tokio::time::timeout(Duration::from_secs(5), listener.recv())
            .await
            .map_err(|_| "startup probe: NOTIFY not received within 5s".to_string())?
            .map_err(|_| "startup probe: listener recv failed".to_string())?;

        info!("connected to Postgres");
        Ok(())
    }

    /// Gate 3: TLS cert must parse and have ≥24 hours until expiry.
    fn probe_cert(&self) -> Result<(), String> {
        let pem = std::fs::read_to_string(self.cert_path)
            .map_err(|_| format!("startup probe: cannot read cert {}", self.cert_path))?;

        // Parse PEM to DER using rustls-pemfile.
        let mut reader = std::io::BufReader::new(pem.as_bytes());
        let certs: Vec<rustls_pemfile::Item> = rustls_pemfile::read_all(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "startup probe: cert PEM parse failed".to_string())?;

        // Extract the first X.509 certificate DER block.
        let cert_der = certs
            .iter()
            .find_map(|item| {
                if let rustls_pemfile::Item::X509Certificate(der) = item {
                    Some(der.as_ref())
                } else {
                    None
                }
            })
            .ok_or_else(|| "startup probe: cert file contains no X.509 certificate".to_string())?;

        // Parse the DER with x509-parser.
        let (_, cert) = x509_parser::parse_x509_certificate(cert_der)
            .map_err(|_| "startup probe: x509 cert parse failed".to_string())?;

        let not_after_ts = cert.validity().not_after.timestamp();
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let remaining_secs = not_after_ts - now_ts;
        let hours_remaining = remaining_secs / 3600;

        if hours_remaining < 24 {
            return Err(format!(
                "startup probe: cert expires too soon ({hours_remaining}h remaining, need >=24h)"
            ));
        }

        Ok(())
    }
}
