#![cfg_attr(windows, windows_subsystem = "windows")]
//! Uninstaller for Nexus Mod Dedupe: after the user confirms, it removes the app's settings and the app in the
//! uninstaller's own folder. It never touches mods, dedupe logs or `_outdated_trash` folders.
use std::{fs, path::{Path, PathBuf}};

const TITLE: &str = "Uninstall Nexus Mod Dedupe";
const APP_ID: &str = "Nexus Mod Dedupe";   // TITLE in main.rs: eframe names the settings folder after it
const MARK: &[u8] = b"_outdated_trash";     // a file only counts as the app if it contains this
const EXES: [&str; 2] = if cfg!(windows) { ["NexusModCleanup.exe", "NexusModCleanup-windows.exe"] } else { ["NexusModCleanup", "NexusModCleanup-linux"] };

/// A regular file (not a link) that carries the app's marker.
fn is_app(p: &Path) -> bool {
    p.symlink_metadata().is_ok_and(|m| m.is_file()) && fs::read(p).is_ok_and(|b| b.windows(MARK.len()).any(|w| w == MARK))
}

/// What exists to remove, in order: the settings file, its folders (deepest first, only removed when empty) and the app.
/// `true` marks a folder.
fn plan(settings: Option<PathBuf>, dir: Option<&Path>, me: &Path) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = settings.iter().flat_map(|s| std::iter::once((s.join("app.ron"), false))
        .chain(s.ancestors().take(if cfg!(windows) { 2 } else { 1 }).map(|a| (a.to_path_buf(), true)))).collect();   // Windows: <app>\data
    v.extend(dir.into_iter().flat_map(|d| EXES.map(|n| d.join(n))).filter(|p| p != me && is_app(p)).map(|p| (p, false)));
    v.retain(|(p, _)| p.symlink_metadata().is_ok());
    v
}

fn remove(todo: &[(PathBuf, bool)]) -> Vec<String> {
    todo.iter().map(|(p, dir)| match if *dir { fs::remove_dir(p) } else { fs::remove_file(p) } {
        Ok(()) => format!("Removed {}", p.display()),
        Err(e) => format!("Could not remove {}: {e}", p.display()),
    }).collect()
}

#[cfg(windows)]
fn ask(text: &str, confirm: bool) -> bool {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult};
    let buttons = if confirm { MessageButtons::OkCancel } else { MessageButtons::Ok };
    MessageDialog::new().set_title(TITLE).set_description(text).set_buttons(buttons).show() == MessageDialogResult::Ok
}

#[cfg(not(windows))]
fn ask(text: &str, confirm: bool) -> bool {   // a terminal prompt; with no terminal, nothing is confirmed
    use std::io::{self, Write};
    println!("{TITLE}\n\n{text}\n");
    if !confirm { return true }
    print!("Type y and press Enter to continue: ");
    io::stdout().flush().ok();
    let mut s = String::new();
    io::stdin().read_line(&mut s).is_ok() && s.trim().eq_ignore_ascii_case("y")
}

fn main() {
    let me = std::env::current_exe().unwrap_or_default();
    let todo = plan(eframe::storage_dir(APP_ID), me.parent(), &me);
    if todo.is_empty() {
        ask("Nothing to remove: no settings were found, and the app isn't in this uninstaller's folder.", false);
        return;
    }
    let list: Vec<String> = todo.iter().map(|(p, _)| format!("  {}", p.display())).collect();
    if ask(&format!("This removes Nexus Mod Dedupe from this computer:\n\n{}\n\nYour mods, _outdated_trash folders and \
        dedupe_log files are not touched. Close Nexus Mod Dedupe before you continue.", list.join("\n")), true) {
        ask(&format!("{}\n\nTo finish, delete this uninstaller and the files that came with the app \
            (README.md, LICENSE.txt, THIRD-PARTY-NOTICES.txt).", remove(&todo).join("\n")), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_the_apps_own_files() {
        let d = std::env::temp_dir().join(format!("nmc-uninstall-{}", std::process::id()));
        let s = d.join("Nexus Mod Dedupe").join("data");
        fs::create_dir_all(&s).unwrap();
        fs::write(s.join("app.ron"), "{}").unwrap();
        fs::write(d.join(EXES[0]), b"..._outdated_trash...").unwrap();   // the app
        fs::write(d.join(EXES[1]), b"another program").unwrap();         // the app's name, but not the app
        fs::write(d.join("README.md"), "kept").unwrap();
        let mut want = vec![(s.join("app.ron"), false), (s.clone(), true)];
        if cfg!(windows) { want.push((s.parent().unwrap().to_path_buf(), true)) }
        want.push((d.join(EXES[0]), false));
        assert_eq!(plan(Some(s.clone()), Some(&d), &d.join("Uninstall")), want);
        assert!(remove(&want).iter().all(|r| r.starts_with("Removed")));
        assert!(!s.exists() && !d.join(EXES[0]).exists() && d.join(EXES[1]).exists() && d.join("README.md").exists());
        assert_eq!(plan(Some(s), Some(&d), &d.join("Uninstall")), vec![]);   // a second run finds nothing
        fs::remove_dir_all(d).unwrap();
    }
}
