#!/usr/bin/env python3
"""Turns the [TestCase(...)] attributes of an upstream NUnit fixture (Sonarr, Radarr) into JSON, so the Rust
ports run the upstream cases unchanged.

    python3 scripts/upstream_cases.py <Fixture.cs> > server/tests/upstream/<app>/<Fixture>.json

The JSON maps each test method's name to its cases; each case is the list of arguments. C# strings, numbers,
true/false and null become JSON values; an enum or constant (`Quality.HDTV720p`) becomes {"ref": "Quality.HDTV720p"};
`new[] { ... }` becomes a list (`new string[0]` an empty one). Cases with `Ignore = ...` are left out, like NUnit does.
"""
import json
import re
import sys


def parse_args(s, i):
    """Parses comma-separated C# expressions from s[i] up to the matching ')' or '}'; returns (values, named, end)."""
    vals, named = [], {}
    while True:
        while s[i] in " \t\r\n,":
            i += 1
        if s[i] in ")}":
            return vals, named, i + 1
        m = re.match(r"([A-Za-z_]\w*)\s*=(?!=)\s*", s[i:])
        key = None
        if m:
            key = m.group(1)
            i += m.end()
        v, i = parse_value(s, i)
        if key:
            named[key] = v
        else:
            vals.append(v)


def parse_value(s, i):
    """Parses one C# expression at s[i]; returns (value, next index)."""
    if s.startswith('@"', i):
        j, out = i + 2, []
        while True:
            if s[j] == '"':
                if s[j + 1:j + 2] == '"':
                    out.append('"')
                    j += 2
                    continue
                return "".join(out), j + 1
            out.append(s[j])
            j += 1
    if s[i] == '"':
        j, out = i + 1, []
        esc = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", '"': '"', "\\": "\\", "'": "'"}
        while s[j] != '"':
            if s[j] == "\\":
                c = s[j + 1]
                if c == "u":
                    out.append(chr(int(s[j + 2:j + 6], 16)))
                    j += 6
                    continue
                out.append(esc.get(c, c))
                j += 2
                continue
            out.append(s[j])
            j += 1
        return "".join(out), j + 1
    m = re.match(r"new\s*(?:[\w.<>]+\s*)?\[\s*\]\s*\{", s[i:])
    if m:
        vals, _, j = parse_args(s, i + m.end())
        return vals, j
    m = re.match(r"new\s*[\w.<>]+\s*\[\s*0\s*\]", s[i:])
    if m:
        return [], i + m.end()
    m = re.match(r"-?\d+(?:\.\d+)?[dDfFmMlL]?", s[i:])
    if m:
        t = m.group(0).rstrip("dDfFmMlL")
        return (float(t) if "." in t else int(t)), i + m.end()
    m = re.match(r"(?:\(\s*[\w.]+\s*\)\s*)?([A-Za-z_][\w.]*)", s[i:])
    if m:
        name = m.group(1)
        j = i + m.end()
        if name == "true":
            return True, j
        if name == "false":
            return False, j
        if name == "null":
            return None, j
        if name == "nameof" or s[j:j + 1] == "(":
            depth, k = 0, j
            while True:
                if s[k] == "(":
                    depth += 1
                elif s[k] == ")":
                    depth -= 1
                    if depth == 0:
                        return {"expr": s[i:k + 1]}, k + 1
                k += 1
        return {"ref": name}, j
    raise ValueError(f"can't read a C# value at: {s[i:i+40]!r}")


def main(path):
    src = open(path, encoding="utf-8-sig").read()
    out, pending = {}, []
    i = 0
    attr = re.compile(r"\[\s*(TestCase|Test|TestCaseSource)\b")
    method = re.compile(r"public\s+(?:async\s+)?[\w<>\[\]]+\s+(\w+)\s*\(")
    while i < len(src):
        a = attr.match(src, i)
        if a:
            j = a.end()
            while src[j] in " \t":
                j += 1
            if a.group(1) == "TestCase" and src[j] == "(":
                vals, named, j = parse_args(src, j + 1)
                if "Ignore" not in named and "IgnoreReason" not in named:
                    pending.append(vals)
            elif a.group(1) == "TestCaseSource" and src[j] == "(":
                _, named, j = parse_args(src, j + 1)
            i = j
            continue
        m = method.match(src, i)
        if m:
            if pending:
                out.setdefault(m.group(1), []).extend(pending)
            pending = []
            i = m.end()
            continue
        i += 1
    json.dump(out, sys.stdout, ensure_ascii=False, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
