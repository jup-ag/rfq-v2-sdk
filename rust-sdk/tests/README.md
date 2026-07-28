# RFQ V2 Integration Tests

End-to-end tests against the preprod Ultra API, plus one offline decode/tamper check.

| Test | Network | Needs a key |
|---|---|---|
| `malicious_order_e2e::test_tamper_inflates_out_offline` | no | no |
| `ultra_api_e2e::test_ultra_api_order` | yes | no |
| `ultra_api_e2e::test_decode_spl_token_order` | yes | no |
| `ultra_api_e2e::test_execute_order` | yes — **submits a real transaction** | yes |
| `malicious_order_e2e::test_malicious_order_rejected` | yes — **submits a real transaction** | yes |

Everything in the "network" rows is `#[ignore]`d, so a plain `cargo test` runs only the offline
check. Pass `--ignored` to opt in.

## Environment

| Variable | Description | Required |
|---|---|---|
| `SOLANA_PRIVATE_KEY` | Base58-encoded private key of the **taker** wallet | For the two submitting tests |
| `TAKER` | Taker public key – derived from `SOLANA_PRIVATE_KEY` when omitted | No |
| `INPUT_MINT` | Mint for the input side (default: USDC) | No |
| `OUTPUT_MINT` | Mint for the output side (default: `A3QAoKnf3jFcCfTGvEpE7KVBMZqXQJwvwt6Uc4UExkDp`) | No |
| `ULTRA_API_BASE` | Ultra API base URL (default: `https://preprod.ultra-api.jup.ag`) | No |

## Running

```bash
# Offline only — safe, no network, no key
cargo test
```

```bash
# All integration tests, including the ones that submit transactions
cargo test -- --ignored --nocapture
```

```bash
# A single test
cargo test --test ultra_api_e2e test_ultra_api_order -- --ignored --nocapture
```
