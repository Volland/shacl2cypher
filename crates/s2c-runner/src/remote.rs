//! Remote `owl:imports` over HTTP(S), shared by the CLI and the language bindings
//! so the core crate stays free of network I/O.

use std::io::Read;
use std::time::Duration;

use shacl2cypher_core::load::RemoteFetcher;

/// Largest remote import accepted with `--allow-remote-imports`.
pub const MAX_REMOTE_BYTES: u64 = 16 * 1024 * 1024;

/// Time allowed for fetching one remote import.
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

/// Fetches imports with `ureq`, bounded by [`REMOTE_TIMEOUT`] and [`MAX_REMOTE_BYTES`].
// @lat: [[architecture#CLI]]
pub struct HttpFetcher;

impl RemoteFetcher for HttpFetcher {
    fn fetch(&self, iri: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(iri)
            .timeout(REMOTE_TIMEOUT)
            .set(
                "Accept",
                "text/turtle, application/n-triples, application/trig;q=0.9",
            )
            .call()
            .map_err(|e| e.to_string())?;
        read_capped(response.into_reader(), MAX_REMOTE_BYTES)
    }
}

/// Reads a whole body, failing when it is larger than `cap` bytes.
fn read_capped(reader: impl Read, cap: u64) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    reader
        .take(cap + 1)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() as u64 > cap {
        return Err(format!("larger than {cap} bytes"));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_up_to_the_cap_are_read() {
        assert_eq!(read_capped(&b"abcd"[..], 4).unwrap(), b"abcd");
    }

    #[test]
    fn bodies_over_the_cap_are_rejected() {
        let error = read_capped(&b"abcde"[..], 4).unwrap_err();
        assert_eq!(error, "larger than 4 bytes");
    }
}
