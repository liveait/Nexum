use percent_encoding::percent_decode_str;
use reqwest::header::{CONTENT_DISPOSITION, HeaderMap, RANGE};
use reqwest::{Client, Url};
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn suggest_download_filename(source: String) -> Option<String> {
    let source = source.trim();
    if source.contains(['\r', '\n'])
        || !source
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            && !source
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    {
        return None;
    }
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut redirected_path_name = None;
        if let Ok(response) = client.head(url.clone()).send().await
            && response.status().is_success()
        {
            redirected_path_name = url_filename(response.url());
            if let Some(name) = header_filename(response.headers()) {
                return Some(name);
            }
        }

        // Some servers omit Content-Disposition on HEAD or do not support it.
        // Dropping the response after headers avoids downloading its body even
        // if the server ignores Range.
        if let Ok(response) = client.get(url).header(RANGE, "bytes=0-0").send().await
            && response.status().is_success()
        {
            if let Some(name) = header_filename(response.headers()) {
                return Some(name);
            }
            if let Some(name) = url_filename(response.url()) {
                return Some(name);
            }
        }
        redirected_path_name
    })
    .await
    .ok()
    .flatten()
}

fn url_filename(url: &Url) -> Option<String> {
    if url.path().ends_with('/') {
        return None;
    }
    let encoded = url.path_segments()?.next_back()?;
    let decoded = percent_decode_str(encoded).decode_utf8().ok()?;
    safe_file_name(&decoded)
}

fn header_filename(headers: &HeaderMap) -> Option<String> {
    for header in headers.get_all(CONTENT_DISPOSITION) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        let mut extended = None;
        let mut plain = None;
        for parameter in split_header_parameters(header).into_iter().skip(1) {
            let Some((key, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case("filename*") {
                extended = parse_extended_filename(value);
            } else if key.trim().eq_ignore_ascii_case("filename") {
                plain = parse_header_value(value).and_then(|name| safe_file_name(&name));
            }
        }
        if let Some(name) = extended.or(plain) {
            return Some(name);
        }
    }
    None
}

fn split_header_parameters(header: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in header.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == ';' && !quoted {
            parts.push(&header[start..index]);
            start = index + 1;
        }
    }
    parts.push(&header[start..]);
    parts
}

fn parse_header_value(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('"') {
        let inner = inner.strip_suffix('"')?;
        let mut unescaped = String::with_capacity(inner.len());
        let mut characters = inner.chars();
        while let Some(character) = characters.next() {
            if character == '\\' {
                unescaped.push(characters.next()?);
            } else {
                unescaped.push(character);
            }
        }
        Some(unescaped)
    } else {
        Some(value.to_owned())
    }
}

fn parse_extended_filename(value: &str) -> Option<String> {
    let value = parse_header_value(value)?;
    let (charset, rest) = value.split_once('\'')?;
    let (_, encoded) = rest.split_once('\'')?;
    if !charset.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    let decoded = percent_decode_str(encoded).decode_utf8().ok()?;
    safe_file_name(&decoded)
}

fn safe_file_name(name: &str) -> Option<String> {
    if name.trim().is_empty()
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|character| character == '/' || character == '\\' || character.is_ascii_control())
    {
        return None;
    }
    Some(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{header_filename, suggest_download_filename, url_filename};
    use reqwest::Url;
    use reqwest::header::{CONTENT_DISPOSITION, HeaderMap, HeaderValue};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    fn disposition(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_DISPOSITION, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn content_disposition_beats_codeload_path() {
        let url =
            Url::parse("https://codeload.github.com/liveait/omlx/zip/refs/heads/main").unwrap();
        assert_eq!(url_filename(&url).as_deref(), Some("main"));
        assert_eq!(
            header_filename(&disposition("attachment; filename=omlx-main.zip")).as_deref(),
            Some("omlx-main.zip")
        );
    }

    #[test]
    fn utf8_extended_filename_precedes_plain_and_rejects_unsafe_names() {
        let headers = disposition(
            "attachment; filename=fallback.zip; filename*=UTF-8''%E6%8A%A5%E5%91%8A.zip",
        );
        assert_eq!(header_filename(&headers).as_deref(), Some("报告.zip"));
        assert_eq!(
            header_filename(&disposition(
                "attachment; filename*=UTF-8''..%2Fsecret; filename=safe.zip"
            ))
            .as_deref(),
            Some("safe.zip")
        );
        assert_eq!(
            header_filename(&disposition("attachment; filename=../secret")),
            None
        );
        assert_eq!(
            header_filename(&disposition("attachment; filename=\"archive; final.zip\"")).as_deref(),
            Some("archive; final.zip")
        );
    }

    #[test]
    fn probe_uses_get_headers_when_head_omits_the_filename() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(6);
            let mut methods = Vec::new();
            while methods.len() < 2 && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = [0; 2048];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let method = request.split_whitespace().next().unwrap().to_owned();
                let response = if method == "HEAD" {
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    assert_eq!(method, "GET");
                    assert!(
                        request.contains("range: bytes=0-0")
                            || request.contains("Range: bytes=0-0")
                    );
                    "HTTP/1.1 206 Partial Content\r\nContent-Disposition: attachment; filename=omlx-main.zip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                stream.write_all(response.as_bytes()).unwrap();
                methods.push(method);
            }
            methods
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let name = runtime.block_on(suggest_download_filename(format!("http://{address}/main")));
        assert_eq!(name.as_deref(), Some("omlx-main.zip"));
        assert_eq!(server.join().unwrap(), ["HEAD", "GET"]);
    }
}
