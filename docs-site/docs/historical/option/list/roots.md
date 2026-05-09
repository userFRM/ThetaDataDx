---
title: option_list_roots
description: List all available option underlying symbols.
---

# option_list_roots

<TierBadge tier="free" />

List all available option underlying symbols (roots). Use this to discover which tickers have option chains available in ThetaData.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_list_symbols().await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.option_list_symbols()
for item in data:
    print(item)
```
```typescript [TypeScript]
const data = tdx.optionListSymbols();
console.log(data);
```
```cpp [C++]
auto data = client.option_list_symbols();
for (const auto& item : data) {
    printf("%s\n", item.c_str());
}
```
:::

## Parameters

None.

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>(list)</code><span class="param-type">string[]</span></div>
<div class="param-desc">Underlying ticker symbols with available option chains</div>
</div>
</div>


### Sample Response

```json
["SPY", "SPY1", "SPY2", "SPY7"]
```

> Returns the full list of underlying symbols with available option chains.

## Notes

- Returns all underlying symbols, not individual contracts. Use [option_list_expirations](./expirations) and [option_list_strikes](./strikes) to drill into a specific chain.
- The Rust SDK method is `option_list_symbols`. The legacy "roots" term is preserved in this page's URL slug for inbound-link stability; the SDK and protobuf surface use `symbol` exclusively.
