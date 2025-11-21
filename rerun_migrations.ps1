# mkdir -p data
# rm data/wallet.db*
rm test.db
sqlx database create
sqlx migrate run