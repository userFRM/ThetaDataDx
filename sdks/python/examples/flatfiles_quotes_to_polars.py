"""FLATFILES dynamic-schema decode -> polars.DataFrame.

The flat-file surface returns whole-universe data for a single
`(SecType, ReqType, date)` tuple. The decoded shape is determined at
runtime by the request type, so the binding chains through Arrow:
``rows.to_polars()`` -> ``rows.to_arrow()`` -> pyarrow.Table -> polars.

Run with a credentials file: ``python flatfiles_quotes_to_polars.py``.
"""
from thetadatadx import Config, Credentials, Client

creds = Credentials.from_file("creds.txt")
client = Client(creds, Config.production())

# Whole-universe option quotes for one trading day. Several hundred MB
# decoded -- expect this call to take a few seconds even on a fast link.
rows = client.flat_files.option_quote(date="20260428")
print(f"option_quote rows: {len(rows)}")

# polars.DataFrame -- one column per vendor field, plus the contract key
# columns (symbol, expiration, strike, right). Schema inferred from the
# first row by `flatfiles::arrow::rows_to_arrow`.
df = rows.to_polars()
print(df.head())

# Same path, dispatched dynamically -- same `FlatFileRowList` return.
oi = client.flat_files.request("OPTION", "OPEN_INTEREST", "20260428")
print(f"open_interest rows: {len(oi)}")

# Drop the raw vendor CSV bytes to disk without materialising the
# decoded `FlatFileRowList`. Returns the final on-disk path with the
# format extension auto-appended if missing.
path = client.flatfile_to_path(
    "OPTION", "QUOTE", "20260428", "/tmp/option-quote", format="csv"
)
print(f"raw vendor CSV at {path}")
