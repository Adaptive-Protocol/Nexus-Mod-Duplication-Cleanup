#!/usr/bin/env python3
"""
nexus_dedupe.py - identify and remove outdated Nexus mod archives.

Nexus filename formats (a trailing "(1)" re-download suffix is ignored):
    old: <name>-<mod_id>-<v1>-<v2>...-<10-digit unix timestamp>.<ext>
    new: <name> <mod_id> <version> [<YYYY-MM-DDTHH-MMZ>] <9-char id>.<ext>

Two files belong to the same series when mod_id matches and the names match
after dropping case, punctuation and embedded versions ("Foo v1.2" == "foo").
The file with the newest upload timestamp wins (undated new-format files count
as oldest). Files that don't match either pattern are left alone.

Judgment calls (names whose embedded versions differ, e.g. "CET 1.35.0 - Patch
2.21" vs "CET 1.37.0 - Patch 2.31", or upload order disagreeing with version
order) are flagged for review; --execute asks before collapsing each one.

Defaults to a dry run. Use --execute to act; with --hard, outdated files are
permanently deleted, otherwise they're moved to a _outdated_trash subfolder.
"""
import argparse, re, sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

ARCHIVE_EXTS = {".zip", ".rar", ".7z", ".archive", ".zipmod"}
OLD_RE     = re.compile(r"^(?P<name>.+?)-(?P<mid>\d+)-(?P<ver>(?:[a-zA-Z0-9]+-+)+)(?P<ts>\d{10})$")
NEW_RE     = re.compile(r"^(?P<name>.+?) (?P<mid>\d+) (?P<ver>\S+) (?:(?P<iso>\d{4}-\d\d-\d\dT\d\d-\d\dZ) )?[a-zA-Z0-9]{9}$")
CHUNK_RE   = re.compile(r"\d+|[a-zA-Z]+")
EMBED_VER  = re.compile(r"(?<![a-z0-9])(?:v?\d+(?:\.\d+)+[a-z]?|v\d+)(?![a-z0-9])", re.I)


# --- parsing & formatting helpers -----------------------------------------

def parse_nexus(filename):
    """Return {name, mod_id, version, timestamp} or None."""
    stem = Path(filename).stem if Path(filename).suffix.lower() in ARCHIVE_EXTS else filename
    stem = re.sub(r"\s*\(\d+\)$", "", stem)
    m = OLD_RE.match(stem) or NEW_RE.match(stem)
    if not m:
        return None
    if m.re is OLD_RE:
        ts = int(m["ts"])
    else:
        ts = m["iso"] and datetime.strptime(m["iso"], "%Y-%m-%dT%H-%MZ").replace(tzinfo=timezone.utc)
        ts = int(ts.timestamp()) if ts else 0
    return {
        "name":      m["name"],
        "mod_id":    m["mid"],
        "version":   tuple(p for p in re.split(r"[.-]", m["ver"]) if p),
        "timestamp": ts,
    }

def norm_name(name):
    """'Tony's Store (VA) v3.1' -> 'tonysstoreva'."""
    return re.sub(r"[^a-z0-9]", "", EMBED_VER.sub("", name).lower()) or name.lower()

def version_sort_key(parts):
    """Natural-sort key. Handles 'v5', '0g', '17b2', 'potat', etc."""
    def chunked(p):
        if len(p) >= 2 and p[0] in "vV" and p[1].isdigit():
            p = p[1:]
        return tuple((0, int(c)) if c.isdigit() else (1, c.lower())
                     for c in CHUNK_RE.findall(p))
    return tuple(chunked(p) for p in parts)

def fmt_ver(parts):
    """'v1.2.6', 'potat.07', 'CC'."""
    s = ".".join(p[1:] if len(p) >= 2 and p[0] in "vV" and p[1].isdigit() else p for p in parts)
    return f"v{s}" if s[:1].isdigit() else s

def fmt_size(n):
    n = float(n)
    for u in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024:
            return f"{n:,.1f} {u}"
        n /= 1024
    return f"{n:,.1f} PB"


# --- core logic -----------------------------------------------------------

def scan_and_decide(folder, by_mod_id=False, keep=1):
    """Walk folder, group files, pick a keeper for each group."""
    groups, unparsed = defaultdict(list), []
    for p in sorted(Path(folder).iterdir()):
        if not p.is_file() or p.name.startswith("dedupe_"):    # this script, its logs
            continue
        info = parse_nexus(p.name)
        if not info:
            unparsed.append(p.name)
            continue
        key = info["mod_id"] if by_mod_id else (norm_name(info["name"]), info["mod_id"])
        groups[key].append({**info, "path": p, "size": p.stat().st_size})

    ver, decisions = lambda f: version_sort_key(f["version"]), []
    for files in groups.values():
        ranked   = sorted(files, key=lambda f: (f["timestamp"], ver(f), -len(f["path"].name)), reverse=True)
        top_v    = max(files, key=ver)
        names    = sorted({f["name"] for f in files})
        conflict = ver(ranked[0]) != ver(top_v)
        decisions.append({
            "files":      ranked,
            "delete":     ranked[keep:],
            "names":      names,
            "ver_winner": top_v,
            "conflict":   conflict,
            "review":     len(files) > keep and (conflict or
                          len({tuple(EMBED_VER.findall(n)) for n in names}) > 1),
        })
    return decisions, unparsed


# --- reporting ------------------------------------------------------------

def key_str(d):
    return f"{d['files'][0]['name']}-{d['files'][0]['mod_id']}"

def totals(decisions):
    """(file count, bytes) currently marked for deletion."""
    return (sum(len(d["delete"]) for d in decisions),
            sum(f["size"] for d in decisions for f in d["delete"]))

def build_report(folder, args, decisions, unparsed):
    parseable    = sum(len(d["files"]) for d in decisions)
    multi        = sorted((d for d in decisions if d["delete"]), key=key_str)
    conflicts    = [d for d in decisions if d["conflict"]]
    renamed      = [d for d in multi if len(d["names"]) > 1]
    singletons   = len(decisions) - len(multi)
    delete_count, delete_bytes = totals(decisions)

    L = ["=" * 80,
         f"Nexus Mod Dedupe Report  -  {datetime.now().isoformat(timespec='seconds')}",
         "=" * 80,
         f"Folder         : {folder}",
         f"Mode           : {'EXECUTE' if args.execute else 'DRY RUN'}  "
         f"({'PERMANENT DELETE' if args.hard else 'move to _outdated_trash'})",
         f"Grouping       : {'mod_id ONLY (aggressive)' if args.by_mod_id else 'normalized name + mod_id'}",
         f"Keep per series: {args.keep} newest",
         f"Files scanned  : {parseable + len(unparsed)}",
         f"  Parseable    : {parseable}",
         f"  Unparseable  : {len(unparsed)} (left alone)",
         f"Series found   : {len(decisions)}  ({singletons} singletons, {len(multi)} multi-version)",
         f"Files to keep  : {len(decisions)}",
         f"Files outdated : {delete_count}",
         f"Space to free  : {fmt_size(delete_bytes)}"]
    if renamed:
        L.append(f"Renamed series : {len(renamed)} (merged across name variants)")
    L.append("")

    if renamed:
        L += ["-" * 80, f"!! MERGED NAME VARIANTS ({len(renamed)})", "-" * 80,
              "Same mod_id, names equal after dropping case/punctuation/embedded versions.", ""]
        for d in renamed:
            L += [f"Series: {key_str(d)}"] + [f"  - {n}" for n in d["names"]] + [""]

    if conflicts:
        L += ["-" * 80, f"!! REVIEW: {len(conflicts)} series where timestamp-order != version-order",
              "-" * 80, "Kept the timestamp-newest. Eyeball each.", ""]
        for d in conflicts:
            L += [f"Series: {key_str(d)}",
                  f"  Kept (newest upload) : {fmt_ver(d['files'][0]['version'])}  {d['files'][0]['path'].name}",
                  f"  Highest version#     : {fmt_ver(d['ver_winner']['version'])}  {d['ver_winner']['path'].name}",
                  ""]

    if multi:
        L += ["-" * 80, "MULTI-VERSION SERIES (decisions)", "-" * 80]
        for d in multi:
            L.append(f"\nSeries: {key_str(d)}")
            for i, f in enumerate(d["files"]):
                tag = "KEEP  " if i == 0 else "delete"
                L.append(f"  [{tag}]  {fmt_ver(f['version']):<12}  {fmt_size(f['size']):>10}   {f['path'].name}")

    if unparsed:
        L += ["", "-" * 80, f"UNPARSEABLE FILES ({len(unparsed)} - left alone)", "-" * 80]
        L += [f"  {n}" for n in unparsed]

    return "\n".join(L), delete_count, delete_bytes


# --- execution ------------------------------------------------------------

def execute(decisions, folder, hard):
    trash = folder / "_outdated_trash"
    if not hard:
        trash.mkdir(exist_ok=True)
    ok = fail = 0
    for d in decisions:
        for f in d["delete"]:
            try:
                if hard:
                    f["path"].unlink()
                else:
                    dest = trash / f["path"].name
                    if dest.exists():
                        dest = trash / f"{f['path'].stem}__{f['timestamp']}{f['path'].suffix}"
                    f["path"].rename(dest)
                ok += 1
            except OSError as e:
                print(f"  FAILED: {f['path'].name}: {e}")
                fail += 1
    return ok, fail


def main():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--folder", default=str(Path(__file__).parent),
                   help="Folder to scan (default: folder containing this script)")
    p.add_argument("--execute", action="store_true",
                   help="Actually act. Default is DRY RUN.")
    p.add_argument("--hard", action="store_true",
                   help="With --execute, PERMANENTLY DELETE (default: move to _outdated_trash).")
    p.add_argument("--by-mod-id", action="store_true",
                   help="Group by mod_id only (aggressive; may merge variants).")
    p.add_argument("--keep", type=int, default=1, metavar="N",
                   help="Keep the N newest files per series, e.g. 2 for a rollback copy (default 1).")
    p.add_argument("--yes", action="store_true", help="Skip confirmation prompts (collapses judgment calls).")
    args = p.parse_args()

    folder = Path(args.folder).resolve()
    if not folder.is_dir():
        sys.exit(f"ERROR: '{folder}' is not a directory.")
    if args.keep < 1:
        sys.exit("ERROR: --keep must be at least 1.")

    decisions, unparsed = scan_and_decide(folder, args.by_mod_id, args.keep)
    report, n_del, bytes_del = build_report(folder, args, decisions, unparsed)

    log_path = folder / f"dedupe_log_{datetime.now():%Y%m%d_%H%M%S}.txt"
    log_path.write_text(report, encoding="utf-8")
    print("\n".join(report.splitlines()[:14]))
    print(f"\nFull log: {log_path}")

    if not n_del:
        print("\nNothing to do.")
        return
    if not args.execute:
        print(f"\nDRY RUN: {n_del} files ({fmt_size(bytes_del)}) would be "
              f"{'deleted' if args.hard else 'moved to _outdated_trash'}.")
        print("Re-run with --execute to act.")
        return
    if not args.yes:
        for d in (d for d in decisions if d["review"]):
            names = " | ".join(d["names"])
            if input(f"\nJudgment call: {key_str(d)}  [{names}]\n  Collapse to newest? [Y/n] ").strip().lower() == "n":
                d["delete"] = []
        n_del, bytes_del = totals(decisions)
        verb = "PERMANENTLY DELETE" if args.hard else "move to _outdated_trash"
        if input(f"\n{verb} {n_del} files ({fmt_size(bytes_del)})? Type 'yes': ").strip().lower() != "yes":
            print("Aborted.")
            return

    ok, fail = execute(decisions, folder, args.hard)
    print(f"\nDone. {ok} {'deleted' if args.hard else 'moved'}, {fail} failures.")
    if not args.hard and ok:
        print(f"Trash: {folder / '_outdated_trash'}")


if __name__ == "__main__":
    main()
