"""ThetaDataDx Live Option Chain --- Streamlit Dashboard.

A real-time option chain viewer powered by thetadatadx's native Rust SDK.
Connects directly to ThetaData servers (no JVM) and displays a live,
color-coded option chain with Greeks computed via the built-in
Black-Scholes calculator.

Run:
    pip install -r requirements.txt
    streamlit run app.py
"""

import math
import time

import numpy as np
import pandas as pd
import streamlit as st

from thetadatadx import (
    Config,
    Credentials,
    ThetaDataDx,
    all_greeks,
    implied_volatility,
)

# ---------------------------------------------------------------------------
# Page config
# ---------------------------------------------------------------------------

st.set_page_config(
    page_title="ThetaDataDx Live Chain",
    page_icon="\u0398",  # Theta
    layout="wide",
)

# ---------------------------------------------------------------------------
# Custom CSS for the option chain look
# ---------------------------------------------------------------------------

st.markdown(
    """
<style>
    /* Tighten up table row height */
    .stDataFrame td, .stDataFrame th {
        padding: 2px 6px !important;
        font-size: 13px !important;
    }
    /* Sidebar styling */
    section[data-testid="stSidebar"] {
        background-color: #0e1117;
    }
    /* Status badges */
    .status-connected { color: #00c853; font-weight: bold; }
    .status-disconnected { color: #ff5252; font-weight: bold; }
    /* ATM strike highlight */
    .atm-row { background-color: rgba(255, 215, 0, 0.15) !important; }
</style>
""",
    unsafe_allow_html=True,
)

# ---------------------------------------------------------------------------
# Session state defaults
# ---------------------------------------------------------------------------

for key, default in [
    ("client", None),
    ("fpss", None),
    ("ticker", "SPY"),
    ("connected", False),
    ("spot_price", None),
    ("last_refresh", None),
    ("auto_refresh", False),
    ("refresh_interval", 5),
    ("num_strikes", 20),
]:
    if key not in st.session_state:
        st.session_state[key] = default

# ---------------------------------------------------------------------------
# Sidebar --- Credentials & Controls
# ---------------------------------------------------------------------------

with st.sidebar:
    st.header("Connection")

    auth_method = st.radio(
        "Authentication",
        ["Credentials", "Credentials file"],
        horizontal=True,
    )

    if auth_method == "Credentials":
        email = st.text_input("Email", key="email_input")
        password = st.text_input("Password", type="password", key="password_input")
        creds_ready = bool(email and password)
    else:
        creds_file = st.text_input(
            "Credentials file path",
            value="creds.txt",
            help="Line 1 = email, Line 2 = password",
        )
        creds_ready = bool(creds_file)

    ticker = st.text_input("Ticker", value=st.session_state.ticker).upper().strip()

    connect_btn = st.button(
        "Connect" if not st.session_state.connected else "Reconnect",
        type="primary",
        use_container_width=True,
    )

    if st.session_state.connected:
        st.markdown('<span class="status-connected">Connected</span>', unsafe_allow_html=True)
    else:
        st.markdown(
            '<span class="status-disconnected">Not connected</span>',
            unsafe_allow_html=True,
        )

    st.divider()
    st.header("Display")
    st.session_state.num_strikes = st.slider(
        "Strikes around ATM", 5, 50, st.session_state.num_strikes, step=5,
    )
    st.session_state.refresh_interval = st.slider(
        "Refresh interval (sec)", 2, 30, st.session_state.refresh_interval,
    )
    st.session_state.auto_refresh = st.checkbox(
        "Auto-refresh", value=st.session_state.auto_refresh,
    )

    st.divider()
    st.header("Greeks Parameters")
    risk_free = st.number_input("Risk-free rate", value=0.05, format="%.4f", step=0.005)
    div_yield = st.number_input("Dividend yield", value=0.013, format="%.4f", step=0.001)

# ---------------------------------------------------------------------------
# Connect to ThetaData
# ---------------------------------------------------------------------------

if connect_btn and creds_ready:
    with st.spinner("Authenticating with ThetaData..."):
        try:
            if auth_method == "Credentials":
                creds = Credentials(email, password)
            else:
                creds = Credentials.from_file(creds_file)

            config = Config.production()
            client = ThetaDataDx(creds, config)
            st.session_state.client = client
            st.session_state.ticker = ticker
            st.session_state.connected = True
            st.toast(f"Connected! Fetching {ticker} chain...", icon="\u2705")
        except Exception as exc:
            st.error(f"Connection failed: {exc}")
            st.session_state.connected = False
elif connect_btn and not creds_ready:
    st.sidebar.error("Please provide credentials.")

# ---------------------------------------------------------------------------
# Title
# ---------------------------------------------------------------------------

st.title(f"Live Option Chain --- {st.session_state.ticker}")

if not st.session_state.connected:
    st.info("Enter your ThetaData credentials in the sidebar and click Connect.")
    st.stop()

client: ThetaDataDx = st.session_state.client
ticker = st.session_state.ticker

# ---------------------------------------------------------------------------
# Fetch spot price
# ---------------------------------------------------------------------------


@st.cache_data(ttl=5)
def fetch_spot(tkr: str) -> float:
    """Get the latest spot price for the underlying."""
    snap = client.stock_snapshot_ohlc([tkr])
    if snap:
        return snap[0]["close"]
    return 0.0


try:
    spot = fetch_spot(ticker)
    st.session_state.spot_price = spot
except Exception as exc:
    st.error(f"Failed to fetch spot price: {exc}")
    st.stop()

# ---------------------------------------------------------------------------
# Fetch expirations
# ---------------------------------------------------------------------------


@st.cache_data(ttl=60)
def fetch_expirations(tkr: str) -> list[str]:
    return client.option_list_expirations(tkr)


try:
    expirations = fetch_expirations(ticker)
except Exception as exc:
    st.error(f"Failed to fetch expirations: {exc}")
    st.stop()

if not expirations:
    st.warning(f"No option expirations found for {ticker}.")
    st.stop()

# ---------------------------------------------------------------------------
# Expiration picker as tabs
# ---------------------------------------------------------------------------

# Show the nearest 12 expirations as tabs
from datetime import datetime

max_tabs = 12
tab_exps = expirations[:max_tabs]

# Format labels: "Apr 18 (23d)" style
tab_labels = []
today = datetime.now()
for exp_str in tab_exps:
    exp_dt = datetime.strptime(exp_str, "%Y%m%d")
    dte = (exp_dt - today).days
    label = f"{exp_dt.strftime('%b %d')} ({dte}d)"
    tab_labels.append(label)

tabs = st.tabs(tab_labels)

# ---------------------------------------------------------------------------
# Helper: build the option chain for one expiration
# ---------------------------------------------------------------------------


def build_chain(tkr: str, exp_str: str, spot_price: float, n_strikes: int) -> pd.DataFrame:
    """Fetch quotes + compute Greeks for an expiration, returning the chain DataFrame."""

    exp_dt = datetime.strptime(exp_str, "%Y%m%d")
    dte = (exp_dt - today).days
    tte = max(dte / 365.0, 1 / 365.0)  # floor at 1 day to avoid div-by-zero

    # 1. Get all strikes
    all_strikes = client.option_list_strikes(tkr, exp_str)
    if not all_strikes:
        return pd.DataFrame()

    # Convert strike encoding: ThetaData stores strikes * 1000 as strings
    strike_pairs = [(s, float(s) / 1000) for s in all_strikes]

    # 2. Find ATM and filter to N strikes around it
    atm_idx = min(range(len(strike_pairs)), key=lambda i: abs(strike_pairs[i][1] - spot_price))
    half = n_strikes // 2
    start = max(0, atm_idx - half)
    end = min(len(strike_pairs), atm_idx + half + 1)
    selected = strike_pairs[start:end]

    # 3. Fetch call and put quotes for each strike
    rows = []
    for strike_str, strike_val in selected:
        row = {"strike": strike_val, "strike_raw": strike_str}

        for right, prefix in [("C", "call_"), ("P", "put_")]:
            try:
                quote = client.option_snapshot_quote(tkr, exp_str, strike_str, right)
            except Exception:
                quote = None

            if quote:
                bid = quote[0]["bid"]
                ask = quote[0]["ask"]
                mid = (bid + ask) / 2
                spread = ask - bid
                row[f"{prefix}bid"] = bid
                row[f"{prefix}ask"] = ask
                row[f"{prefix}mid"] = mid
                row[f"{prefix}spread"] = spread
            else:
                row[f"{prefix}bid"] = np.nan
                row[f"{prefix}ask"] = np.nan
                row[f"{prefix}mid"] = np.nan
                row[f"{prefix}spread"] = np.nan

            # Fetch last trade
            try:
                trade = client.option_snapshot_trade(tkr, exp_str, strike_str, right)
                if trade:
                    row[f"{prefix}last"] = trade[0]["price"]
                    row[f"{prefix}volume"] = trade[0].get("size", 0)
                else:
                    row[f"{prefix}last"] = np.nan
                    row[f"{prefix}volume"] = 0
            except Exception:
                row[f"{prefix}last"] = np.nan
                row[f"{prefix}volume"] = 0

            # Fetch open interest
            try:
                oi_data = client.option_snapshot_open_interest(tkr, exp_str, strike_str, right)
                if oi_data:
                    oi_val = oi_data[0].get("open_interest", 0)
                    if isinstance(oi_val, dict):
                        oi_val = oi_val.get("value", 0)
                    row[f"{prefix}oi"] = int(oi_val) if oi_val else 0
                else:
                    row[f"{prefix}oi"] = 0
            except Exception:
                row[f"{prefix}oi"] = 0

            # Compute Greeks from mid price
            mid_price = row.get(f"{prefix}mid", np.nan)
            if pd.notna(mid_price) and mid_price > 0.01:
                try:
                    g = all_greeks(
                        spot=spot_price,
                        strike=strike_val,
                        rate=risk_free,
                        div_yield=div_yield,
                        tte=tte,
                        option_price=mid_price,
                        right=right,
                    )
                    if g["iv_error"] < 0.05:
                        row[f"{prefix}iv"] = g["iv"]
                        row[f"{prefix}delta"] = g["delta"]
                        row[f"{prefix}gamma"] = g["gamma"]
                        row[f"{prefix}theta"] = g["theta"]
                    else:
                        row[f"{prefix}iv"] = np.nan
                        row[f"{prefix}delta"] = np.nan
                        row[f"{prefix}gamma"] = np.nan
                        row[f"{prefix}theta"] = np.nan
                except Exception:
                    row[f"{prefix}iv"] = np.nan
                    row[f"{prefix}delta"] = np.nan
                    row[f"{prefix}gamma"] = np.nan
                    row[f"{prefix}theta"] = np.nan
            else:
                row[f"{prefix}iv"] = np.nan
                row[f"{prefix}delta"] = np.nan
                row[f"{prefix}gamma"] = np.nan
                row[f"{prefix}theta"] = np.nan

        rows.append(row)

    return pd.DataFrame(rows)


# ---------------------------------------------------------------------------
# Helper: style the chain dataframe
# ---------------------------------------------------------------------------


def style_chain(df: pd.DataFrame, spot_price: float) -> pd.io.formats.style.Styler:
    """Apply conditional formatting to the option chain."""

    # Ordered columns: Calls (left) | Strike (center) | Puts (right)
    call_cols = [
        "call_iv", "call_delta", "call_gamma", "call_theta",
        "call_bid", "call_ask", "call_last", "call_volume", "call_oi",
    ]
    put_cols = [
        "put_bid", "put_ask", "put_last", "put_volume", "put_oi",
        "put_iv", "put_delta", "put_gamma", "put_theta",
    ]
    display_cols = [c for c in call_cols if c in df.columns]
    display_cols.append("strike")
    display_cols.extend([c for c in put_cols if c in df.columns])

    styled_df = df[display_cols].copy()

    # Rename columns for display
    rename_map = {}
    for c in styled_df.columns:
        parts = c.split("_", 1)
        if len(parts) == 2 and parts[0] in ("call", "put"):
            rename_map[c] = parts[1].upper()
        else:
            rename_map[c] = c.upper()
    styled_df.columns = [rename_map.get(c, c) for c in styled_df.columns]

    def color_itm(row):
        """Highlight ITM cells: calls where strike < spot, puts where strike > spot."""
        n = len(row)
        styles = [""] * n
        strike_idx = list(row.index).index("STRIKE") if "STRIKE" in row.index else -1
        strike = row["STRIKE"] if "STRIKE" in row.index else None

        if strike is None:
            return styles

        # ITM call: strike < spot -> green tint on call columns (left of strike)
        if strike < spot_price:
            for i in range(strike_idx):
                styles[i] = "background-color: rgba(76, 175, 80, 0.12)"

        # ITM put: strike > spot -> green tint on put columns (right of strike)
        if strike > spot_price:
            for i in range(strike_idx + 1, n):
                styles[i] = "background-color: rgba(76, 175, 80, 0.12)"

        # ATM row: golden highlight on the strike cell
        if abs(strike - spot_price) == min(abs(df["strike"] - spot_price)):
            styles[strike_idx] = "background-color: rgba(255, 215, 0, 0.3); font-weight: bold"

        return styles

    def color_spread(val, col_name):
        """Color narrow spreads green, wide spreads red."""
        if pd.isna(val) or not isinstance(val, (int, float)):
            return ""
        if "BID" in col_name or "ASK" in col_name:
            return ""
        return ""

    def format_iv(val):
        if pd.isna(val):
            return ""
        return f"{val * 100:.1f}%"

    def format_greek(val):
        if pd.isna(val):
            return ""
        return f"{val:.4f}"

    def format_price(val):
        if pd.isna(val):
            return ""
        return f"{val:.2f}"

    def format_int(val):
        if pd.isna(val) or val == 0:
            return ""
        return f"{int(val):,}"

    format_dict = {}
    for c in styled_df.columns:
        if c in ("IV",):
            format_dict[c] = format_iv
        elif c in ("DELTA", "GAMMA", "THETA"):
            format_dict[c] = format_greek
        elif c in ("VOLUME", "OI"):
            format_dict[c] = format_int
        elif c == "STRIKE":
            format_dict[c] = lambda x: f"${x:.1f}" if pd.notna(x) else ""
        else:
            format_dict[c] = format_price

    styler = styled_df.style.apply(color_itm, axis=1).format(format_dict)

    return styler


# ---------------------------------------------------------------------------
# Render each expiration tab
# ---------------------------------------------------------------------------

for tab, exp_str in zip(tabs, tab_exps):
    with tab:
        exp_dt = datetime.strptime(exp_str, "%Y%m%d")
        dte = (exp_dt - today).days

        col1, col2, col3 = st.columns([2, 2, 2])
        with col1:
            st.metric("Spot", f"${spot:.2f}")
        with col2:
            st.metric("Expiration", exp_dt.strftime("%Y-%m-%d"))
        with col3:
            st.metric("DTE", f"{dte}")

        with st.spinner(f"Loading {ticker} {exp_str} chain..."):
            try:
                chain = build_chain(
                    ticker, exp_str, spot,
                    st.session_state.num_strikes,
                )
            except Exception as exc:
                st.error(f"Error building chain: {exc}")
                continue

        if chain.empty:
            st.warning("No data returned for this expiration.")
            continue

        # Render with two-panel header
        st.markdown(
            '<div style="display:flex; justify-content:space-between;">'
            '<span style="color:#4FC3F7; font-weight:bold;">CALLS</span>'
            '<span style="color:#FF8A65; font-weight:bold;">PUTS</span>'
            "</div>",
            unsafe_allow_html=True,
        )

        styled = style_chain(chain, spot)
        st.dataframe(
            styled,
            use_container_width=True,
            hide_index=True,
            height=min(35 * len(chain) + 38, 800),
        )

        st.session_state.last_refresh = time.strftime("%H:%M:%S")

# ---------------------------------------------------------------------------
# Footer with refresh info
# ---------------------------------------------------------------------------

st.divider()
footer_cols = st.columns([3, 1])
with footer_cols[0]:
    if st.session_state.last_refresh:
        st.caption(f"Last refresh: {st.session_state.last_refresh}")
with footer_cols[1]:
    if st.button("Refresh Now", use_container_width=True):
        # Clear caches and rerun
        fetch_spot.clear()
        fetch_expirations.clear()
        st.rerun()

# ---------------------------------------------------------------------------
# Auto-refresh via rerun
# ---------------------------------------------------------------------------

if st.session_state.auto_refresh and st.session_state.connected:
    time.sleep(st.session_state.refresh_interval)
    fetch_spot.clear()
    fetch_expirations.clear()
    st.rerun()
