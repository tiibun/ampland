//! Shared HTTP client configuration.
//!
//! All network access goes through [`agent`] so timeouts and proxy behaviour
//! are decided in one place instead of relying on ureq's defaults.

use std::sync::OnceLock;
use std::time::Duration;

/// Upper bound for establishing a TCP connection (ureq 2's former default).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum size of a text response body (release JSON, checksum files).
pub const MAX_TEXT_BYTES: u64 = 10 * 1024 * 1024;

/// Returns the process-wide HTTP agent.
///
/// Proxies are taken from the environment (`HTTPS_PROXY`, `HTTP_PROXY`,
/// `ALL_PROXY`, `NO_PROXY`), which is ureq's default behaviour.
pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .build();
        ureq::Agent::new_with_config(config)
    })
}

/// Reads a response body as UTF-8 text, failing on invalid UTF-8 instead of
/// silently replacing bytes, and on bodies larger than [`MAX_TEXT_BYTES`].
pub fn read_text(response: ureq::http::Response<ureq::Body>) -> Result<String, ureq::Error> {
    response
        .into_body()
        .into_with_config()
        .limit(MAX_TEXT_BYTES)
        .lossy_utf8(false)
        .read_to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_once(body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
        });
        format!("http://{addr}/")
    }

    #[test]
    fn agent_sets_connect_timeout() {
        let timeouts = agent().config().timeouts();
        assert_eq!(timeouts.connect, Some(CONNECT_TIMEOUT));
    }

    #[test]
    fn read_text_returns_valid_utf8() {
        let url = serve_once("abc ✓".as_bytes());
        let response = agent().get(&url).call().expect("call");
        assert_eq!(read_text(response).expect("text"), "abc ✓");
    }

    #[test]
    fn read_text_rejects_invalid_utf8() {
        let url = serve_once(b"abc\xff\xfe");
        let response = agent().get(&url).call().expect("call");
        assert!(read_text(response).is_err());
    }
}
