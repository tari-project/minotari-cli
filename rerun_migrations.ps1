rm data/wallet.db*
sqlx database create
sqlx migrate run