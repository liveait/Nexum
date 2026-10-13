//! Async filename probing and partial download metadata.
use percent_encoding::percent_decode_str;
use reqwest::header::{CONTENT_DISPOSITION, HeaderMap, RANGE};
use reqwest::{Client, Url};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Asynchronously suggests a file name from an HTTP response.
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

// ---------------------------------------------------------------------------
// Partial download metadata (interruption recovery)
// ---------------------------------------------------------------------------

/// Metadata persisted alongside a partial download so that an interrupted task
/// can be reconstructed after a server or app crash.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PartialMetadata {
    pub task_id: String,
    pub source: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Writes partial metadata for a task into the server data directory.
/// The metadata file is placed with a task_id-based name following the
/// existing .nexum.part.json naming convention.
pub fn save_partial_metadata(
    directory: &Path,
    task_id: &str,
    source: &str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> Result<PathBuf, String> {
    let metadata = directory.join(format!(".{task_id}.nexum.part.json"));
    let content = serde_json::to_string(&PartialMetadata {
        task_id: task_id.to_owned(),
        source: source.to_owned(),
        downloaded_bytes,
        total_bytes,
    })
    .map_err(|e| format!("could not serialize partial metadata: {e}"))?;
    let tmp = directory.join(format!(".{task_id}.nexum.part.json.tmp"));
    std::fs::write(&tmp, &content).map_err(|e| format!("could not write partial metadata: {e}"))?;
    std::fs::rename(&tmp, &metadata)
        .map_err(|e| format!("could not save partial metadata: {e}"))?;
    Ok(metadata)
}

/// Reads partial metadata for a task from the server data directory.
/// Returns `Ok(None)` when no metadata file exists.
pub fn read_partial_metadata(directory: &Path, task_id: &str) -> Result<Option<PartialMetadata>, String> {
    let metadata = directory.join(format!(".{task_id}.nexum.part.json"));
    if !metadata.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&metadata)
        .map_err(|e| format!("could not read partial metadata: {e}"))?;
    let meta = serde_json::from_str(&content)
        .map_err(|e| format!("could not parse partial metadata: {e}"))?;
    Ok(Some(meta))
}

/// Removes partial metadata file for a completed task.
pub fn clear_partial_metadata(directory: &Path, task_id: &str) -> Result<(), String> {
    let metadata = directory.join(format!(".{task_id}.nexum.part.json"));
    std::fs::remove_file(&metadata).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_metadata_round_trips() {
        let dir = std::env::temp_dir();
        std::fs::create_dir_all(&dir).unwrap();

        save_partial_metadata(
            &dir,
            "download-abc",
            "https://example.com/file.zip",
            1024,
            Some(2048),
        )
        .unwrap();

        let meta = read_partial_metadata(&dir, "download-abc").unwrap().unwrap();
        assert_eq!(meta.task_id, "download-abc");
        assert_eq!(meta.source, "https://example.com/file.zip");
        assert_eq!(meta.downloaded_bytes, 1024);
        assert_eq!(meta.total_bytes, Some(2048));

        // Missing task_id returns None.
        assert!(read_partial_metadata(&dir, "missing").unwrap().is_none());

        clear_partial_metadata(&dir, "download-abc").unwrap();
        assert!(read_partial_metadata(&dir, "download-abc").unwrap().is_none());

        std::fs::remove_file(dir.join(".download-abc.nexum.part.json")).ok();
    }
}
