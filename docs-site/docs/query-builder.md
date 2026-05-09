---
layout: page
title: Query Builder
---

<script setup>
import QueryBuilder from './.vitepress/theme/components/QueryBuilder.vue'
</script>

<div class="vp-doc" style="max-width:860px;margin:0 auto;padding:48px 24px 0">

# Query Builder

Build a working SDK snippet in seconds. Choose your data type, asset, parameters, and language — no docs diving required.

<QueryBuilder />

## FLATFILES requests

The interactive builder above generates per-contract MDDS (request/response) snippets. Whole-universe **FLATFILES** requests use a different parameter set — `(sec_type, req_type, date)` instead of `(symbol, start, end)` — and ship as a separate surface in the SDK. Below is the same snippet shape for FLATFILES so you can construct calls in the same builder pattern.

### Parameters

| Parameter | Values | Required |
|---|---|---|
| `sec_type` | `OPTION`, `STOCK`, `INDEX` | yes |
| `req_type` | `EOD`, `QUOTE`, `OPEN_INTEREST`, `OHLC`, `TRADE`, `TRADE_QUOTE` | yes |
| `date` | `YYYYMMDD` string | yes |
| `format` | `csv`, `jsonl` | optional (defaults to `csv` for the on-disk path; the in-memory path returns typed rows) |
| `output_path` | filesystem path | optional (the on-disk paths only) |

### Snippet shapes

```python
# Python — typed terminal:
rows = tdx.flat_files.option_quote(date="20260428")
rows.to_polars()        # or .to_pandas() / .to_arrow() / .to_list()

# Python — bytes on disk:
tdx.flatfile_to_path("OPTION", "QUOTE", "20260428", "out.csv", "csv")
```

```ts
// TypeScript:
const rows = await tdx.flatFiles.optionQuote('20260428');
rows.toArrowIpc();
```

```bash
# CLI:
tdx flatfile quotes 20260428 --format csv -o spy_quotes.csv

# REST:
curl 'http://127.0.0.1:25503/v3/flatfile/option/quote?date=20260428&format=csv'
```

See the dedicated [Flat Files](./flatfiles/) section for the full method signatures, format specifications, and bandwidth caveats.

</div>
