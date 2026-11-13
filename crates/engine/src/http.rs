//! HTTPS through Windows' own WinHTTP (ADR 0003, E7), so the app ships no
//! HTTP or TLS library of its own: `deny.toml` still bans them. Used only to
//! read SMF's release list on GitHub and download a release, when the user
//! checks or updates. HTTPS only; redirects are followed the way WinHTTP does
//! by default (never from HTTPS to HTTP).

use std::io::Write;
use std::path::Path;

use vmm_core::error::{ErrorCode, VmmError};

/// The User-Agent GitHub's API requires. Names the app, nothing about the user.
pub const USER_AGENT: &str = concat!("VaexilModManager/", env!("CARGO_PKG_VERSION"));

fn failed(why: impl std::fmt::Display) -> VmmError {
    VmmError::new(ErrorCode::NetworkFailed, format!("The download failed: {why}"))
}

/// Splits `https://host/path?query` into host and path.
fn split_url(url: &str) -> Result<(&str, &str), VmmError> {
    let rest =
        url.strip_prefix("https://").ok_or_else(|| failed(format!("only https addresses are used, not {url}")))?;
    Ok(match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    })
}

/// Fetches `url`, passing the body to `sink` in pieces, and `progress` the
/// bytes so far and the total when the server gives it.
#[cfg(windows)]
pub fn fetch(
    url: &str,
    accept: &str,
    sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), VmmError> {
    use windows::Win32::Networking::WinHttp::*;
    use windows::core::{HSTRING, PCWSTR};

    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: a handle WinHTTP returned, closed once.
                let _ = unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    let open = |p: *mut core::ffi::c_void, what: &str| -> Result<Handle, VmmError> {
        if p.is_null() {
            Err(failed(format!("{what}: {}", windows::core::Error::from_thread())))
        } else {
            Ok(Handle(p))
        }
    };

    let (host, path) = split_url(url)?;
    // SAFETY: plain WinHTTP calls on handles this function owns; every
    // buffer passed is valid for the length given.
    unsafe {
        let session = open(
            WinHttpOpen(
                &HSTRING::from(USER_AGENT),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        // Resolve, connect, send: 30 s each; receive: 60 s between pieces.
        let _ = WinHttpSetTimeouts(session.0, 30_000, 30_000, 30_000, 60_000);
        let connect =
            open(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0), "connecting")?;
        let request = open(
            WinHttpOpenRequest(
                connect.0,
                &HSTRING::from("GET"),
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "opening the request",
        )?;
        let headers: Vec<u16> = format!("Accept: {accept}\r\n").encode_utf16().collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(failed)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(failed)?;

        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(failed)?;
        if status != 200 {
            return Err(failed(format!("{host} answered with HTTP status {status}")));
        }
        let mut total = 0u64;
        let mut len = 8u32;
        let total = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER64,
            PCWSTR::null(),
            Some((&mut total as *mut u64).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .ok()
        .map(|()| total);

        let mut done = 0u64;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut available).map_err(failed)?;
            if available == 0 {
                break;
            }
            let want = (available as usize).min(buf.len());
            let mut read = 0u32;
            WinHttpReadData(request.0, buf.as_mut_ptr().cast(), want as u32, &mut read).map_err(failed)?;
            if read == 0 {
                break;
            }
            sink(&buf[..read as usize])
                .map_err(|e| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't save the download: {e}")))?;
            done += u64::from(read);
            progress(done, total);
        }
        if let Some(t) = total
            && done != t
        {
            return Err(failed(format!("the connection ended after {done} of {t} bytes")));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn fetch(
    _url: &str,
    _accept: &str,
    _sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    _progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), VmmError> {
    Err(failed("downloads are only made on Windows"))
}

/// Fetches `url` into memory (for small JSON answers, up to 8 MB).
pub fn get(url: &str, accept: &str) -> Result<Vec<u8>, VmmError> {
    let mut body = Vec::new();
    fetch(
        url,
        accept,
        &mut |piece| {
            if body.len() + piece.len() > 8 << 20 {
                return Err(std::io::Error::other("the answer is larger than expected"));
            }
            body.extend_from_slice(piece);
            Ok(())
        },
        &mut |_, _| {},
    )?;
    Ok(body)
}

/// Downloads `url` to `dest` (through `dest.part`, renamed when complete).
pub fn download(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), VmmError> {
    let part = dest.with_extension("part");
    let save = |e: std::io::Error| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't save the download: {e}"));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&part).map_err(save)?);
    let r = fetch(url, "application/octet-stream", &mut |piece| file.write_all(piece), progress);
    let r = r.and_then(|()| file.flush().map_err(save));
    drop(file);
    match r {
        Ok(()) => std::fs::rename(&part, dest).map_err(save),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls_are_used() {
        assert_eq!(split_url("https://api.github.com/repos/x?y=1").unwrap(), ("api.github.com", "/repos/x?y=1"));
        assert_eq!(split_url("https://example.com").unwrap(), ("example.com", "/"));
        assert_eq!(split_url("http://example.com/").unwrap_err().code, ErrorCode::NetworkFailed);
    }
}
