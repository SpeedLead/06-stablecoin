# Stablecoin Program

A Solana stablecoin program built with [Anchor](https://www.anchor-lang.com/) that demonstrates how to issue and manage a Token-2022 (Token Extensions) token with controlled minting, allowance management, emergency pause, and compliance controls (force burn / force transfer) via the Permanent Delegate extension.

## Overview

The program creates a Token-2022 mint whose authority is a PDA owned by the program itself, so no single private key controls issuance. An admin account manages a set of authorized minters, each with an individual allowance that caps how many tokens they can mint in total. This pattern mirrors how real-world stablecoins (e.g. USDC) segregate the admin role from individual mint operators.

The mint is created with two Token-2022 extensions:

- **PermanentDelegate** — the `Config` PDA is a permanent delegate over every token account of this mint, letting the admin burn or move tokens from any account (e.g. to comply with a court order or recover stolen funds).
- **DefaultAccountState** — new token accounts start in the `Initialized` (unfrozen) state.

## Features

- **Token-2022 mint with extensions** — uses the Token Extensions program (`TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`)
- **Controlled minting** — only authorized minters can create new tokens
- **Per-minter allowances** — each minter has a cap on their cumulative mint volume
- **Emergency pause** — admin can halt all minting instantly
- **Token burning** — any user can burn their own tokens (e.g. for fiat redemption)
- **Compliance controls** — admin can force-burn or force-transfer tokens from any account
- **Rent reclamation** — removing a minter closes their config account and returns rent to the admin

## Program ID

```
rYXfi25x9JMgau82aGMJMVUokq7JzueqehiJUmwR97Q
```

## Prerequisites

- [Rust](https://rustup.rs/) — `rust-toolchain.toml` tracks the latest `stable` channel (run `rustup update` to upgrade)
- [Solana CLI](https://solana.com/developers/guides/getstarted/setup-local-development)
- [Anchor CLI](https://www.anchor-lang.com/docs/installation) v1.0.0

The `anchor-lang` and `anchor-spl` crates are pinned to exactly `=1.0.0` in `programs/stablecoin/Cargo.toml` to match the CLI. A plain `"1.0.0"` would let Cargo resolve any newer 1.x release.

## Building

```bash
anchor build
```

If `target/` is fresh, Anchor generates a new program keypair that won't match the `declare_id!` above. Either place the original keypair at `target/deploy/stablecoin-keypair.json`, run `anchor keys sync` to adopt the new ID, or build with `anchor build --ignore-keys`.

After changing the program's instructions or accounts, copy the generated IDL so the tests pick it up:

```bash
cp target/idl/stablecoin.json idls/stablecoin.json
```

## Testing

Tests use [LiteSVM](https://github.com/LiteSVM/litesvm) via [`anchor-litesvm`](https://crates.io/crates/anchor-litesvm) — an in-process Solana VM that runs tests without a local validator, making them fast and deterministic. The client types are generated from `idls/stablecoin.json` with `declare_program!`, and the tests load the compiled program from `target/deploy/stablecoin.so`, so run `anchor build` first.

```bash
cargo test -p stablecoin
```

All 24 tests should pass.

## Instructions

### `initialize`

Creates the stablecoin mint and config account. Must be called once before any other instruction.

```
_name: String    — token name (currently unused)
_symbol: String  — token symbol (currently unused)
_uri: String     — metadata URI (currently unused)
```

- Creates a `Config` PDA at seeds `["config"]`
- Creates a Token-2022 mint PDA at seeds `["mint"]` with the PermanentDelegate and DefaultAccountState extensions, via CPIs to the System and Token-2022 programs
- Sets the `Config` PDA as mint authority, freeze authority, and permanent delegate
- Uses 6 decimals (USDC-style)

### `configure_minter`

Authorizes a new minter or updates an existing minter's allowance. Admin only.

```
allowance: u64  — cumulative token cap for this minter
```

### `remove_minter`

Revokes a minter's authorization. Closes the `MinterConfig` account and returns rent to the admin. Admin only.

### `mint_tokens`

Mints tokens to an arbitrary destination account (created as an ATA if it doesn't exist). Reverts if:

- The program is paused
- The caller has no `MinterConfig`
- The requested amount exceeds the minter's remaining allowance (`allowance - amount_minted`)

### `burn_tokens`

Burns tokens from the caller's own token account. Anyone can call this instruction.

### `pause` / `unpause`

Toggles the global `paused` flag on the `Config` account. When paused, all `mint_tokens` calls revert. Admin only.

### `force_burn`

Burns tokens from **any** token account of this mint, signed by the `Config` PDA as permanent delegate. The owner's signature is not required. Admin only.

```
amount: u64  — number of tokens to burn
```

### `force_transfer`

Moves tokens from **any** token account of this mint to another, using `transfer_checked` signed by the `Config` PDA as permanent delegate. The destination token account must already exist. Admin only.

```
amount: u64  — number of tokens to transfer
```

## Account Structure

### `Config` — PDA seeds: `["config"]`

| Field       | Type   | Description                                            |
|-------------|--------|--------------------------------------------------------|
| `admin`     | Pubkey | Can manage minters, pause/unpause, and force burn/transfer |
| `mint`      | Pubkey | The Token-2022 mint address                            |
| `paused`    | bool   | Minting disabled when `true`                           |
| `bump`      | u8     | PDA bump seed                                          |
| `mint_bump` | u8     | Mint PDA bump seed                                     |

### `MinterConfig` — PDA seeds: `["minter", minter_pubkey]`

| Field            | Type   | Description                              |
|------------------|--------|------------------------------------------|
| `minter`         | Pubkey | The authorized minter's public key       |
| `allowance`      | u64    | Total tokens this minter may ever mint   |
| `amount_minted`  | u64    | Tokens minted so far                     |
| `is_initialized` | bool   | Set on first `configure_minter` call     |
| `bump`           | u8     | PDA bump seed                            |

## Token-2022 Notes

The mint is created using the **Token Extensions program** (`Token-2022`). Associated token accounts for this mint use a different ATA derivation path than legacy SPL Token accounts — the token program ID is included as a seed:

```
ATA = find_program_address(
    [wallet, TOKEN_2022_PROGRAM_ID, mint],
    ATA_PROGRAM_ID
)
```

Because the mint carries extensions, its size is computed with `ExtensionType::try_calculate_account_len` and the account is created manually. The extensions are initialized **before** `initialize_mint2`, as Token-2022 requires.

### Anchor 1.0.0 notes

- `CpiContext::new` / `new_with_signer` take the target program's `Pubkey` (e.g. `ctx.accounts.token_program.key()`), not an `AccountInfo`.
- The Token-2022 interface crate is re-exported as `anchor_spl::token_2022::spl_token_2022`, so no separate `spl-token-2022` dependency is needed.
- `declare_program!` expands to `use super::anchor_lang`, so the test crate imports `anchor_lang::prelude::*` before invoking it.

## License

MIT
