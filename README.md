# MixNet

A desktop launcher for DDNet source clients and native mixin mods, powered by
[ddmix](../ddnet-mixins). Build a modded client without editing its original
checkout, or launch an existing prebuilt client with a separate user profile.

Built with Rust and GPUI Fast. Fonts and icons are embedded in the executable;
no runtime `assets` folder is required. Build tools and game files are external.
Windows is the currently supported platform.

## Build

Requires Rust/Cargo with the MSVC toolchain, Visual Studio C++ Build Tools and
the Windows SDK.

```sh
cargo build --release --locked -p mixnet-launcher
```

Run `target/release/mixnet-launcher.exe`.

## Usage

The first launch creates a **DDNet** source instance pointing to the official
repository. Source code is downloaded only when you click **Build and play**.

Use **New instance** to add a client:

- **Open source:** enter a complete HTTPS Git repository URL. MixNet validates
  access and loads its tags. Select a tag to fill its commit hash, or enter a
  custom full commit hash. Instances are pinned to that commit; Home shows its
  first eight characters. You can also select a readable local DDNet source
  folder. Local checkouts are used as-is, without switching their Git revision.
- **Prebuilt / closed source:** select the client's executable. Keep its DLLs
  and assets in their original location. Prebuilt instances do not support
  mixin mods and do not need a compiler.

For a source instance, click **Choose folder** to select a directory containing
`<mod>/mixin.toml` packages, or a single package. Toggle the mods you want, then
click **Build and play**. **Refresh** reloads the mod list from disk.

MixNet snapshots enabled packages, runs `ddmix check` and `ddmix apply`, then
configures CMake and builds `game-client` in Release mode. The original source
and mod packages stay unchanged. Build output appears in the console; **Stop**
terminates the active process tree. Full session logs are saved per instance.
Git downloads include submodules; existing checkouts are reused without an
automatic pull. Client dependencies must already be available.

Only build sources and mods you trust: their build scripts and native code run
on your machine. Compatibility depends on the selected client and ddmix version.

## Tools

Open **Tools** to check ddmix, Git, Python, Rust/Cargo, CMake and a C++ compiler.
Ninja and sccache are shown as optional tools. MixNet does not install tools
automatically; detection alone does not verify every required SDK or library.

Use **Set path** to choose an executable, or **Use automatic** to remove an
override. Detection order is manual path, `<TOOL>_EXE`, then `PATH`, for example:

```text
DDMIX_EXE=C:\tools\ddmix.exe
CMAKE_EXE=C:\Program Files\CMake\bin\cmake.exe
```

MSVC is also detected through Visual Studio Installer. ddmix can be placed next
to the launcher; when running from this workspace, the sibling
`../ddnet-mixins/target/release` and `debug` directories are checked as well.
The integration targets the CLI and staging layout of the sibling ddmix project.

## Storage

Application data lives in `%APPDATA%\mixnet-launcher\` (Roaming):

```text
instances.json                 # Instances, selection and tool overrides
instances/<id>/client/         # Built executable, DLLs and assets
instances/<id>/user/           # Per-instance game profile
instances/<id>/logs/
sources/<id>/                  # Downloaded source checkout
workspaces/<id>/               # Selected mod snapshot and ddmix staging
cache/downloads/
cache/builds/<id>/             # CMake build tree
```

Games start with a per-instance `storage.cfg`, with `user/` as the first storage
path. The shared DDNet `$USERDIR` is not included. Custom forks must support
standard DDNet storage behavior for profile isolation to work.

## Development

```sh
cargo test --locked -p mixnet-core -p mixnet-builder
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## License

[MIT](LICENSE). Geist and Geist Mono are distributed under the
[SIL Open Font License](assets/fonts/OFL-Geist.txt).
