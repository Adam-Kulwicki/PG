# --- ETAP 1: Budowanie aplikacji ---
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# 1. Kopiujemy manifesty zależności, żeby pobrać je i zacache'ować
COPY Cargo.toml Cargo.lock ./

# 2. Tworzymy pusty projekt, żeby skompilować same zależności (szybki cache w Dockerze)
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# 3. Kopiujemy migracje SQL (wymagane w czasie kompilacji przez makro sqlx::migrate!)
COPY migrations ./migrations

# 4. Kopiujemy właściwy kod źródłowy (i ewentualny folder .sqlx jeśli użyjesz sqlx::query!)
COPY src ./src

# 5. Wymuszamy przekompilowanie naszej binarki z właściwym kodem
RUN touch src/main.rs && cargo build --release --locked

# --- ETAP 2: Lekki obraz uruchomieniowy ---
FROM debian:bookworm-slim

WORKDIR /app

# Instalujemy tylko ca-certificates (potrzebne dla reqwest do łączenia się po HTTPS z API sportowymi)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Kopiujemy skompilowaną binarkę (dla name = "backend" w Cargo.toml)
COPY --from=builder /app/target/release/backend /app/backend-app

EXPOSE 3000

CMD ["./backend-app"]