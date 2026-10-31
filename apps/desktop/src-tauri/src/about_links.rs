use serde::Deserialize;

/// IPC accepts a project-link ID, never an arbitrary URL, path or application.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLink {
    Website,
    Source,
    Documentation,
}

impl ProjectLink {
    fn url(self) -> &'static str {
        match self {
            Self::Website => "https://liveait.github.io/Nexum/",
            Self::Source => "https://github.com/liveait/Nexum",
            Self::Documentation => "https://github.com/liveait/Nexum/blob/main/docs/DEVELOPMENT.md",
        }
    }
}

pub fn open_project_link<E>(
    link: ProjectLink,
    open: impl FnOnce(&str) -> Result<(), E>,
) -> Result<(), String> {
    open(link.url()).map_err(|_| "Could not open project link".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ids_open_only_the_three_fixed_urls() {
        for (id, url) in [
            ("website", "https://liveait.github.io/Nexum/"),
            ("source", "https://github.com/liveait/Nexum"),
            (
                "documentation",
                "https://github.com/liveait/Nexum/blob/main/docs/DEVELOPMENT.md",
            ),
        ] {
            let link: ProjectLink = serde_json::from_value(serde_json::json!(id)).unwrap();
            let mut calls = Vec::new();
            open_project_link(link, |actual| {
                calls.push(actual.to_owned());
                Ok::<(), &str>(())
            })
            .unwrap();
            assert_eq!(calls, [url]);
        }
    }

    #[test]
    fn ipc_rejects_urls_paths_unknown_ids_and_app_choices() {
        for value in [
            serde_json::json!("https://example.com"),
            serde_json::json!("https://liveait.github.io/Nexum/?redirect=evil"),
            serde_json::json!("file:///tmp/file"),
            serde_json::json!("javascript:alert(1)"),
            serde_json::json!("/bin/sh"),
            serde_json::json!("--help"),
            serde_json::json!("Website"),
            serde_json::json!({ "website": "https://example.com" }),
            serde_json::json!({ "website": { "with": "/bin/sh" } }),
        ] {
            assert!(serde_json::from_value::<ProjectLink>(value).is_err());
        }
    }

    #[test]
    fn native_open_failures_are_reported_without_os_details() {
        assert_eq!(
            open_project_link(ProjectLink::Website, |_| Err("private path or OS error")),
            Err("Could not open project link".to_owned())
        );
    }

    #[test]
    fn webview_has_no_generic_shell_or_opener_permission() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(permissions.iter().any(|p| p == "core:default"));
        assert!(!permissions.iter().any(|p| {
            let id = p.as_str().or_else(|| p["identifier"].as_str()).unwrap();
            id.starts_with("opener:") || id.starts_with("shell:")
        }));
    }
}
