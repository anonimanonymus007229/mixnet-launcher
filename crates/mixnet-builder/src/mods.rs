use anyhow::{Context, Result, bail};
use mixnet_core::{Instance, Store};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub enabled: bool,
}
pub fn discover(instance: &Instance, store: &Store) -> Result<Vec<ModInfo>> {
    let root = store.mods();
    let mut dirs = vec![];
    if root.join("mixin.toml").is_file() {
        dirs.push(root.clone());
    } else {
        for e in fs::read_dir(&root).with_context(|| format!("Cannot read {}", root.display()))? {
            let p = e?.path();
            if p.join("mixin.toml").is_file() {
                dirs.push(p);
            }
        }
    }
    dirs.sort();
    let mut result = vec![];
    for dir in dirs {
        let path = dir.join("mixin.toml");
        let manifest: toml::Value = toml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| path.display().to_string())?;
        let package = manifest.get("mod").context("Missing [mod] table")?;
        let id = package
            .get("id")
            .and_then(|v| v.as_str())
            .context("Missing mod.id")?
            .to_owned();
        if result.iter().any(|m: &ModInfo| m.id == id) {
            bail!("Duplicate mod ID: {id}");
        }
        let enabled = package
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
            && !instance.disabled_mods.contains(&id);
        result.push(ModInfo {
            name: package
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_owned(),
            version: package
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_owned(),
            id,
            root: dir,
            enabled,
        });
    }
    Ok(result)
}

/// Never follow links in untrusted mod packages or mutate the originals.
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if fs::symlink_metadata(from)?.file_type().is_symlink() {
        bail!("Symlinks are not supported: {}", from.display());
    }
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let source = e.path();
        let dest = to.join(e.file_name());
        let kind = e.file_type()?;
        if kind.is_symlink() {
            bail!("Symlink in package: {}", source.display());
        }
        if kind.is_dir() {
            if e.file_name() != ".git" {
                copy_tree(&source, &dest)?;
            }
        } else if kind.is_file() {
            fs::copy(source, dest)?;
        }
    }
    Ok(())
}
pub fn snapshot(instance: &Instance, store: &Store, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::create_dir_all(destination)?;
    for m in discover(instance, store)?.into_iter().filter(|m| m.enabled) {
        if m.id.is_empty()
            || !m
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bail!("Invalid mod ID: {}", m.id);
        }
        copy_tree(&m.root, &destination.join(&m.id))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mixnet_core::Client;
    #[test]
    fn selection_does_not_modify_original_manifest() {
        let root = std::env::temp_dir().join(format!("mixnet-mods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = Store { root: root.clone() };
        store.initialize().unwrap();
        fs::create_dir_all(store.mods().join("a")).unwrap();
        let text = "[mod]\nid = 'a'\nname = 'A'\nversion = '1'\n";
        fs::write(store.mods().join("a/mixin.toml"), text).unwrap();
        let mut i = Instance::new(
            "Test".into(),
            Client::Binary {
                executable: "test.exe".into(),
            },
        );
        assert_eq!(discover(&i, &store).unwrap().len(), 1);
        let mut other = i.clone();
        other.id = "another-instance".into();
        i.disabled_mods.push("a".into());
        assert!(!discover(&i, &store).unwrap()[0].enabled);
        assert!(discover(&other, &store).unwrap()[0].enabled);
        snapshot(&other, &store, &root.join("enabled-output")).unwrap();
        assert!(root.join("enabled-output/a/mixin.toml").is_file());
        snapshot(&i, &store, &root.join("output")).unwrap();
        assert!(!root.join("output/a").exists());
        assert_eq!(
            fs::read_to_string(store.mods().join("a/mixin.toml")).unwrap(),
            text
        );
        fs::remove_dir_all(root).unwrap();
    }
}
