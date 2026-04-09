# Fix-057: OpenRouter String Pricing + top_provider Metadata

## Problem

OpenRouter's `/api/v1/models` returns pricing as **string values** (per-token), not floats:
```json
{
  "id": "anthropic/claude-sonnet-4",
  "context_length": 200000,
  "pricing": {
    "prompt": "0.000003",
    "completion": "0.000015"
  },
  "top_provider": {
    "context_length": 200000,
    "max_completion_tokens": 64000
  }
}
```

But `PricingInfo` in `refresher.rs` expects `Option<f64>` for `prompt` and `completion`, causing serde deserialization to fail with "error decoding response body".

Additionally, OpenRouter puts useful metadata under `top_provider` (context_length, max_completion_tokens) which is not being parsed.

## Fix Requirements

### 1. Make PricingInfo accept both string and float values

In `sidecar/src/models/refresher.rs`:

The `PricingInfo` struct needs a custom deserializer for `prompt` and `completion` that accepts:
- A JSON number: `0.000003` → `Some(0.000003)`
- A JSON string: `"0.000003"` → `Some(0.000003)` 
- Null or missing: `None`

Use a helper function with `#[serde(deserialize_with = "...")]` on both fields. Example approach:

```rust
fn deserialize_string_or_float<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    match val {
        None => Ok(None),
        Some(serde_json::Value::Number(n)) => Ok(n.as_f64()),
        Some(serde_json::Value::String(s)) => s.parse::<f64>().ok().or_else(|| {
            // Handle "-1" or other non-numeric sentinel values
            None
        }).into(),
        Some(_) => Ok(None),
    }
}
```

Apply this deserializer to `PricingInfo.prompt` and `PricingInfo.completion`.

Also apply the same deserializer to `OpenAiModelEntry.input_cost_per_token` and `OpenAiModelEntry.output_cost_per_token`, and `OpenAiModelDetail.input_cost_per_token` and `OpenAiModelDetail.output_cost_per_token` since those could also be strings on some providers.

### 2. Add top_provider parsing for OpenRouter

Add a `TopProvider` struct:
```rust
#[derive(Debug, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_completion_tokens: Option<u64>,
}
```

Add to `OpenAiModelEntry`:
```rust
#[serde(default)]
top_provider: Option<TopProvider>,
```

In the list response metadata extraction (around line 135-155), add `top_provider` as a fallback source for context and max_output_tokens:
- `entry_ctx` should also fall back to `entry.top_provider.as_ref().and_then(|tp| tp.context_length)`
- `entry_max_out` should also fall back to `entry.top_provider.as_ref().and_then(|tp| tp.max_completion_tokens)`

### 3. Fix extract_from_raw_json for string pricing

In `extract_from_raw_json` (around line 238-244), the pricing extraction uses `.as_f64()` which only works on JSON numbers. Add fallback to parse string values:

```rust
if let Some(pricing) = obj.get("pricing").and_then(|v| v.as_object()) {
    if let Some(prompt) = pricing.get("prompt") {
        let parsed = prompt.as_f64().or_else(|| prompt.as_str().and_then(|s| s.parse::<f64>().ok()));
        if let Some(p) = parsed {
            model.input_cost_per_1m = Some(p * 1_000_000.0);
        }
    }
    // same for completion
}
```

### 4. Handle OpenRouter pricing conversion correctly

OpenRouter pricing values are **per-token** (e.g., `"0.000003"` = $0.000003/token = $3.00/million tokens). The existing code in the list extraction (around line 143-155) already multiplies by 1_000_000 for `pricing.prompt` and `pricing.completion`:
```rust
.or_else(|| entry.pricing.as_ref().and_then(|p| p.prompt).map(|c| c * 1_000_000.0))
```
This is correct for OpenRouter since their values are per-token. Keep this behavior.

### 5. Add tests

Add tests for:
- Parsing OpenRouter-style model entry with string pricing, context_length, and top_provider
- `extract_from_raw_json` handling string pricing values
- TopProvider fallback for context_length and max_completion_tokens

### Files to modify
- `sidecar/src/models/refresher.rs`

### Build & Test Commands
```bash
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo test -p coderouter-proxy
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo build
```
