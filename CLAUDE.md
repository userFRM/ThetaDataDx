# Working in this repository

This is the standing brief for anyone maintaining ThetaDataDx, human or agent. It is not a style
guide. It is the set of judgements that have already been made here, written down so they are not
relitigated, and so a change made in good faith does not quietly undo one of them.

Read it before the first edit.

## What this is

A market-data SDK: one Rust core with Python, TypeScript, C++ and C surfaces generated from it, a
local HTTP server, and an MCP server that holds a live subscription and answers questions about it.

The vendor ships a terminal. **The terminal is the baseline.** Whatever it serves, this SDK serves,
byte for byte. Whatever it does not serve, this SDK does not invent. Match the data exactly and beat
it on engineering. That is the whole of the positioning, and the reason a user can swap one for the
other without checking.

## The bar

**Build the complete thing.** Do not offer a minimal version and a proper version and ask which.
Build the one that holds. Scope can be cut; the primitive underneath it cannot.

**The smallest diff that is actually right.** Deleting beats adding. An abstraction with one
implementation, a factory for one product, a config key for a value that never changes: none of
these are neutral. They are code someone decodes at three in the morning.

**Fix the cause, not the symptom.** A report names a symptom. Before editing, find every caller of
the function you are about to touch. One guard in the shared path is a smaller diff than a guard in
every caller, and patching only the path the report names leaves every sibling broken.

**No dead code.** Not commented out, not behind a flag nobody sets, not kept because it might be
useful. Version control remembers.

**Names say what a thing is.** No `Manager`, `Helper`, `Handler`, `Oracle`, `Util`. A name that
needs the surrounding code to make sense is the wrong name. Renaming is a separate decision from
whatever you came to do: propose it, do not fold it in.

**Nothing derived that the vendor did not send.** No greeks, no implied volatility, no aggressor
side, no bars built here. Where a value is computed locally it is named as such in the surface that
exposes it. The vendor's own bar is served as it arrived and never reconstructed from trades.

## Reading code

The most productive question to ask of any line is whether it can fail.

A condition that is always true. A test that would still pass with the logic under it deleted. A
guard on a state the code cannot reach. An assertion comparing a value to itself. A check that
cannot fail is not a check, and it is worse than none, because it is believed.

This applies to tests as hard as to production code.

A guard must name the thing it guards rather than depend on running first. If a change is mostly
repairing the guards from the previous change, stop patching and do the design.

## Verification

**Prove a non-trivial fix by breaking it.** Change the production path so the defect returns, watch
the named test fail, restore the file and confirm it is byte-identical. A fix without that is a fix
you hope works.

Three ways this proves nothing, all worth checking for:

- It did not compile, usually on a now-unused binding under `-D warnings`. Nothing ran. Keep the
  binding live.
- It did not apply, because the formatter moved the line it anchored on. Assert the anchor exists
  before replacing it.
- It was equivalent and changed nothing observable. Then the test is not discriminating, and the
  test is what needs work.

Restore from a copy taken beforehand. Never use version control to undo it: that reverts everything
else in the file too.

**Run the full gate locally before pushing**, including the crates the default invocation excludes.
Some gates need artefacts built first and report a false failure otherwise; the gate says so when it
does.

**Never push in the same command as the gate.** The push runs whether the gate passed or not.

**Check CI after every push.** Not the next morning.

**Empty output is a failure, not a pass.** If a check prints nothing, find out whether it ran.

## The public record

Commits, pull requests, issues, review comments, the changelog and the release notes are all read by
users. They describe the software, not the work.

State the defect and the reasoning. Never the process: no round numbers, no finding counts, no
review or audit vocabulary, no before-and-after narration of how the change was arrived at, no
mention of the tools used to produce it. A reader in two years wants to know what was wrong and why
the fix is correct. Everything else is noise in the permanent record.

Conventional Commits. The subject says what changed; the body says why it was wrong before.

Changelog entries describe what a user can now do, or no longer has to work around, in prose, with
the reason it was wrong before. They are not commit summaries. The repository changelog and the
documentation site copy stay identical.

Issues are Problem, Solution, Impact. Check the open ones first, including your own.

Prose does not hard-wrap. Write paragraphs and let the renderer wrap them.

Also never published, in any of the above:

- **Fabricated numbers.** Every published figure has a committed builder anyone can re-run. A number
  that cannot be reproduced on demand does not go in a changelog, a README or a PR body.
- **Claims a client-side measurement cannot support.** A number measured on one machine against one
  account is a number measured on one machine against one account.
- **Competitor names**, self-grading, or readiness claims.
- **Credentials, account identifiers, hostnames or addresses**, in code, docs, issues or fixtures.
- **Internal module paths and runtime details**, in customer-facing documentation. Public API names
  are fine. Contributor documents like this one are the exception.

Write in a plain voice. No em-dashes, no marketing cadence.

## Git

No force pushes. To undo your own unpushed mistake, reset; to undo something already shared, revert.

## Streaming

The live surfaces carry the subtlest constraints in the repository. These are facts, not opinions.

**Tick shapes differ by transport.** The streaming trade tick and the request-response trade tick are
not the same shape and never have been. Conflating them produces code that looks right and decodes
garbage. Check the schema, not your memory.

**Not every asset class has every stream.** An index has no quote stream and no whole-market stream;
its price arrives on the trade subscription. Open interest counts option contracts outstanding and a
stock does not have one. Offering a subscription the vendor does not publish opens a book that stays
silent for ever.

**The whole-market trade stream wraps each print.** The quote that stood before it, the vendor's bar,
and the quotes after it arrive as separate messages around the trade. Correlating them is the
consumer's job, and the correlation must not span an interval nobody observed.

**Rates are far higher than a mean suggests.** A per-second average hides the burst that overflows
the buffer. At a market open the whole-market stock stream has been measured at roughly forty times
the mean of the minute containing it. Anything sized in rows states what it costs in memory and never
promises a length of time.

**The development cluster replays faster than real time.** Never size anything against numbers taken
from it.

**The idle wait is a deliberate choice.** The streaming consumer defaults to a spin wait, which holds
a full core for as long as the stream is connected. That is correct for a colocated consumer chasing
microseconds and wrong for anything running beside an editor, so tools that answer a question every
few seconds set a backing-off wait instead. Do not change that back without measuring the cost.

**Nothing in the ingest path allocates or takes a second lock.** It runs per message, at rates where
that matters.

## Telling a caller what you do not know

The live surfaces exist to answer questions that cannot be answered by reading a feed directly. That
only works if every answer is honest about its own limits.

- An age describes the rows returned, not the newest thing on the feed.
- A window's stated bounds contain the rows it came back with.
- A count says which population it covers.
- A completeness claim is wrong in both directions. Reporting missing data that is present costs
  trust; reporting complete data that is not costs more.
- A timestamp the clock could not produce has no age at all, rather than one measured from the epoch.
- A request the server cannot honour is refused by name. It is never silently answered as a
  different, wider request: dropping a filter that failed to parse returns everything and says
  nothing.
- A read settles only once its answer is going to reach the caller. A call that fails leaves its
  rows, cursors and disclosures for the next one.

## Generated code

Several surfaces are generated from schema files. The schema is the source of truth; the generated
files are committed so a consumer does not need the generator. Edit the schema, run the generator,
commit both, and let the parity gate confirm every binding agrees.

Never hand-edit a file whose header says it is generated.

## Acting on a review

Findings are not a to-do list. Reproduce each one in the code before acting on it, and reject the
ones that do not hold, with the reason. A reviewer reasoning by analogy from a real defect will
produce a plausible one that is not there, and a fix for a defect that does not exist is a new defect
with a test that locks it in.

Reject in particular anything that trades a real cost for a hypothetical one. A guard that discards
live data to close a race nobody can reach is not an improvement.

When two independent reviewers land on the same point, look again at it even if you dismissed it the
first time.

## Not done without asking

- Renaming anything on a public surface.
- Anything destructive: force pushes, history rewrites, removing files or directories outside the
  change that was asked for.
- Pointing undocumented calls at the vendor's production systems, including read-only probes.
- Publishing, releasing or tagging.

When a request is ambiguous in a way that changes the work, ask once and then commit. When it is not,
do the whole thing rather than half of it and a question.
