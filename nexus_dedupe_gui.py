#!/usr/bin/env python3
"""Nexus Mod Dedupe - desktop front end for nexus_dedupe.py (Windows & Linux)."""
import sys, tkinter as tk
from datetime import datetime
from pathlib import Path
from tkinter import filedialog, messagebox, ttk
from types import SimpleNamespace

import nexus_dedupe as nd

APP_DIR = Path(sys.executable if getattr(sys, "frozen", False) else __file__).resolve().parent
REVIEW_HELP = ("These mods need a judgment call: their file names carry different embedded versions "
               "(e.g. CET builds for different game patches), or upload order disagrees with version "
               "numbers.\n\nSelected rows are collapsed to the newest file. Deselect any mod whose older "
               "files you want to keep, e.g. as a rollback option.")


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("Nexus Mod Dedupe")
        self.geometry("960x620")
        self.folder, self.keep, self.hard = tk.StringVar(value=str(APP_DIR)), tk.IntVar(value=1), tk.BooleanVar()
        self.decisions = None

        bar = ttk.Frame(self, padding=8)
        bar.pack(fill="x")
        ttk.Label(bar, text="Mods folder:").pack(side="left")
        ttk.Entry(bar, textvariable=self.folder).pack(side="left", fill="x", expand=True, padx=4)
        ttk.Button(bar, text="Browse…", command=self.browse).pack(side="left")

        opts = ttk.Frame(self, padding=(8, 0, 8, 8))
        opts.pack(fill="x")
        ttk.Label(opts, text="Keep newest per mod:").pack(side="left")
        ttk.Spinbox(opts, from_=1, to=9, width=3, textvariable=self.keep).pack(side="left", padx=(4, 16))
        ttk.Checkbutton(opts, text="Delete permanently (skip _outdated_trash)", variable=self.hard).pack(side="left")
        self.clean_btn = ttk.Button(opts, text="Clean up", command=self.clean, state="disabled")
        self.clean_btn.pack(side="right")
        ttk.Button(opts, text="Scan", command=self.scan).pack(side="right", padx=4)
        self.keep.trace_add("write", lambda *_: self.clean_btn.state(["disabled"]))   # rescan needed

        box = ttk.Frame(self)
        box.pack(fill="both", expand=True)
        self.out = tk.Text(box, wrap="none", font="TkFixedFont", state="disabled")
        ys = ttk.Scrollbar(box, command=self.out.yview)
        xs = ttk.Scrollbar(box, orient="horizontal", command=self.out.xview)
        self.out.configure(yscrollcommand=ys.set, xscrollcommand=xs.set)
        ys.pack(side="right", fill="y")
        xs.pack(side="bottom", fill="x")
        self.out.pack(fill="both", expand=True)
        self.show("Pick your Nexus downloads folder and press Scan. Nothing is changed until you press Clean up.")

    def show(self, text):
        self.out.configure(state="normal")
        self.out.replace("1.0", "end", text)
        self.out.configure(state="disabled")

    def browse(self):
        if d := filedialog.askdirectory(initialdir=self.folder.get()):
            self.folder.set(d)

    def args(self, execute=False):
        return SimpleNamespace(execute=execute, hard=self.hard.get(), by_mod_id=False, keep=self.keep.get())

    def scan(self):
        folder = Path(self.folder.get())
        try:
            keep = self.keep.get()
        except tk.TclError:
            keep = 0
        if not folder.is_dir() or keep < 1:
            return messagebox.showerror("Nexus Mod Dedupe", "Choose an existing folder and keep at least 1 file.")
        self.decisions, self.unparsed = nd.scan_and_decide(folder, keep=keep)
        if reviews := [d for d in self.decisions if d["review"]]:
            self.review(reviews)
        report, n_del, _ = nd.build_report(folder, self.args(), self.decisions, self.unparsed)
        self.show(report)
        self.clean_btn.state(["!disabled"] if n_del else ["disabled"])

    def review(self, reviews):
        """Modal popup: choose which judgment-call mods to collapse."""
        win = tk.Toplevel(self)
        win.title(f"Review {len(reviews)} judgment calls")
        win.transient(self)
        ttk.Label(win, text=REVIEW_HELP, wraplength=760, padding=8).pack(fill="x")
        lb = tk.Listbox(win, selectmode="multiple", width=110, height=min(len(reviews), 18), font="TkFixedFont")
        for d in reviews:
            lb.insert("end", f"{nd.key_str(d)}  ({len(d['delete'])} older)  -  " + " | ".join(d["names"]))
        lb.select_set(0, "end")
        lb.pack(fill="both", expand=True, padx=8)
        btns = ttk.Frame(win, padding=8)
        btns.pack(fill="x")
        ttk.Button(btns, text="Select all", command=lambda: lb.select_set(0, "end")).pack(side="left")
        ttk.Button(btns, text="Select none", command=lambda: lb.select_clear(0, "end")).pack(side="left", padx=4)
        def done():                                   # unselected mods keep every file
            chosen = set(lb.curselection())
            for i, d in enumerate(reviews):
                if i not in chosen:
                    d["delete"] = []
            win.destroy()
        ttk.Button(btns, text="Continue", command=done).pack(side="right")
        win.protocol("WM_DELETE_WINDOW", done)
        win.grab_set()
        self.wait_window(win)

    def clean(self):
        folder, hard = Path(self.folder.get()), self.hard.get()
        n, size = nd.totals(self.decisions)
        verb = "PERMANENTLY DELETE" if hard else "move to _outdated_trash"
        if not messagebox.askyesno("Confirm", f"{verb} {n} outdated files ({nd.fmt_size(size)})?"):
            return
        report = nd.build_report(folder, self.args(execute=True), self.decisions, self.unparsed)[0]
        (folder / f"dedupe_log_{datetime.now():%Y%m%d_%H%M%S}.txt").write_text(report, encoding="utf-8")
        ok, fail = nd.execute(self.decisions, folder, hard)
        self.decisions = None
        self.clean_btn.state(["disabled"])
        messagebox.showinfo("Done", f"{ok} files {'deleted' if hard else 'moved to _outdated_trash'}, "
                                    f"{fail} failures.\nA log was saved in the mods folder.")


if __name__ == "__main__":
    try:                                              # crisp text on high-DPI Windows screens
        import ctypes
        ctypes.windll.shcore.SetProcessDpiAwareness(1)
    except (ImportError, AttributeError, OSError):
        pass
    App().mainloop()
