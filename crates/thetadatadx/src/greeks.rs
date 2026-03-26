//! Black-Scholes Greeks calculator, ported from ThetaData's Java implementation.
//!
//! Parameters:
//! - `s`: Spot price (underlying)
//! - `x`: Strike price
//! - `v`: Volatility (sigma)
//! - `r`: Risk-free rate
//! - `q`: Dividend yield
//! - `t`: Time to expiration (years)
//! - `is_call`: true for call, false for put
//!
//! # Edge-case guards
//!
//! All public Greek functions guard against `t <= 0.0` or `v <= 0.0` with
//! early returns of 0.0 (or the mathematically correct limit). This prevents
//! NaN/Inf contamination when Black-Scholes degenerates.

// 1 / sqrt(2 * pi)
const ONE_ROOT2PI: f64 = 0.3989422804014327;

const MAX_TRIES: usize = 128;

/// Standard normal PDF: phi(x)
fn f1(x: f64) -> f64 {
    ONE_ROOT2PI * (-0.5 * x * x).exp()
}

/// Clamp Inf/NaN to 0.
fn realize(x: f64) -> f64 {
    if x.is_infinite() || x.is_nan() {
        0.0
    } else {
        x
    }
}

/// Return true if t or v make Black-Scholes degenerate.
#[inline]
fn is_degenerate(v: f64, t: f64) -> bool {
    t <= 0.0 || v <= 0.0
}

/// Standard normal CDF approximation (Abramowitz & Stegun).
// TODO(perf): Replace Abramowitz & Stegun 5-term polynomial (max error ~1.5e-7)
// with the faster Hart approximation or a minimax rational function that achieves
// the same error in fewer multiplications. For IV solver loops (128 iterations),
// this is the dominant cost.
fn norm_cdf(x: f64) -> f64 {
    if x >= 0.0 {
        norm_cdf_positive(x)
    } else {
        1.0 - norm_cdf_positive(-x)
    }
}

fn norm_cdf_positive(x: f64) -> f64 {
    const A1: f64 = 0.254829592;
    const A2: f64 = -0.284496736;
    const A3: f64 = 1.421413741;
    const A4: f64 = -1.453152027;
    const A5: f64 = 1.061405429;
    const P: f64 = 0.3275911;
    let t = 1.0 / (1.0 + P * x);
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;
    let t5 = t4 * t;
    1.0 - f1(x) * (A1 * t + A2 * t2 + A3 * t3 + A4 * t4 + A5 * t5)
}

pub fn d1(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    ((s / x).ln() + t * (r - q + v * v / 2.0)) / (v * t.sqrt())
}

pub fn d2(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    d1(s, x, v, r, q, t) - v * t.sqrt()
}

fn e1_from_d1(d1_val: f64) -> f64 {
    (-d1_val.powi(2) / 2.0).exp()
}

/// Black-Scholes theoretical option value.
pub fn value(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        // At expiry / zero vol, value is intrinsic value.
        let intrinsic = if is_call {
            (s * (-q * t.max(0.0)).exp() - x * (-r * t.max(0.0)).exp()).max(0.0)
        } else {
            (x * (-r * t.max(0.0)).exp() - s * (-q * t.max(0.0)).exp()).max(0.0)
        };
        return intrinsic;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    if is_call {
        s * (-q * t).exp() * norm_cdf(d1_val) - (-r * t).exp() * x * norm_cdf(d2_val)
    } else {
        (-r * t).exp() * x * norm_cdf(-d2_val) - s * (-q * t).exp() * norm_cdf(-d1_val)
    }
}

pub fn delta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    if is_call {
        (-q * t).exp() * norm_cdf(d1_val)
    } else {
        (-q * t).exp() * (norm_cdf(d1_val) - 1.0)
    }
}

pub fn theta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    let term1 = -(-q * t).exp() * (s * f1(d1_val) * v) / (2.0 * t.sqrt());
    if is_call {
        (term1 - r * x * (-r * t).exp() * norm_cdf(d2_val)
            + q * s * (-q * t).exp() * norm_cdf(d1_val))
            / 365.0
    } else {
        (term1 + r * x * (-r * t).exp() * norm_cdf(-d2_val)
            - q * s * (-q * t).exp() * norm_cdf(-d1_val))
            / 365.0
    }
}

pub fn vega(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    s * (-q * t).exp() * t.sqrt() * ONE_ROOT2PI * e1_from_d1(d1_val)
}

pub fn rho(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d2_val = d2(s, x, v, r, q, t);
    if is_call {
        x * t * (-r * t).exp() * norm_cdf(d2_val)
    } else {
        -x * t * (-r * t).exp() * norm_cdf(-d2_val)
    }
}

pub fn epsilon(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    if is_call {
        realize(-s * t * (-q * t).exp() * norm_cdf(d1_val))
    } else {
        realize(s * t * (-q * t).exp() * norm_cdf(-d1_val))
    }
}

pub fn lambda(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    realize(delta(s, x, v, r, q, t, is_call) * s / value(s, x, v, r, q, t, is_call))
}

pub fn gamma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    (-q * t).exp() / (s * v * t.sqrt()) * ONE_ROOT2PI * e1_from_d1(d1_val)
}

pub fn vanna(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    -(-q * t).exp() * f1(d1_val) * d2_val / v
}

pub fn charm(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    let p1 = (2.0 * (r - q) * t - d2_val * v * t.sqrt()) / (2.0 * t * v * t.sqrt());
    if is_call {
        q * (-q * t).exp() * norm_cdf(d1_val) - (-q * t).exp() * f1(d1_val) * p1
    } else {
        -q * (-q * t).exp() * norm_cdf(-d1_val) - (-q * t).exp() * f1(d1_val) * p1
    }
}

pub fn vomma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    vega(s, x, v, r, q, t) * (d1_val * d2_val / v)
}

pub fn veta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    -s * (-q * t).exp()
        * f1(d1_val)
        * t.sqrt()
        * (q + (r - q) * d1_val / (v * t.sqrt()) - (1.0 + d1_val * d2_val) / (2.0 * t))
}

pub fn speed(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    -(-q * t).exp() * f1(d1_val) / (s * s * v * t.sqrt()) * (d1_val / (v * t.sqrt()) + 1.0)
}

pub fn zomma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    (-q * t).exp() * f1(d1_val) * (d1_val * d2_val - 1.0) / (s * v * v * t.sqrt())
}

pub fn color(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    -(-q * t).exp() * f1(d1_val) / (2.0 * s * t * v * t.sqrt())
        * (2.0 * q * t
            + 1.0
            + (2.0 * (r - q) * t - d2_val * v * t.sqrt()) / (v * t.sqrt()) * d1_val)
}

pub fn ultima(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    let d2_val = d2(s, x, v, r, q, t);
    let out = -vega(s, x, v, r, q, t) / (v * v)
        * (d1_val * d2_val * (1.0 - d1_val * d2_val) + d1_val * d1_val + d2_val * d2_val);
    out.clamp(-100.0, 100.0)
}

pub fn dual_delta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d2_val = d2(s, x, v, r, q, t);
    if is_call {
        -(-r * t).exp() * norm_cdf(d2_val)
    } else {
        (-r * t).exp() * norm_cdf(-d2_val)
    }
}

pub fn dual_gamma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d2_val = d2(s, x, v, r, q, t);
    (-r * t).exp() * f1(d2_val) / (x * v * t.sqrt())
}

/// Implied volatility solver using bisection. Returns `(iv, error)`.
pub fn implied_volatility(
    s: f64,
    x: f64,
    r: f64,
    q: f64,
    t: f64,
    option_price: f64,
    is_call: bool,
) -> (f64, f64) {
    if t <= 0.0 || option_price <= 0.0 {
        return (0.0, 0.0);
    }
    let mut out = [0.0f64; 2];
    iv_bisection(s, x, r, q, t, option_price, is_call, &mut out);
    (out[0], out[1])
}

#[allow(clippy::too_many_arguments)]
fn iv_bisection(s: f64, x: f64, r: f64, q: f64, t: f64, o: f64, is_call: bool, out: &mut [f64; 2]) {
    // Check intrinsic value boundary
    if value(s, x, 0.0, r, q, t, is_call) > o {
        out[0] = 0.0;
        out[1] = ((value(s, x, 0.0, r, q, t, is_call) - o) / o).clamp(-100.0, 100.0);
        return;
    }

    let mut guess = 0.5;
    let start = 0.0;
    let mut end = guess;
    let mut changer = 0.2;

    // Find upper bound
    for _ in 0..32 {
        end += changer;
        if value(s, x, end, r, q, t, is_call) > o {
            break;
        }
        changer *= 2.0;
    }

    let mut start = start;
    for _ in 0..MAX_TRIES {
        let v = value(s, x, guess, r, q, t, is_call);
        if (v - o).abs() < 0.001 {
            out[0] = guess;
            out[1] = ((v - o) / o).clamp(-100.0, 100.0);
            return;
        }
        if v > o {
            end = guess;
            guess -= (end - start) / 2.0;
        } else {
            start = guess;
            guess += (end - start) / 2.0;
        }
    }

    let v = value(s, x, guess, r, q, t, is_call);
    out[0] = guess;
    out[1] = ((v - o) / o).clamp(-100.0, 100.0);
}

/// All Greeks computed in a single struct.
#[derive(Debug, Clone, Copy)]
pub struct GreeksResult {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub iv: f64,
    pub iv_error: f64,
    // Second order
    pub vanna: f64,
    pub charm: f64,
    pub vomma: f64,
    pub veta: f64,
    // Third order
    pub speed: f64,
    pub zomma: f64,
    pub color: f64,
    pub ultima: f64,
    // Auxiliary
    pub d1: f64,
    pub d2: f64,
    pub dual_delta: f64,
    pub dual_gamma: f64,
    pub epsilon: f64,
    pub lambda: f64,
}

/// Compute all Greeks at once.
///
/// Precomputes `d1` and `d2` once and passes them to each Greek function
/// internally, avoiding ~20 redundant recalculations.
pub fn all_greeks(
    s: f64,
    x: f64,
    r: f64,
    q: f64,
    t: f64,
    option_price: f64,
    is_call: bool,
) -> GreeksResult {
    let (iv_val, iv_err) = implied_volatility(s, x, r, q, t, option_price, is_call);
    let v = iv_val;

    // Guard: if vol or time is degenerate, return all zeros (except value = intrinsic).
    if is_degenerate(v, t) {
        return GreeksResult {
            value: value(s, x, v, r, q, t, is_call),
            delta: 0.0,
            gamma: 0.0,
            theta: 0.0,
            vega: 0.0,
            rho: 0.0,
            iv: iv_val,
            iv_error: iv_err,
            vanna: 0.0,
            charm: 0.0,
            vomma: 0.0,
            veta: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
            ultima: 0.0,
            d1: 0.0,
            d2: 0.0,
            dual_delta: 0.0,
            dual_gamma: 0.0,
            epsilon: 0.0,
            lambda: 0.0,
        };
    }

    // Precompute d1 and d2 once.
    let sqrt_t = t.sqrt();
    let d1_val = ((s / x).ln() + t * (r - q + v * v / 2.0)) / (v * sqrt_t);
    let d2_val = d1_val - v * sqrt_t;
    let e1_val = (-d1_val.powi(2) / 2.0).exp();
    let f1_d1 = f1(d1_val);
    let exp_neg_qt = (-q * t).exp();
    let exp_neg_rt = (-r * t).exp();
    let nd1 = norm_cdf(d1_val);
    let nd2 = norm_cdf(d2_val);
    let n_neg_d1 = norm_cdf(-d1_val);
    let n_neg_d2 = norm_cdf(-d2_val);

    // Value
    let value_val = if is_call {
        s * exp_neg_qt * nd1 - exp_neg_rt * x * nd2
    } else {
        exp_neg_rt * x * n_neg_d2 - s * exp_neg_qt * n_neg_d1
    };

    // Delta
    let delta_val = if is_call {
        exp_neg_qt * nd1
    } else {
        exp_neg_qt * (nd1 - 1.0)
    };

    // Theta
    let theta_term1 = -exp_neg_qt * (s * f1_d1 * v) / (2.0 * sqrt_t);
    let theta_val = if is_call {
        (theta_term1 - r * x * exp_neg_rt * nd2 + q * s * exp_neg_qt * nd1) / 365.0
    } else {
        (theta_term1 + r * x * exp_neg_rt * n_neg_d2 - q * s * exp_neg_qt * n_neg_d1) / 365.0
    };

    // Vega
    let vega_val = s * exp_neg_qt * sqrt_t * ONE_ROOT2PI * e1_val;

    // Rho
    let rho_val = if is_call {
        x * t * exp_neg_rt * nd2
    } else {
        -x * t * exp_neg_rt * n_neg_d2
    };

    // Epsilon
    let epsilon_val = if is_call {
        realize(-s * t * exp_neg_qt * nd1)
    } else {
        realize(s * t * exp_neg_qt * n_neg_d1)
    };

    // Lambda
    let lambda_val = if value_val.abs() > f64::EPSILON {
        realize(delta_val * s / value_val)
    } else {
        0.0
    };

    // Gamma
    let gamma_val = exp_neg_qt / (s * v * sqrt_t) * ONE_ROOT2PI * e1_val;

    // Vanna
    let vanna_val = -exp_neg_qt * f1_d1 * d2_val / v;

    // Charm
    let charm_p1 = (2.0 * (r - q) * t - d2_val * v * sqrt_t) / (2.0 * t * v * sqrt_t);
    let charm_val = if is_call {
        q * exp_neg_qt * nd1 - exp_neg_qt * f1_d1 * charm_p1
    } else {
        -q * exp_neg_qt * n_neg_d1 - exp_neg_qt * f1_d1 * charm_p1
    };

    // Vomma
    let vomma_val = vega_val * (d1_val * d2_val / v);

    // Veta
    let veta_val = -s
        * exp_neg_qt
        * f1_d1
        * sqrt_t
        * (q + (r - q) * d1_val / (v * sqrt_t) - (1.0 + d1_val * d2_val) / (2.0 * t));

    // Speed
    let speed_val = -exp_neg_qt * f1_d1 / (s * s * v * sqrt_t) * (d1_val / (v * sqrt_t) + 1.0);

    // Zomma
    let zomma_val = exp_neg_qt * f1_d1 * (d1_val * d2_val - 1.0) / (s * v * v * sqrt_t);

    // Color
    let color_val = -exp_neg_qt * f1_d1 / (2.0 * s * t * v * sqrt_t)
        * (2.0 * q * t + 1.0 + (2.0 * (r - q) * t - d2_val * v * sqrt_t) / (v * sqrt_t) * d1_val);

    // Ultima
    let ultima_raw = -vega_val / (v * v)
        * (d1_val * d2_val * (1.0 - d1_val * d2_val) + d1_val * d1_val + d2_val * d2_val);
    let ultima_val = ultima_raw.clamp(-100.0, 100.0);

    // Dual delta
    let dual_delta_val = if is_call {
        -exp_neg_rt * nd2
    } else {
        exp_neg_rt * n_neg_d2
    };

    // Dual gamma
    let f1_d2 = f1(d2_val);
    let dual_gamma_val = exp_neg_rt * f1_d2 / (x * v * sqrt_t);

    GreeksResult {
        value: value_val,
        delta: delta_val,
        gamma: gamma_val,
        theta: theta_val,
        vega: vega_val,
        rho: rho_val,
        iv: iv_val,
        iv_error: iv_err,
        vanna: vanna_val,
        charm: charm_val,
        vomma: vomma_val,
        veta: veta_val,
        speed: speed_val,
        zomma: zomma_val,
        color: color_val,
        ultima: ultima_val,
        d1: d1_val,
        d2: d2_val,
        dual_delta: dual_delta_val,
        dual_gamma: dual_gamma_val,
        epsilon: epsilon_val,
        lambda: lambda_val,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert that a value is finite (not NaN, not Inf).
    fn assert_finite(val: f64, label: &str) {
        assert!(val.is_finite(), "{label} must be finite, got {val}");
    }

    #[test]
    fn test_call_value() {
        // SPY ~$450, strike $450, vol 20%, r=5%, q=1.5%, 30 days
        let v = value(450.0, 450.0, 0.20, 0.05, 0.015, 30.0 / 365.0, true);
        assert!(v > 5.0 && v < 15.0, "ATM call value: {v}");
    }

    #[test]
    fn test_put_call_parity() {
        let s = 100.0;
        let x = 100.0;
        let v = 0.25;
        let r = 0.05;
        let q = 0.02;
        let t = 0.5;

        let call = value(s, x, v, r, q, t, true);
        let put = value(s, x, v, r, q, t, false);
        let parity = s * (-q * t).exp() - x * (-r * t).exp();
        assert!(
            (call - put - parity).abs() < 1e-10,
            "Put-call parity violated: call={call}, put={put}, parity={parity}"
        );
    }

    #[test]
    fn test_iv_roundtrip() {
        let s = 150.0;
        let x = 155.0;
        let r = 0.05;
        let q = 0.015;
        let t = 45.0 / 365.0;
        let true_vol = 0.22;

        let price = value(s, x, true_vol, r, q, t, true);
        let (iv, err) = implied_volatility(s, x, r, q, t, price, true);
        assert!(
            (iv - true_vol).abs() < 0.005,
            "IV roundtrip: expected {true_vol}, got {iv}, err={err}"
        );
    }

    // ── Edge-case tests (Fix #10 + Fix #16) ──

    #[test]
    fn edge_t_zero_returns_finite() {
        let s = 100.0;
        let x = 100.0;
        let v = 0.20;
        let r = 0.05;
        let q = 0.01;
        let t = 0.0;

        // All public Greeks must return finite values.
        assert_finite(d1(s, x, v, r, q, t), "d1(t=0)");
        assert_finite(d2(s, x, v, r, q, t), "d2(t=0)");
        assert_finite(value(s, x, v, r, q, t, true), "value(t=0, call)");
        assert_finite(value(s, x, v, r, q, t, false), "value(t=0, put)");
        assert_finite(delta(s, x, v, r, q, t, true), "delta(t=0)");
        assert_finite(theta(s, x, v, r, q, t, true), "theta(t=0)");
        assert_finite(vega(s, x, v, r, q, t), "vega(t=0)");
        assert_finite(rho(s, x, v, r, q, t, true), "rho(t=0)");
        assert_finite(gamma(s, x, v, r, q, t), "gamma(t=0)");
        assert_finite(vanna(s, x, v, r, q, t), "vanna(t=0)");
        assert_finite(charm(s, x, v, r, q, t, true), "charm(t=0)");
        assert_finite(vomma(s, x, v, r, q, t), "vomma(t=0)");
        assert_finite(veta(s, x, v, r, q, t), "veta(t=0)");
        assert_finite(speed(s, x, v, r, q, t), "speed(t=0)");
        assert_finite(zomma(s, x, v, r, q, t), "zomma(t=0)");
        assert_finite(color(s, x, v, r, q, t), "color(t=0)");
        assert_finite(ultima(s, x, v, r, q, t), "ultima(t=0)");
        assert_finite(dual_delta(s, x, v, r, q, t, true), "dual_delta(t=0)");
        assert_finite(dual_gamma(s, x, v, r, q, t), "dual_gamma(t=0)");
        assert_finite(epsilon(s, x, v, r, q, t, true), "epsilon(t=0)");
        assert_finite(lambda(s, x, v, r, q, t, true), "lambda(t=0)");
    }

    #[test]
    fn edge_v_zero_returns_finite() {
        let s = 100.0;
        let x = 100.0;
        let v = 0.0;
        let r = 0.05;
        let q = 0.01;
        let t = 0.5;

        assert_finite(d1(s, x, v, r, q, t), "d1(v=0)");
        assert_finite(d2(s, x, v, r, q, t), "d2(v=0)");
        assert_finite(value(s, x, v, r, q, t, true), "value(v=0, call)");
        assert_finite(value(s, x, v, r, q, t, false), "value(v=0, put)");
        assert_finite(delta(s, x, v, r, q, t, true), "delta(v=0)");
        assert_finite(theta(s, x, v, r, q, t, true), "theta(v=0)");
        assert_finite(gamma(s, x, v, r, q, t), "gamma(v=0)");
        assert_finite(vega(s, x, v, r, q, t), "vega(v=0)");
    }

    #[test]
    fn edge_option_price_zero_returns_finite() {
        let s = 100.0;
        let x = 100.0;
        let r = 0.05;
        let q = 0.01;
        let t = 0.5;

        let (iv, err) = implied_volatility(s, x, r, q, t, 0.0, true);
        assert_finite(iv, "iv(option_price=0)");
        assert_finite(err, "iv_err(option_price=0)");
        assert_eq!(iv, 0.0);

        let g = all_greeks(s, x, r, q, t, 0.0, true);
        assert_finite(g.value, "all_greeks(option_price=0).value");
        assert_finite(g.delta, "all_greeks(option_price=0).delta");
        assert_finite(g.gamma, "all_greeks(option_price=0).gamma");
        assert_finite(g.theta, "all_greeks(option_price=0).theta");
    }

    #[test]
    fn edge_atm_at_expiry_returns_finite() {
        // s == x (ATM) and t == 0 (at expiry).
        let s = 100.0;
        let x = 100.0;
        let r = 0.05;
        let q = 0.01;
        let t = 0.0;

        let g = all_greeks(s, x, r, q, t, 5.0, true);
        assert_finite(g.value, "all_greeks(ATM, t=0).value");
        assert_finite(g.delta, "all_greeks(ATM, t=0).delta");
        assert_finite(g.gamma, "all_greeks(ATM, t=0).gamma");
        assert_finite(g.theta, "all_greeks(ATM, t=0).theta");
        assert_finite(g.vega, "all_greeks(ATM, t=0).vega");
        assert_finite(g.rho, "all_greeks(ATM, t=0).rho");
        assert_finite(g.iv, "all_greeks(ATM, t=0).iv");
        assert_finite(g.iv_error, "all_greeks(ATM, t=0).iv_error");
        assert_finite(g.vanna, "all_greeks(ATM, t=0).vanna");
        assert_finite(g.charm, "all_greeks(ATM, t=0).charm");
        assert_finite(g.d1, "all_greeks(ATM, t=0).d1");
        assert_finite(g.d2, "all_greeks(ATM, t=0).d2");
    }

    #[test]
    fn all_greeks_precomputed_matches_individual() {
        // Verify that the precomputed all_greeks produces the same results
        // as calling each individual function.
        let s = 150.0;
        let x = 155.0;
        let r = 0.05;
        let q = 0.015;
        let t = 45.0 / 365.0;
        let price = value(s, x, 0.22, r, q, t, true);

        let g = all_greeks(s, x, r, q, t, price, true);
        let v = g.iv;

        let eps = 1e-10;
        assert!(
            (g.value - value(s, x, v, r, q, t, true)).abs() < eps,
            "value mismatch"
        );
        assert!(
            (g.delta - delta(s, x, v, r, q, t, true)).abs() < eps,
            "delta mismatch"
        );
        assert!(
            (g.gamma - gamma(s, x, v, r, q, t)).abs() < eps,
            "gamma mismatch"
        );
        assert!(
            (g.theta - theta(s, x, v, r, q, t, true)).abs() < eps,
            "theta mismatch"
        );
        assert!(
            (g.vega - vega(s, x, v, r, q, t)).abs() < eps,
            "vega mismatch"
        );
        assert!(
            (g.rho - rho(s, x, v, r, q, t, true)).abs() < eps,
            "rho mismatch"
        );
        assert!((g.d1 - d1(s, x, v, r, q, t)).abs() < eps, "d1 mismatch");
        assert!((g.d2 - d2(s, x, v, r, q, t)).abs() < eps, "d2 mismatch");
    }
}
