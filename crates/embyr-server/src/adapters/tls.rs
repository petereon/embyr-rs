//! firestore-tls-support: shared TLS-accept helper for the 2 axum-based
//! listeners (REST/gRPC-Web `:8081`, Admin `:9090`). Writing this
//! handshake step twice would duplicate a security-sensitive boundary —
//! extracted once here, used by both.

use tokio::io::{AsyncRead, AsyncWrite};

/// Marker trait unifying a plain `TcpStream` and a `TlsStream<TcpStream>`
/// behind one boxable type, so the accept loop can hold either without a
/// hand-rolled enum. Blanket-implemented for anything already satisfying
/// the bounds hyper's connection builder requires.
pub trait TlsOrPlainStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> TlsOrPlainStream for T {}

/// Complete the TLS handshake on `stream` if `tls_acceptor` is `Some`;
/// otherwise return it unwrapped. The ONE place either listener performs a
/// TLS handshake — shared so this security-sensitive step exists exactly
/// once.
pub async fn accept_maybe_tls(
    stream: tokio::net::TcpStream,
    tls_acceptor: Option<&tokio_rustls::TlsAcceptor>,
) -> std::io::Result<Box<dyn TlsOrPlainStream>> {
    match tls_acceptor {
        Some(acceptor) => Ok(Box::new(acceptor.accept(stream).await?)),
        None => Ok(Box::new(stream)),
    }
}
