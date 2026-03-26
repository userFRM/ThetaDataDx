# Tools

Standalone applications and utilities built on the `thetadatadx` SDK.

## live-chain

A real-time option chain dashboard built with [Streamlit](https://streamlit.io/). Connects directly to ThetaData production servers via the native Rust SDK (no JVM required) and displays a live, color-coded option chain with Greeks computed by the built-in Black-Scholes calculator.

**Features:**

- Sidebar authentication (inline credentials or `creds.txt` file)
- Tabbed expirations (nearest 12, labeled with date and DTE)
- Full option chain: calls on the left, strike in the center, puts on the right
- Greeks (IV, Delta, Gamma, Theta) computed locally via the Rust `all_greeks` function
- ITM cells tinted green, ATM strike highlighted gold
- Bid, Ask, Last, Volume, Open Interest columns
- Configurable number of strikes around ATM (5--50)
- Auto-refresh with configurable interval (2--30 seconds)
- Works with any optionable ticker (SPY, AAPL, TSLA, QQQ, etc.)

**Running:**

```bash
cd tools/live-chain
pip install -r requirements.txt
streamlit run app.py
```

Then open `http://localhost:8501` in your browser.

**Requirements:** Python 3.9+, a ThetaData account with an active market data subscription. See [tools/live-chain/README.md](live-chain/README.md) for full details.
