FROM rust:1.90 as builder

WORKDIR /usr/src/app
COPY . .

RUN cargo install --path .

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/tg-bot-for-maintaining /usr/local/bin/tg-bot-for-maintaining

WORKDIR /app
# Directory for sqlite db
VOLUME /app/data 

CMD ["tg-bot-for-maintaining"]


