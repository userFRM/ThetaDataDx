"""Fetch historical stock and option data from ThetaData via Rust SDK."""
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
client = ThetaDataDx(creds, Config.production())

# End-of-day stock data
print("=== AAPL EOD (Jan-Mar 2024) ===")
eod = client.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod[:5]:
    print(f"  {tick['date']}: O={tick['open']:.2f} H={tick['high']:.2f} "
          f"L={tick['low']:.2f} C={tick['close']:.2f} V={tick['volume']}")
print(f"  ... {len(eod)} total days\n")

# Intraday 1-minute bars
print("=== AAPL 1-min OHLC (Mar 15, 2024) ===")
bars = client.stock_history_ohlc("AAPL", "20240315", "60000")
for bar in bars[:5]:
    print(f"  {bar['ms_of_day']}ms: O={bar['open']:.2f} H={bar['high']:.2f} "
          f"L={bar['low']:.2f} C={bar['close']:.2f}")
print(f"  ... {len(bars)} total bars\n")

# Option expirations
print("=== SPY Option Expirations ===")
exps = client.option_list_expirations("SPY")
print(f"  Next 5: {exps[:5]}\n")

# Option strikes
if exps:
    strikes = client.option_list_strikes("SPY", exps[0])
    print(f"=== SPY {exps[0]} Strikes ===")
    print(f"  {len(strikes)} strikes, range: {strikes[0]} - {strikes[-1]}")
