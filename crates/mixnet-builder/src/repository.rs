//! Source validation and Git metadata discovery. Never modifies a checkout.
use crate::tools;
use anyhow::{Context, Result, bail, ensure};
use mixnet_core::State;
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    pub commit: String,
}
#[derive(Clone, Debug)]
pub struct Repository {
    pub location: String,
    pub local: bool,
    pub head: Option<String>,
    pub tags: Vec<Tag>,
    refs: BTreeMap<String, String>,
}
pub fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
/// Syntactic validation runs before Git, so partial URLs never become path guesses.
pub fn validate_location(value: &str) -> Result<(String, bool)> {
    ensure!(
        !value.is_empty(),
        "Enter a complete HTTPS repository URL or choose a source folder."
    );
    if value.contains("://") || value.starts_with("https:") || value.starts_with("http:") {
        ensure!(
            value.starts_with("https://"),
            "Enter a complete URL beginning with https://."
        );
        let url = url::Url::parse(value).context("Enter a complete HTTPS repository URL.")?;
        ensure!(
            url.scheme() == "https",
            "Only HTTPS repository URLs are supported."
        );
        ensure!(
            url.host_str()
                .is_some_and(|h| h.contains('.') || h == "localhost"),
            "The repository URL needs a valid host."
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "Use a repository URL without embedded credentials."
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "Remove query parameters and fragments from the Git URL."
        );
        ensure!(
            !value.chars().any(char::is_whitespace),
            "Repository URLs cannot contain spaces."
        );
        let segments: Vec<_> = url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        ensure!(
            !segments.is_empty(),
            "The URL must include a repository path, not just a host."
        );
        if matches!(
            url.host_str(),
            Some("github.com" | "gitlab.com" | "bitbucket.org")
        ) {
            ensure!(
                segments.len() >= 2,
                "Include both the repository owner and repository name."
            );
        }
        if matches!(url.host_str(), Some("github.com" | "bitbucket.org")) {
            ensure!(
                segments.len() == 2,
                "Use the repository root URL, not a branch, release or file page."
            );
        }
        ensure!(
            segments.last().is_some_and(|s| *s != ".git"),
            "The repository name is missing."
        );
        return Ok((url.to_string(), false));
    }
    let path = Path::new(value);
    ensure!(
        path.is_dir(),
        "Source folder does not exist or is not a directory. For Git, enter a full https:// URL."
    );
    for file in ["CMakeLists.txt", "src/game/client/gameclient.cpp"] {
        std::fs::File::open(path.join(file))
            .with_context(|| format!("Not a readable DDNet source folder: missing {file}."))?;
    }
    ensure!(
        path.join("src/engine").is_dir(),
        "Not a DDNet source folder: src/engine is missing."
    );
    Ok((path.canonicalize()?.to_string_lossy().into_owned(), true))
}

fn git(state: &State, args: &[&str], allow_empty: bool) -> Result<String> {
    let spec = *tools::TOOLS.iter().find(|s| s.id == "git").unwrap();
    let path = tools::resolve(spec, state)
        .0
        .context("Git is not available. Set its path in Tools.")?;
    let mut child = tools::command(path)
        .args([
            "-c",
            "credential.helper=",
            "-c",
            "core.askPass=",
            "-c",
            "protocol.file.allow=never",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start Git")?;
    let readers: Vec<_> = [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .map(|stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.take(8 * 1024 * 1024).read_to_end(&mut bytes);
            bytes
        })
    })
    .collect();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(20) {
            #[cfg(windows)]
            {
                let _ = tools::command("taskkill.exe")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .output();
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("Git repository check timed out. Check the URL and network, then retry.");
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let mut output = readers
        .into_iter()
        .map(|r| String::from_utf8_lossy(&r.join().unwrap_or_default()).into_owned());
    let stdout = output.next().unwrap_or_default();
    let stderr = output.next().unwrap_or_default();
    ensure!(
        status.success() || (allow_empty && status.code() == Some(1) && stderr.is_empty()),
        "Cannot read Git repository: {}",
        stderr.trim().chars().take(500).collect::<String>()
    );
    Ok(stdout)
}

pub fn inspect(location: &str, state: &State) -> Result<Repository> {
    let (location, local) = validate_location(location)?;
    if local {
        let mut repo = Repository {
            location: location.clone(),
            local,
            head: None,
            tags: vec![],
            refs: BTreeMap::new(),
        };
        if Path::new(&location).join(".git").exists() {
            let head = git(
                state,
                &["-C", &location, "rev-parse", "--verify", "HEAD^{commit}"],
                false,
            )?;
            let head = head.trim();
            ensure!(
                is_commit(head),
                "Local Git checkout has no valid HEAD commit."
            );
            repo.head = Some(head.into());
        }
        return Ok(repo);
    }
    let output = git(
        state,
        &[
            "ls-remote",
            "--",
            &location,
            "HEAD",
            "refs/heads/*",
            "refs/tags/*",
        ],
        false,
    )?;
    let mut repo = parse_refs(&output);
    ensure!(
        repo.head.is_some() || !repo.refs.is_empty(),
        "The URL is not a readable, non-empty Git repository."
    );
    repo.location = location;
    Ok(repo)
}
fn parse_refs(text: &str) -> Repository {
    let mut refs = BTreeMap::new();
    let mut tags = BTreeMap::new();
    let mut peeled = BTreeMap::new();
    let mut head = None;
    for line in text.lines() {
        let mut columns = line.split_whitespace();
        let (Some(hash), Some(name)) = (columns.next(), columns.next()) else {
            continue;
        };
        if !is_commit(hash) {
            continue;
        }
        if name == "HEAD" {
            head = Some(hash.into());
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            if let Some(tag) = tag.strip_suffix("^{}") {
                peeled.insert(tag.to_string(), hash.to_string());
            } else {
                tags.insert(tag.to_string(), hash.to_string());
            }
        } else if let Some(branch) = name.strip_prefix("refs/heads/") {
            refs.insert(branch.into(), hash.into());
        }
    }
    for (tag, hash) in peeled {
        tags.insert(tag, hash);
    }
    let mut tags: Vec<_> = tags
        .into_iter()
        .map(|(name, commit)| {
            refs.insert(name.clone(), commit.clone());
            Tag { name, commit }
        })
        .collect();
    tags.sort_by(|a, b| version_cmp(&b.name, &a.name));
    Repository {
        location: String::new(),
        local: false,
        head,
        tags,
        refs,
    }
}
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    // Natural numeric ordering: 19.10 comes before 19.8 in a newest-first list.
    let parts = |s: &str| {
        let mut result = vec![];
        let mut start = 0;
        for (i, _) in s.char_indices().skip(1) {
            if s.as_bytes()[i].is_ascii_digit() != s.as_bytes()[i - 1].is_ascii_digit() {
                result.push(s[start..i].to_owned());
                start = i;
            }
        }
        result.push(s[start..].to_owned());
        result
    };
    let aa = parts(a);
    let bb = parts(b);
    for (a, b) in aa.iter().zip(&bb) {
        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => a.cmp(b),
        };
        if !order.is_eq() {
            return order;
        }
    }
    aa.len().cmp(&bb.len()).then_with(|| a.cmp(b))
}
impl Repository {
    pub fn resolve(&self, value: &str) -> Result<String> {
        if self.local {
            return Ok(self.head.clone().unwrap_or_default());
        }
        if value.is_empty() || value == "HEAD" {
            return self
                .head
                .clone()
                .context("Select a tag or enter a commit hash.");
        }
        if is_commit(value) {
            return Ok(value.to_ascii_lowercase());
        }
        if let Some(hash) = self.refs.get(value) {
            return Ok(hash.clone());
        }
        if value.len() >= 8 && value.len() < 40 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
            let prefix = value.to_ascii_lowercase();
            let matches: std::collections::BTreeSet<_> = self
                .refs
                .values()
                .chain(self.head.iter())
                .filter(|h| h.starts_with(&prefix))
                .collect();
            if matches.len() == 1 {
                return Ok((*matches.first().unwrap()).clone());
            }
        }
        bail!(
            "Select a tag or enter a full 40-character commit hash. Short hashes must uniquely match an advertised reference."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    #[test]
    #[ignore = "Requires Git and network access to GitHub"]
    fn discovers_real_ddnet_tags() {
        let repo = inspect("https://github.com/ddnet/ddnet.git", &State::default()).unwrap();
        let tag = repo
            .tags
            .iter()
            .find(|tag| tag.name == "19.8")
            .expect("DDNet 19.8 tag");
        assert_eq!(tag.commit, "c8c3a7904d076449054b51976c4a56bd382e85ab");
        assert_eq!(repo.resolve("19.8").unwrap(), tag.commit);
        assert!(repo.head.as_deref().is_some_and(is_commit));
    }
    #[test]
    fn rejects_incomplete_urls() {
        for url in [
            "",
            "https://",
            "https://github.com",
            "https://github.com/ddnet",
            "https://github.com/ddnet/",
            "https://github.com/ddnet/.git",
            "https://github.com/ddnet/ddnet/tree/master",
            "http://github.com/ddnet/ddnet",
            "https://user:pass@github.com/ddnet/ddnet",
            "https://github.com/ddnet/ddnet?tab=readme",
            "github.com/ddnet/ddnet",
        ] {
            assert!(validate_location(url).is_err(), "accepted {url}");
        }
        assert!(validate_location("https://github.com/ddnet/ddnet.git").is_ok());
        assert!(validate_location("https://gitlab.com/group/subgroup/repo.git").is_ok());
    }
    #[test]
    fn requires_a_readable_ddnet_source_tree() {
        let root =
            std::env::temp_dir().join(format!("mixnet-source-validation-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("CMakeLists.txt"), "project(example)").unwrap();
        assert!(validate_location(root.to_str().unwrap()).is_err());
        std::fs::create_dir_all(root.join("src/game/client")).unwrap();
        std::fs::create_dir_all(root.join("src/engine")).unwrap();
        std::fs::write(root.join("src/game/client/gameclient.cpp"), "// source").unwrap();
        assert!(validate_location(root.to_str().unwrap()).unwrap().1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn peels_annotated_tags_and_sorts_versions() {
        let repo = parse_refs(&format!(
            "{A}\tHEAD\n{A}\trefs/tags/19.8\n{A}\trefs/tags/19.10\n{B}\trefs/tags/19.10^{{}}\n{A}\trefs/heads/master\n"
        ));
        assert_eq!(
            repo.tags[0],
            Tag {
                name: "19.10".into(),
                commit: B.into()
            }
        );
        assert_eq!(repo.resolve("19.10").unwrap(), B);
        assert_eq!(repo.resolve("master").unwrap(), A);
        assert_eq!(repo.resolve("bbbbbbbb").unwrap(), B);
        assert!(repo.resolve("not-a-commit").is_err());
        assert_eq!(repo.resolve("").unwrap(), A);
    }
}
