//! Keep the repository identity in the checkout basename: ddmix's `clients`
//! filters intentionally match that basename, not the launcher's instance UUID.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn client_folder(location: &str) -> Result<String> {
    let url = url::Url::parse(location).context("Invalid repository URL")?;
    let parts: Vec<_> = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    let repository = parts
        .last()
        .context("Repository URL has no name")?
        .trim_end_matches(".git");
    // Forks named ddnet need their owner to retain their distinct client identity.
    let name = if repository.eq_ignore_ascii_case("ddnet")
        && parts.len() >= 2
        && !parts[parts.len() - 2].eq_ignore_ascii_case("ddnet")
    {
        format!("{}-{repository}", parts[parts.len() - 2])
    } else {
        repository.to_owned()
    };
    ensure!(
        !name.is_empty()
            && name.len() <= 100
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && !name.starts_with('.')
            && !name.ends_with('.'),
        "Repository name is not a safe client folder name"
    );
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    ensure!(
        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9')),
        "Repository name is reserved on Windows"
    );
    Ok(name)
}

pub(crate) fn directory(root: &Path, location: &str) -> Result<PathBuf> {
    let destination = root.join(client_folder(location)?);
    if root.join(".git").exists() {
        // Older launchers cloned directly into sources/<instance-id>. Move the
        // complete checkout (including .git and submodules), without re-cloning.
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let backup = root.with_file_name(format!(
            "{}.migrating-{nonce}",
            root.file_name()
                .context("Missing instance source folder")?
                .to_string_lossy()
        ));
        fs::rename(root, &backup)
            .context("Cannot relocate legacy source checkout; close programs using it and retry")?;
        let moved = fs::create_dir(root).and_then(|()| fs::rename(&backup, &destination));
        if let Err(error) = moved {
            let _ = fs::remove_dir(root); // Only removes an empty parent; never delete checkout contents.
            fs::rename(&backup, root).with_context(|| {
                format!(
                    "Migration failed ({error}); original checkout is preserved at {}",
                    backup.display()
                )
            })?;
            return Err(error)
                .context("Could not migrate the source checkout; original layout restored");
        }
    } else {
        fs::create_dir_all(root)?;
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derives_client_identity_from_repository_not_instance_name() {
        assert_eq!(
            client_folder("https://github.com/TaterClient/TClient").unwrap(),
            "TClient"
        );
        assert_eq!(
            client_folder("https://github.com/TaterClient/TClient.git/").unwrap(),
            "TClient"
        );
        assert_eq!(
            client_folder("https://github.com/ddnet/ddnet.git").unwrap(),
            "ddnet"
        );
        assert_eq!(
            client_folder("https://github.com/RushieClient/ddnet.git").unwrap(),
            "RushieClient-ddnet"
        );
        assert!(client_folder("https://example.com/user/CON.git").is_err());
        assert!(client_folder("https://example.com/user/%2Fescape.git").is_err());
    }
    #[test]
    fn migrates_legacy_checkout_without_losing_git_or_submodules() {
        let root =
            std::env::temp_dir().join(format!("mixnet-checkout-migration-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let instance = root.join("instance-uuid");
        fs::create_dir_all(instance.join(".git/modules/ddnet-libs")).unwrap();
        fs::create_dir_all(instance.join("ddnet-libs")).unwrap();
        fs::write(instance.join(".git/HEAD"), "original head").unwrap();
        fs::write(
            instance.join("ddnet-libs/.git"),
            "gitdir: ../.git/modules/ddnet-libs",
        )
        .unwrap();
        fs::write(instance.join("CMakeLists.txt"), "original source").unwrap();
        let path = directory(&instance, "https://github.com/TaterClient/TClient.git").unwrap();
        assert_eq!(path, instance.join("TClient"));
        assert_eq!(
            fs::read_to_string(path.join(".git/HEAD")).unwrap(),
            "original head"
        );
        assert_eq!(
            fs::read_to_string(path.join("ddnet-libs/.git")).unwrap(),
            "gitdir: ../.git/modules/ddnet-libs"
        );
        assert_eq!(
            fs::read_to_string(path.join("CMakeLists.txt")).unwrap(),
            "original source"
        );
        assert_eq!(
            directory(&instance, "https://github.com/TaterClient/TClient.git").unwrap(),
            path
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
