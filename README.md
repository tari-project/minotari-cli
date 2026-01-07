# Minotari CLI Wallet Example

A command-line lightweight wallet implementation for the Tari blockchain
network. This wallet uses view-only keys to scan the blockchain for
transactions without requiring a full node.

## Features

- **View-Only Wallet**: Import and manage wallets using view keys and spend
  public keys
- **Blockchain Scanning**: Efficiently scan the blockchain for outputs and track
  confirmations
- **Balance Tracking**: Monitor account balances with detailed transaction history
- **Privacy-First Logging**: Sensitive data (PII) is masked in logs by default
- **Reorg Detection**: Automatically detects and handles blockchain reorganizations
- **Encrypted Storage**: Wallet keys are encrypted using XChaCha20-Poly1305
- **SQLite Database**: All wallet data stored in a local SQLite database with migrations
- **Memo Support**: Parse and display payment memos attached to transactions
- **Multi-Account**: Support for multiple wallet accounts in a single database

## Build

- A starting database is required to build the application.
  - [Install the prerequisite tooling.](#prerequisites)
  - [Create a database file](#create) if you don't have one.

Then, you can build as usual:

```bash
cargo build --release
```

### Prerequisites

- Rust toolchain (2024 edition)
- [SQLx CLI](https://crates.io/crates/sqlx) for database migrations

```bash
cargo install sqlx-cli --no-default-features --features sqlite
```

## Usage

### Logging and Privacy

To protect user privacy, the wallet masks Personally Identifiable Information (PII) such as addresses and transaction amounts in the application logs by default.

If you are debugging and need to see the full, unmasked data, you can set the `REVEAL_PII` environment variable:

```bash
# Reveal full addresses and amounts in logs
REVEAL_PII=true cargo run --bin minotari -- scan [ARGS]
```

**Supported values for `REVEAL_PII`:** `true`, `1`.

### Import a Wallet

Import a wallet using your view private key and spend public key:

```bash
cargo run --bin minotari -- import-view-key \
  --view-private-key <HEX_VIEW_KEY> \
  --spend-public-key <HEX_SPEND_KEY> \
  --password <PASSWORD> \
  --database-file data/wallet.db \
  --birthday <BLOCK_HEIGHT>
```

**Parameters:**

- `--view-private-key`: Your view private key in hexadecimal format
- `--spend-public-key`: Your spend public key in hexadecimal format
- `--password`: Password to encrypt the wallet (minimum 32 characters recommended)
- `--database-file`: Path to the SQLite database file (default: `data/wallet.db`)
- `--birthday`: Block height to start scanning from (default: `0`)

### Scan the Blockchain

Scan the blockchain for transactions:

```bash
cargo run --bin minotari -- scan \
  --password <PASSWORD> \
  --base-url https://rpc.tari.com \
  --database-file data/wallet.db \
  --max-blocks-to-scan 100 \
  --batch-size 10
```

### Check Balance

View your wallet balance:

```bash
cargo run --bin minotari -- balance \
  --database-file data/wallet.db \
  --account-name default
```

## Database

The wallet uses SQLite to store:

- **Accounts**: Encrypted wallet keys and metadata
- **Outputs**: Detected outputs with confirmation status
- **Inputs**: Spent outputs (inputs to transactions)
- **Balance Changes**: Detailed transaction history with credits/debits
- **Wallet Events**: Timeline of wallet activity
- **Scanned Blocks**: Track scanning progress and detect reorgs

### Create

- A database is required for building the application.

```shell
mkdir -p data
sqlx database create
sqlx migrate run
```

### Migrations

- Database migrations are located in the `migrations/` directory.
- It is always recommended that you backup your `data/wallet.db` if it's precious.

```shell
sqlx migrate run
```

### Reset

To reset the database via a powershell script:

```powershell
# PowerShell
.\rerun_migrations.ps1
```

Or manually:

```bash
mkdir -p data
rm data/wallet.db
sqlx database create
sqlx migrate run
```

## Security

- **PII Masking**: By default, logs redact transaction amounts and truncate addresses (e.g., `abcd12...wxyz34`) to prevent sensitive data from leaking into log files.
- **Key Encryption**: Wallet keys are encrypted with XChaCha20-Poly1305 using a user-provided password.
- **Local Privacy**: Private keys never leave your local machine.
- **View-Only**: The wallet uses view-only scanning, meaning it cannot spend funds even if the database is compromised.
- **Password Strength**: Passwords are padded to 32 characters for encryption (ensure strong passwords).

## Architecture

### Key Components

- **`src/main.rs`**: CLI interface and core scanning logic
- **`src/log/`**: Structured logging implementation and PII masking utilities.
- **`src/db/`**: Database layer with SQLite queries
  - `accounts.rs`: Account management
  - `outputs.rs`: Output tracking and confirmations
  - `inputs.rs`: Input (spent output) tracking
  - `balance_changes.rs`: Transaction history
  - `events.rs`: Wallet event log
  - `scanned_tip_blocks.rs`: Scan progress and reorg detection
- **`src/models/mod.rs`**: Data models and types

### Scanning Process

1. Load encrypted wallet keys from database
2. Decrypt keys using password
3. Check for blockchain reorganizations
4. Scan blocks in batches starting from last scanned height
5. Detect outputs belonging to the wallet
6. Track confirmations (6 blocks required)
7. Parse memos and payment information
8. Update balance changes and generate events

### OpenAPI Specification

The OpenAPI specification (`openapi.json`) is generated from the API
definitions. If the API changes, you need to regenerate the `openapi.json` file
using the following command:

```bash
cargo run --bin generate-openapi
```

## Dependencies

- **lightweight_wallet_libs**: Tari blockchain scanning library
- **sqlx**: Async SQLite database access
- **chacha20poly1305**: Encryption for wallet keys
- **clap**: Command-line argument parsing
- **tokio**: Async runtime

## Contributing
