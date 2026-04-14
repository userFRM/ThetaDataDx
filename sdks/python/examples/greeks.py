"""Compute option Greeks using the Rust Black-Scholes calculator."""
from thetadatadx import all_greeks, implied_volatility

# SPY ATM call: spot=$450, strike=$450, r=5%, q=1.5%, 30 DTE, price=$8.50
g = all_greeks(
    spot=450.0,
    strike=450.0,
    rate=0.05,
    div_yield=0.015,
    tte=30.0 / 365.0,
    option_price=8.50,
    right="C",
)

print("=== SPY 450C 30 DTE Greeks ===")
print(f"  Value:     {g['value']:.4f}")
print(f"  IV:        {g['iv']:.4f} (error: {g['iv_error']:.6f})")
print(f"  Delta:     {g['delta']:.4f}")
print(f"  Gamma:     {g['gamma']:.6f}")
print(f"  Theta:     {g['theta']:.4f}")
print(f"  Vega:      {g['vega']:.4f}")
print(f"  Rho:       {g['rho']:.4f}")
print(f"  Vanna:     {g['vanna']:.6f}")
print(f"  Charm:     {g['charm']:.6f}")
print(f"  Vomma:     {g['vomma']:.6f}")
print(f"  Speed:     {g['speed']:.8f}")
print(f"  Zomma:     {g['zomma']:.8f}")
print(f"  Color:     {g['color']:.8f}")
print(f"  Ultima:    {g['ultima']:.6f}")

# Just IV
iv, err = implied_volatility(450.0, 450.0, 0.05, 0.015, 30.0 / 365.0, 8.50, "C")
print(f"\n  IV only:   {iv:.4f} (error: {err:.6f})")
