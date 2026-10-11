//! Backend validation and collision allocation for the Desktop download form.
use serde_json::Value;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

pub fn random_task_id() -> Result<String, String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| "could not generate a download identifier".to_owned())?;
    Ok(format!(
        "download-{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

pub fn validate_source(source: &str) -> Result<String, String> {
    if source.chars().any(char::is_control) {
        return Err("download source must be one HTTP or HTTPS URL".to_owned());
    }
    let source = source.trim();
    if source.chars().any(char::is_control)
        || !source
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
            && !source
                .get(..8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        return Err("download source must be one HTTP or HTTPS URL".to_owned());
    }
    let url = reqwest::Url::parse(source)
        .map_err(|_| "download source must be a valid HTTP or HTTPS URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("download source must be a valid HTTP or HTTPS URL".to_owned());
    }
    Ok(source.to_owned())
}

pub fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.trim().is_empty()
        || matches!(filename, "." | "..")
        || filename.len() > 255
        || filename.contains(['/', '\\', ':'])
        || filename.chars().any(char::is_control)
    {
        return Err(
            "file name must be a single safe name without separators or control characters"
                .to_owned(),
        );
    }
    Ok(())
}

pub fn local_directory(directory: &Path) -> Result<PathBuf, String> {
    if !directory.is_absolute() {
        return Err("download folder must be an absolute path".to_owned());
    }
    let directory = directory.canonicalize().map_err(|error| {
        format!("could not open download folder; choose another folder: {error}")
    })?;
    if !directory.is_dir() {
        return Err("download folder must be an existing directory".to_owned());
    }
    // Test the actual user permission without allocating the final destination.
    let probe = directory.join(format!(".nexum-write-check-{}", random_task_id()?));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| {
            format!("download folder is not writable; choose another folder: {error}")
        })?;
    drop(file);
    std::fs::remove_file(&probe)
        .map_err(|error| format!("could not remove download folder permission check: {error}"))?;
    Ok(directory)
}

fn exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not inspect download destination: {error}")),
    }
}

pub fn allocate(
    directory: &Path,
    filename: &str,
    tasks: &[Value],
    inspect_local: bool,
) -> Result<PathBuf, String> {
    validate_filename(filename)?;
    if !directory.is_absolute() {
        return Err("download folder must be an absolute path".to_owned());
    }
    let mut reserved = HashSet::new();
    for path in tasks
        .iter()
        .filter_map(|task| task.get("destination").and_then(Value::as_str))
        .map(PathBuf::from)
    {
        let path = if inspect_local {
            path.parent()
                .and_then(|parent| parent.canonicalize().ok())
                .zip(path.file_name())
                .map(|(parent, name)| parent.join(name))
                .unwrap_or(path)
        } else {
            path
        };
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            reserved.insert(path.with_file_name(format!(".{name}.nexum.part")));
            reserved.insert(path.with_file_name(format!(".{name}.nexum.part.json")));
        }
        reserved.insert(path);
    }
    let name = Path::new(filename);
    let stem = name
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let extension = name
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    for index in 0..10_000 {
        let candidate_name = if index == 0 {
            filename.to_owned()
        } else {
            format!("{stem} ({index}){extension}")
        };
        validate_filename(&candidate_name)?;
        let candidate = directory.join(&candidate_name);
        let partial = directory.join(format!(".{candidate_name}.nexum.part"));
        let metadata = directory.join(format!(".{candidate_name}.nexum.part.json"));
        if reserved
            .iter()
            .any(|path| path == &candidate || path == &partial || path == &metadata)
        {
            continue;
        }
        if inspect_local && (exists(&candidate)? || exists(&partial)? || exists(&metadata)?) {
            continue;
        }
        return Ok(candidate);
    }
    Err("could not allocate a free file name; choose another name".to_owned())
}

/// Remote paths belong to the Server's OS, not to this Mac's Path implementation.
pub fn allocate_remote(directory: &str, filename: &str, tasks: &[Value]) -> Result<String, String> {
    validate_filename(filename)?;
    let drive = directory
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && directory.as_bytes().get(1) == Some(&b':')
        && directory
            .as_bytes()
            .get(2)
            .is_some_and(|byte| matches!(byte, b'/' | b'\\'));
    let unc = directory.starts_with("\\\\")
        && directory[2..]
            .split('\\')
            .filter(|part| !part.is_empty())
            .count()
            >= 2;
    let windows = drive || unc;
    if directory.chars().any(char::is_control) || !windows && !directory.starts_with('/') {
        return Err("download folder must be an absolute path on the Server host".to_owned());
    }
    let separator = if windows { '\\' } else { '/' };
    let normalize = |path: &str| {
        if windows {
            path.replace('/', "\\").to_lowercase()
        } else {
            path.to_owned()
        }
    };
    let join = |name: &str| {
        format!(
            "{}{separator}{name}",
            directory.trim_end_matches(if windows {
                &['/', '\\'][..]
            } else {
                &['/'][..]
            })
        )
    };
    let mut reserved = HashSet::new();
    for destination in tasks
        .iter()
        .filter_map(|task| task.get("destination").and_then(Value::as_str))
    {
        let path = normalize(destination);
        if let Some((parent, name)) = path.rsplit_once(separator) {
            reserved.insert(format!("{parent}{separator}.{name}.nexum.part"));
            reserved.insert(format!("{parent}{separator}.{name}.nexum.part.json"));
        }
        reserved.insert(path);
    }
    let name = Path::new(filename);
    let stem = name
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let extension = name
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    for index in 0..10_000 {
        let name = if index == 0 {
            filename.to_owned()
        } else {
            format!("{stem} ({index}){extension}")
        };
        validate_filename(&name)?;
        let destination = join(&name);
        if ![
            destination.clone(),
            join(&format!(".{name}.nexum.part")),
            join(&format!(".{name}.nexum.part.json")),
        ]
        .iter()
        .any(|path| reserved.contains(&normalize(path)))
        {
            return Ok(destination);
        }
    }
    Err("could not allocate a free file name; choose another name".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_url_and_filename_before_any_filesystem_access() {
        for source in [
            "ftp://example.com/f",
            "https:example.com",
            "https://",
            "https://example.com/\nsecond",
        ] {
            assert!(validate_source(source).is_err());
        }
        assert!(validate_source("https://example.com/archive.zip?q=1").is_ok());
        for name in ["", " ", ".", "..", "../escape", "a/b", "a\\b", "a\n", "a:b"] {
            assert!(validate_filename(name).is_err());
        }
        assert!(validate_filename("下载.zip").is_ok());
    }

    #[test]
    fn allocates_around_files_tasks_partials_and_orphan_metadata_without_creating_final_file() {
        let dir = std::env::temp_dir().join(random_task_id().unwrap());
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("file.zip"), b"keep").unwrap();
        std::fs::write(dir.join(".file (2).zip.nexum.part"), b"partial").unwrap();
        std::fs::write(dir.join(".file (3).zip.nexum.part.json"), b"metadata").unwrap();
        let tasks = [serde_json::json!({"destination":dir.join("file (1).zip")})];
        let target = allocate(&local_directory(&dir).unwrap(), "file.zip", &tasks, true).unwrap();
        assert_eq!(target, dir.canonicalize().unwrap().join("file (4).zip"));
        assert!(!target.exists());
        assert_eq!(std::fs::read(dir.join("file.zip")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remote_allocation_never_inspects_this_mac_but_honors_task_reservations() {
        let tasks = [serde_json::json!({"destination":"/not-mounted/file.zip"})];
        assert_eq!(
            allocate(Path::new("/not-mounted"), "file.zip", &tasks, false).unwrap(),
            Path::new("/not-mounted/file (1).zip")
        );
        assert_eq!(
            allocate_remote("/not-mounted", "file.zip", &tasks).unwrap(),
            "/not-mounted/file (1).zip"
        );
        let tasks = [serde_json::json!({"destination":"c:/downloads/FILE.zip"})];
        assert_eq!(
            allocate_remote("C:\\Downloads", "file.zip", &tasks).unwrap(),
            "C:\\Downloads\\file (1).zip"
        );
        assert_eq!(
            allocate_remote("\\\\host\\share", "file.zip", &[]).unwrap(),
            "\\\\host\\share\\file.zip"
        );
        for directory in ["relative", "C:Downloads", "\\\\host", ""] {
            assert!(allocate_remote(directory, "file.zip", &[]).is_err());
        }
    }
}
