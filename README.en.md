<p align="center">
  <img src="assets/icons/packporter.png" width="104" height="104" alt="PackPorter icon: a grass-block parcel with a transfer arrow">
</p>

<h1 align="center">PackPorter</h1>

<p align="center"><a href="README.md">简体中文</a> · <strong>English</strong></p>

<p align="center"><strong>A new modpack. Your world comes with you.</strong></p>
<p align="center">A migration tool for Minecraft Java Edition modpacks · Saves, settings, waypoints, and mod data</p>

<p align="center">
  <a href="#getting-started">Getting started</a> ·
  <a href="#what-gets-migrated">Migration rules</a> ·
  <a href="#backups-and-limitations">Backups and limitations</a> ·
  <a href="#development">Development</a>
</p>

---

When upgrading a modpack, you want to keep your progress while respecting the new pack's configuration. PackPorter copies files, adds missing assets, or merges allowed settings according to asset type. Review the migration plan before confirming any changes.

<p align="center">
  <img src="assets/readme/workflow.en.svg" width="100%" alt="Migration workflow: select the old and new instances, review the plan, then confirm the transfer of personal assets.">
</p>

- **Review before migrating**: selecting a source and target automatically lists assets with copy, keep, and merge statistics.
- **Keep your preferences**: merge eligible key bindings, audio, graphics, and language settings while preserving key bindings introduced by the new pack.
- **Choose what moves**: toggle each of the four migration levels, add or edit paths, and enable or disable individual rules.

## Getting started

Download the Windows x64 archive from [Releases](https://github.com/Mowonhua/PackPorter/releases), extract all files, and run `packporter.exe`. Keep `packporter-shim.exe` in the same directory. The current version, `v0.1.0-alpha.2`, is a prerelease. A SHA-256 checksum file is included on the release page.

To build from source, the instructions below target **Windows** and require Rust/Cargo, a linker for your Rust toolchain, and Windows resource compilation tools.

From the repository root:

```powershell
cargo run
```

The current UI uses Chinese labels; they are included below to help you find each control.

1. Open **Settings (设置)** in the top-right corner, select your launcher's `.minecraft/versions` directory, and save.
2. Return to the main page. Choose the old version as the **source instance (源实例)** and the new version as the **target instance (目标实例)**. Scan again when you need to refresh the list.
3. Review the automatically generated **migration plan (迁移计划)**. Adjust migration rules and the backup toggle in Settings as needed.
4. Close the relevant game processes, click **Start migration (开始迁移)**, and check the result. Use **Open backup folder (打开备份目录)** to inspect backup files.

To build a standalone executable:

```powershell
cargo build --release
```

The Windows executables are `target/release/packporter.exe` and `target/release/packporter-shim.exe`; distribute them in the same directory. Both have embedded icons, so no separate image files need to be distributed.

Select **关联启动器…** in Settings, choose a PCL2/HMCL executable, enable following, and save. Selection remains a draft until saved. Installation preserves the original as `xxx.bak.exe` in the same directory and places the shim at `xxx.exe`, so existing shortcuts, direct double-clicks, and command lines activate following. Multiple launchers are supported; PackPorter closes after the last linked launcher exits, waiting for migration and settings saves.

**Disabling following and saving restores every linked executable automatically.** Selected paths are retained for re-enabling; removing one path and saving restores that entry. Existing `.bak.exe` files, locked files, or externally updated launchers produce an error instead of overwriting unknown files with an old backup. Remove the association before moving either application.

Keep `packporter-shim.exe` beside `packporter.exe`. The shim at the original path exits immediately after handing off to the central shim, which runs only during launcher sessions. There is no sign-in entry or idle monitor. HMCL's Java launcher descendants remain tracked; restoring executable paths does not terminate launchers or Minecraft games.

Original-path installation supports EXE files. JAR launchers can still use the standalone command line, with `javaw.exe` from `PATH` by default. Arguments after `--` are forwarded to the launcher:

```powershell
.\packporter-shim.exe --launcher "D:\Minecraft\HMCL.jar" --java "D:\Java\bin\javaw.exe" -- <launcher arguments>
```

## What gets migrated

These are the defaults for first use. You can edit paths through **Configure (配置)** for each level in Settings.

| Assets | Behavior |
| --- | --- |
| **L1 · Personal files** | Copy saves, server lists, screenshots, and schematics |
| **L2 · Resource and shader packs** | Add missing files; keep the target's version at matching paths |
| **L3 · Mod data** | Copy directories containing maps, waypoints, and other personal data |
| **L4 · Preferences** | Merge allowed key-value pairs in `options.txt` |

For example, if a resource pack file already exists at the same path in the target, the target version is kept. Eligible preferences in `options.txt` favor the old values. L1/L3 overwrite files at matching paths but do not delete files that exist only in the target.

<details>
<summary><strong>Default path examples</strong></summary>

- **L1**: `saves/`, `servers.dat`, `screenshots/`, `schematics/`.
- **L2**: `resourcepacks/`, `shaderpacks/`.
- **L3**: `xaero/`, `journeymap/`, `config/xaero/`, `config/jei/world/`, `local/`, and others.
- **L4**: `options.txt`.

</details>

<details>
<summary><strong>How preferences are merged</strong></summary>

- **Initialization**: if the target `options.txt` does not exist, create it with only eligible preferences and key bindings. You do not need to launch the new modpack first. An existing empty file still follows the merge path.
- **Read failures**: only `NotFound` triggers initialization. Other read errors, including permission and encoding errors, cause planning to fail.
- **Key bindings (`key_*`)**: old values take precedence for matching keys. Old bindings absent from the target are retained and counted as “unverified key bindings.” Bindings introduced by the new pack remain unchanged.
- **Preferences**: allowed settings such as `soundCategory_*`, `fov`, `gamma`, `guiScale`, `renderDistance`, `maxFps`, and language favor old values. Valid values present only in the old file are added to the target.
- **Other old keys**: keys outside the allowed rules are not migrated.
- **Format validation**: some numeric preferences receive basic format checks. Invalid values leave the target value unchanged; if the target has no such key, nothing is written. These checks do not guarantee valid ranges or compatibility across versions.
- **Preview**: shows initialization or merge mode, the total number of settings, and the number of unverified key bindings.

Unverified bindings may not be supported by the target game or mods. Conversion of key names and formats between Minecraft versions is not implemented.

</details>

## Backups and limitations

**Automatic backups are enabled by default and can be turned off in Settings.** When enabled, existing target files about to be overwritten are saved to `backups/<timestamp>-pre-migrate.zip` inside the target instance before writing. This is not a complete instance backup. No ZIP is created if no existing files need to be overwritten.

If migration fails, PackPorter attempts to restore overwritten files and remove newly created items in reverse order. Rollback relies on process memory and may partially fail; automatic recovery after a process crash is not guaranteed. Check the actual result report.

- **Running instances**: migration is blocked when a related Java game process is detected. Detection looks for the instance path in process command lines; it is not an operating-system file lock.
- **Instance detection**: parses version JSON and inheritance chains, recognizes NeoForge, Forge, Fabric, and Quilt, and reads the `clientVersion` field used by PCL2/HMCL.
- **Compatibility**: recognizing Minecraft and loader versions does not establish that saves or mod data are compatible across versions. PackPorter migrates assets; it does not convert save or mod data formats.

## Development

GitHub Actions tests and builds Windows x64 packages on pushes to `main`, pull requests, and manual runs. Packages are available in each run's Artifacts section for 14 days. CI uses Rust 1.93.0 and the dependencies pinned in `Cargo.lock`.

To publish, update the version in `Cargo.toml` and `Cargo.lock`, then push a matching `v<version>` tag. After tests and builds pass, the workflow creates a GitHub Release with the archive and checksum. Versions with a suffix such as `-alpha`, `-beta`, or `-rc` are marked as prereleases. Release notes come from `docs/releases/<tag>.md` when present, or are generated by GitHub otherwise.

Built with **Rust + Slint**, with domain, service, and infrastructure layers. The domain layer performs no I/O, services orchestrate migration, and infrastructure handles files, processes, and directory watching.

```text
src/
  domain/        Data types, migration rules, merge semantics, transaction contracts
  services/      Instance scanning, settings merges, backups, watching, migration
  infra/         JSON parsing, file operations, process detection, ZIP, watching
  app_config.rs  User configuration persistence
  main.rs        Slint UI wiring
ui/              Interface and animations
tests/           Integration and UI callback tests
examples/        Read-only scanning example and backup benchmark
```

```powershell
cargo test

# Read-only scan of an actual versions directory
cargo run --example smoke_real -- "E:\Minecraft\.minecraft\versions"
```

The merge preview in `smoke_real` runs only when instance directory names containing `0.9.3` and `0.9.6` are found. Otherwise, that part is skipped.

<details>
<summary><strong>Configuration and directory watching</strong></summary>

On Windows, user configuration is stored in `%APPDATA%/packporter/config.json`. It includes the instances directory, recent selections, migration toggles, and custom path rules. Configuration is written atomically. A missing `rules` field falls back to built-in defaults; migration plans use the configured rules.

`FolderWatcherService` watches for new directories under `versions/`. Three consecutive matching snapshots, taken 800 ms apart, are treated as a stability signal before notifying the UI. This is a heuristic, not direct confirmation that an extraction program has finished.

</details>

See [development notes](docs/experience.md) for implementation details and [application icons](assets/icons/README.md) for assets and generation notes. Both documents are in Chinese.

## License

[MIT License](LICENSE) · Copyright © 2026 Mowon
