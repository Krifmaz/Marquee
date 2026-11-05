#!/usr/bin/env python3
"""Writes the endpoint tables of docs/parity/ from the pinned upstream API specs and Marquee's routes.

Usage (from the repository root):

    python3 -I scripts/parity.py <folder with sonarr/, radarr/ and seerr/ checked out at UPSTREAM.lock>

Each upstream checkout must sit at the commit UPSTREAM.lock pins; the script refuses otherwise. For every
path and method in the upstream API spec it looks for the matching Marquee route and marks it:

- Done: Marquee answers it with real code.
- Stub: Marquee answers it with a fixed empty answer (so clients don't fail), but does nothing yet.
- Missing: Marquee doesn't answer it.

Only the endpoint tables and the counts in docs/PARITY.md's API table are written here. The module and UI
tables in docs/PARITY.md are kept by hand.
"""

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
METHODS = ("get", "post", "put", "delete", "patch")

# Where each Marquee file's routes are served, per upstream app. arr_common.rs serves both arr apps.
ROUTE_FILES = {
    "sonarr": [("server/src/sonarr.rs", "/sonarr/api/v3"), ("server/src/arr_common.rs", "/sonarr/api/v3")],
    "radarr": [("server/src/radarr.rs", "/radarr/api/v3"), ("server/src/arr_common.rs", "/radarr/api/v3")],
    "seerr": [
        ("server/src/requests/seerr.rs", "/api/v1"),
        ("server/src/requests/discover.rs", "/api/v1"),
        ("server/src/requests/sliders.rs", "/api/v1"),
    ],
}
# Routes served outside the nested v3 router (path as upstream spells it -> Marquee path).
EXTRA = {
    "sonarr": {("/api", "get"): "/sonarr/api", ("/ping", "get"): "/sonarr/ping"},
    "radarr": {("/api", "get"): "/radarr/api", ("/ping", "get"): "/radarr/ping"},
    "seerr": {},
}
# A handler that matches one of these and never reads the app's state only answers a fixed empty value.
STUB = re.compile(r"empty_list|vec!\[\]|json!\(\[\]\)|json!\(\{\}\)")
# A fallback for a bad data file is not a stub; it is dropped before the check above.
FALLBACK = re.compile(r"\.unwrap_or_else\(\|_\| json!\(\[\]\)\)")


def matching(text, start):
    """Returns the index just past the parenthesis that closes the one opened right before `start`."""
    depth = 1
    i = start
    in_str = False
    while i < len(text) and depth:
        c = text[i]
        if in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        i += 1
    return i


def marquee_routes(app):
    """Returns {(path, method): "Done" | "Stub"} for every route Marquee serves for one upstream app."""
    found = {}
    for file, prefix in ROUTE_FILES[app]:
        text = (ROOT / file).read_text()
        for m in re.finditer(r"\.route\(\s*\"([^\"]+)\"\s*,", text):
            end = matching(text, m.end()) - 1
            body = text[m.end() : end]
            # Method calls at the top level of the route's second argument.
            depth = 0
            i = 0
            while i < len(body):
                c = body[i]
                if c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
                elif depth == 0:
                    mm = re.match(r"(?:axum::routing::)?(get|post|put|delete|patch)\(", body[i:])
                    if mm and (i == 0 or not (body[i - 1].isalnum() or body[i - 1] == "_")):
                        arg_start = i + mm.end()
                        arg_end = matching(body, arg_start) - 1
                        handler = body[arg_start:arg_end]
                        plain = FALLBACK.sub("", handler)
                        found[(prefix + m.group(1), mm.group(1))] = "Stub" if STUB.search(plain) and "State" not in plain else "Done"
                        i = arg_end + 1
                        continue
                i += 1
    return found


def norm(path):
    """Replaces every `{name}` with `{}` so parameter names don't matter."""
    return re.sub(r"\{[^}]+\}", "{}", path)


def lookup(routes, path, method):
    """Finds the Marquee route for a full path: exact first, then a Marquee `{param}` standing for a word.

    `{id}` only ever stands for an id, so it never matches a word such as `bulk` or `schema`.
    """
    want = norm(path)
    for (p, m), status in routes.items():
        if m == method and norm(p) == want:
            return status
    parts = want.split("/")
    for (p, m), status in routes.items():
        mine = norm(p).split("/")
        if m == method and len(mine) == len(parts) and all(a == b or (a == "{}" and o != "{id}") for a, b, o in zip(mine, parts, p.split("/"))):
            return status
    return "Missing"


def upstream_paths(app, folder):
    """Returns sorted [(path, method, summary)] from the upstream API spec."""
    if app == "seerr":
        spec = yaml.safe_load((folder / "seerr-api.yml").read_text())
    else:
        api = "Sonarr.Api.V3" if app == "sonarr" else "Radarr.Api.V3"
        spec = json.loads((folder / "src" / api / "openapi.json").read_text())
    out = []
    for path, ops in spec["paths"].items():
        for method, op in ops.items():
            if method in METHODS:
                summary = (op.get("summary") or "").strip().splitlines()[0] if op.get("summary") else ""
                out.append((path, method, summary))
    return sorted(out)


def table(app, lock, folder):
    """Writes docs/parity/<app>-api.md and returns (done, stub, missing) counts."""
    routes = marquee_routes(app)
    prefix = {"sonarr": "/sonarr", "radarr": "/radarr", "seerr": "/api/v1"}[app]
    rows = []
    counts = {"Done": 0, "Stub": 0, "Missing": 0}
    for path, method, summary in upstream_paths(app, folder):
        if (path, method) in EXTRA[app]:
            status = "Done"
        elif app == "seerr":
            status = lookup(routes, prefix + path, method)
        elif path.startswith("/api/v3"):
            status = lookup(routes, prefix + path, method)
        else:
            status = "Missing"
        counts[status] += 1
        mark = {"Done": "✅ Done", "Stub": "🟡 Stub", "Missing": "⬜ Missing"}[status]
        rows.append(f"| `{method.upper()}` | `{path}` | {mark} |" + (f" {summary.replace('|', '/')} |" if app == "seerr" else ""))
    info = lock[app]
    title = {"sonarr": "Sonarr", "radarr": "Radarr", "seerr": "Seerr"}[app]
    base = {"sonarr": "`/sonarr`", "radarr": "`/radarr`", "seerr": "`/api/v1` (no prefix)"}[app]
    total = sum(counts.values())
    head = [
        f"# {title} API parity",
        "",
        f"Written by `scripts/parity.py`; don't edit by hand. Upstream: {title} {info['tag']} "
        f"([`{info['commit'][:10]}`]({info['upstream']}/tree/{info['commit']})). Marquee serves it under {base}.",
        "",
        f"**{counts['Done']} done, {counts['Stub']} stub, {counts['Missing']} missing of {total}.** Stub means Marquee "
        "answers with a fixed empty value so clients don't fail, but does nothing yet.",
        "",
        "| Method | Path | Marquee |" + (" Upstream summary |" if app == "seerr" else ""),
        "| --- | --- | --- |" + (" --- |" if app == "seerr" else ""),
    ]
    (ROOT / "docs" / "parity" / f"{app}-api.md").write_text("\n".join(head + rows) + "\n")
    return counts


def main():
    """Checks the checkouts against UPSTREAM.lock and writes the three endpoint tables."""
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    folder = Path(sys.argv[1])
    lock = tomllib.loads((ROOT / "UPSTREAM.lock").read_text())
    (ROOT / "docs" / "parity").mkdir(parents=True, exist_ok=True)
    for app in ("sonarr", "radarr", "seerr"):
        head = subprocess.run(["git", "-C", str(folder / app), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
        if head != lock[app]["commit"]:
            sys.exit(f"{folder / app} is at {head}, but UPSTREAM.lock pins {lock[app]['commit']}. Check out that commit first.")
        c = table(app, lock, folder / app)
        summary(app, c)
        print(f"{app}: {c['Done']} done, {c['Stub']} stub, {c['Missing']} missing")


def summary(app, c):
    """Rewrites this app's counts in the API endpoints table of docs/PARITY.md (the row naming its table file)."""
    path = ROOT / "docs" / "PARITY.md"
    text = path.read_text()
    row = re.compile(r"^\| ([^|]+) \| \d+ \| \d+ \| \d+ \| (\[parity/" + app + r"-api\.md\].*)$", re.M)
    text = row.sub(lambda m: f"| {m.group(1)} | {c['Done']} | {c['Stub']} | {c['Missing']} | {m.group(2)}", text)
    path.write_text(text)


if __name__ == "__main__":
    main()
