//! Streaming verified update session.
//!
//! Pure and allocation-free. [`apply_update`] streams a bounded body from a
//! [`BodyReader`] into a [`Flasher`] while hashing it with streaming SHA-256, and
//! only commits once every byte has been written and the digest matches.

#![allow(async_fn_in_trait)]

use sha2::{Digest, Sha256};

use crate::layout::ACTIVE_BYTES;

use super::CHUNK_BYTES;

/// Sink for a streamed image: positional writes plus a final commit.
pub trait Flasher {
    type Error;

    async fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Self::Error>;

    async fn mark_updated(&mut self) -> Result<(), Self::Error>;
}

/// Source of an image body; `Ok(0)` signals EOF.
pub trait BodyReader {
    type Error;

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateError<FE, RE> {
    Oversize { length: u64, capacity: u32 },
    TooShort,
    TooLong,
    HashMismatch,
    Flash(FE),
    Read(RE),
}

/// Stream `content_length` bytes from `body` to `flasher`, verifying SHA-256.
///
/// Ordering invariant: an image larger than `ACTIVE_BYTES` is rejected before
/// any `flasher` call, and `mark_updated` is only reached after every chunk has
/// been written and the digest matches `expected`. No error path marks the DFU.
pub async fn apply_update<F: Flasher, R: BodyReader>(
    flasher: &mut F,
    body: &mut R,
    content_length: u64,
    expected: &[u8; 32],
) -> Result<(), UpdateError<F::Error, R::Error>> {
    if content_length > ACTIVE_BYTES as u64 {
        return Err(UpdateError::Oversize {
            length: content_length,
            capacity: ACTIVE_BYTES,
        });
    }

    let mut buf = [0u8; CHUNK_BYTES];
    let mut hasher = Sha256::new();
    let mut offset = 0usize;
    let mut total: u64 = 0;
    loop {
        let n = body.read(&mut buf).await.map_err(UpdateError::Read)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
        flasher
            .write(offset, &buf[..n])
            .await
            .map_err(UpdateError::Flash)?;
        offset += n;
        if total > content_length {
            return Err(UpdateError::TooLong);
        }
    }

    if total != content_length {
        return Err(UpdateError::TooShort);
    }
    let digest = hasher.finalize();
    if digest[..] != expected[..] {
        return Err(UpdateError::HashMismatch);
    }

    flasher.mark_updated().await.map_err(UpdateError::Flash)?;
    Ok(())
}
