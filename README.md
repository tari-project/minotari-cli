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

#### Reveal PII
If you are debugging and need to see the full, unmasked data, you can set the `REVEAL_PII` environment variable:

```bash
# Reveal full addresses and amounts in logs
REVEAL_PII=true cargo run --bin minotari -- scan [ARGS]
```

**Supported values for `REVEAL_PII`:** `true`, `1`.

#### Advanced Logging Configuration
The application comes with an embedded logging configuration. However, you can override this by placing a `log4rs.yml` file in the application's working directory. 

The application supports a custom encoder kind called `structured_console`, which renders structured log attributes (key-value pairs) as colorized text in the console.

**Example `log4rs.yml`:**
```yaml
appenders:
  stdout:
    kind: console
    encoder:
      kind: structured_console
      pattern: "{d(%Y-%m-%d %H:%M:%S)} {h({l}):5} {m}"

root:
  level: info
  appenders:
    - stdout
```

#### Running with Docker
If running inside a Docker container, you can mount your custom configuration file:

```bash
docker run -v $(pwd)/log4rs.yml:/app/log4rs.yml minotari-wallet scan [ARGS]
```

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

**Parameters:**

- `--password`: Password used to decrypt the wallet
- `--base-url`: Tari RPC endpoint URL (default: `https://rpc.tari.com`)
- `--database-file`: Path to the database file (default: `data/wallet.db`)
- `--account-name`: Optional account name to scan (scans all accounts if not specified)
- `--max-blocks-to-scan`: Maximum number of blocks to scan per run (default: `50`)
- `--batch-size`: Number of blocks to scan per batch (default: `1`)

### Check Balance

View your wallet balance:

```bash
cargo run --bin minotari -- balance \
  --database-file data/wallet.db \
  --account-name default
```

**Parameters:**

- `--database-file`: Path to the database file (default: `data/wallet.db`)
- `--account-name`: Optional account name (shows all accounts if not specified)

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

- Wallet keys are encrypted with XChaCha20-Poly1305 using a user-provided password
- Private keys never leave your local machine
- View-only scanning means the wallet cannot spend funds
- Passwords are padded to 32 characters for encryption (ensure strong passwords)
- PII Masking: By default, logs redact transaction amounts and truncate addresses (e.g., `abcd12...wxyz34`) to prevent sensitive data from leaking into log files.

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

## Testing

The project includes a comprehensive Cucumber BDD integration testing suite that covers all major features:

```bash
# Run all integration tests
cargo test --test integration_tests

# Run specific test (requires building first)
cargo test --test integration_tests -- --nocapture
```

See [tests/cucumber/README.md](tests/cucumber/README.md) for detailed testing documentation.

## Contributing
