//! Persistent launcher state. No UI or process dependencies.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Client {
    Source { location: String, revision: String },
    Binary { executable: PathBuf },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub client: Client,
    #[serde(default)]
    pub disabled_mods: Vec<String>,
}
impl Instance {
    pub fn new(name: String, client: Client) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            client,
            disabled_mods: vec![],
        }
    }
    pub fn is_source(&self) -> bool {
        matches!(self.client, Client::Source { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub schema_version: u32,
    pub selected: String,
    pub instances: Vec<Instance>,
    #[serde(default)]
    pub tools: BTreeMap<String, PathBuf>,
}
impl Default for State {
    fn default() -> Self {
        let instance = Instance::new(
            "DDNet".into(),
            Client::Source {
                location: "https://github.com/ddnet/ddnet.git".into(),
                revision: "master".into(),
            },
        );
        Self {
            schema_version: 1,
            selected: instance.id.clone(),
            instances: vec![instance],
            tools: BTreeMap::new(),
        }
    }
}
impl State {
    /// Update only editable metadata, retaining identity, selection and enabled mods.
    pub fn edit_instance(&mut self, id: &str, name: String, client: Client) -> Result<bool> {
        if name.trim().is_empty() || name.chars().count() > 80 {
            bail!("Use an instance name between 1 and 80 characters.");
        }
        let instance = self
            .instances
            .iter_mut()
            .find(|i| i.id == id)
            .context("Instance no longer exists")?;
        let changed = instance.client != client;
        instance.name = name;
        instance.client = client;
        Ok(changed)
    }
    pub fn current_opt(&self) -> Option<&Instance> {
        self.instances
            .iter()
            .find(|i| i.id == self.selected)
            .or(self.instances.first())
    }
    pub fn current(&self) -> &Instance {
        self.current_opt()
            .expect("An instance must be selected for this operation")
    }
    pub fn current_mut(&mut self) -> &mut Instance {
        let index = self
            .instances
            .iter()
            .position(|i| i.id == self.selected)
            .unwrap_or(0);
        &mut self.instances[index]
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn roaming() -> Result<Self> {
        let root = std::env::var_os("APPDATA").context("APPDATA is not set")?;
        Ok(Self {
            root: PathBuf::from(root).join("mixnet-launcher"),
        })
    }
    pub fn initialize(&self) -> Result<State> {
        for dir in [
            "instances",
            "mods",
            "sources",
            "workspaces",
            "cache/downloads",
            "cache/builds",
        ] {
            fs::create_dir_all(self.root.join(dir))?;
        }
        let file = self.root.join("instances.json");
        if file.exists() {
            let state: State = serde_json::from_slice(&fs::read(&file)?)
                .context("Cannot read instances.json; existing data was NOT reset")?;
            if state.schema_version != 1 {
                bail!("Unsupported instance database version");
            }
            for i in &state.instances {
                self.prepare(i)?;
            }
            Ok(state)
        } else {
            let state = State::default();
            self.prepare(state.current())?;
            self.save(&state)?;
            Ok(state)
        }
    }
    pub fn mods(&self) -> PathBuf {
        self.root.join("mods")
    }
    pub fn instance(&self, id: &str) -> PathBuf {
        self.root.join("instances").join(id)
    }
    pub fn workspace(&self, id: &str) -> PathBuf {
        self.root.join("workspaces").join(id)
    }
    pub fn build(&self, id: &str) -> PathBuf {
        self.root.join("cache/builds").join(id)
    }
    pub fn prepare(&self, instance: &Instance) -> Result<()> {
        if instance.id.is_empty()
            || !instance
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!("Invalid instance ID");
        }
        for dir in ["client", "user", "logs"] {
            fs::create_dir_all(self.instance(&instance.id).join(dir))?;
        }
        fs::create_dir_all(self.workspace(&instance.id))?;
        Ok(())
    }
    /// Persist the new instance list before deleting any owned files. External
    /// source folders, prebuilt clients and the shared mod catalog are untouched.
    pub fn delete_instance(&self, state: &mut State, id: &str) -> Result<Vec<String>> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            bail!("Invalid instance ID");
        }
        if !state.instances.iter().any(|i| i.id == id) {
            bail!("Instance no longer exists");
        }
        let mut next = state.clone();
        next.instances.retain(|i| i.id != id);
        if next.selected == id || !next.instances.iter().any(|i| i.id == next.selected) {
            next.selected = next
                .instances
                .first()
                .map(|i| i.id.clone())
                .unwrap_or_default();
        }
        self.save(&next)?;
        *state = next;
        let mut warnings = Vec::new();
        for path in [
            self.instance(id),
            self.workspace(id),
            self.build(id),
            self.root.join("sources").join(id),
        ] {
            if let Err(error) = fs::remove_dir_all(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warnings.push(format!("Could not remove {}: {error}", path.display()));
            }
        }
        Ok(warnings)
    }
    pub fn save(&self, state: &State) -> Result<()> {
        atomic_write(
            &self.root.join("instances.json"),
            &serde_json::to_vec_pretty(state)?,
        )
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    use std::io::Write;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path).context("Could not replace state file")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_preserves_identity_mods_and_user_data() {
        let root = std::env::temp_dir().join(format!("mixnet-edit-{}", uuid::Uuid::new_v4()));
        let store = Store { root: root.clone() };
        let mut state = store.initialize().unwrap();
        let id = state.selected.clone();
        state.current_mut().disabled_mods.push("my-mod".into());
        let config = store.instance(&id).join("user/settings_ddnet.cfg");
        fs::write(&config, "player_name Test").unwrap();
        let client = Client::Source {
            location: "https://github.com/ddnet/ddnet.git".into(),
            revision: "c8c3a7904d076449054b51976c4a56bd382e85ab".into(),
        };
        assert!(
            state
                .edit_instance(&id, "DDNet 19.8".into(), client.clone())
                .unwrap()
        );
        assert!(
            !state
                .edit_instance(&id, "Renamed".into(), client.clone())
                .unwrap()
        );
        store.save(&state).unwrap();
        let loaded = store.initialize().unwrap();
        assert_eq!(loaded.selected, id);
        assert_eq!(loaded.instances.len(), 1);
        assert_eq!(loaded.current().client, client);
        assert_eq!(loaded.current().disabled_mods, ["my-mod"]);
        assert_eq!(fs::read_to_string(config).unwrap(), "player_name Test");
        assert!(
            state
                .edit_instance("missing", "Name".into(), client.clone())
                .is_err()
        );
        assert!(state.edit_instance(&id, " ".into(), client).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn deleting_instances_preserves_shared_and_external_files() {
        let root = std::env::temp_dir().join(format!("mixnet-delete-{}", uuid::Uuid::new_v4()));
        let store = Store { root: root.clone() };
        let mut state = store.initialize().unwrap();
        let external_source = root.join("external-source");
        fs::create_dir_all(&external_source).unwrap();
        fs::write(external_source.join("original.cpp"), "original source").unwrap();
        state.current_mut().client = Client::Source {
            location: external_source.to_string_lossy().into_owned(),
            revision: String::new(),
        };
        let external = root.join("external-client.exe");
        fs::write(&external, "original").unwrap();
        let second = Instance::new(
            "External".into(),
            Client::Binary {
                executable: external.clone(),
            },
        );
        store.prepare(&second).unwrap();
        state.instances.push(second.clone());
        fs::write(store.mods().join("keep.txt"), "shared").unwrap();
        let first_id = state.selected.clone();
        for path in [
            store.workspace(&first_id),
            store.build(&first_id),
            root.join("sources").join(&first_id),
        ] {
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("owned"), "owned").unwrap();
        }
        assert!(
            store
                .delete_instance(&mut state, &first_id)
                .unwrap()
                .is_empty()
        );
        assert_eq!(state.selected, second.id);
        assert!(!store.instance(&first_id).exists());
        assert!(!store.workspace(&first_id).exists());
        assert!(!store.build(&first_id).exists());
        assert!(!root.join("sources").join(&first_id).exists());
        assert!(
            store
                .delete_instance(&mut state, &second.id)
                .unwrap()
                .is_empty()
        );
        assert!(state.current_opt().is_none());
        assert!(state.selected.is_empty());
        assert!(store.initialize().unwrap().instances.is_empty());
        assert_eq!(fs::read_to_string(external).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(external_source.join("original.cpp")).unwrap(),
            "original source"
        );
        assert_eq!(
            fs::read_to_string(store.mods().join("keep.txt")).unwrap(),
            "shared"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_delete_save_leaves_instance_data_intact() {
        let root =
            std::env::temp_dir().join(format!("mixnet-delete-failure-{}", uuid::Uuid::new_v4()));
        let store = Store { root: root.clone() };
        let mut state = store.initialize().unwrap();
        let id = state.selected.clone();
        fs::create_dir(root.join("instances.json.tmp")).unwrap();
        assert!(store.delete_instance(&mut state, &id).is_err());
        assert_eq!(state.selected, id);
        assert_eq!(state.instances.len(), 1);
        assert!(store.instance(&id).join("user").is_dir());
        assert!(store.delete_instance(&mut state, "../outside").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn state_roundtrip_and_layout() {
        let root = std::env::temp_dir().join(format!("mixnet-test-{}", uuid::Uuid::new_v4()));
        let store = Store { root: root.clone() };
        let mut state = store.initialize().unwrap();
        state.current_mut().name = "My DDNet".into();
        store.save(&state).unwrap();
        assert_eq!(store.initialize().unwrap().current().name, "My DDNet");
        assert!(store.instance(&state.selected).join("user").is_dir());
        assert!(store.mods().is_dir());
        // Legacy per-instance paths are ignored; the global mod folder is authoritative.
        let mut legacy = serde_json::to_value(&state).unwrap();
        legacy["instances"][0]["mods_root"] = serde_json::json!("old/custom/mods");
        let loaded: State = serde_json::from_value(legacy).unwrap();
        assert!(
            serde_json::to_value(&loaded).unwrap()["instances"][0]
                .get("mods_root")
                .is_none()
        );
        fs::write(root.join("instances.json"), "broken").unwrap();
        assert!(store.initialize().is_err());
        assert_eq!(
            fs::read_to_string(root.join("instances.json")).unwrap(),
            "broken"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
