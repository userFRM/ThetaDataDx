"""Python-side doctest gate.

Scans the Rust source under `thetadatadx-py/src/` for triple-slash doc
comments containing `>>>` blocks, extracts them, and runs each block
against the compiled `thetadatadx` module via `doctest.DocTestRunner`.

Why scan the Rust source and not the runtime docstrings? pyo3 strips
indentation in subtle ways and `inspect.getdoc()` on a #[pyfunction]
sometimes loses the `>>>` prefix. The Rust source is the source of
truth for what we documented; if the example there doesn't run, that
is the bug.

This gate exists because a docstring example can reference a symbol the
module never registered — e.g. `Contract.stock("AAPL")` documented while
the pyclass was unregistered. Running every documented example keeps the
docs and the compiled surface in lockstep, so that class of regression
fails here instead of in a user's session.
"""

from __future__ import annotations

import doctest
import importlib
import pathlib
import re
import textwrap

import pytest


SRC_DIR = pathlib.Path(__file__).resolve().parents[1] / "src"

DOC_BLOCK_RE = re.compile(
    r"(?:^[ \t]*///[^\n]*\n)+",
    re.MULTILINE,
)
LINE_RE = re.compile(r"^[ \t]*///[ \t]?(.*)$", re.MULTILINE)


def _extract_doc_blocks(text: str) -> list[str]:
    out: list[str] = []
    for match in DOC_BLOCK_RE.finditer(text):
        block = match.group(0)
        body_lines = LINE_RE.findall(block)
        body = "\n".join(body_lines)
        if ">>>" in body:
            out.append(textwrap.dedent(body))
    return out


def _collect_doctest_examples() -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for rs in SRC_DIR.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for body in _extract_doc_blocks(text):
            out.append((rs.relative_to(SRC_DIR).as_posix(), body))
    return out


def _doc_blocks() -> list[tuple[str, str]]:
    return _collect_doctest_examples()


@pytest.fixture(scope="module")
def thetadatadx_mod():
    return importlib.import_module("thetadatadx")


@pytest.mark.parametrize(
    ("source_path", "body"),
    _doc_blocks() or [("<no-examples>", ">>> 1 + 1\n2")],
    ids=lambda x: x if isinstance(x, str) and "/" not in x and len(x) < 60 else "block",
)
def test_doctest_block(thetadatadx_mod, source_path: str, body: str) -> None:
    """Run every `>>>` block extracted from Rust doc comments.

    The compiled `thetadatadx` module is injected into the doctest
    globals so `>>> import thetadatadx` and bare references to
    `thetadatadx.<X>` both resolve without the test having to
    re-import in every block.
    """
    parser = doctest.DocTestParser()
    runner = doctest.DocTestRunner(
        verbose=False,
        optionflags=doctest.NORMALIZE_WHITESPACE | doctest.ELLIPSIS,
    )
    examples = parser.get_examples(body)
    if not examples:
        pytest.skip(f"no >>> examples in {source_path}")
    test = doctest.DocTest(
        examples=examples,
        globs={"thetadatadx": thetadatadx_mod},
        name=source_path,
        filename=source_path,
        lineno=0,
        docstring=body,
    )
    result = runner.run(test)
    assert result.failed == 0, (
        f"{result.failed}/{result.attempted} doctest(s) failed in "
        f"{source_path}; see stdout above for diffs."
    )


def test_scanner_finds_known_block() -> None:
    """Sanity check the scanner - `split_date_range` documents a
    `>>>` block on every commit of `lib.rs`, so the parser must find
    at least one entry."""
    blocks = _collect_doctest_examples()
    assert blocks, (
        "the doctest scanner found zero `>>>` blocks under "
        "thetadatadx-py/src/. Either the regex regressed or every "
        "documented example was deleted - both deserve investigation."
    )
