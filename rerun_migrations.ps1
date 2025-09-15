rm migrations/example_schema.sqlite
sqlx database create
sqlx migrate run