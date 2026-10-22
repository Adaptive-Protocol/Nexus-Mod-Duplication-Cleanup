# Nexus-Mod-Duplication-Cleanup

A lightweight app that finds outdated versions of mods in your Nexus Mods download folder and removes them, keeping only the newest file of each mod. Mods that need a judgment call, such as builds for different game patches, stay untouched until you say what to remove. Works on Windows and Linux.

## Download

Grab the latest build from the [Releases](../../releases) page:

- **Windows:** `NexusModCleanup-windows.exe`
- **Linux:** `NexusModCleanup-linux` (run `chmod +x NexusModCleanup-linux` once)

Or run from source with Python 3.8+ (no extra packages): `python nexus_dedupe_gui.py`. On Linux you may need your distro's Tk package, e.g. `sudo apt install python3-tk`.

## Using the app

1. Pick your mod downloads folder (the default is the folder the app is in).
2. Press **Scan**. Nothing on disk changes. You get a report of what would be removed.
3. If any mods need a judgment call, a popup lists them. Rows that stay selected are collapsed to the newest file. Deselect any mod whose older files you want to keep, for example to roll back after a game patch.
4. Press **Clean up**. By default outdated files go to an `_outdated_trash` subfolder, so you can check them before deleting that folder yourself. Tick **Delete permanently** to skip the trash.

**Keep newest per mod** keeps more than one file per mod, e.g. `2` keeps the previous version of everything as a rollback copy.

A `dedupe_log_<date>.txt` file recording every decision is saved in the folder after each cleanup.

## How it decides

Both Nexus filename formats are recognised:

```
old: Autoloot-5202-3-10-2-1779529077.zip
new: Autoloot 5202 3.11.2 2026-10-17T10-20Z ozVyzbbhu.zip
```

- Files belong to the same mod when the **mod ID** matches and the names match after ignoring case, punctuation and versions written into the name. `Hotscenes Add-on 5.23.7` and `Hotscenes Add-on` are one mod; `(FF) Christmas Moments` and `(MF) Christmas Moments` stay separate variants.
- The newest **upload date** from the filename wins, not the file's modified date on disk, which changes when files are copied or restored. Version numbers break ties.
- **Judgment calls** are mods whose file names carry different versions (e.g. `CET 1.35.0 - Patch 2.21` vs `CET 1.37.0 - Patch 2.31`), or whose upload order disagrees with their version numbers. These are always shown in the popup before anything is removed.
- Files that don't follow a Nexus naming pattern (`setup.exe`, `Autoloot.zip`, …) are never touched.

## Command line

`nexus_dedupe.py` is the same engine without the window:

```
python nexus_dedupe.py --folder "path/to/mod downloads"               # dry run, writes a log
python nexus_dedupe.py --folder ... --execute                         # move outdated files to _outdated_trash
python nexus_dedupe.py --folder ... --execute --hard                  # delete permanently
python nexus_dedupe.py --folder ... --keep 2                          # keep a rollback copy of each mod
```

With `--execute`, it asks about each judgment call and then confirms once before acting. `--yes` skips all prompts and collapses everything.

## Verifying downloads

Every release binary is built by GitHub Actions straight from this repository, with a `.sha256` checksum and a signed build-provenance attestation alongside it:

```
sha256sum -c NexusModCleanup-linux.sha256                       # Linux
Get-FileHash NexusModCleanup-windows.exe                        # Windows PowerShell; compare with the .sha256 file
gh attestation verify NexusModCleanup-windows.exe -R Adaptive-Protocol/Nexus-Mod-Duplication-Cleanup
```

## Building

Pushing a `v*` tag builds the Windows and Linux binaries with PyInstaller and attaches them to a GitHub release (see `.github/workflows/build.yml`). To build locally:

```
pip install pyinstaller
pyinstaller --onefile --windowed --name NexusModCleanup nexus_dedupe_gui.py
```

## License

MIT, see [LICENSE](LICENSE).
