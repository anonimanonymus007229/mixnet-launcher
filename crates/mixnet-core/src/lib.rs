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
    pub fn current(&self) -> &Instance {
        self.instances
            .iter()
            .find(|i| i.id == self.selected)
            .unwrap_or(&self.instances[0])
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
            if state.schema_version != 1 || state.instances.is_empty() {
                bail!("Unsupported or empty instance database");
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
