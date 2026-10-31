#!/usr/bin/env python3
"""Validate checked-in documentation against the code it describes.

`cargo run -p xtask --bin generate_sdk_surfaces -- --check` holds every
generated page, `llms.txt` and the OpenAPI routes to the endpoint table byte
for byte. This script checks what that comparison cannot see:

- the OpenAPI path set matching the routes the server serves, its `servers`
  block naming the local server, and no global security requirement
- the server docs' flag defaults matching the server's command line
- the MCP docs listing and counting every tool the server advertises
- the getting-started install line naming the current version

Exits non-zero on any mismatch. Run from repo root; the script resolves
the workspace root from its own path so `cd` is not required.
"""

from __future__ import annotations

from pathlib import Path
import re
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]
ENDPOINT_TABLE_RS = ROOT / "thetadatadx-rs/src/surface/endpoints.rs"
# Every entry of `__endpoints!` and `__stream_endpoints!` in table order: its
# shape, name, subcategory and REST path.
TABLE_ENTRIES = re.findall(
    r'^\s*(list|buffered|chunked|stream)\s+(\w+)\s*\{\s*group:\s*\w+\s*/\s*(\w+),\s*rest:\s*"([^"]+)"',
    ENDPOINT_TABLE_RS.read_text(),
    re.MULTILINE,
)
if not TABLE_ENTRIES:
    raise SystemExit(f"docs consistency error: {ENDPOINT_TABLE_RS} parsed to no endpoints")
# The OpenAPI `servers` block points at the HTTP server root
# (`http://localhost:25503`), so each documented path must carry the full
# served route (the registry `rest_path` verbatim, `/v3/...`) for a
# generated client to resolve it. Compare the registry paths as-is.
REST_PATHS = {rest for _, _, _, rest in TABLE_ENTRIES}

# The single server a generated client targets: the local HTTP server root.
# `tools/server` listens on this port by default (`tools/server/src/main.rs`),
# and the documented paths are the `/v3/...` routes served there. The block
# must name an `http(s)` URL on this host; a backend scheme/host (a `grpc://`
# URL, or an `mdds`/private-backend host) is a request a generated client
# cannot issue and must trip the gate.
OPENAPI_SERVER_URL = "http://localhost:25503"

DOCS_SITE = ROOT / "docs-site/docs"
OPENAPI_YAML = DOCS_SITE / "public/thetadatadx.yaml"

# The MCP server source is the single source of truth for the tool surface a
# connected `tools/list` returns: the registry market-data endpoints (one tool
# per endpoint), the offline utilities (`OFFLINE_TOOL_NAMES` in `main.rs`), and
# the flat-file tools advertised by `push_flatfile_tool_definitions` in
# `flatfile_tools.rs`. The gate parses the non-registry tool names from these
# files so a tool added in code but left out of the docs trips the gate rather
# than the docs carrying a hand-maintained count that silently drifts.
MCP_MAIN_RS = ROOT / "tools/mcp/src/main.rs"
MCP_FLATFILE_TOOLS_RS = ROOT / "tools/mcp/src/flatfile_tools.rs"

# Docs that must enumerate the connection-only MCP tools by name, each paired
# with the `##` heading of the section that carries the tool listing. The
# enumeration is checked inside that section only: a tool name mentioned in
# unrelated prose elsewhere in the file (an intro blockquote, an example) must
# not mask a missing entry in the actual tool list, so removing a tool from the
# listing trips the gate even when the name survives elsewhere.
MCP_DOC_TOOL_SECTIONS = (
    (ROOT / "tools/mcp/README.md", "## Available Tools"),
    (DOCS_SITE / "mcp.md", "## Tools"),
)

# Server source files that register the `/v3` HTTP routes. The server is the
# source of truth for the served route set, so the gate parses the `.route(...)`
# literals from these files rather than carrying a hand-maintained duplicate
# list that could silently drift from what the binary actually serves.
SERVER_ROUTER_FILES = (ROOT / "tools/server/src/router.rs",)

# The server CLI is the source of truth for the documented flag defaults.
# The `#[arg(...)]` attributes on the `struct Args` fields carry each
# flag's default; the docs render those in a `| --flag | default | ... |`
# Markdown table. The gate parses the `#[arg(...)]` defaults from this file
# and asserts the doc-table rows match value-by-value, so a changed server
# default that the docs don't reflect trips the gate (the same
# derive-from-source pattern as `served_v3_routes`). A substring
# check ("25503" appears somewhere) could not catch a default that changed
# to a value still mentioned elsewhere on the page.
SERVER_MAIN_RS = ROOT / "tools/server/src/main.rs"

# `.route("/v3/...")` literal. The path string may sit on the same line as
# `.route(` or wrap onto the next, so `\s*` (which spans newlines) bridges the
# two. Restricting the capture to `/v3/...` naturally excludes any non-served
# test-only route (e.g. a `/probe` probe router under `#[cfg(test)]`).
ROUTE_LITERAL_RE = re.compile(r"\.route\(\s*\"(/v3/[^\"]*)\"")


def served_v3_routes() -> set[str]:
    """The full `/v3` route set the server binary actually serves.

    Parsed from the server router source (`SERVER_ROUTER_FILES`), so a route
    added to or removed from the binary moves this set with it and the OpenAPI
    path-set assertion below tracks the real surface with no edit to this gate.
    Axum path params use `{name}` braces, matching the OpenAPI path templating.
    """
    routes: set[str] = set()
    for path in SERVER_ROUTER_FILES:
        routes |= set(ROUTE_LITERAL_RE.findall(path.read_text()))
    return routes


# Routes the server serves beyond the upstream-tracking registry endpoints:
# the system status / lifecycle routes and the v3 path aliases. These
# are documented in the OpenAPI contract (they are real served routes) but are
# not in `REST_PATHS`. Derived, not hand-listed, so it cannot drift.
SERVER_ONLY_PATHS = served_v3_routes() - REST_PATHS

# The registry endpoints: every table entry but the raw streams, which share
# the route of the endpoint they stream and have no page of their own.
REGISTRY_ENDPOINTS = [
    {"name": name, "rest_path": rest, "subcategory": sub}
    for shape, name, sub, rest in TABLE_ENTRIES
    if shape != "stream"
]


def fail(message: str) -> None:
    print(f"docs consistency error: {message}", file=sys.stderr)
    raise SystemExit(1)


def expect_contains(path: Path, snippet: str) -> None:
    text = path.read_text()
    if snippet not in text:
        fail(f"{path.relative_to(ROOT)} missing expected text: {snippet!r}")


def check_static_docs() -> None:
    # Version strings in getting-started docs must match the canonical
    # workspace version; derive it from the root `Cargo.toml` so the pin
    # tracks bumps automatically instead of pinning a literal that silently
    # ages.
    cargo_version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    expect_contains(
        DOCS_SITE / "articles/getting-started.md",
        f'thetadatadx-rs = "{cargo_version}"',
    )


def _openapi_server_urls(text: str) -> list[str]:
    """Return the `url:` values under the top-level `servers:` block.

    Reads from the `servers:` key to the next top-level key (a line starting in
    column zero), collecting each `- url: <value>` entry in order. Keeps the
    parse local to the block so a `url:` under `info`/`contact`/`license` never
    leaks in.
    """
    lines = text.splitlines()
    urls: list[str] = []
    in_block = False
    for line in lines:
        if not in_block:
            if re.match(r"^servers:\s*$", line):
                in_block = True
            continue
        # A new top-level key (no indentation) ends the servers block.
        if line and not line[0].isspace():
            break
        m = re.match(r"\s*-\s*url:\s*(\S+)", line)
        if m:
            urls.append(m.group(1).strip())
    return urls


def check_openapi() -> None:
    text = OPENAPI_YAML.read_text()

    # The `servers` block is the base URL a generated client prepends to every
    # documented path. It must name the local HTTP server root; a missing /
    # empty block, or a backend scheme/host (a `grpc://` URL or an
    # `mdds`/private-backend host), yields a contract a client cannot call.
    server_urls = _openapi_server_urls(text)
    if not server_urls:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} has no `servers:` entry. "
            f"Declare the local HTTP server: {OPENAPI_SERVER_URL}"
        )
    for url in server_urls:
        scheme = url.split("://", 1)[0].lower() if "://" in url else ""
        if scheme not in {"http", "https"}:
            fail(
                f"{OPENAPI_YAML.relative_to(ROOT)} servers url {url!r} is not an "
                f"http(s) endpoint a generated client can call. Use the local "
                f"HTTP server: {OPENAPI_SERVER_URL}"
            )
        host = url.split("://", 1)[1].split("/", 1)[0].lower()
        if "mdds" in host:
            fail(
                f"{OPENAPI_YAML.relative_to(ROOT)} servers url {url!r} points at a "
                f"backend host. Use the local HTTP server: {OPENAPI_SERVER_URL}"
            )
    if OPENAPI_SERVER_URL not in server_urls:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} servers block is missing the local "
            f"HTTP server url {OPENAPI_SERVER_URL!r}; found {server_urls}"
        )

    # OpenAPI path keys, including templated segments (`{request_type}`): the
    # brace characters must be in the class or a templated path is invisible to
    # the gate, the gap that let a server-only route go undocumented.
    actual_paths = {
        match.group(1)
        for match in re.finditer(r"^  (/[A-Za-z0-9_/{}-]+):\s*$", text, re.MULTILINE)
    }
    # The contract must document the FULL served route set: the upstream-tracking
    # registry endpoints plus every server-only `/v3` route the binary serves
    # (system status / lifecycle + v3 path aliases), derived from the server
    # router source. A route the binary serves but the spec omits leaves a
    # generated client unable to call it; a spec path the binary does not serve
    # is a dangling contract. Either direction trips.
    expected_paths = REST_PATHS | SERVER_ONLY_PATHS
    if actual_paths != expected_paths:
        missing = sorted(expected_paths - actual_paths)
        extra = sorted(actual_paths - expected_paths)
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} path set drifted from the served "
            f"route set. missing={missing or '[]'} extra={extra or '[]'}"
        )

    # No global per-request security scheme. The server authenticates its
    # upstream connection once at startup; request paths carry no per-request
    # credential. The only allowed requirement is the route-scoped shutdown
    # token on POST /v3/system/shutdown. A top-level `security:` block (a
    # document-wide default applied to every operation) is a contract for a
    # credential the server never reads, so it must not reappear.
    if re.search(r"(?m)^security:\s*$", text):
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} declares a global `security:` block. "
            f"The server reads no per-request credential; keep only the "
            f"route-scoped shutdown token on POST /v3/system/shutdown."
        )


def _struct_body(text: str, struct_name: str) -> str | None:
    """Return the `{ ... }` body of `struct <struct_name> { ... }`,
    brace-balanced. Used to scope the `#[arg(...)]` scan to the CLI args
    struct so an `#[arg]` on an unrelated type cannot leak in.
    """
    header = re.search(rf"struct\s+{re.escape(struct_name)}\s*\{{", text)
    if not header:
        return None
    depth = 1
    i = header.end()
    while i < len(text) and depth > 0:
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return text[header.end() : i - 1]


def server_arg_defaults() -> dict[str, str | None]:
    """Derive `{--flag: default}` for every server CLI flag from the
    `#[arg(...)]` attributes on `struct Args` in `tools/server/src/main.rs`.

    The default is:

    * the `default_value = "X"` string literal, verbatim;
    * the `default_value_t = N` literal (an integer such as `25503`);
    * for `default_value_t = <path>::Variant` (a clap `ValueEnum`), the
      kebab/lower-cased last path segment — clap renders `LogFormat::Text`
      as `text` with no `#[value(name=...)]` override;
    * `None` when the field carries no `default_value*` (an `Option<T>` or
      `bool` flag), meaning the doc-table default cell must be empty.

    The flag long name is clap's snake→kebab derivation of the field name
    (`http_port` -> `--http-port`). `value_parser = [...]` does not set a
    default and is ignored.
    """
    text = SERVER_MAIN_RS.read_text()
    body = _struct_body(text, "Args")
    if body is None:
        fail(f"{SERVER_MAIN_RS.relative_to(ROOT)} has no `struct Args` body")

    defaults: dict[str, str | None] = {}
    # Each field is an `#[arg(...)]` attribute followed by `<field>: <type>`.
    # The attribute arg list can itself contain `[...]` (a
    # `value_parser = ["production", "dev"]` allow-list) and nested `(...)`,
    # so the inner span is read with a bracket/paren balance counter — a
    # naive `#[arg([^\]]*)]` stops at the first `]` inside the value_parser
    # array and drops the flag. After the attribute's closing `]`, the next
    # identifier before a `:` is the field name.
    after_field_re = re.compile(r"\s*(?:#\[[^\]]*\]\s*)*([a-z_][a-z0-9_]*)\s*:")
    for m in re.finditer(r"#\[\s*arg\b", body):
        i = m.end()
        while i < len(body) and body[i].isspace():
            i += 1
        attr = ""
        if i < len(body) and body[i] == "(":
            attr, after = _balanced_paren_span(body, i)
            if after is None:
                continue
        else:
            after = i
        # Advance past the attribute's closing `]`.
        while after < len(body) and body[after] != "]":
            after += 1
        after += 1
        fm = after_field_re.match(body, after)
        if not fm:
            continue
        field = fm.group(1)
        flag = "--" + field.replace("_", "-")
        defaults[flag] = _arg_default_from_attr(attr)
    if not defaults:
        fail(
            f"{SERVER_MAIN_RS.relative_to(ROOT)} `struct Args` parsed to no "
            "`#[arg(...)]` flags"
        )
    return defaults


def _balanced_paren_span(text: str, open_idx: int) -> tuple[str, int | None]:
    """Given `open_idx` at a `(`, return `(inner, after)` where `inner` is
    the balanced content up to the matching `)` and `after` is the index
    just past that `)`. Tracks `"..."` string literals (with `\\` escapes)
    and nested `(` / `[` so a `)` or `]` inside a string or a
    `value_parser = [...]` array does not close the span early. Returns
    `("", None)` if unbalanced.
    """
    depth = 0
    in_str = False
    esc = False
    start = open_idx + 1
    i = open_idx
    n = len(text)
    while i < n:
        c = text[i]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return text[start:i], i + 1
        i += 1
    return "", None


def _arg_default_from_attr(attr: str) -> str | None:
    """Extract the rendered default from one `#[arg(...)]` inner text, or
    `None` when the attribute sets no default.
    """
    # `default_value = "X"` — string literal verbatim.
    m = re.search(r'default_value\s*=\s*"([^"]*)"', attr)
    if m:
        return m.group(1)
    # `default_value_t = <expr>`.
    m = re.search(r"default_value_t\s*=\s*([^,]+)", attr)
    if m:
        expr = m.group(1).strip()
        # A clap `ValueEnum` path (`logging::LogFormat::Text`) renders as
        # the kebab/lower-cased last segment.
        if "::" in expr:
            variant = expr.rsplit("::", 1)[1]
            return _value_enum_render(variant)
        # Otherwise a literal (integer like `25503`, or `true`/`false`).
        return expr
    return None


def _value_enum_render(variant: str) -> str:
    """Render a clap `ValueEnum` PascalCase variant the way clap does by
    default: lower-case, words separated by `-` (`OpenInterest` ->
    `open-interest`, `Text` -> `text`). Matches clap's default
    `to_possible_value` kebab-casing when no `#[value(name=...)]` override
    is present.
    """
    out: list[str] = []
    for i, ch in enumerate(variant):
        if ch.isupper() and i > 0:
            out.append("-")
        out.append(ch.lower())
    return "".join(out)


# Markdown flag-table row: `| --flag[ <metavar>] | default | description |`.
# The first cell may carry a metavar (`--creds <path>`) or pair two flags
# (`--email / --password`); the default cell may be empty or an em-dash
# placeholder. The parser below normalises both.
_FLAG_ROW_RE = re.compile(
    r"^\|\s*(?P<flags>`[^|]+?`(?:\s*/\s*`[^|]+?`)*)\s*\|"
    r"\s*(?P<default>[^|]*?)\s*\|",
    re.MULTILINE,
)
_FLAG_CELL_RE = re.compile(r"`(--[a-z][a-z0-9-]*)(?:\s+[^`]*)?`")


def _parse_doc_flag_table(text: str) -> dict[str, str | None]:
    """Parse a Markdown flags table into `{--flag: default-or-None}`.

    Each flag in the first cell (a row may list two, `--email / --password`)
    maps to the row's default cell. An empty cell or an em-dash / hyphen
    placeholder (`—` / `-`) means "no default" and maps to `None`. The
    default literal is unwrapped from surrounding backticks so it compares
    against the raw `#[arg]` default.
    """
    out: dict[str, str | None] = {}
    for m in _FLAG_ROW_RE.finditer(text):
        flags = _FLAG_CELL_RE.findall(m.group("flags"))
        if not flags:
            continue
        raw_default = m.group("default").strip()
        # Strip backticks and treat placeholders as "no default".
        unwrapped = raw_default.strip("`").strip()
        default: str | None
        if unwrapped in ("", "—", "-", "–"):
            default = None
        else:
            default = unwrapped
        for flag in flags:
            out[flag] = default
    return out


def check_server_flag_defaults() -> None:
    """Assert the server flag-default tables in the docs match the
    `#[arg(...)]` defaults derived from `tools/server/src/main.rs`,
    value-by-value. A substring check could not catch a default that
    silently changed to a value still printed elsewhere on the page.

    Both the server README and the docs-site server page carry the table;
    each is checked. A doc may legitimately omit a flag from its table
    (e.g. a terse page), so the gate asserts agreement only for the flags a
    given table DOES document — but a documented flag whose default
    disagrees with the source, or a documented default for a flag the
    source says has none (and vice versa), trips.
    """
    source_defaults = server_arg_defaults()
    doc_tables = (
        ROOT / "tools/server/README.md",
        DOCS_SITE / "server/index.md",
    )
    for doc in doc_tables:
        if not doc.is_file():
            fail(f"{doc.relative_to(ROOT)}: server flag-table doc not found")
        documented = _parse_doc_flag_table(doc.read_text())
        # The table must document a meaningful subset of the real flags;
        # if it parsed to nothing, the table shape drifted and the gate is
        # silently blind — fail loudly.
        overlap = set(documented) & set(source_defaults)
        if not overlap:
            fail(
                f"{doc.relative_to(ROOT)}: no server flags parsed from the "
                "flag-default table (the table shape changed — the gate would "
                "be blind to default drift)"
            )
        for flag in sorted(overlap):
            want = source_defaults[flag]
            got = documented[flag]
            if want != got:
                fail(
                    f"{doc.relative_to(ROOT)} documents {flag} default as "
                    f"{got!r}, but `tools/server/src/main.rs` `#[arg]` sets "
                    f"{want!r}"
                )


def _rust_str_array_items(text: str, const_name: str, path: Path) -> list[str]:
    """The string literals in a `const NAME: [...] = [ "a", "b", ... ];` array.

    Used to read `OFFLINE_TOOL_NAMES` from the MCP `main.rs` and `TOOL_NAMES`
    from `stream.rs`, so each tool set tracks its source const rather than a
    copy in this gate.
    """
    m = re.search(rf"const {const_name}:[^=]*=\s*\[(.*?)\];", text, re.DOTALL)
    if not m:
        fail(f"{path.relative_to(ROOT)} missing the {const_name} array")
    return re.findall(r'"([^"]+)"', m.group(1))


def _rust_fn_body(text: str, fn_signature_re: str, path: Path) -> str:
    """The brace-balanced body of the first `fn` matching `fn_signature_re`.

    Scopes a literal scan to one function so an identically-spelled literal in a
    sibling function (e.g. a temp-path format string, a test fixture) is never
    swept in.
    """
    m = re.search(fn_signature_re, text)
    if not m:
        fail(f"{path.relative_to(ROOT)} missing fn matching {fn_signature_re!r}")
    depth = 0
    start = None
    for i in range(m.start(), len(text)):
        ch = text[i]
        if ch == "{":
            if depth == 0:
                start = i + 1
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return text[start:i]
    fail(f"{path.relative_to(ROOT)} fn matching {fn_signature_re!r} has no closed body")
    return ""  # unreachable; fail() raises


def mcp_tool_inventory() -> dict[str, list[str]]:
    """The connected `tools/list` surface, derived from the MCP server source.

    Returns the tool names grouped by origin:

    - ``registry``: one tool per non-stream registry endpoint (the historical
      surface), named after the endpoint.
    - ``offline``: the always-available utilities, read from
      ``OFFLINE_TOOL_NAMES`` in ``main.rs``.
    - ``flatfile``: the ``<endpoint>_to_path`` tools
      ``push_flatfile_tool_definitions`` in ``flatfile_tools.rs`` advertises,
      one per flat-file endpoint of the table.
    - ``utility``: the utility tools ``tool_definitions`` in ``main.rs``
      advertises, which include the offline ones; the offline set is
      subtracted so a tool is reported under one origin only.

    A connected `tools/list` returns the union; the docs gate asserts the docs
    enumerate it, so a tool added in code but absent from the docs fails here.
    Every source of tool names the connected list draws from is read here: a
    family parsed from none of these files would be undocumented with this
    gate green.
    """
    registry = [ep["name"] for ep in REGISTRY_ENDPOINTS]

    offline = _rust_str_array_items(
        MCP_MAIN_RS.read_text(), "OFFLINE_TOOL_NAMES", MCP_MAIN_RS
    )
    if not offline:
        fail(f"{MCP_MAIN_RS.relative_to(ROOT)} OFFLINE_TOOL_NAMES parsed to no tools")

    utility_body = _rust_fn_body(
        MCP_MAIN_RS.read_text(), r"fn tool_definitions\b", MCP_MAIN_RS
    )
    utility = [
        name
        for name in dict.fromkeys(re.findall(r'"name":\s*"([a-z_]+)"', utility_body))
        if name not in offline
    ]

    flatfile_text = MCP_FLATFILE_TOOLS_RS.read_text()
    flatfile_body = _rust_fn_body(
        flatfile_text,
        r"fn push_flatfile_tool_definitions\b",
        MCP_FLATFILE_TOOLS_RS,
    )
    # The tools are named in code, not by literals: read the suffix and the
    # endpoint subcategory they are built from, and apply them to the table.
    suffix = re.search(r'const TO_PATH_SUFFIX: &str = "(\w+)";', flatfile_text)
    subcategory = re.search(r'ep\.subcategory == "(\w+)"', flatfile_body)
    if not suffix or not subcategory:
        fail(
            f"{MCP_FLATFILE_TOOLS_RS.relative_to(ROOT)} push_flatfile_tool_definitions "
            f"no longer names its tools from TO_PATH_SUFFIX and an endpoint subcategory"
        )
    flatfile = sorted(
        ep["name"] + suffix.group(1)
        for ep in REGISTRY_ENDPOINTS
        if ep["subcategory"] == subcategory.group(1)
    )
    if not flatfile:
        fail(
            f"{MCP_FLATFILE_TOOLS_RS.relative_to(ROOT)} push_flatfile_tool_definitions "
            f"advertised no flat-file tools"
        )

    return {
        "registry": registry,
        "offline": offline,
        "flatfile": flatfile,
        "utility": utility,
    }


def _markdown_section(text: str, heading: str, path: Path) -> str:
    """The body under a `##` `heading` up to the next `##`-or-shallower heading.

    Scopes a tool-enumeration scan to the doc's tool-listing section so a tool
    name appearing in unrelated prose elsewhere in the file cannot satisfy the
    check for a tool dropped from the listing itself.
    """
    lines = text.splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.strip() == heading)
    except StopIteration:
        fail(f"{path.relative_to(ROOT)} missing the {heading!r} section this gate scans")
    level = len(heading) - len(heading.lstrip("#"))
    out: list[str] = []
    for line in lines[start + 1 :]:
        stripped = line.lstrip("#")
        depth = len(line) - len(stripped)
        if line.startswith("#") and stripped.startswith(" ") and depth <= level:
            break
        out.append(line)
    return "\n".join(out)


SECTION_COUNT_RE = re.compile(r"^### .*?\((\d+)")


def check_mcp_tool_counts(inventory: set[str]) -> None:
    """Each `### ... (N tools)` heading must count the tools listed under it.

    The counts are published in the package README, so each one is derived
    from the list beneath it rather than maintained by hand: a heading saying
    fourteen over thirteen tools is a number with nothing behind it. Only
    names in the server's own tool inventory count, so a backticked parameter
    or type in a bullet's prose is not mistaken for a tool.
    """
    readme, heading = MCP_DOC_TOOL_SECTIONS[0]
    section = _markdown_section(readme.read_text(), heading, readme)
    current: tuple[str, int] | None = None
    listed: set[str] = set()
    checked = 0

    def settle() -> int:
        if current is None:
            return 0
        title, claimed = current
        if claimed != len(listed):
            fail(
                f"{readme.relative_to(ROOT)} {title!r} claims {claimed} tools and lists "
                f"{len(listed)}. Correct the heading or the list."
            )
        return 1

    for line in section.splitlines():
        if line.startswith("### "):
            checked += settle()
            m = SECTION_COUNT_RE.match(line)
            current = (line.strip(), int(m.group(1))) if m else None
            listed = set()
        elif current is not None:
            listed |= {
                name for name in re.findall(r"`([a-z_]+)`", line) if name in inventory
            }
    checked += settle()
    if checked == 0:
        fail(
            f"{readme.relative_to(ROOT)} {heading!r} section has no counted "
            f"'### ... (N tools)' heading, so no count was checked."
        )


def check_mcp_tool_inventory() -> None:
    """The MCP docs must enumerate the connected `tools/list` surface.

    Every connection-only tool (the offline utilities, the flat-file tools, the
    live-feed tools and the generated utilities) must appear by name in each MCP
    doc's tool-listing section, so a tool advertised by the server but missing
    from the listing trips this check.
    The README also lists every registry market-data endpoint by name in its tool
    tables, so the full inventory is asserted against it; `mcp.md` defers the
    per-endpoint listing to the generated reference pages, so only the
    connection-only tools are pinned there.
    """
    inv = mcp_tool_inventory()
    connection_only = inv["offline"] + inv["flatfile"] + inv["utility"]

    for doc, heading in MCP_DOC_TOOL_SECTIONS:
        section = _markdown_section(doc.read_text(), heading, doc)
        missing = [name for name in connection_only if f"`{name}`" not in section]
        if missing:
            fail(
                f"{doc.relative_to(ROOT)} {heading!r} section does not enumerate MCP "
                f"tools that the connected tools/list advertises: {missing}. Document "
                f"them so the docs match the server's tool surface."
            )

    readme, readme_heading = MCP_DOC_TOOL_SECTIONS[0]
    readme_section = _markdown_section(readme.read_text(), readme_heading, readme)
    missing_registry = [name for name in inv["registry"] if f"`{name}`" not in readme_section]
    if missing_registry:
        fail(
            f"{readme.relative_to(ROOT)} {readme_heading!r} section does not list every "
            f"registry MCP tool by name: {missing_registry}. The README tool tables must "
            f"cover the full market-data surface the connected tools/list advertises."
        )

    check_mcp_tool_counts(set(inv["registry"]) | set(connection_only))


def _selftest() -> int:
    """Hermetic checks for the server flag-default derivation (G5).

    Drives `server_arg_defaults`, `_parse_doc_flag_table`, and
    `_value_enum_render` on synthetic inputs:

    * The `#[arg]` parser reads string (`default_value = "x"`), integer
      (`default_value_t = 25503`), and `ValueEnum` (`default_value_t =
      LogFormat::Text` -> `text`) defaults, treats a flag with no default
      as `None`, and — critically — is NOT truncated by a
      `value_parser = ["a", "b"]` allow-list whose `]` would otherwise
      close a naive attribute scan (the inner-delimiter bypass).
    * The Markdown table parser maps `--flag <metavar>` and paired
      `--a / --b` rows to their default cells, reads an em-dash / empty cell
      as `None`, and unwraps backticked defaults.
    * A documented default that disagrees with the source value is caught
      value-by-value, where a substring scan would miss a default that
      changed to a value still printed elsewhere.
    """
    import tempfile

    global SERVER_MAIN_RS
    failures: list[str] = []

    # `_value_enum_render`.
    if _value_enum_render("Text") != "text":
        failures.append("value-enum: `Text` did not render to `text`")
    if _value_enum_render("OpenInterest") != "open-interest":
        failures.append("value-enum: `OpenInterest` did not render to `open-interest`")

    synthetic_main = (
        "struct Args {\n"
        '    #[arg(long, default_value = "creds.txt")]\n'
        "    creds: String,\n"
        "    #[arg(long)]\n"
        "    api_key: Option<String>,\n"
        '    #[arg(long, default_value = "production", '
        'value_parser = ["production", "dev"])]\n'
        "    streaming_region: String,\n"
        "    #[arg(long, default_value_t = 25503)]\n"
        "    http_port: u16,\n"
        "    #[arg(long, value_enum, default_value_t = logging::LogFormat::Text)]\n"
        "    log_format: logging::LogFormat,\n"
        "    #[arg(long)]\n"
        "    no_streaming: bool,\n"
        "}\n"
    )
    saved_main = SERVER_MAIN_RS
    with tempfile.TemporaryDirectory() as td:
        fake_main = Path(td) / "main.rs"
        fake_main.write_text(synthetic_main, encoding="utf-8")
        SERVER_MAIN_RS = fake_main
        try:
            derived = server_arg_defaults()
        finally:
            SERVER_MAIN_RS = saved_main

    expected = {
        "--creds": "creds.txt",
        "--api-key": None,
        "--streaming-region": "production",  # not truncated by value_parser `]`
        "--http-port": "25503",
        "--log-format": "text",
        "--no-streaming": None,
    }
    for flag, want in expected.items():
        got = derived.get(flag, "<<missing>>")
        if got != want:
            failures.append(
                f"arg-parse: {flag} derived as {got!r}, expected {want!r}"
            )

    # Markdown table parser: metavar, paired flags, em-dash, backticks.
    table = (
        "| Flag | Default | Description |\n"
        "|------|---------|-------------|\n"
        "| `--creds <path>` | `creds.txt` | creds file |\n"
        "| `--email` / `--password` | — | inline creds |\n"
        "| `--http-port <port>` | `25503` | port |\n"
        "| `--log-format <fmt>` | `text` | format |\n"
    )
    parsed = _parse_doc_flag_table(table)
    table_expected = {
        "--creds": "creds.txt",
        "--email": None,
        "--password": None,
        "--http-port": "25503",
        "--log-format": "text",
    }
    for flag, want in table_expected.items():
        got = parsed.get(flag, "<<missing>>")
        if got != want:
            failures.append(
                f"table-parse: {flag} parsed as {got!r}, expected {want!r}"
            )

    # Value-by-value mismatch detection. The comparison has to be exercised
    # through the gate itself: comparing two dicts written here would assert
    # that "25503" differs from "25504" and would still pass if
    # `check_server_flag_defaults` stopped comparing values altogether.
    # So move the port in the source and require the gate to refuse it
    # against the real docs, which still document the old one.
    moved_main = synthetic_main.replace("25503", "25504")
    if moved_main == synthetic_main:
        failures.append(
            "value-by-value: the fixture no longer carries the port default "
            "this case moves, so the case proves nothing"
        )
    with tempfile.TemporaryDirectory() as td:
        fake_main = Path(td) / "main.rs"
        fake_main.write_text(moved_main, encoding="utf-8")
        SERVER_MAIN_RS = fake_main
        try:
            check_server_flag_defaults()
        except SystemExit:
            pass
        else:
            failures.append(
                "value-by-value: a source default that moved away from the "
                "documented one was not refused by the gate"
            )
        finally:
            SERVER_MAIN_RS = saved_main

    if failures:
        print("check_docs_consistency --selftest: FAILED")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("check_docs_consistency --selftest: ok")
    return 0


def main() -> None:
    check_static_docs()
    check_server_flag_defaults()
    check_openapi()
    check_mcp_tool_inventory()
    print("docs consistency: ok")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="Run the embedded self-test and exit.",
    )
    args = parser.parse_args()
    if args.selftest:
        sys.exit(_selftest())
    main()
