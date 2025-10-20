mkdir -p data
rm data/wallet.db*
sqlx database create
sqlx migrate run