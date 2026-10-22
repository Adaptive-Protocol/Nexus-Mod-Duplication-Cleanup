//! Port of nexus_dedupe.py: identify and remove outdated Nexus mod archives.
//!
//! Nexus filename formats (a trailing "(1)" re-download suffix is ignored):
//!     old: <name>-<mod_id>-<v1>-<v2>...-<10-digit unix timestamp>.<ext>
//!     new: <name> <mod_id> <version> [<YYYY-MM-DDTHH-MMZ>] <9-char id>.<ext>
//!
//! Files belong to the same series when mod_id matches and the names match after
//! dropping case, punctuation and embedded versions. The newest upload wins.
use chrono::{Local, NaiveDateTime};
use fancy_regex::Regex;
use std::{collections::HashMap, fs, io, path::{Path, PathBuf}, sync::LazyLock};

const ARCHIVE_EXTS: [&str; 5] = [".zip", ".rar", ".7z", ".archive", ".zipmod"];
macro_rules! re { ($n:ident, $p:literal) => { static $n: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).unwrap()); } }
re!(OLD_RE, r"^(?P<name>.+?)-(?P<mid>\d+)-(?P<ver>(?:[a-zA-Z0-9]+-+)+)(?P<ts>\d{10})$");
re!(NEW_RE, r"^(?P<name>.+?) (?P<mid>\d+) (?P<ver>\S+) (?:(?P<iso>\d{4}-\d\d-\d\dT\d\d-\d\dZ) )?[a-zA-Z0-9]{9}$");
re!(REDL_RE, r"\s*\(\d+\)$");
re!(CHUNK_RE, r"[0-9]+|[a-zA-Z]+");
re!(EMBED_VER, r"(?i)(?<![a-z0-9])(?:v?\d+(?:\.\d+)+[a-z]?|v\d+)(?![a-z0-9])");

#[derive(Clone, Debug)]
pub struct File { pub name: String, pub mod_id: String, pub version: Vec<String>, pub ts: i64, pub path: PathBuf, pub size: u64 }

pub struct Decision {
    pub files: Vec<File>,      // ranked, newest first
    pub keep: usize,
    pub collapse: bool,        // false = keep every file (judgment call declined)
    pub names: Vec<String>,
    pub ver_winner: File,
    pub conflict: bool,
    pub review: bool,
}

impl Decision {
    pub fn delete(&self) -> &[File] { if self.collapse { &self.files[self.keep.min(self.files.len())..] } else { &[] } }
    pub fn key_str(&self) -> String { format!("{}-{}", self.files[0].name, self.files[0].mod_id) }
}

/// Python's Path.stem / Path.suffix split.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') { Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i..]), _ => (name, "") }
}

fn file_name(p: &Path) -> String { p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }

/// (name, mod_id, version parts, upload timestamp), or None.
pub fn parse_nexus(filename: &str) -> Option<(String, String, Vec<String>, i64)> {
    let (stem, ext) = split_ext(filename);
    let stem = if ARCHIVE_EXTS.contains(&ext.to_lowercase().as_str()) { stem } else { filename };
    let stem = REDL_RE.replace(stem, "").into_owned();
    let stem = stem.as_str();
    let (m, old) = match OLD_RE.captures(stem).ok()? { Some(m) => (m, true), None => (NEW_RE.captures(stem).ok()??, false) };
    let ts = if old { m["ts"].parse().ok()? } else {
        m.name("iso").and_then(|i| NaiveDateTime::parse_from_str(i.as_str(), "%Y-%m-%dT%H-%MZ").ok()).map_or(0, |t| t.and_utc().timestamp())
    };
    let ver = m["ver"].split(['.', '-']).filter(|p| !p.is_empty()).map(String::from).collect();
    Some((m["name"].to_string(), m["mid"].to_string(), ver, ts))
}

/// "Tony's Store (VA) v3.1" -> "tonysstoreva".
pub fn norm_name(name: &str) -> String {
    let n: String = EMBED_VER.replace_all(name, "").to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect();
    if n.is_empty() { name.to_lowercase() } else { n }
}

fn embedded(name: &str) -> Vec<String> { EMBED_VER.find_iter(name).filter_map(Result::ok).map(|m| m.as_str().to_string()).collect() }

fn strip_v(p: &str) -> &str { if p.len() >= 2 && p.starts_with(['v', 'V']) && p[1..].starts_with(|c: char| c.is_ascii_digit()) { &p[1..] } else { p } }

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Chunk { Num(u128), Alpha(String) }

/// Natural-sort key. Handles "v5", "0g", "17b2", "potat", etc.
fn version_key(parts: &[String]) -> Vec<Vec<Chunk>> {
    parts.iter().map(|p| CHUNK_RE.find_iter(strip_v(p)).filter_map(Result::ok).map(|c| {
        c.as_str().parse().map_or_else(|_| Chunk::Alpha(c.as_str().to_lowercase()), Chunk::Num)
    }).collect()).collect()
}

/// "v1.2.6", "potat.07", "CC".
pub fn fmt_ver(parts: &[String]) -> String {
    let s = parts.iter().map(|p| strip_v(p)).collect::<Vec<_>>().join(".");
    if s.starts_with(|c: char| c.is_ascii_digit()) { format!("v{s}") } else { s }
}

pub fn fmt_size(n: u64) -> String {
    let (mut n, mut u) = (n as f64, 0);
    while n >= 1024.0 && u < 5 { n /= 1024.0; u += 1; }
    let s = format!("{n:.1}");
    let (int, frac) = s.split_once('.').unwrap();
    let int: String = int.chars().rev().collect::<Vec<_>>().chunks(3).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(",").chars().rev().collect();
    format!("{int}.{frac} {}", ["B", "KB", "MB", "GB", "TB", "PB"][u])
}

/// Walk folder, group files, pick the keepers for each group.
pub fn scan_and_decide(folder: &Path, keep: usize) -> io::Result<(Vec<Decision>, Vec<String>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(folder)?.filter_map(|e| Some(e.ok()?.path())).collect();
    paths.sort_by_cached_key(|p| { let s = p.to_string_lossy(); if cfg!(windows) { s.to_lowercase() } else { s.into_owned() } });
    let (mut groups, mut index, mut unparsed) = (Vec::<Vec<File>>::new(), HashMap::new(), vec![]);
    for path in paths {
        let fname = file_name(&path);
        if !path.is_file() || fname.starts_with("dedupe_") { continue; }    // its logs
        let Some((name, mod_id, version, ts)) = parse_nexus(&fname) else { unparsed.push(fname); continue };
        let i = *index.entry((norm_name(&name), mod_id.clone())).or_insert_with(|| { groups.push(vec![]); groups.len() - 1 });
        groups[i].push(File { name, mod_id, version, ts, size: path.metadata()?.len(), path });
    }

    let decisions = groups.into_iter().map(|files| {
        let mut top_v = &files[0];
        for f in &files { if version_key(&f.version) > version_key(&top_v.version) { top_v = f } }
        let ver_winner = top_v.clone();
        let mut names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        names.sort();
        names.dedup();
        let mut ranked = files;
        ranked.sort_by_cached_key(|f| std::cmp::Reverse((f.ts, version_key(&f.version), -(file_name(&f.path).chars().count() as i64))));
        let conflict = version_key(&ranked[0].version) != version_key(&ver_winner.version);
        let mut embeds: Vec<_> = names.iter().map(|n| embedded(n)).collect();
        embeds.sort();
        embeds.dedup();
        let review = ranked.len() > keep && (conflict || embeds.len() > 1);
        Decision { files: ranked, keep, collapse: true, names, ver_winner, conflict, review }
    }).collect();
    Ok((decisions, unparsed))
}

/// (file count, bytes) currently marked for deletion.
pub fn totals(decisions: &[Decision]) -> (usize, u64) {
    decisions.iter().flat_map(Decision::delete).fold((0, 0), |(n, b), f| (n + 1, b + f.size))
}

pub fn build_report(folder: &Path, execute: bool, hard: bool, keep: usize, decisions: &[Decision], unparsed: &[String]) -> String {
    let parseable: usize = decisions.iter().map(|d| d.files.len()).sum();
    let mut multi: Vec<&Decision> = decisions.iter().filter(|d| !d.delete().is_empty()).collect();
    multi.sort_by_cached_key(|d| d.key_str());
    let conflicts: Vec<_> = decisions.iter().filter(|d| d.conflict).collect();
    let renamed: Vec<_> = multi.iter().filter(|d| d.names.len() > 1).collect();
    let (n_del, b_del) = totals(decisions);
    let (eq, dash) = ("=".repeat(80), "-".repeat(80));

    let mut l = vec![eq.clone(), format!("Nexus Mod Dedupe Report  -  {}", Local::now().format("%Y-%m-%dT%H:%M:%S")), eq,
        format!("Folder         : {}", folder.display()),
        format!("Mode           : {}  ({})", if execute { "EXECUTE" } else { "DRY RUN" }, if hard { "PERMANENT DELETE" } else { "move to _outdated_trash" }),
        "Grouping       : normalized name + mod_id".into(),
        format!("Keep per series: {keep} newest"),
        format!("Files scanned  : {}", parseable + unparsed.len()),
        format!("  Parseable    : {parseable}"),
        format!("  Unparseable  : {} (left alone)", unparsed.len()),
        format!("Series found   : {}  ({} singletons, {} multi-version)", decisions.len(), decisions.len() - multi.len(), multi.len()),
        format!("Files to keep  : {}", decisions.len()),
        format!("Files outdated : {n_del}"),
        format!("Space to free  : {}", fmt_size(b_del))];
    if !renamed.is_empty() { l.push(format!("Renamed series : {} (merged across name variants)", renamed.len())); }
    l.push(String::new());

    if !renamed.is_empty() {
        l.extend([dash.clone(), format!("!! MERGED NAME VARIANTS ({})", renamed.len()), dash.clone(),
            "Same mod_id, names equal after dropping case/punctuation/embedded versions.".into(), String::new()]);
        for d in &renamed {
            l.push(format!("Series: {}", d.key_str()));
            l.extend(d.names.iter().map(|n| format!("  - {n}")));
            l.push(String::new());
        }
    }
    if !conflicts.is_empty() {
        l.extend([dash.clone(), format!("!! REVIEW: {} series where timestamp-order != version-order", conflicts.len()), dash.clone(),
            "Kept the timestamp-newest. Eyeball each.".into(), String::new()]);
        for d in &conflicts {
            l.extend([format!("Series: {}", d.key_str()),
                format!("  Kept (newest upload) : {}  {}", fmt_ver(&d.files[0].version), file_name(&d.files[0].path)),
                format!("  Highest version#     : {}  {}", fmt_ver(&d.ver_winner.version), file_name(&d.ver_winner.path)),
                String::new()]);
        }
    }
    if !multi.is_empty() {
        l.extend([dash.clone(), "MULTI-VERSION SERIES (decisions)".into(), dash.clone()]);
        for d in &multi {
            l.push(format!("\nSeries: {}", d.key_str()));
            for (i, f) in d.files.iter().enumerate() {
                l.push(format!("  [{}]  {:<12}  {:>10}   {}", if i == 0 { "KEEP  " } else { "delete" }, fmt_ver(&f.version), fmt_size(f.size), file_name(&f.path)));
            }
        }
    }
    if !unparsed.is_empty() {
        l.extend([String::new(), dash.clone(), format!("UNPARSEABLE FILES ({} - left alone)", unparsed.len()), dash]);
        l.extend(unparsed.iter().map(|n| format!("  {n}")));
    }
    l.join("\n")
}

/// Move outdated files to _outdated_trash (or delete them). Returns (ok, failed).
pub fn execute(decisions: &[Decision], folder: &Path, hard: bool) -> (usize, usize) {
    let trash = folder.join("_outdated_trash");
    if !hard && fs::create_dir_all(&trash).is_err() { return (0, totals(decisions).0) }
    decisions.iter().flat_map(Decision::delete).fold((0, 0), |(ok, fail), f| {
        let res = if hard { fs::remove_file(&f.path) } else {
            let name = file_name(&f.path);
            let mut dest = trash.join(&name);
            if dest.exists() { let (stem, ext) = split_ext(&name); dest = trash.join(format!("{stem}__{}{ext}", f.ts)); }
            fs::rename(&f.path, dest)
        };
        if res.is_ok() { (ok + 1, fail) } else { (ok, fail + 1) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &[&str]) -> Vec<String> { s.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn parses_both_formats() {
        assert_eq!(parse_nexus("Autoloot-5202-3-10-2-1779529077.zip"), Some(("Autoloot".into(), "5202".into(), v(&["3", "10", "2"]), 1779529077)));
        assert_eq!(parse_nexus("Autoloot 5202 3.11.2 2026-10-17T10-20Z ozVyzbbhu (1).zip"), Some(("Autoloot".into(), "5202".into(), v(&["3", "11", "2"]), 1792232400)));
        assert_eq!(parse_nexus("Autoloot 5202 3.11.2 ozVyzbbhu.7z").unwrap().3, 0);
        assert_eq!(parse_nexus("setup.exe"), None);
        assert_eq!(parse_nexus("Autoloot.zip"), None);
    }

    #[test]
    fn normalises_names_and_versions() {
        assert_eq!(norm_name("Tony's Store (VA) v3.1"), "tonysstoreva");
        assert_eq!(norm_name("Hotscenes Add-on 5.23.7"), norm_name("Hotscenes Add-on"));
        assert_ne!(norm_name("(FF) Christmas Moments"), norm_name("(MF) Christmas Moments"));
        assert_eq!(embedded("1.2.3ab and v2"), v(&["1.2", "v2"]));     // backtracks like Python's re
        assert!(version_key(&v(&["v10"])) > version_key(&v(&["9"])));
        assert_eq!(fmt_ver(&v(&["v1", "2", "6"])), "v1.2.6");
        assert_eq!(fmt_ver(&v(&["potat", "07"])), "potat.07");
        assert_eq!(fmt_size(1_048_575), "1,024.0 KB");
    }

    #[test]
    fn moves_outdated_to_trash_and_respects_declined_reviews() {
        let dir = std::env::temp_dir().join(format!("nmc-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("_outdated_trash")).unwrap();
        for f in ["A-1-1-0-1600000000.zip", "A-1-1-1-1600000100.zip", "A-1-1-2-1600000200.zip", "_outdated_trash/A-1-1-0-1600000000.zip",
                  "CET 1.35.0 - Patch 2.21 107 1.35.0 2026-03-01T12-00Z aB3dE5fG7.zip", "CET 1.37.0 - Patch 2.31 107 1.37.0 2026-09-15T08-30Z hI9jK1lM3.zip"] {
            fs::write(dir.join(f), f).unwrap();
        }
        let (mut d, _) = scan_and_decide(&dir, 2).unwrap();
        assert_eq!(d.iter().filter(|d| d.review).count(), 0);         // CET has 2 files, keep 2: nothing to decide
        let (mut d1, _) = scan_and_decide(&dir, 1).unwrap();
        d1.iter_mut().filter(|d| d.review).for_each(|d| d.collapse = false);
        assert_eq!(totals(&d1).0, 2);                                  // only the A series is collapsed
        d.retain(|d| !d.delete().is_empty());
        assert_eq!(execute(&d, &dir, false), (1, 0));
        assert!(dir.join("_outdated_trash/A-1-1-0-1600000000__1600000000.zip").exists());   // name clash renamed
        fs::remove_dir_all(dir).unwrap();
    }
}
