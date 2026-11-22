use crate::{mods, tools};
use anyhow::{Context, Result, bail};
use mixnet_core::{Client, Instance, State, Store};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

pub enum Event {
    Log(String),
    Phase(String),
    Finished(Result<(), String>),
}
pub fn start(store: Store, state: State) -> (Receiver<Event>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    std::thread::spawn(move || {
        let result = run(&store, &state, &tx, &stop).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::Finished(result));
    });
    (rx, cancel)
}
fn phase(tx: &Sender<Event>, text: &str) {
    let _ = tx.send(Event::Phase(text.into()));
}
fn tool(state: &State, id: &str) -> Result<PathBuf> {
    tools::resolve(*tools::TOOLS.iter().find(|s| s.id == id).unwrap(), state)
        .0
        .filter(|p| p.is_file())
        .with_context(|| format!("{id} is not available. Open Tools and choose its executable."))
}
fn execute(mut cmd: Command, log: &Path, tx: &Sender<Event>, cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Cancelled");
    }
    let _ = tx.send(Event::Log(format!("> {cmd:?}")));
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start process")?;
    let file = Arc::new(std::sync::Mutex::new(
        fs::OpenOptions::new().create(true).append(true).open(log)?,
    ));
    let mut readers = vec![];
    for stream in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let tx = tx.clone();
        let file = file.clone();
        readers.push(std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut bytes = vec![];
            loop {
                bytes.clear();
                match reader.read_until(b'\n', &mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(mut f) = file.lock() {
                            let _ = f.write_all(&bytes);
                        }
                        let _ = tx.send(Event::Log(
                            String::from_utf8_lossy(&bytes).trim_end().into(),
                        ));
                    }
                }
            }
        }));
    }
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            #[cfg(windows)]
            {
                let _ = tools::command("taskkill.exe")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .output();
            }
            let _ = child.kill();
            let _ = child.wait();
            for reader in readers {
                let _ = reader.join();
            }
            bail!("Cancelled");
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(80));
    };
    for reader in readers {
        let _ = reader.join();
    }
    if !status.success() {
        bail!("Process exited with {status}. See the build log for details.");
    }
    Ok(())
}
fn configured(path: &Path, state: &State) -> Command {
    let mut cmd = tools::command(path);
    cmd.envs(tools::environment(state));
    cmd
}
fn run(store: &Store, state: &State, tx: &Sender<Event>, cancel: &AtomicBool) -> Result<()> {
    let i = state.current_opt().context("No instance selected")?;
    store.prepare(i)?;
    let log = store.instance(&i.id).join("logs/session.log");
    fs::write(&log, format!("MixNet session — {}\n", i.name))?;
    let executable = match &i.client {
        Client::Binary { executable } => executable
            .canonicalize()
            .context("Client executable no longer exists")?,
        Client::Source { location, revision } => {
            phase(tx, "Preparing source");
            crate::repository::validate_location(location)?;
            let ddmix = tool(state, "ddmix")?;
            tool(state, "cmake")?;
            let source = if Path::new(location).is_dir() {
                Path::new(location).canonicalize()?
            } else {
                if !location.starts_with("https://") {
                    bail!("Choose a local folder or an HTTPS Git URL");
                }
                let git = tool(state, "git")?;
                let dest =
                    crate::checkout::directory(&store.root.join("sources").join(&i.id), location)?;
                if !dest.exists() {
                    let mut cmd = configured(&git, state);
                    cmd.args(["clone", "--recursive", "--", location])
                        .arg(&dest);
                    execute(cmd, &log, tx, cancel)?;
                }
                update_checkout(state, &dest, location, revision, &log, tx, cancel)?;
                dest
            };
            if !source.join("CMakeLists.txt").is_file() {
                bail!("Source folder has no CMakeLists.txt");
            }
            let workspace = store.workspace(&i.id);
            let selected = workspace.join("selected-mods");
            phase(tx, "Preparing selected mods");
            mods::snapshot(i, store, &selected)?;
            phase(tx, "Validating mixins");
            let mut check = configured(&ddmix, state);
            check
                .args(["--color", "never", "check", "--ddnet"])
                .arg(&source)
                .arg("--mods")
                .arg(&selected);
            execute(check, &log, tx, cancel)?;
            phase(tx, "Staging · original source stays untouched");
            let mut apply = configured(&ddmix, state);
            apply
                .args(["--color", "never", "apply", "--workspace"])
                .arg(&workspace)
                .arg("--ddnet")
                .arg(&source)
                .arg("--mods")
                .arg(&selected);
            execute(apply, &log, tx, cancel)?;
            let staged = workspace.join(".ddmix/staging/ddnet");
            anyhow::ensure!(
                staged.join("CMakeLists.txt").is_file(),
                "ddmix did not produce the expected staging directory"
            );
            phase(tx, "Configuring · Release");
            let build = store.build(&i.id);
            let cmake = tool(state, "cmake")?;
            let mut configure = configured(&cmake, state);
            configure
                .arg("-S")
                .arg(cmake_path(&staged))
                .arg("-B")
                .arg(cmake_path(&build))
                .arg(format!("-DDDMIX_WORKSPACE_ROOT={}", cmake_path(&workspace)))
                .args([
                    "-DSERVER=OFF",
                    "-DTOOLS=OFF",
                    "-DDOWNLOAD_GTEST=OFF",
                    "-DCMAKE_BUILD_TYPE=Release",
                ])
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("GIT_CEILING_DIRECTORIES", workspace.join(".ddmix/staging"));
            for (id, key) in [
                ("python", "Python_EXECUTABLE"),
                ("python", "Python3_EXECUTABLE"),
                ("python", "PYTHON_EXECUTABLE"),
                ("rustc", "RUST_RUSTC"),
                ("cargo", "RUST_CARGO"),
                ("git", "GIT_EXECUTABLE"),
            ] {
                if let Ok(path) = tool(state, id) {
                    configure.arg(format!("-D{key}={}", cmake_path(&path)));
                }
            }
            if let Some(path) = state.tools.get("compiler") {
                configure.arg(format!("-DCMAKE_CXX_COMPILER={}", cmake_path(path)));
            }
            execute(configure, &log, tx, cancel)?;
            phase(tx, "Compiling · Release");
            let mut compile = configured(&cmake, state);
            compile
                .arg("--build")
                .arg(cmake_path(&build))
                .args([
                    "--target",
                    "game-client",
                    "--config",
                    "Release",
                    "--parallel",
                ])
                .arg(
                    std::thread::available_parallelism()
                        .map_or(2, usize::from)
                        .to_string(),
                );
            execute(compile, &log, tx, cancel)?;
            phase(tx, "Preparing client files");
            publish(store, i, &build, &source)?
        }
    };
    phase(tx, "Running · close the game to return");
    launch(store, i, state, &executable, &log, tx, cancel)?;
    Ok(())
}
fn update_checkout(
    state: &State,
    dest: &Path,
    location: &str,
    revision: &str,
    log: &Path,
    tx: &Sender<Event>,
    cancel: &AtomicBool,
) -> Result<()> {
    let git = tool(state, "git")?;
    let revision = if revision.trim().is_empty() {
        "HEAD"
    } else {
        revision
    };
    anyhow::ensure!(
        !revision.starts_with('-') && !revision.contains(['\r', '\n']),
        "Invalid revision"
    );
    let mut remote = configured(&git, state);
    remote
        .arg("-C")
        .arg(dest)
        .args(["remote", "set-url", "origin", location]);
    execute(remote, log, tx, cancel)?;
    let pinned = crate::repository::is_commit(revision);
    let available = pinned
        && configured(&git, state)
            .arg("-C")
            .arg(dest)
            .args(["cat-file", "-e", &format!("{revision}^{{commit}}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success();
    if !available {
        let mut fetch = configured(&git, state);
        fetch.arg("-C").arg(dest).args([
            "fetch",
            "--recurse-submodules=no",
            "--",
            "origin",
            revision,
        ]);
        execute(fetch, log, tx, cancel)?;
    }
    let mut checkout = configured(&git, state);
    checkout.arg("-C").arg(dest).args([
        "checkout",
        "--detach",
        if pinned { revision } else { "FETCH_HEAD" },
    ]);
    execute(checkout, log, tx, cancel)?;
    for args in [
        &["submodule", "sync", "--recursive"][..],
        &["submodule", "update", "--init", "--recursive"][..],
    ] {
        let mut submodule = configured(&git, state);
        submodule.arg("-C").arg(dest).args(args);
        execute(submodule, log, tx, cancel)?;
    }
    Ok(())
}
fn cmake_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    path.strip_prefix("//?/").unwrap_or(&path).to_owned()
}
fn publish(store: &Store, i: &Instance, build: &Path, source: &Path) -> Result<PathBuf> {
    let candidates = [
        "DDNet.exe",
        "Release/DDNet.exe",
        "ddnet.exe",
        "Release/ddnet.exe",
        "DDNet",
        "ddnet",
    ];
    let exe = candidates
        .iter()
        .map(|p| build.join(p))
        .find(|p| p.is_file())
        .context("Build succeeded, but DDNet executable was not found")?;
    let dest = store.instance(&i.id).join("client");
    let final_exe = dest.join(exe.file_name().unwrap());
    fs::copy(&exe, &final_exe)?;
    for root in [build, exe.parent().unwrap()] {
        for e in fs::read_dir(root)? {
            let path = e?.path();
            if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("dll"))
            {
                fs::copy(&path, dest.join(path.file_name().unwrap()))?;
            }
        }
    }
    // Data may be a CMake-generated link. Read its canonical target, never create links in the instance.
    let data = if build.join("data").is_dir() {
        build.join("data").canonicalize()?
    } else {
        source.join("data")
    };
    if data.is_dir() {
        mods::copy_tree(&data, &dest.join("data"))?;
    }
    Ok(final_exe)
}
fn launch(
    store: &Store,
    i: &Instance,
    state: &State,
    exe: &Path,
    log: &Path,
    tx: &Sender<Event>,
    cancel: &AtomicBool,
) -> Result<()> {
    execute(client_command(store, i, state, exe)?, log, tx, cancel)
}

fn client_command(store: &Store, i: &Instance, state: &State, exe: &Path) -> Result<Command> {
    let client = exe.parent().context("Executable has no parent folder")?;
    let mut cmd = configured(exe, state);
    if !i.isolate_config {
        // Preserve the client's native storage.cfg / $USERDIR behavior. Do not
        // read, overwrite or remove the instance's previous isolated settings.
        cmd.current_dir(client);
        return Ok(cmd);
    }
    let user = store.instance(&i.id).join("user").canonicalize()?;
    // DDNet reads storage.cfg in cwd first. The first path is the save path.
    // Never add $USERDIR: no reads/writes to the shared APPDATA/DDNet profile.
    let paths = [user.clone(), client.to_path_buf(), client.join("data")];
    let mut storage = String::new();
    for p in paths {
        let p = p.to_string_lossy().replace('\\', "/");
        // Strip Windows extended path prefix, unsupported by older client storage code.
        let p = p.strip_prefix("//?/").unwrap_or(&p);
        if p.contains(['\n', '\r']) {
            bail!("Invalid storage path");
        }
        storage.push_str(&format!("add_path {p}\n"));
    }
    fs::write(user.join("storage.cfg"), storage)?;
    cmd.current_dir(user);
    Ok(cmd)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn edited_commit_and_repository_refresh_existing_checkout() {
        let root =
            std::env::temp_dir().join(format!("mixnet-checkout-edit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for folder in ["a", "b", "no-hooks"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        let state = State::default();
        let git = tool(&state, "git").unwrap();
        let hooks = format!("core.hooksPath={}", root.join("no-hooks").display());
        let run = |dir: &Path, args: &[&str]| {
            let output = configured(&git, &state)
                .current_dir(dir)
                .args([
                    "-c",
                    "user.name=MixNet Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    &hooks,
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        let a = root.join("a");
        let b = root.join("b");
        let checkout = root.join("checkout");
        run(&a, &["init", "--quiet"]);
        fs::write(a.join("version.txt"), "first").unwrap();
        run(&a, &["add", "."]);
        run(&a, &["commit", "--quiet", "-m", "first"]);
        run(
            &root,
            &[
                "clone",
                "--quiet",
                "--no-hardlinks",
                a.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
        );
        fs::write(a.join("version.txt"), "new commit").unwrap();
        run(&a, &["add", "."]);
        run(&a, &["commit", "--quiet", "-m", "second"]);
        let revision = run(&a, &["rev-parse", "HEAD"]);
        let (tx, _rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let log = root.join("build.log");
        update_checkout(
            &state,
            &checkout,
            a.to_str().unwrap(),
            &revision,
            &log,
            &tx,
            &cancel,
        )
        .unwrap();
        assert_eq!(run(&checkout, &["rev-parse", "HEAD"]), revision);
        assert_eq!(
            fs::read_to_string(checkout.join("version.txt")).unwrap(),
            "new commit"
        );
        run(&b, &["init", "--quiet"]);
        fs::write(b.join("version.txt"), "other repository").unwrap();
        run(&b, &["add", "."]);
        run(&b, &["commit", "--quiet", "-m", "other"]);
        let revision = run(&b, &["rev-parse", "HEAD"]);
        update_checkout(
            &state,
            &checkout,
            b.to_str().unwrap(),
            &revision,
            &log,
            &tx,
            &cancel,
        )
        .unwrap();
        assert_eq!(run(&checkout, &["rev-parse", "HEAD"]), revision);
        assert_eq!(
            run(&checkout, &["remote", "get-url", "origin"]),
            b.to_str().unwrap()
        );
        assert_eq!(
            fs::read_to_string(checkout.join("version.txt")).unwrap(),
            "other repository"
        );
        assert_eq!(
            fs::read_to_string(a.join("version.txt")).unwrap(),
            "new commit"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn streams_stdout_stderr_and_reports_failure() {
        let log = std::env::temp_dir().join(format!("mixnet-process-{}.log", std::process::id()));
        let (tx, rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let mut cmd = tools::command("cmd.exe");
        cmd.args(["/D", "/C", "echo out & echo err 1>&2"]);
        execute(cmd, &log, &tx, &cancel).unwrap();
        let lines: Vec<_> = rx
            .try_iter()
            .filter_map(|e| if let Event::Log(l) = e { Some(l) } else { None })
            .collect();
        assert!(lines.iter().any(|l| l == "out" || l.trim() == "out"));
        assert!(lines.iter().any(|l| l.trim() == "err"));
        let mut cmd = tools::command("cmd.exe");
        cmd.args(["/D", "/C", "exit 7"]);
        assert!(execute(cmd, &log, &tx, &cancel).is_err());
        fs::remove_file(log).unwrap();
    }
    #[test]
    fn cancelled_job_never_starts() {
        let (tx, _) = mpsc::channel();
        let cancel = AtomicBool::new(true);
        assert!(
            execute(
                tools::command("nonexistent-tool"),
                Path::new("unused.log"),
                &tx,
                &cancel
            )
            .unwrap_err()
            .to_string()
            .contains("Cancelled")
        );
    }
    #[test]
    fn storage_configuration_does_not_reference_shared_userdir() {
        let root = std::env::temp_dir().join(format!("mixnet-storage-test-{}", std::process::id()));
        let store = Store { root: root.clone() };
        let mut state = store.initialize().unwrap();
        state.current_mut().isolate_config = true;
        let exe = root.join("client/DDNet.exe");
        let cmd = client_command(&store, state.current(), &state, &exe).unwrap();
        let user = store
            .instance(&state.selected)
            .join("user")
            .canonicalize()
            .unwrap();
        assert_eq!(cmd.get_current_dir(), Some(user.as_path()));
        let cfg = fs::read_to_string(user.join("storage.cfg")).unwrap();
        assert!(!cfg.contains("$USERDIR"));
        assert!(!cfg.contains("//?/"));
        assert!(cfg.lines().next().unwrap().ends_with("/user"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_storage_is_default_and_toggling_preserves_both_profiles() {
        let root =
            std::env::temp_dir().join(format!("mixnet-native-storage-{}", std::process::id()));
        let store = Store { root: root.clone() };
        let mut state = store.initialize().unwrap();
        let client = root.join("external-client");
        fs::create_dir_all(&client).unwrap();
        let exe = client.join("DDNet.exe");
        let native_config = client.join("storage.cfg");
        fs::write(&native_config, "add_path $USERDIR\n").unwrap();
        let user = store.instance(&state.selected).join("user");
        let isolated_storage = user.join("storage.cfg");
        let settings = user.join("settings_ddnet.cfg");
        fs::write(&settings, "player_name Isolated").unwrap();
        let cmd = client_command(&store, state.current(), &state, &exe).unwrap();
        assert_eq!(cmd.get_current_dir(), Some(client.as_path()));
        assert!(!isolated_storage.exists());
        state.current_mut().isolate_config = true;
        client_command(&store, state.current(), &state, &exe).unwrap();
        let previous = fs::read(&isolated_storage).unwrap();
        state.current_mut().isolate_config = false;
        let cmd = client_command(&store, state.current(), &state, &exe).unwrap();
        assert_eq!(cmd.get_current_dir(), Some(client.as_path()));
        assert_eq!(fs::read(&isolated_storage).unwrap(), previous);
        state.current_mut().isolate_config = true;
        client_command(&store, state.current(), &state, &exe).unwrap();
        assert_eq!(fs::read(&isolated_storage).unwrap(), previous);
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "player_name Isolated"
        );
        assert_eq!(
            fs::read_to_string(&native_config).unwrap(),
            "add_path $USERDIR\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
