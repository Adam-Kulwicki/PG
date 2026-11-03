# --- ETAP 1: Budowanie aplikacji ---
FROM rust:1.85-slim AS builder

WORKDIR /app

# Kopiujemy manifesty zależności, żeby pobrać je i zacache'ować
COPY Cargo.toml Cargo.lock ./

# Tworzymy pusty projekt, żeby skompilować same zależności (dla szybszego cache w Dockerze)
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Kopiujemy właściwy kod źródłowy
COPY src ./src

# Kompilujemy aplikację na produkcję
RUN touch src/main.rs && cargo build --release --locked

# --- ETAP 2: Lekki obraz uruchomieniowy ---
FROM debian:bookworm-slim

WORKDIR /app

# Instalujemy niezbędne biblioteki systemowe (np. openssl, ca-certificates do HTTPS)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Kopiujemy skompilowaną binarkę z etapu budowania
COPY --from=builder /app/target/release/backend /app/backend-app

# Port, na którym nasłuchuje Axum
EXPOSE 3000

# Uruchomienie aplikacji
CMD ["./backend-app"]