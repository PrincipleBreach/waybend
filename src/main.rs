#[tokio::main]
async fn main() {
    if let Err(error) = waybend::cli::run().await {
        eprintln!("waybend: {error:#}");
        std::process::exit(1);
    }
}
