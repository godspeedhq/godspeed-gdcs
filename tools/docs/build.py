#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generates the documentation site from the repository.

    python3 tools/docs/build.py [<out dir>]      default: .site

Everything on the site comes from the code and the documents beside it, so it is never edited by hand and
is never out of date once rebuilt:

    Home             README.md
    Specification    SPECIFICATION.md, one page per numbered section
    Commands         the clap definitions and doc comments in src/main.rs, and main's exit codes
    Tests            every test identifier in src/, with its version, title and criterion
    Declaration      the module comment and limits in src/declare.rs
    Report format    schema/report.schema.json
    Examples         examples/*: each README and its gdcs.toml
    Fixtures         fixtures/README.md
    Source           the module comment of every file in src/
    Status           backlog/

The output is Markdown with front matter for Jekyll and the Just the Docs theme; tools/docs/publish.sh,
through .github/workflows/pages.yml, turns it into the GitHub Pages site. Needs only Python 3.

A generator that cannot read what it expects stops with an error: a page quietly missing its table would
look like a suite with nothing in it.
"""
import json
import pathlib
import posixpath
import re
import subprocess
import sys
from datetime import datetime, timezone

SLUG = "godspeedhq/godspeed-gdcs"
BASE = "/godspeed-gdcs"
BLOB = f"https://github.com/{SLUG}/blob/main"
TREE = f"https://github.com/{SLUG}/tree/main"
REPO = pathlib.Path(__file__).resolve().parents[2]


def die(why):
    sys.exit(f"build: {why}")


def read(rel):
    p = REPO / rel
    if not p.is_file():
        die(f"{rel} is missing")
    return p.read_text(encoding="utf-8")


def page(out, rel, front, body):
    """Writes one page: front matter, then the body inside raw tags so Liquid never reads it."""
    if "{% endraw" in body:
        die(f"{rel}: the body closes the raw block")
    target = out / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    head = "".join(f"{k}: {v}\n" for k, v in front.items())
    target.write_text(f"---\n{head}---\n\n{{% raw %}}\n{body.rstrip()}\n\n{{% endraw %}}\n", encoding="utf-8")


def outside_fences(text):
    """Yields (line, in_fence) for each line, so nothing inside a code block is rewritten."""
    fence = False
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            yield line, True
            fence = not fence
        else:
            yield line, fence


def clean(text):
    """Drops the licence comment and the document's own title line."""
    text = re.sub(r"^<!-- SPDX-License-Identifier:.*?-->\n", "", text)
    return re.sub(r"^# .*\n", "", text, count=1)


def relink(text, here, anchors):
    """Sends a repository link to its page on the site, or to GitHub if the site has no page for it.

    `here` is the directory the document lives in, so a link in examples/hda/README.md resolves from there.
    """

    def one(m):
        target = m.group(2)
        if re.match(r"[a-z]+:", target) or target.startswith("#"):
            return m.group(0)
        path, _, frag = target.partition("#")
        rel = posixpath.normpath(posixpath.join(here, path))
        if rel == "README.md":
            url = f"{BASE}/" + (f"#{frag}" if frag else "")
        elif rel == "SPECIFICATION.md":
            url = anchors.get(frag, f"{BASE}/spec/") if frag else f"{BASE}/spec/"
        elif (REPO / rel).is_dir():
            url = f"{TREE}/{rel}"
        elif (REPO / rel).is_file():
            url = f"{BLOB}/{rel}" + (f"#{frag}" if frag else "")
        else:
            print(f"build: a link in {here or '.'} names {target}, which does not exist", file=sys.stderr)
            return m.group(0)
        return f"{m.group(1)}({url})"

    return "\n".join(
        line if fenced else re.sub(r"(\[[^\]]*\])\(([^)\s]+)\)", one, line) for line, fenced in outside_fences(text)
    )


def demote(text, levels):
    """Moves every heading down `levels`, leaving code blocks alone."""
    out = []
    for line, fenced in outside_fences(text):
        out.append("#" * levels + line if not fenced and re.match(r"#+ ", line) else line)
    return "\n".join(out)


def cell(text):
    """Makes text safe inside a table cell."""
    return " ".join(text.split()).replace("|", "\\|")


def heading_id(title):
    """The id kramdown's GFM parser gives a heading."""
    title = re.sub(r"[^\w\- ]", "", title.lower())
    return title.replace(" ", "-")


def module_comment(rel):
    """The //! comment at the top of a Rust file, as Markdown."""
    lines = [m.group(1) for m in re.finditer(r"^//! ?(.*)$", read(rel), re.M)]
    if not lines:
        die(f"{rel} has no module comment")
    return "\n".join(lines)


# --- Specification, one page per "## N. Title" -------------------------------------------------------------


def split_spec():
    """Returns the preamble, the sections as (number, title, lines), and each heading's address on the site."""
    preamble, sections, anchors = [], [], {}
    current = preamble
    for line, fenced in outside_fences(clean(read("SPECIFICATION.md"))):
        m = None if fenced else re.match(r"## (\d+)\. (.+)$", line)
        if m:
            number = int(m.group(1))
            current = []
            sections.append((number, m.group(2).strip(), current))
            anchors[heading_id(f"{number}. {m.group(2)}")] = f"{BASE}/spec/{number:02d}/"
            continue
        if not fenced and sections and line.startswith("###"):
            title = line.lstrip("#").strip()
            anchors[heading_id(title)] = f"{BASE}/spec/{sections[-1][0]:02d}/#{heading_id(title)}"
            line = line[1:]  # A section page starts at "#": its subsections move up a level.
        current.append(line)
    if not sections:
        die("SPECIFICATION.md has no numbered sections")
    return "\n".join(preamble), sections, anchors


# --- Commands, from the clap definitions ---------------------------------------------------------------------


def parse_commands():
    source = read("src/main.rs")
    m = re.search(r"enum Command \{\n(.*?)^\}", source, re.S | re.M)
    if not m:
        die("src/main.rs: no `enum Command`")
    commands, doc, attrs, current = [], [], [], None
    for raw in m.group(1).split("\n"):
        line = raw.strip()
        if line.startswith("///"):
            doc.append(line[3:].strip())
        elif line.startswith("#["):
            attrs.append(line)
        elif re.match(r"[A-Z]\w* \{$", line):
            current = {"name": line.split()[0].lower(), "doc": " ".join(doc), "args": []}
            commands.append(current)
            doc, attrs = [], []
        elif current is not None and re.match(r"\w+: ", line):
            name, kind = line.rstrip(",").split(": ", 1)
            attr = " ".join(attrs)
            default = re.search(r'default_value(?:_t)? = ("?)([^",)]+)\1', attr)
            current["args"].append(
                {
                    "flag": ("--" + name.replace("_", "-")) if "long" in attr else f"<{name}>",
                    "short": "short" in attr,
                    "kind": kind,
                    "default": default.group(2) if default else None,
                    "doc": " ".join(doc),
                }
            )
            doc, attrs = [], []
    if not commands:
        die("src/main.rs: `enum Command` has no commands")
    return commands


def commands_page():
    source = read("src/main.rs")
    version = re.search(r'^version = "([^"]+)"', read("Cargo.toml"), re.M).group(1)
    exits = re.search(r"^/// (Exit codes: .*(?:\n/// .*)*)\nfn main", source, re.M)
    if not exits:
        die("src/main.rs: main has no exit-code comment")
    out = [
        "# Commands\n",
        f"`gdcs` {version}. Taken from the command-line definitions in [`src/main.rs`]({BLOB}/src/main.rs), "
        "so this is what the program accepts.\n",
    ]
    for c in parse_commands():
        out.append(f"## `gdcs {c['name']}`\n\n{c['doc']}\n")
        out.append("| Argument | Default | Meaning |\n| --- | --- | --- |")
        for a in c["args"]:
            flag = f"`{a['flag']}`" + (f", `-{a['flag'][2]}`" if a["short"] else "")
            required = a["default"] is None and not a["kind"].startswith(("Option", "bool"))
            default = f"`{a['default']}`" if a["default"] else ("required" if required else "")
            out.append(f"| {flag} | {default} | {cell(a['doc'])} |")
        out.append("")
    text = re.sub(r"\n/// ", " ", exits.group(1))
    out.append("## Exit codes\n")
    head, _, rest = text.partition(": ")
    for item in re.split(r",\s*(?=\d )", rest.rstrip(".")):
        code, _, meaning = item.partition(" ")
        out.append(f"- `{code}`: {meaning}")
    return "\n".join(out)


# --- Tests, from every identifier in src/ ------------------------------------------------------------------


def literal(text):
    """Joins a Rust string literal that may run over lines with a trailing backslash."""
    return re.sub(r"\\\n\s*", "", text)


def parse_tests():
    tests = {}
    sources = sorted((REPO / "src").glob("*.rs"))
    for path in sources:
        text = path.read_text(encoding="utf-8")
        rel = f"src/{path.name}"
        for m in re.finditer(
            r'id: "(S\d\.[\w-]+)",\s*version: (\d+),\s*title: "((?:[^"\\]|\\[\s\S])*)",\s*criterion:\s*"((?:[^"\\]|\\[\s\S])*)"',
            text,
        ):
            tests[m.group(1)] = (m.group(2), literal(m.group(3)), literal(m.group(4)), rel)
        for m in re.finditer(r'id: "(S\d\.[\w-]+)",\s*tag: "[^"]*",\s*title: "((?:[^"\\]|\\[\s\S])*)"', text):
            tests[m.group(1)] = (None, literal(m.group(2)), None, rel)
        for m in re.finditer(r'\(\s*"(S\d\.[\w-]+)",\s*(\d+),\s*"((?:[^"\\]|\\[\s\S])*)",?\s*\)', text):
            tests[m.group(1)] = (m.group(2), literal(m.group(3)), None, rel)
        for m in re.finditer(r'\(\s*"(S\d\.[\w-]+)",\s*"((?:[^"\\]|\\[\s\S])*)",?\s*\)', text):
            tests.setdefault(m.group(1), (None, literal(m.group(2)), None, rel))
    # Every identifier the code names must be in the table: one it missed is a test the site would hide.
    named = set()
    for path in sources:
        named |= set(re.findall(r'"(S\d\.[\w-]+)"', path.read_text(encoding="utf-8")))
    missed = sorted(named - set(tests))
    if missed:
        die(f"tests named in src/ that this generator did not read: {', '.join(missed)}")
    return tests


def tests_page(sections):
    tests = parse_tests()
    titles = {}
    for line in read("SPECIFICATION.md").split("\n"):
        m = re.match(r"### Stage (\d) - (.+)$", line)
        if m:
            titles[m.group(1)] = m.group(2).strip()
    out = [
        "# Tests\n",
        f"Every test identifier in [`src/`]({TREE}/src), {len(tests)} in all, read from the definitions a report "
        "takes its results from. A test's version rises when its criterion changes. Where a test states its "
        "criterion in the code it is shown here; the rest are stated in the "
        f"[specification]({BASE}/spec/04/).\n",
    ]
    stages = sorted({i[1] for i in tests})
    for stage in sorted(set(stages) | set(titles)):
        name = titles.get(stage, "")
        out.append(f"## Stage {stage}" + (f" - {name}\n" if name else "\n"))
        rows = sorted(i for i in tests if i[1] == stage)
        if not rows:
            out.append("No test is built for this stage yet.\n")
            continue
        out.append("| Test | Version | What it shows | Criterion | Defined in |\n| --- | --- | --- | --- | --- |")
        for i in rows:
            version, title, criterion, rel = tests[i]
            link = f"[`{rel}`]({BLOB}/{rel})"
            out.append(f"| `{i}` | {version or ''} | {cell(title)} | {cell(criterion or '')} | {link} |")
        out.append("")
    return "\n".join(out)


# --- Declaration ---------------------------------------------------------------------------------------------


def declaration_page():
    source = read("src/declare.rs")
    limits = []
    for name, what in (("FORMAT", "the format this suite reads"),
                       ("DEFAULT_WITHIN_SECS", "seconds a behaviour is waited for when it does not say"),
                       ("MAX_WITHIN_SECS", "the most seconds a behaviour may ask for")):
        m = re.search(rf"pub const {name}: \w+ = (\d+);", source)
        if not m:
            die(f"src/declare.rs: no {name}")
        limits.append(f"| `{name}` | {m.group(1)} | {what} |")
    table = "| Constant | Value | Meaning |\n| --- | --- | --- |\n" + "\n".join(limits)
    return (
        "# The driver's declaration\n\n"
        + re.sub(r"^.*?\n", "", module_comment("src/declare.rs"), count=1).lstrip("\n")
        + f"\n\n## Limits\n\nFrom [`src/declare.rs`]({BLOB}/src/declare.rs).\n\n{table}\n"
    )


# --- Report format, from the JSON Schema ----------------------------------------------------------------------


def walk(schema, path, required, rows):
    kind = schema.get("type", "")
    if isinstance(kind, list):
        kind = " or ".join(kind)
    if "const" in schema:
        kind = f"always {json.dumps(schema['const'])}"
    if "enum" in schema:
        kind = "one of " + ", ".join(f"`{v}`" for v in schema["enum"])
    if "pattern" in schema:
        kind = f"{kind}, `{schema['pattern']}`"
    rows.append((path, kind, "yes" if required else "", schema.get("description", "")))
    for name, sub in schema.get("properties", {}).items():
        walk(sub, f"{path}.{name}" if path else name, name in schema.get("required", []), rows)
    if isinstance(schema.get("items"), dict):
        walk(schema["items"], f"{path}[]", True, rows)


def report_page():
    schema = json.loads(read("schema/report.schema.json"))
    rows = []
    for name, sub in schema["properties"].items():
        walk(sub, name, name in schema.get("required", []), rows)
    out = [
        "# Report format\n",
        cell(schema.get("description", "")) + "\n",
        f"Generated from [`schema/report.schema.json`]({BLOB}/schema/report.schema.json).\n",
        "| Field | Type | Required | Meaning |\n| --- | --- | --- | --- |",
    ]
    out += [f"| `{p}` | {cell(k)} | {r} | {cell(d)} |" for p, k, r, d in rows]
    return "\n".join(out)


# --- Examples, fixtures, source, status ---------------------------------------------------------------------


def examples_page(anchors):
    out = [f"# Examples\n\nThe drivers in [`examples/`]({TREE}/examples), each with the declaration GDCS reads.\n"]
    folders = sorted(p for p in (REPO / "examples").iterdir() if p.is_dir())
    if not folders:
        die("examples/ is empty")
    for folder in folders:
        rel = f"examples/{folder.name}"
        out.append(f"## {folder.name}\n")
        readme = relink(clean(read(f"{rel}/README.md")), rel, anchors)
        out.append(demote(readme, 2).strip() + "\n")
        out.append(f"### `gdcs.toml`\n\n```toml\n{read(f'{rel}/gdcs.toml').strip()}\n```\n")
    return "\n".join(out)


def source_page():
    out = [f"# Source\n\nThe module comment of each file in [`src/`]({TREE}/src).\n"]
    for path in sorted((REPO / "src").glob("*.rs")):
        rel = f"src/{path.name}"
        out.append(f"## [`{rel}`]({BLOB}/{rel})\n\n{demote(module_comment(rel), 2)}\n")
    return "\n".join(out)


def status_page():
    out = [f"# Status\n\nOpen and closed items, from [`backlog/`]({TREE}/backlog).\n"]
    entries = sorted(p for p in (REPO / "backlog").glob("*.md") if p.name != "README.md")
    for path in entries:
        text = path.read_text(encoding="utf-8")
        title = re.search(r"^# (.+)$", text, re.M)
        status = re.search(r"^\*\*Status:\*?\*?\s*(.+)$", text, re.M)
        rel = f"backlog/{path.name}"
        out.append(f"## [{title.group(1) if title else path.stem}]({BLOB}/{rel})\n")
        out.append((status.group(0) if status else "No status line.") + "\n")
    if not entries:
        out.append("Nothing is open.\n")
    return "\n".join(out)


def main():
    out = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else REPO / ".site"
    if out.exists():
        import shutil

        shutil.rmtree(out)
    out.mkdir(parents=True)
    try:
        commit = subprocess.check_output(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"], text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        commit = "unknown"
    built = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

    preamble, sections, anchors = split_spec()

    page(out, "index.md", {"title": "Home", "layout": "home", "nav_order": 1, "permalink": "/"},
         relink(clean(read("README.md")), "", anchors))

    page(out, "spec/index.md",
         {"title": "Specification", "nav_order": 2, "has_children": True, "permalink": "/spec/"},
         "# Specification\n\n" + relink(preamble, "", anchors))
    for number, title, lines in sections:
        quoted = json.dumps(f"{number}. {title}")
        page(out, f"spec/{number:02d}.md",
             {"title": quoted, "parent": "Specification", "nav_order": number, "permalink": f"/spec/{number:02d}/"},
             f"# {number}. {title}\n" + relink("\n".join(lines), "", anchors))

    page(out, "reference/index.md",
         {"title": "Reference", "nav_order": 3, "has_children": True, "permalink": "/reference/"},
         "# Reference\n\nGenerated from the code: the commands, the tests, the two file formats and the source.\n")
    reference = [
        ("commands", "Commands", commands_page()),
        ("tests", "Tests", tests_page(sections)),
        ("declaration", "Driver declaration", declaration_page()),
        ("report", "Report format", report_page()),
        ("examples", "Examples", examples_page(anchors)),
        ("fixtures", "Fixtures", "# Fixtures\n\n" + relink(clean(read("fixtures/README.md")), "fixtures", anchors)),
        ("source", "Source", source_page()),
    ]
    for order, (slug, title, body) in enumerate(reference, 1):
        page(out, f"reference/{slug}.md",
             {"title": title, "parent": "Reference", "nav_order": order, "permalink": f"/reference/{slug}/"},
             body)

    page(out, "status.md", {"title": "Status", "nav_order": 4, "permalink": "/status/"}, status_page())

    (out / "_config.yml").write_text(
        f"""title: GDCS
description: The Godspeed Driver Conformance Suite.
baseurl: {BASE}
remote_theme: just-the-docs/just-the-docs@v0.10.1
# Just the Docs' built-in dark scheme.
color_scheme: dark
plugins:
  - jekyll-remote-theme
search_enabled: true
heading_anchors: true
aux_links:
  GitHub: https://github.com/{SLUG}
footer_content: "Generated from <a href=\\"https://github.com/{SLUG}/commit/{commit}\\">{commit}</a> on {built}. Apache License 2.0."
markdown: kramdown
kramdown:
  input: GFM
  hard_wrap: false
""",
        encoding="utf-8",
    )
    count = len(list(out.rglob("*.md")))
    print(f"built {count} pages in {out} from {commit}")


if __name__ == "__main__":
    main()
