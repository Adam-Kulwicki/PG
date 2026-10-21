use axum::{routing::get, Router};

#[tokio::main]
async fn main() {
    // Definicja routingu
    let app = Router::new().route("/", get(root));

    // Bindowanie portu
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Serwer nasłuchuje na http://localhost:3000");
    
    axum::serve(listener, app).await.unwrap();
}

async fn root() -> &'static str {
    "Hello, Axum!"
}