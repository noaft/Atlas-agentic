//! Public GitHub imports. Downloads are data only and never execute repository code.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_urls_and_parses_supported_locations() {
        for url in [
            "http://github.com/o/r",
            "https://user:secret@github.com/o/r",
            "https://github.com/o/r?token=secret",
            "https://github.com/o/r/tree/main/%2e%2e/x",
            "https://evil.test/o/r",
            "https://github.com/o/r/blob/main/file.txt",
        ] {
            assert!(Location::parse(url).is_err(), "{url}");
        }
        let repo = Location::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(repo.repository, "owner/repo");
        let file =
            Location::parse("https://github.com/o/r/blob/main/skills/demo/SKILL.md").unwrap();
        assert_eq!(file.tail, vec!["main", "skills", "demo"]);
    }

    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use std::thread;

    const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BLOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const RESOURCE: &str = "cccccccccccccccccccccccccccccccccccccccc";
    const SKILL: &str =
        "---\nname: sample\ndescription: Example skill\n---\nRead references/guide.md.\n";

    struct Server {
        github: Github,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(handler: impl Fn(&str) -> (u16, Vec<u8>) + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let worker = thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split_whitespace().nth(1).unwrap().to_string();
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        assert!(!line.to_ascii_lowercase().starts_with("authorization:"));
                    }
                    seen.lock().unwrap().push(path.clone());
                    let (status, body) = handler(&path);
                    write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    let _ = stream.write_all(&body);
                }
            });
            let mut github = Github::new().unwrap();
            github.base = base;
            Self {
                github,
                requests,
                stop,
                worker: Some(worker),
            }
        }
        fn fixture(tree: serde_json::Value) -> Self {
            Self::new(move |path| {
                let result = match path {
                    "/repos/o/r" => json!({"default_branch":"main"}).to_string().into_bytes(),
                    "/repos/o/r/commits/main"
                    | "/repos/o/r/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" => {
                        json!({"sha":COMMIT}).to_string().into_bytes()
                    }
                    "/repos/o/r/git/trees/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?recursive=1" => {
                        tree.to_string().into_bytes()
                    }
                    "/repos/o/r/git/blobs/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" => {
                        SKILL.as_bytes().to_vec()
                    }
                    "/repos/o/r/git/blobs/cccccccccccccccccccccccccccccccccccccccc" => {
                        b"Supporting guide".to_vec()
                    }
                    _ => return (404, Vec::new()),
                };
                (200, result)
            })
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.worker.take().unwrap().join().unwrap();
        }
    }
    fn tree() -> serde_json::Value {
        json!({"truncated":false,"tree":[
            {"path":"skills/a/SKILL.md","mode":"100644","type":"blob","sha":BLOB,"size":SKILL.len()},
            {"path":"skills/a/references/guide.md","mode":"100644","type":"blob","sha":RESOURCE,"size":16}
        ]})
    }
    fn source() -> SkillSource {
        SkillSource {
            repository: "o/r".into(),
            reference: "main".into(),
            revision: COMMIT.into(),
            path: "skills/a".into(),
        }
    }

    #[test]
    fn preview_pins_default_branch_and_download_preserves_resources() {
        let server = Server::fixture(tree());
        let previews = server.github.preview("https://github.com/o/r").unwrap();
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].source, source());
        assert_eq!(previews[0].content, SKILL);
        assert_eq!(previews[0].files, ["SKILL.md", "references/guide.md"]);
        let package = server.github.download(&previews[0].source).unwrap();
        assert_eq!(package.files["SKILL.md"], SKILL.as_bytes());
        assert_eq!(package.files["references/guide.md"], b"Supporting guide");
        let seen = server.requests.lock().unwrap();
        assert!(seen
            .iter()
            .any(|p| p.contains(COMMIT) && p.contains("trees")));
        assert!(!seen.iter().any(|p| p.contains("trees/main")));
    }

    #[test]
    fn folder_and_skill_file_urls_select_one_skill() {
        for url in [
            "https://github.com/o/r/tree/main/skills/a",
            "https://github.com/o/r/blob/main/skills/a/SKILL.md",
        ] {
            let server = Server::fixture(tree());
            assert_eq!(server.github.preview(url).unwrap()[0].source, source());
        }
    }

    #[test]
    fn repository_with_duplicate_skill_names_requires_specific_folder() {
        let mut data = tree();
        data["tree"].as_array_mut().unwrap().push(json!({"path":"skills/b/SKILL.md","mode":"100644","type":"blob","sha":BLOB,"size":SKILL.len()}));
        let server = Server::fixture(data);
        assert!(server
            .github
            .preview("https://github.com/o/r")
            .unwrap_err()
            .contains("same name"));
        assert_eq!(
            server
                .github
                .preview("https://github.com/o/r/tree/main/skills/a")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn rejects_symlinks_submodules_traversal_and_truncated_trees() {
        for (path, mode, kind) in [
            ("../SKILL.md", "100644", "blob"),
            ("skills/a/link", "120000", "blob"),
            ("skills/a/submodule", "160000", "commit"),
        ] {
            let server = Server::fixture(
                json!({"tree":[{"path":path,"mode":mode,"type":kind,"sha":BLOB,"size":8}],"truncated":false}),
            );
            assert!(server.github.download(&source()).is_err());
        }
        let server = Server::fixture(json!({"tree":[],"truncated":true}));
        assert!(server
            .github
            .download(&source())
            .unwrap_err()
            .contains("too large"));
    }

    #[test]
    fn rejects_oversized_files_and_excessive_file_counts_before_blob_reads() {
        let mut data = tree();
        data["tree"][0]["size"] = json!(MAX_FILE_BYTES + 1);
        let server = Server::fixture(data);
        assert!(server
            .github
            .download(&source())
            .unwrap_err()
            .contains("limit"));
        assert!(!server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("blobs")));
        let entries: Vec<_> = (0..=MAX_FILES).map(|n| json!({"path":format!("skills/a/{}",if n==0 {"SKILL.md".into()} else {n.to_string()}),"mode":"100644","type":"blob","sha":BLOB,"size":1})).collect();
        let server = Server::fixture(json!({"tree":entries}));
        assert!(server
            .github
            .download(&source())
            .unwrap_err()
            .contains("file limit"));
    }

    #[test]
    fn network_errors_and_rate_limits_do_not_echo_remote_content() {
        let server = Server::new(|_| (429, b"token=do-not-echo".to_vec()));
        let error = server.github.preview("https://github.com/o/r").unwrap_err();
        assert!(error.contains("rate limit"));
        assert!(!error.contains("do-not-echo"));
        let server = Server::new(|_| (302, b"private-endpoint-secret".to_vec()));
        let error = server.github.preview("https://github.com/o/r").unwrap_err();
        assert!(!error.contains("private-endpoint-secret"));
    }

    #[test]
    fn discovers_multiple_distinct_skills() {
        let second = "---\nname: second\ndescription: Another skill\n---\nInstructions";
        let mut data = tree();
        data["tree"].as_array_mut().unwrap().push(json!({"path":"skills/b/SKILL.md","mode":"100644","type":"blob","sha":RESOURCE,"size":second.len()}));
        let server = Server::new(move |path| match path {
            "/repos/o/r" => (
                200,
                json!({"default_branch":"main"}).to_string().into_bytes(),
            ),
            "/repos/o/r/commits/main" => (200, json!({"sha":COMMIT}).to_string().into_bytes()),
            "/repos/o/r/git/trees/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?recursive=1" => {
                (200, data.to_string().into_bytes())
            }
            "/repos/o/r/git/blobs/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" => {
                (200, SKILL.as_bytes().to_vec())
            }
            "/repos/o/r/git/blobs/cccccccccccccccccccccccccccccccccccccccc" => {
                (200, second.as_bytes().to_vec())
            }
            _ => (404, Vec::new()),
        });
        let previews = server.github.preview("https://github.com/o/r").unwrap();
        assert_eq!(
            previews.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["sample", "second"]
        );
    }

    #[test]
    fn resolves_slash_containing_branch_before_reading_pinned_tree() {
        let server = Server::new(|path| match path {
            "/repos/o/r/commits/feature%2Fbranch" => {
                (200, json!({"sha":COMMIT}).to_string().into_bytes())
            }
            "/repos/o/r/git/trees/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?recursive=1" => {
                (200, tree().to_string().into_bytes())
            }
            "/repos/o/r/git/blobs/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" => {
                (200, SKILL.as_bytes().to_vec())
            }
            _ => (404, Vec::new()),
        });
        let previews = server
            .github
            .preview("https://github.com/o/r/tree/feature/branch/skills/a")
            .unwrap();
        assert_eq!(previews[0].source.reference, "feature/branch");
        assert_eq!(previews[0].source.path, "skills/a");
        assert_eq!(previews[0].source.revision, COMMIT);
    }

    #[test]
    fn missing_long_references_returning_422_fall_back_to_branch() {
        let server = Server::new(|path| {
            match path {
            "/repos/o/r/commits/main" => (200, json!({"sha":COMMIT}).to_string().into_bytes()),
            "/repos/o/r/git/trees/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?recursive=1" => (200, tree().to_string().into_bytes()),
            "/repos/o/r/git/blobs/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" => (200, SKILL.as_bytes().to_vec()),
            path if path.starts_with("/repos/o/r/commits/") => (422, br#"{"message":"No commit found for SHA: main/skills/a","documentation_url":"https://docs.github.com/rest/commits/commits#get-a-commit","status":"422"}"#.to_vec()),
            _ => (404, Vec::new()),
        }
        });
        let previews = server
            .github
            .preview("https://github.com/o/r/tree/main/skills/a")
            .unwrap();
        assert_eq!(previews[0].source, source());
        assert!(server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with("main%2Fskills%2Fa")));
    }

    #[test]
    fn validation_errors_outside_reference_resolution_remain_failures() {
        let server = Server::new(|_| (422, b"private error body".to_vec()));
        let error = server.github.preview("https://github.com/o/r").unwrap_err();
        assert!(!error.contains("private error body"));
        assert!(error.contains("could not provide"));
        assert!(server
            .github
            .bytes(
                &["repos", "o", "r", "git", "blobs", BLOB],
                true,
                MAX_FILE_BYTES
            )
            .is_err());
    }

    #[test]
    fn bounds_response_bytes_and_package_total() {
        let server = Server::new(|_| (200, vec![b'x'; 128]));
        assert!(server
            .github
            .bytes(&["test"], false, 16)
            .unwrap_err()
            .contains("limit"));
        let mut entries = vec![
            json!({"path":"skills/a/SKILL.md","mode":"100644","type":"blob","sha":BLOB,"size":SKILL.len()}),
        ];
        for n in 0..=(MAX_PACKAGE_BYTES / MAX_FILE_BYTES) {
            entries.push(json!({"path":format!("skills/a/{n}"),"mode":"100644","type":"blob","sha":BLOB,"size":MAX_FILE_BYTES}));
        }
        let server = Server::fixture(json!({"tree":entries}));
        assert!(server
            .github
            .download(&source())
            .unwrap_err()
            .contains("8 MiB"));
        assert!(!server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("blobs")));
    }

    #[test]
    fn unavailable_network_and_invalid_pinned_commit_are_actionable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let mut github = Github::new().unwrap();
        github.base = base;
        let error = github.preview("https://github.com/o/r").unwrap_err();
        assert!(error.contains("connection"));
        assert!(!error.contains("127.0.0.1"));
        let server = Server::new(|_| {
            (
                200,
                json!({"sha":"dddddddddddddddddddddddddddddddddddddddd"})
                    .to_string()
                    .into_bytes(),
            )
        });
        assert!(server
            .github
            .download(&source())
            .unwrap_err()
            .contains("Pinned skill commit"));
    }

    #[test]
    fn unrelated_symlinks_and_submodules_do_not_block_selected_skill() {
        let mut data = tree();
        data["tree"].as_array_mut().unwrap().extend([
            json!({"path":"unrelated-link","mode":"120000","type":"blob","sha":BLOB,"size":8}),
            json!({"path":"unrelated-module","mode":"160000","type":"commit","sha":BLOB}),
        ]);
        let server = Server::fixture(data);
        assert!(server.github.download(&source()).is_ok());
    }

    #[test]
    fn preview_checks_selected_package_paths_with_store_validator() {
        let mut data = tree();
        let relative = format!("{}guide.md", "refs/".repeat(33));
        data["tree"].as_array_mut().unwrap().push(json!({"path":format!("skills/a/{relative}"),"mode":"100644","type":"blob","sha":RESOURCE,"size":16}));
        let server = Server::fixture(data);
        assert!(server
            .github
            .preview("https://github.com/o/r/tree/main/skills/a")
            .is_err());
    }

    #[test]
    fn unrelated_windows_unsafe_names_do_not_block_selected_package() {
        let mut data = tree();
        data["tree"].as_array_mut().unwrap().extend([
            json!({"path":"unrelated/CON","mode":"100644","type":"blob","sha":BLOB,"size":8}),
            json!({"path":"unrelated/with:colon","mode":"100644","type":"blob","sha":BLOB,"size":8})
        ]);
        let server = Server::fixture(data);
        assert!(server
            .github
            .preview("https://github.com/o/r/tree/main/skills/a")
            .is_ok());
        assert!(server.github.download(&source()).is_ok());
    }

    #[test]
    fn rejects_windows_ambiguous_components() {
        for path in [
            "skills/.. /SKILL.md",
            "skills/CON",
            "skills/file.",
            "skills/LPT1.txt",
        ] {
            assert!(!safe_path(path, false), "{path}");
        }
    }

    #[test]
    fn rejects_untrusted_download_sources_before_network() {
        for (repository, revision, path) in [
            ("../r", "a".repeat(40), "skill"),
            ("o/r", "main".into(), "skill"),
            ("o/r", "a".repeat(40), "../skill"),
        ] {
            let source = SkillSource {
                repository: repository.into(),
                reference: "main".into(),
                revision,
                path: path.into(),
            };
            assert!(validate_source(&source).is_err());
        }
    }
}

use super::{
    parse_skill, SkillPackage, SkillPreview, SkillSource, MAX_FILES, MAX_FILE_BYTES,
    MAX_PACKAGE_BYTES,
};
use reqwest::blocking::Client;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 10_000;
const MAX_PREVIEWS: usize = 64;
const MAX_URL_BYTES: usize = 4096;
const API_BASE: &str = "https://api.github.com";

struct Location {
    repository: String,
    tail: Vec<String>,
}

fn safe_component(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    !reserved
        && !value.ends_with(['.', ' '])
        && !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '%' | ':' | '?' | '#'))
}

fn safe_path(value: &str, allow_empty: bool) -> bool {
    (allow_empty && value.is_empty())
        || (value.len() <= 2048 && value.split('/').all(safe_component))
}

// Tree paths outside a selected package are GitHub data, not local paths.
// Keep structural traversal checks separate from platform filename restrictions.
fn safe_tree_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn repository_valid(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            safe_component(p)
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

impl Location {
    fn parse(input: &str) -> Result<Self, String> {
        let invalid =
            || "Enter a public HTTPS github.com repository, folder, or SKILL.md URL.".to_string();
        // Validate before URL parsing, which normalizes dot segments.
        if input.len() > MAX_URL_BYTES
            || input.contains('%')
            || input.contains('\\')
            || input.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        let url = Url::parse(input).map_err(|_| invalid())?;
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        let raw_path = input
            .strip_prefix("https://github.com/")
            .ok_or_else(invalid)?;
        let segments: Vec<_> = raw_path.trim_end_matches('/').split('/').collect();
        if segments.len() < 2 || !segments.iter().all(|s| safe_component(s)) {
            return Err(invalid());
        }
        let repository = format!("{}/{}", segments[0], segments[1]);
        if !repository_valid(&repository) {
            return Err(invalid());
        }
        let tail = if segments.len() == 2 {
            Vec::new()
        } else {
            if segments.len() < 4 || !matches!(segments[2], "tree" | "blob") {
                return Err(invalid());
            }
            let mut tail: Vec<String> = segments[3..].iter().map(|s| (*s).into()).collect();
            if segments[2] == "blob" && tail.last().map(String::as_str) != Some("SKILL.md") {
                return Err(invalid());
            }
            if tail.last().map(String::as_str) == Some("SKILL.md") {
                tail.pop();
            }
            if tail.is_empty() {
                return Err(invalid());
            }
            tail
        };
        Ok(Self { repository, tail })
    }
}

fn validate_source(source: &SkillSource) -> Result<(), String> {
    super::validate_source(source)?;
    if !repository_valid(&source.repository)
        || !safe_path(&source.reference, false)
        || !safe_path(&source.path, true)
        || source.revision.len() != 40
        || !source.revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid skill source; preview the public GitHub link again.".into());
    }
    Ok(())
}

struct Github {
    client: Client,
    base: String,
}

#[derive(Deserialize)]
struct Repository {
    default_branch: String,
}
#[derive(Deserialize)]
struct Commit {
    sha: String,
}
#[derive(Deserialize)]
struct Tree {
    tree: Vec<Entry>,
    #[serde(default)]
    truncated: bool,
}
#[derive(Deserialize)]
struct Entry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<usize>,
}

impl Github {
    fn new() -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Atlas-Skill-Importer")
            .build()
            .map_err(|_| "Could not initialize GitHub connection.".to_string())?;
        Ok(Self {
            client,
            base: API_BASE.into(),
        })
    }

    fn bytes(&self, path: &[&str], raw: bool, limit: usize) -> Result<Option<Vec<u8>>, String> {
        self.request_bytes(path, raw, limit, false)
    }

    fn request_bytes(
        &self,
        path: &[&str],
        raw: bool,
        limit: usize,
        missing_reference_is_none: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut url = Url::parse(&self.base)
            .map_err(|_| "Could not initialize GitHub connection.".to_string())?;
        url.path_segments_mut()
            .map_err(|_| "Invalid GitHub connection.".to_string())?
            .extend(path);
        let mut response = self
            .client
            .get(url)
            .header(
                "Accept",
                if raw {
                    "application/vnd.github.raw+json"
                } else {
                    "application/vnd.github+json"
                },
            )
            .send()
            .map_err(|_| "Could not reach GitHub; check your connection and retry.".to_string())?;
        match response.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            // GitHub uses 422 for a nonexistent commit reference, including
            // candidate branch names containing a folder suffix. Only commit
            // reference lookups opt into this interpretation.
            StatusCode::UNPROCESSABLE_ENTITY if missing_reference_is_none => return Ok(None),
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => return Err("GitHub denied the request or its public API rate limit was reached; try again later.".into()),
            status if !status.is_success() => return Err("GitHub could not provide this public repository; check the link and try again.".into()),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err("GitHub skill data exceeds the download limit.".into());
        }
        let mut bytes = Vec::new();
        (&mut response)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "GitHub download was interrupted; retry the preview.".to_string())?;
        if bytes.len() > limit {
            return Err("GitHub skill data exceeds the download limit.".into());
        }
        Ok(Some(bytes))
    }

    fn json<T: serde::de::DeserializeOwned>(&self, path: &[&str]) -> Result<Option<T>, String> {
        self.bytes(path, false, MAX_RESPONSE_BYTES)?
            .map(|b| {
                serde_json::from_slice(&b)
                    .map_err(|_| "GitHub returned invalid repository metadata.".into())
            })
            .transpose()
    }

    fn commit(&self, repository: &str, reference: &str) -> Result<Option<String>, String> {
        let (owner, repo) = repository
            .split_once('/')
            .ok_or("Invalid skill repository.")?;
        let value = self
            .request_bytes(
                &["repos", owner, repo, "commits", reference],
                false,
                MAX_RESPONSE_BYTES,
                true,
            )?
            .map(|bytes| {
                serde_json::from_slice::<Commit>(&bytes)
                    .map_err(|_| "GitHub returned invalid repository metadata.".to_string())
            })
            .transpose()?;
        value
            .map(|c| {
                if c.sha.len() == 40 && c.sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    Ok(c.sha)
                } else {
                    Err("GitHub returned an invalid commit revision.".into())
                }
            })
            .transpose()
    }

    fn resolve(&self, location: Location) -> Result<SkillSource, String> {
        let (owner, repo) = location
            .repository
            .split_once('/')
            .ok_or("Invalid skill repository.")?;
        if location.tail.is_empty() {
            let metadata: Repository = self
                .json(&["repos", owner, repo])?
                .ok_or("Public GitHub repository was not found.")?;
            if !safe_path(&metadata.default_branch, false) {
                return Err("GitHub returned an invalid default branch.".into());
            }
            let revision = self
                .commit(&location.repository, &metadata.default_branch)?
                .ok_or("GitHub default branch was not found.")?;
            return Ok(SkillSource {
                repository: location.repository,
                reference: metadata.default_branch,
                revision,
                path: String::new(),
            });
        }
        // Longest matching ref supports GitHub branches containing slashes.
        // Bound discovery requests even for unusually deep folder URLs.
        if location.tail.len() > 16 {
            return Err("GitHub skill URL has too many path segments.".into());
        }
        for split in (1..=location.tail.len()).rev() {
            let reference = location.tail[..split].join("/");
            if let Some(revision) = self.commit(&location.repository, &reference)? {
                return Ok(SkillSource {
                    repository: location.repository,
                    reference,
                    revision,
                    path: location.tail[split..].join("/"),
                });
            }
        }
        Err("GitHub branch or tag was not found; check the link.".into())
    }

    fn tree(&self, source: &SkillSource) -> Result<Vec<Entry>, String> {
        validate_source(source)?;
        let (owner, repo) = source
            .repository
            .split_once('/')
            .ok_or("Invalid skill repository.")?;
        // A query is added internally; user supplied URL queries are never accepted.
        let mut url =
            Url::parse(&self.base).map_err(|_| "Invalid GitHub connection.".to_string())?;
        url.path_segments_mut()
            .map_err(|_| "Invalid GitHub connection.".to_string())?
            .extend(["repos", owner, repo, "git", "trees", &source.revision]);
        // bytes() encodes path segments; this dedicated fetch keeps the recursive query fixed.
        let tree: Tree = self.tree_response(url)?;
        if tree.truncated || tree.tree.len() > MAX_TREE_ENTRIES {
            return Err(
                "Repository tree is too large to import safely; use a smaller repository.".into(),
            );
        }
        let mut paths = BTreeSet::new();
        for entry in &tree.tree {
            if !safe_tree_path(&entry.path)
                || !paths.insert(&entry.path)
                || entry.sha.len() != 40
                || !entry.sha.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Repository contains an unsafe or duplicate file path.".into());
            }
        }
        Ok(tree.tree)
    }

    fn tree_response(&self, mut url: Url) -> Result<Tree, String> {
        url.query_pairs_mut().append_pair("recursive", "1");
        let response = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|_| "Could not reach GitHub; check your connection and retry.".to_string())?;
        if matches!(
            response.status(),
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
        ) {
            return Err(
                "GitHub public API rate limit was reached or access was denied; try again later."
                    .into(),
            );
        }
        if !response.status().is_success() {
            return Err(
                "GitHub commit or repository was not found; preview the link again.".into(),
            );
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "GitHub download was interrupted.".to_string())?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("Repository tree exceeds the download limit.".into());
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| "GitHub returned invalid repository metadata.".into())
    }

    fn file(&self, source: &SkillSource, entry: &Entry) -> Result<Vec<u8>, String> {
        if entry.size.is_some_and(|s| s > MAX_FILE_BYTES) {
            return Err("A skill file exceeds the 256 KiB limit.".into());
        }
        let (owner, repo) = source
            .repository
            .split_once('/')
            .ok_or("Invalid skill repository.")?;
        self.bytes(
            &["repos", owner, repo, "git", "blobs", &entry.sha],
            true,
            MAX_FILE_BYTES,
        )?
        .ok_or_else(|| "A skill file was not found at the pinned commit.".into())
    }

    fn package_entries<'a>(
        &self,
        source: &SkillSource,
        tree: &'a [Entry],
    ) -> Result<Vec<(&'a Entry, String)>, String> {
        validate_source(source)?;
        let prefix = if source.path.is_empty() {
            String::new()
        } else {
            format!("{}/", source.path)
        };
        for entry in tree
            .iter()
            .filter(|entry| entry.path == source.path || entry.path.starts_with(&prefix))
        {
            if !matches!(
                (entry.kind.as_str(), entry.mode.as_str()),
                ("tree", "040000") | ("blob", "100644" | "100755")
            ) {
                return Err(
                    "Skill folder contains symlinks, submodules, or unsupported file types.".into(),
                );
            }
        }
        let entries: Vec<_> = tree
            .iter()
            .filter(|e| e.kind == "blob")
            .filter_map(|entry| {
                entry
                    .path
                    .strip_prefix(&prefix)
                    .map(|relative| (entry, relative.to_string()))
            })
            .collect();
        if entries.is_empty() || !entries.iter().any(|(_, path)| path == "SKILL.md") {
            return Err("Selected folder does not contain SKILL.md.".into());
        }
        if entries.len() > MAX_FILES {
            return Err("Skill folder exceeds the 256 file limit.".into());
        }
        let mut total = 0usize;
        for (entry, path) in &entries {
            super::validate_path(path)?;
            let size = entry.size.ok_or("GitHub omitted a skill file size.")?;
            if size > MAX_FILE_BYTES {
                return Err("A skill file exceeds the 256 KiB limit.".into());
            }
            total = total.saturating_add(size);
        }
        if total > MAX_PACKAGE_BYTES {
            return Err("Skill folder exceeds the 8 MiB limit.".into());
        }
        Ok(entries)
    }

    fn preview(&self, url: &str) -> Result<Vec<SkillPreview>, String> {
        let source = self.resolve(Location::parse(url)?)?;
        let tree = self.tree(&source)?;
        let prefix = if source.path.is_empty() {
            String::new()
        } else {
            format!("{}/", source.path)
        };
        let candidates: Vec<_> = tree
            .iter()
            .filter(|e| {
                e.kind == "blob"
                    && e.path.starts_with(&prefix)
                    && e.path.rsplit('/').next() == Some("SKILL.md")
            })
            .collect();
        if candidates.is_empty() {
            return Err("No SKILL.md files were found in this GitHub folder.".into());
        }
        if candidates.len() > MAX_PREVIEWS {
            return Err("Too many skills to preview; use a more specific folder URL.".into());
        }
        let mut previews = Vec::new();
        let mut names = BTreeSet::new();
        let mut total = 0usize;
        for entry in candidates {
            let mut selected = source.clone();
            selected.path = entry.path.strip_suffix("/SKILL.md").unwrap_or("").into();
            let files = self
                .package_entries(&selected, &tree)?
                .into_iter()
                .map(|(_, p)| p)
                .collect();
            let bytes = self.file(&selected, entry)?;
            total = total.saturating_add(bytes.len());
            if total > MAX_PACKAGE_BYTES {
                return Err(
                    "Skill previews exceed the 8 MiB limit; select a smaller folder.".into(),
                );
            }
            let content = String::from_utf8(bytes)
                .map_err(|_| "SKILL.md must contain UTF-8 text.".to_string())?;
            let metadata = parse_skill(&content)?;
            if !names.insert(metadata.name.clone()) {
                return Err("Multiple skill folders use the same name; preview a specific skill folder URL.".into());
            }
            previews.push(SkillPreview {
                source: selected,
                name: metadata.name,
                title: metadata.title,
                description: metadata.description,
                content,
                files,
            });
        }
        Ok(previews)
    }

    fn download(&self, source: &SkillSource) -> Result<SkillPackage, String> {
        validate_source(source)?;
        if self
            .commit(&source.repository, &source.revision)?
            .as_deref()
            != Some(source.revision.as_str())
        {
            return Err("Pinned skill commit was not found; preview the GitHub link again.".into());
        }
        let tree = self.tree(source)?;
        let entries = self.package_entries(source, &tree)?;
        let mut files = BTreeMap::new();
        let mut total = 0usize;
        for (entry, path) in entries {
            let bytes = self.file(source, entry)?;
            total = total.saturating_add(bytes.len());
            if total > MAX_PACKAGE_BYTES {
                return Err("Skill folder exceeds the 8 MiB limit.".into());
            }
            files.insert(path, bytes);
        }
        let content = std::str::from_utf8(
            files
                .get("SKILL.md")
                .ok_or("Skill folder is missing SKILL.md.")?,
        )
        .map_err(|_| "SKILL.md must contain UTF-8 text.")?;
        parse_skill(content)?;
        Ok(SkillPackage {
            source: Some(source.clone()),
            files,
        })
    }
}

/// Resolve a public GitHub source to an immutable preview before installation.
pub fn preview(url: &str) -> Result<Vec<SkillPreview>, String> {
    Location::parse(url)?;
    Github::new()?.preview(url)
}

/// Fetch a previously previewed folder at its immutable commit, revalidating all data.
pub fn download(source: &SkillSource) -> Result<SkillPackage, String> {
    validate_source(source)?;
    Github::new()?.download(source)
}
