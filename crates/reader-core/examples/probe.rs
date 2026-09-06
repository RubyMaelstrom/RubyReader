//! Read-only live-feed diagnostic: cargo run -p reader-core --release --example probe -- URL …
use reader_core::{model::Feed, network::Network};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let net = Network::new(temp.path().to_owned())?;
    for address in std::env::args().skip(1) {
        let choices = net.discover(&address).await?;
        for choice in choices.into_iter().take(3) {
            let feed = Feed {
                id: 0,
                title: choice.title,
                url: choice.url,
                folder_id: None,
                unread: 0,
                error: None,
                last_refresh: None,
                etag: None,
                modified: None,
                failures: 0,
                next_refresh: 0,
                initialized: false,
            };
            let update = net.refresh(&feed).await?;
            println!(
                "{}: {} entries, {} stored article characters",
                feed.title,
                update.items.len(),
                update.items.iter().map(|a| a.content.len()).sum::<usize>()
            );
            anyhow::ensure!(!update.items.is_empty(), "The feed has no articles");
        }
    }
    Ok(())
}
