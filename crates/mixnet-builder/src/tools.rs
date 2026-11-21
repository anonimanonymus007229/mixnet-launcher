use mixnet_core::State;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct ToolSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub purpose: &'static str,
    pub build: bool,
    pub optional: bool,
    pub candidates: &'static [&'static str],
}
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: "ddmix",
        name: "ddmix",
        purpose: "Mixin validation and staging",
        build: false,
        optional: false,
        candidates: &["ddmix"],
    },
    ToolSpec {
        id: "git",
        name: "Git",
        purpose: "Source checkout and submodules",
        build: false,
        optional: false,
        candidates: &["git"],
    },
    ToolSpec {
        id: "python",
        name: "Python",
        purpose: "DDNet code generation",
        build: true,
        optional: false,
        candidates: &["python", "python3"],
    },
    ToolSpec {
        id: "rustc",
        name: "Rust",
        purpose: "DDNet Rust components",
        build: true,
        optional: false,
        candidates: &["rustc"],
    },
    ToolSpec {
        id: "cargo",
        name: "Cargo",
        purpose: "Build Rust dependencies",
        build: true,
        optional: false,
        candidates: &["cargo"],
    },
    ToolSpec {
        id: "cmake",
        name: "CMake",
        purpose: "Configure and build the client",
        build: true,
        optional: false,
        candidates: &["cmake"],
    },
    ToolSpec {
        id: "compiler",
        name: "C++ compiler",
        purpose: "MSVC / Clang / GCC",
        build: true,
        optional: false,
        candidates: &["cl", "clang++", "g++"],
    },
    ToolSpec {
        id: "ninja",
        name: "Ninja",
        purpose: "Optional build backend",
        build: true,
        optional: true,
        candidates: &["ninja"],
    },
    ToolSpec {
        id: "sccache",
        name: "sccache",
        purpose: "Optional compilation cache",
        build: true,
        optional: true,
        candidates: &["sccache"],
    },
];
#[derive(Clone, Debug)]
pub struct ToolStatus {
    pub id: String,
    pub path: Option<PathBuf>,
    pub origin: String,
    pub version: String,
    pub available: bool,
}

pub fn command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        let p = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        p.is_file().then_some(p)
    })
}
fn visual_cpp() -> Option<PathBuf> {
    let vswhere = PathBuf::from(std::env::var_os("ProgramFiles(x86)")?)
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    let output = command(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    let base = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).join("VC/Tools/MSVC");
    let mut versions: Vec<_> = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .collect();
    versions.sort_by_key(|v| std::cmp::Reverse(v.file_name()));
    versions
        .into_iter()
        .map(|v| v.path().join("bin/Hostx64/x64/cl.exe"))
        .find(|p| p.is_file())
}
pub fn resolve(spec: ToolSpec, state: &State) -> (Option<PathBuf>, String) {
    if let Some(p) = state.tools.get(spec.id) {
        return (Some(p.clone()), "Manual override".into());
    }
    let key = format!("{}_EXE", spec.id.to_ascii_uppercase());
    if let Some(p) = std::env::var_os(&key) {
        return (Some(p.into()), key);
    }
    if let Some(p) = spec.candidates.iter().find_map(|n| on_path(n)) {
        return (Some(p), "PATH".into());
    }
    if spec.id == "compiler"
        && let Some(p) = visual_cpp()
    {
        return (Some(p), "Visual Studio Installer".into());
    }
    if spec.id == "ddmix" {
        if let Ok(exe) = std::env::current_exe() {
            let p = exe.with_file_name("ddmix.exe");
            if p.is_file() {
                return (Some(p), "Beside launcher".into());
            }
        }
        // Convenient when running this workspace next to ddnet-mixins.
        if let Ok(dir) = std::env::current_dir() {
            for profile in ["release", "debug"] {
                let p = dir.join(format!("../ddnet-mixins/target/{profile}/ddmix.exe"));
                if p.is_file() {
                    return (
                        p.canonicalize().ok(),
                        "Sibling ddnet-mixins checkout".into(),
                    );
                }
            }
        }
    }
    (None, "Not found".into())
}
fn version(path: &Path) -> Result<String, String> {
    let msvc = path
        .file_stem()
        .is_some_and(|s| s.eq_ignore_ascii_case("cl"));
    let mut child = command(path)
        .arg(if msvc { "/Bv" } else { "--version" })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    // Drain both pipes while waiting: compiler banners can exceed pipe capacity.
    let readers: Vec<_> = [
        Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .map(|stream| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut data = Vec::new();
            let _ = stream.take(1024 * 1024).read_to_end(&mut data);
            data
        })
    })
    .collect();
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if start.elapsed() > Duration::from_secs(4) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Version check timed out".into());
            }
            _ => std::thread::sleep(Duration::from_millis(30)),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = readers
        .into_iter()
        .filter_map(|r| r.join().ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    if !out.status.success() && !(msvc && text.contains("Microsoft")) {
        return Err(text.trim().chars().take(180).collect());
    }
    Ok(text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("Available")
        .chars()
        .take(140)
        .collect())
}
pub fn scan(state: &State) -> Vec<ToolStatus> {
    // Independent probes run in parallel; no probing on the UI thread.
    std::thread::scope(|s| {
        let jobs: Vec<_> = TOOLS
            .iter()
            .map(|spec| {
                s.spawn(move || {
                    let (path, origin) = resolve(*spec, state);
                    let result = path.as_deref().map(version);
                    let available = matches!(&result, Some(Ok(_)));
                    let version = match result {
                        Some(Ok(v)) => v,
                        Some(Err(e)) => e,
                        None => "Choose an executable or add it to PATH".into(),
                    };
                    ToolStatus {
                        id: spec.id.into(),
                        path,
                        origin,
                        version,
                        available,
                    }
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    })
}

pub fn environment(state: &State) -> Vec<(String, std::ffi::OsString)> {
    let resolved: Vec<_> = TOOLS
        .iter()
        .filter_map(|s| resolve(*s, state).0.map(|p| (*s, p)))
        .collect();
    let mut dirs: Vec<_> = resolved
        .iter()
        .filter_map(|(_, p)| p.parent().map(Path::to_path_buf))
        .collect();
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut vars = vec![(
        "PATH".into(),
        std::env::join_paths(dirs).unwrap_or_default(),
    )];
    for (spec, path) in resolved {
        if spec.id == "python" {
            vars.push(("PYTHON".into(), path.into_os_string()));
        }
    }
    vars
}
