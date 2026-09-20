//! OTA transport: DNS, TCP, TLS and the HTTP/1.0 request/response plumbing.
//!
//! Every request opens a fresh connection and the server is told
//! `Connection: close`; there is no connection reuse. [`Opened`] is either a
//! plain [`TcpSocket`] or a [`TlsConnection`] over one, so the request sequence
//! is written once.
//!
//! ## Entropy source (RP2040 has no TRNG)
//!
//! The design note assumed `embassy_rp::trng::Trng`, but on this toolchain
//! (`embassy-rp` at the pinned rev, `rp2040` feature) the `trng` module is
//! gated behind `_rp235x` and does not exist for the RP2040. The RP2040's
//! documented hardware entropy source is the ring-oscillator `RANDOMBIT`
//! register (the same source the Pico SDK's `pico_rand` conditions on).
//! `embassy_rp::clocks::RoscRng` samples that register and implements the
//! `rand_core` 0.6 `RngCore`/`CryptoRng` bounds `embedded-tls` requires. It is
//! not a deterministic PRNG, and it is not a cryptographic DRBG.

use core::fmt::Write as _;
use core::net::Ipv4Addr;

use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpAddress, IpEndpoint, Stack};
use embassy_time::{with_timeout, Duration};
use embedded_io_async::{Read, Write};
use embedded_tls::{
    Aes128GcmSha256, CryptoProvider, CryptoRngCore, TlsConfig, TlsConnection, TlsContext,
};
use garagelight_core::ota::{
    basic_authorization, BodyReader, HeadError, HeadEvent, HeadParser, ResponseHead, Uri,
};

use crate::{logln, secrets};

use super::{FetchError, OtaError, TextBuf};

/// DNS lookup deadline.
const DNS_TIMEOUT: Duration = Duration::from_secs(10);
/// TCP connect deadline.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// TLS 1.3 handshake deadline.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// One read/write deadline; a healthy peer answers well inside this.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Bytes read from the transport while feeding the head parser. Bounds the
/// body that can be inadvertently over-read into the leftover buffer.
pub(super) const READ_CHUNK: usize = 256;
/// TLS record buffer size (read and write halves). A TLS 1.3 record can be up
/// to 2^14 (16384) plaintext plus up to 256 bytes of AEAD/content-type
/// overhead, so the buffer must be at least 16384 + 256 = 16640; a 16384-byte
/// buffer fails on a full-size record. 17 KiB leaves margin.
pub(super) const RECORD_BYTES: usize = 17 * 1024;
/// TCP receive/transmit window. The window bounds the number of round trips and
/// therefore the sustained TLS read throughput; 2 KiB stalls a large download.
pub(super) const TCP_BYTES: usize = 8192;

const AUTH_MAX_BYTES: usize = 192;
const REQUEST_MAX_BYTES: usize = 512;

/// Crypto provider that skips certificate verification and client-cert signing.
///
/// `embedded_tls::UnsecureProvider` behaves identically (it never overrides
/// `verifier`/`signer`, and the handshake treats their `Err` as "skip"), but it
/// hard-wires `Signature = p256::ecdsa::DerSignature`, which links the whole
/// ECDSA/P-256 stack even though it is never executed. Replacing the signature
/// type with a trivial one keeps the same no-verify behaviour without the dead
/// crypto, which matters for the debug-profile FLASH budget.
struct NoVerifyProvider<RNG> {
    rng: RNG,
}

impl<RNG: CryptoRngCore> CryptoProvider for NoVerifyProvider<RNG> {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 64];

    fn rng(&mut self) -> impl CryptoRngCore {
        &mut self.rng
    }
}

/// Resolve `uri.host` to an endpoint. IPv4 literals bypass DNS.
pub(super) async fn resolve(stack: Stack<'static>, uri: &Uri<'_>) -> Result<IpEndpoint, OtaError> {
    if let Ok(ip) = uri.host.parse::<Ipv4Addr>() {
        return Ok(IpEndpoint::new(IpAddress::Ipv4(ip), uri.port));
    }
    match with_timeout(DNS_TIMEOUT, stack.dns_query(uri.host, DnsQueryType::A)).await {
        Ok(Ok(addrs)) => {
            if let Some(IpAddress::Ipv4(v4)) = addrs.iter().next() {
                return Ok(IpEndpoint::new(IpAddress::Ipv4(*v4), uri.port));
            }
            logln!("ota_dns_no_address");
            Err(OtaError::Dns)
        }
        Ok(Err(_)) => {
            logln!("ota_dns_failed");
            Err(OtaError::Dns)
        }
        Err(_) => {
            logln!("ota_dns_timeout");
            Err(OtaError::Timeout)
        }
    }
}

/// Open one TCP (and, for `https`, TLS) connection.
pub(super) async fn open<'a>(
    stack: Stack<'static>,
    endpoint: IpEndpoint,
    uri: &Uri<'_>,
    tcp_rx: &'a mut [u8],
    tcp_tx: &'a mut [u8],
    rec_read: &'a mut [u8],
    rec_write: &'a mut [u8],
) -> Result<Opened<'a>, OtaError> {
    let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
    match with_timeout(CONNECT_TIMEOUT, socket.connect(endpoint)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            logln!("ota_connect_failed err={:?}", error);
            return Err(OtaError::Connect);
        }
        Err(_) => {
            logln!("ota_connect_timeout");
            return Err(OtaError::Timeout);
        }
    }

    if !uri.tls {
        return Ok(Opened::Plain(socket));
    }

    let mut connection = TlsConnection::new(socket, rec_read, rec_write);
    // `TlsConfig::new` already adds the RSA schemes when the `alloc` feature is
    // compiled in, but state it explicitly: the real endpoint presents a
    // Let's Encrypt RSA certificate, and RSA-PSS must be advertised or the
    // handshake fails before any verification runs.
    let config = TlsConfig::new()
        .with_server_name(uri.host)
        .enable_rsa_signatures();
    let provider = NoVerifyProvider {
        rng: embassy_rp::clocks::RoscRng,
    };
    match with_timeout(
        HANDSHAKE_TIMEOUT,
        connection.open(TlsContext::new(&config, provider)),
    )
    .await
    {
        Ok(Ok(())) => Ok(Opened::Tls(connection)),
        Ok(Err(error)) => {
            let _ = error;
            logln!("ota_tls_failed");
            Err(OtaError::Tls)
        }
        Err(_) => {
            logln!("ota_tls_timeout");
            Err(OtaError::Timeout)
        }
    }
}

/// An open connection: plain TCP or TLS over it. Only one variant is ever live
/// and there is no allocator to box the larger TLS state with.
#[allow(clippy::large_enum_variant)]
pub(super) enum Opened<'a> {
    Plain(TcpSocket<'a>),
    Tls(TlsConnection<'a, TcpSocket<'a>, Aes128GcmSha256>),
}

impl Opened<'_> {
    /// Send one request and feed bytes to the head parser until it terminates.
    /// On success the returned length is the body bytes already read into
    /// `leftover`.
    pub(super) async fn send(
        &mut self,
        request: &[u8],
        leftover: &mut [u8],
    ) -> Result<(ResponseHead, usize), FetchError> {
        match self {
            Opened::Plain(transport) => get(transport, request, leftover).await,
            Opened::Tls(transport) => get(transport, request, leftover).await,
        }
    }

    /// Read the next slice of the body.
    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize, FetchError> {
        match self {
            Opened::Plain(transport) => read_timed(transport, buf).await,
            Opened::Tls(transport) => read_timed(transport, buf).await,
        }
    }

    /// Read exactly `len` bytes of a bounded text body, draining the over-read
    /// leftover first. Stops early on EOF.
    pub(super) async fn read_small(
        &mut self,
        leftover: &[u8],
        len: u64,
        out: &mut [u8],
    ) -> Result<usize, FetchError> {
        if len as usize > out.len() {
            return Err(FetchError::BodyTooLarge);
        }
        let len = len as usize;
        let take = core::cmp::min(leftover.len(), len);
        out[..take].copy_from_slice(&leftover[..take]);
        let mut n = take;
        while n < len {
            let read = self.read_some(&mut out[n..len]).await?;
            if read == 0 {
                break;
            }
            n += read;
        }
        Ok(n)
    }
}

/// Write the request, flush, then feed every read byte to the head parser.
async fn get<T: Read + Write>(
    transport: &mut T,
    request: &[u8],
    leftover: &mut [u8],
) -> Result<(ResponseHead, usize), FetchError> {
    with_timeout(IO_TIMEOUT, transport.write_all(request))
        .await
        .map_err(|_| FetchError::Timeout)?
        .map_err(|_| FetchError::Write)?;
    with_timeout(IO_TIMEOUT, transport.flush())
        .await
        .map_err(|_| FetchError::Timeout)?
        .map_err(|_| FetchError::Flush)?;

    let mut parser = HeadParser::new();
    let mut buf = [0u8; READ_CHUNK];
    loop {
        let n = read_timed(transport, &mut buf).await?;
        if n == 0 {
            return Err(FetchError::Closed);
        }
        for (i, &byte) in buf[..n].iter().enumerate() {
            match parser.push(byte) {
                HeadEvent::NeedMore => {}
                HeadEvent::Complete(head) => {
                    let extra = n - i - 1;
                    if extra > leftover.len() {
                        return Err(FetchError::Leftover);
                    }
                    leftover[..extra].copy_from_slice(&buf[i + 1..n]);
                    return Ok((head, extra));
                }
                HeadEvent::Reject(error) => return Err(FetchError::Head(head_code(error))),
            }
        }
    }
}

async fn read_timed<T: Read>(transport: &mut T, buf: &mut [u8]) -> Result<usize, FetchError> {
    with_timeout(IO_TIMEOUT, transport.read(buf))
        .await
        .map_err(|_| FetchError::Timeout)?
        .map_err(|_| FetchError::Read)
}

/// Streaming body source for [`apply_update`](garagelight_core::ota::apply_update):
/// the already-read leftover first, then the connection.
pub(super) struct HttpBody<'a, 'b> {
    pub(super) link: &'a mut Opened<'b>,
    pub(super) leftover: &'a [u8],
}

impl BodyReader for HttpBody<'_, '_> {
    type Error = FetchError;

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if !self.leftover.is_empty() {
            let n = core::cmp::min(buf.len(), self.leftover.len());
            buf[..n].copy_from_slice(&self.leftover[..n]);
            self.leftover = &self.leftover[n..];
            return Ok(n);
        }
        self.link.read_some(buf).await
    }
}

/// Build `GET {path}/{name} HTTP/1.0` with `Host`, optional Basic auth and
/// `Connection: close`.
pub(super) fn build_request(
    uri: &Uri<'_>,
    name: &str,
) -> Result<TextBuf<REQUEST_MAX_BYTES>, OtaError> {
    let mut out = TextBuf::<REQUEST_MAX_BYTES>::new();
    write!(
        out,
        "GET {}/{} HTTP/1.0\r\nHost: {}\r\n",
        uri.path(),
        name,
        uri.host
    )
    .map_err(|_| OtaError::Request)?;
    if !secrets::OTA_USER.is_empty() || !secrets::OTA_PASSWORD.is_empty() {
        let mut auth = [0u8; AUTH_MAX_BYTES];
        let n = basic_authorization(secrets::OTA_USER, secrets::OTA_PASSWORD, &mut auth)
            .ok_or(OtaError::Auth)?;
        let auth = core::str::from_utf8(&auth[..n]).map_err(|_| OtaError::Auth)?;
        write!(out, "Authorization: {}\r\n", auth).map_err(|_| OtaError::Request)?;
    }
    write!(out, "Connection: close\r\n\r\n").map_err(|_| OtaError::Request)?;
    Ok(out)
}

fn head_code(error: HeadError) -> &'static str {
    match error {
        HeadError::MalformedStatusLine => "malformed_status",
        HeadError::TooFewStatusTokens => "too_few_status_tokens",
        HeadError::NonNumericStatus => "non_numeric_status",
        HeadError::UnexpectedStatus => "unexpected_status",
        HeadError::TransferEncoding => "transfer_encoding",
        HeadError::MissingContentLength => "missing_content_length",
        HeadError::DuplicateContentLength => "duplicate_content_length",
        HeadError::MalformedHeader => "malformed_header",
        HeadError::OversizedHeader => "oversized_header",
    }
}
