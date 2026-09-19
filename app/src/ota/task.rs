//! OTA task: resolve, fetch, verify and (only then) commit an update.
//!
//! The task waits on [`TRIGGER`](super::TRIGGER); there is no periodic poll. A
//! failed check is logged and the task waits again. The control path is never
//! blocked: every network wait is a timeout-wrapped await.

use core::fmt::Write as _;

use embassy_net::Stack;
use garagelight_core::ota::{
    apply_update, decide, parse_sha256_hex, parse_url, parse_version, Decision, UpdateError,
};

use crate::{beacon, logln, secrets, update};

use super::transport::{
    build_request, open, resolve, HttpBody, READ_CHUNK, RECORD_BYTES, TCP_BYTES,
};
use super::{FetchError, OtaError, TextBuf};

const VERSION_MAX_BYTES: usize = 64;
const SHA_MAX_BYTES: usize = 128;
const NAME_MAX_BYTES: usize = 48;

/// Wait for a trigger, then run one update check. The task owns `updater` for
/// the life of the program and only touches flash after a response head and a
/// hash have been accepted. A failed check blinks its stage code on the onboard
/// LED (via the shared cyw43 control) so a human can report it without a probe.
#[embassy_executor::task]
pub(super) async fn ota_task(
    stack: Stack<'static>,
    mut updater: update::Updater,
    control: &'static crate::beacon::SharedControl,
) -> ! {
    loop {
        super::TRIGGER.wait().await;
        match check_and_update(stack, &mut updater).await {
            Ok(()) => {}
            Err(error) => {
                logln!("ota_failed code={}", error.code());
                beacon::code(control, error.blink_code()).await;
            }
        }
    }
}

/// Resolve, fetch, verify and (only then) mark the image. Returns `Ok(())` for
/// every transient condition (404, no update) as well as for errors that do not
/// warrant a reset; the caller logs the error code. Never loops, never resets.
async fn check_and_update(
    stack: Stack<'static>,
    updater: &mut update::Updater,
) -> Result<(), OtaError> {
    stack.wait_config_up().await;

    let uri = parse_url(secrets::OTA_URL, secrets::OTA_PROJECT).map_err(|_| {
        logln!("ota_url_invalid");
        OtaError::Url
    })?;
    let endpoint = resolve(stack, &uri).await?;

    let mut tcp_rx = [0u8; TCP_BYTES];
    let mut tcp_tx = [0u8; TCP_BYTES];
    let mut rec_read = [0u8; RECORD_BYTES];
    let mut rec_write = [0u8; RECORD_BYTES];
    let mut leftover = [0u8; READ_CHUNK];

    let remote = {
        let request = build_request(&uri, "version")?;
        let mut link = open(
            stack,
            endpoint,
            &uri,
            &mut tcp_rx,
            &mut tcp_tx,
            &mut rec_read,
            &mut rec_write,
        )
        .await?;
        let (head, extra) = link
            .send(request.as_bytes(), &mut leftover)
            .await
            .map_err(fetch_failed)?;
        if head.status == 404 {
            logln!("ota_version_missing");
            return Ok(());
        }
        if head.status != 200 {
            logln!("ota_version_missing");
            return Ok(());
        }
        let len = head.content_length.ok_or(OtaError::Head)?;
        if len as usize > VERSION_MAX_BYTES {
            logln!("ota_version_too_long");
            return Err(OtaError::Head);
        }
        let mut body = [0u8; VERSION_MAX_BYTES];
        let n = link
            .read_small(&leftover[..extra], len, &mut body)
            .await
            .map_err(fetch_failed)?;
        parse_version(&body[..n]).ok_or_else(|| {
            logln!("ota_version_invalid");
            OtaError::Version
        })?
    };

    match decide(crate::VERSION, remote) {
        Decision::Skip => {
            logln!("ota_check local={} remote={}", crate::VERSION, remote);
            logln!("ota_no_update");
            return Ok(());
        }
        Decision::Update => logln!("ota_check local={} remote={}", crate::VERSION, remote),
    }

    let expected = {
        let mut name = TextBuf::<NAME_MAX_BYTES>::new();
        write!(name, "{}.bin.sha256", remote).map_err(|_| OtaError::Request)?;
        let request = build_request(&uri, name.as_str())?;
        let mut link = open(
            stack,
            endpoint,
            &uri,
            &mut tcp_rx,
            &mut tcp_tx,
            &mut rec_read,
            &mut rec_write,
        )
        .await?;
        let (head, extra) = link
            .send(request.as_bytes(), &mut leftover)
            .await
            .map_err(fetch_failed)?;
        if head.status != 200 {
            logln!("ota_sha_missing");
            return Ok(());
        }
        let len = head.content_length.ok_or(OtaError::Head)?;
        if len as usize > SHA_MAX_BYTES {
            logln!("ota_sha_too_long");
            return Err(OtaError::Head);
        }
        let mut body = [0u8; SHA_MAX_BYTES];
        let n = link
            .read_small(&leftover[..extra], len, &mut body)
            .await
            .map_err(fetch_failed)?;
        parse_sha256_hex(&body[..n]).ok_or_else(|| {
            logln!("ota_sha_missing");
            OtaError::Sha
        })?
    };

    let mut name = TextBuf::<NAME_MAX_BYTES>::new();
    write!(name, "{}.bin", remote).map_err(|_| OtaError::Request)?;
    let request = build_request(&uri, name.as_str())?;
    let mut link = open(
        stack,
        endpoint,
        &uri,
        &mut tcp_rx,
        &mut tcp_tx,
        &mut rec_read,
        &mut rec_write,
    )
    .await?;
    let (head, extra) = link
        .send(request.as_bytes(), &mut leftover)
        .await
        .map_err(fetch_failed)?;
    if head.status != 200 {
        logln!("ota_bin_missing");
        return Ok(());
    }
    let len = head.content_length.ok_or(OtaError::Head)?;
    logln!("ota_head status={} len={}", head.status, len);

    let result = {
        let mut body = HttpBody {
            link: &mut link,
            leftover: &leftover[..extra],
        };
        logln!("ota_streaming len={}", len);
        apply_update(updater, &mut body, len, &expected).await
    };
    result.map_err(|error| {
        let code = update_code(&error);
        if matches!(error, UpdateError::HashMismatch) {
            logln!("ota_hash_mismatch");
        }
        OtaError::Update(code)
    })?;

    logln!("ota_hash_ok");
    logln!("ota_marked resetting");
    logln!("ota_flash_window_max_us={}", update::max_window_us());
    cortex_m::peripheral::SCB::sys_reset();
}

fn fetch_failed(error: FetchError) -> OtaError {
    logln!("ota_fetch_failed code={}", error.code());
    OtaError::Transport(error)
}

fn update_code<FE, RE>(error: &UpdateError<FE, RE>) -> &'static str {
    match error {
        UpdateError::Oversize { .. } => "oversize",
        UpdateError::TooShort => "too_short",
        UpdateError::TooLong => "too_long",
        UpdateError::HashMismatch => "hash_mismatch",
        UpdateError::Flash(_) => "flash",
        UpdateError::Read(_) => "read",
    }
}
