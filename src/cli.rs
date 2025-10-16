use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tari")]
#[command(about = "Tari wallet CLI", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new address, and returns a file with the
    /// seed words, address, birthday, private view key and public spend key,
    /// optionally encrypting the file with a password
    CreateAddress {
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: Option<String>,
        #[arg(short, long, help = "Path to the output file", default_value = "data/output.json")]
        output_file: String,
    },
    /// Scan the blockchain for transactions
    Scan {
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(
            short,
            long,
            help = "Optional account name to scan. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
        #[arg(short = 'n', long, help = "Maximum number of blocks to scan", default_value_t = 50)]
        max_blocks_to_scan: u64,
        #[arg(long, help = "Batch size for scanning", default_value_t = 1)]
        batch_size: u64,
    },
    /// Run the daemon to continuously scan the blockchain
    Daemon {
        #[arg(short, long, help = "Password to decrypt the wallet file")]
        password: String,
        #[arg(
            short = 'u',
            long,
            default_value = "https://rpc.tari.com",
            help = "The base URL of the Tari HTTP API"
        )]
        base_url: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(long, help = "Batch size for scanning", default_value_t = 100)]
        batch_size: u64,
        #[arg(short, long, help = "Interval between scans in seconds", default_value_t = 60)]
        scan_interval_secs: u64,
        #[arg(long, help = "Port for the API server", default_value_t = 9000)]
        api_port: u16,
    },
    /// Show wallet balance
    Balance {
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(
            short,
            long,
            help = "Optional account name to show balance for. If not provided, all accounts will be used"
        )]
        account_name: Option<String>,
    },
    /// Import a wallet from a view key
    ImportViewKey {
        #[arg(short, long, alias = "view_key", help = "The view key in hex format")]
        view_private_key: String,
        #[arg(short, long, alias = "spend_key", help = "The spend public key in hex format")]
        spend_public_key: String,
        #[arg(short, long, help = "Password to encrypt the wallet file")]
        password: String,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
        #[arg(short, long, help = "The wallet birthday (block height)", default_value = "0")]
        birthday: u16,
    },
    /// Commands for tapplet management
    Tapplet {
        #[command(subcommand)]
        tapplet_subcommand: TappletCommand,
    },
}
#[derive(Subcommand)]
pub enum TappletCommand {
    /// Fetch all registries
    Fetch {
        #[arg(
            short,
            long,
            help = "Path to the cache directory",
            default_value = "data/tapplet_cache"
        )]
        cache_directory: String,
    },
    /// Search for tapplets in registries
    Search {
        #[arg(short, long, help = "Query string to search for tapplets")]
        query: String,
        #[arg(
            short,
            long,
            help = "Path to the cache directory",
            default_value = "data/tapplet_cache"
        )]
        cache_directory: String,
    },
    AddRegistry {
        #[arg(short, long, help = "Name of the tapplet registry")]
        name: String,
        #[arg(short, long, help = "URL of the tapplet registry")]
        url: String,
    },
    /// List installed tapplets
    List {
        #[arg(
            short,
            long,
            help = "Path to the cache directory",
            default_value = "data/tapplet_cache"
        )]
        cache_directory: String,
    },
    /// Install a tapplet from a file
    Install {
        #[arg(short, long, help = "The name of the registry to install from")]
        registry: Option<String>,
        #[arg(short, long, help = "Name of the tapplet to install")]
        name: Option<String>,
        #[arg(short, long, help = "Path to a local tapplet file to install")]
        path: Option<String>,
        #[arg(
            short,
            long,
            help = "Path to the cache directory",
            default_value = "data/tapplet_cache"
        )]
        cache_directory: String,
        #[arg(
            short,
            long,
            help = "Optional account name to use for the tapplet. If not provided, all accounts will be installed"
        )]
        account_name: Option<String>,
        #[arg(short, long, help = "Path to the database file", default_value = "data/wallet.db")]
        database_file: String,
    },
    /// Uninstall a tapplet by name
    Uninstall {
        #[arg(short, long, help = "Name of the tapplet to uninstall")]
        name: String,
    },
    Run {
        #[arg(short, long, help = "Name of the tapplet to run")]
        name: String,
        #[arg(short, long, help = "The method to invoke on the tapplet")]
        method: String,
        #[arg(
            short,
            long,
            help = "Arguments to pass to the tapplet method",
            default_value = "",
            alias = "arg"
        )]
        args: Vec<String>,
    },
}
