#!/usr/bin/env python3
"""Writes crates/letters-fonts/library.txt: the families of the Google Fonts library, read from the
METADATA.pb files of github.com/google/fonts at a commit.

    scripts/font-library.py <commit> [folder of METADATA.pb files already fetched]

A line is a family's name, its weights (`100-900` for a weight axis, else a list) and whether it has
italics, with tabs between. The names are the families tagged in the repository's tags/all/families.csv.
"""
import concurrent.futures
import pathlib
import re
import sys
import urllib.request

RAW = "https://raw.githubusercontent.com/google/fonts"


def fetch(url):
    try:
        with urllib.request.urlopen(url, timeout=30) as answer:
            return answer.read().decode("utf-8")
    except OSError:
        return None


def metadata(commit, kept, name):
    folder = name.lower().replace(" ", "")
    if kept and (kept / f"{folder}.pb").exists():
        return (kept / f"{folder}.pb").read_text()
    for licence in ("ofl", "apache", "ufl"):
        text = fetch(f"{RAW}/{commit}/{licence}/{folder}/METADATA.pb")
        if text:
            return text
    return None


def line(name, text):
    if not text or re.search(r'^name: "(.*)"', text, re.M).group(1) != name:
        return None
    faces = re.findall(r'fonts \{.*?style: "(\w+)".*?weight: (\d+).*?\}', text, re.S)
    axis = re.search(r'axes \{\s*tag: "wght"\s*min_value: ([\d.]+)\s*max_value: ([\d.]+)', text)
    if axis:
        weights = f"{int(float(axis.group(1)))}-{int(float(axis.group(2)))}"
    else:
        weights = ",".join(str(weight) for weight in sorted({int(weight) for _, weight in faces}))
    italic = any(style == "italic" for style, _ in faces)
    return f"{name}\t{weights}\t{int(italic)}" if weights else None


def main():
    commit = sys.argv[1]
    kept = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else None
    names = sorted({row.split(",")[0] for row in fetch(f"{RAW}/{commit}/tags/all/families.csv").splitlines()})
    with concurrent.futures.ThreadPoolExecutor(16) as pool:
        lines = pool.map(lambda name: line(name, metadata(commit, kept, name)), names)
    lines = [found for found in lines if found]
    out = pathlib.Path(__file__).resolve().parent.parent / "crates/letters-fonts/library.txt"
    out.write_text(f"# github.com/google/fonts at {commit}\n" + "\n".join(lines) + "\n")
    print(f"{len(lines)} of {len(names)} families")


main()
