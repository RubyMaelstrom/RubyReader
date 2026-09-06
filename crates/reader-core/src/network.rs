use crate::{content, model::*};
use anyhow::{Result, anyhow, bail};
use reqwest::{Client, StatusCode, header};
use scraper::{Html, Selector};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

#[derive(Clone)]
pub struct Network {
    client: Client,
    cache: PathBuf,
}
pub struct FeedUpdate {
    pub items: Vec<NewArticle>,
    pub etag: Option<String>,
    pub modified: Option<String>,
}
struct Document {
    bytes: Vec<u8>,
    url: Url,
    headers: header::HeaderMap,
    status: StatusCode,
}

impl Network {
    pub fn new(cache: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache)?;
        Ok(Self {
            cache,
            client: Client::builder()
                .user_agent("RubyReader/0.1 (+personal RSS reader)")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(35))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()?,
        })
    }
    async fn get(
        &self,
        url: &Url,
        etag: Option<&str>,
        modified: Option<&str>,
        max: usize,
    ) -> Result<Document> {
        anyhow::ensure!(
            content::web_url(url),
            "Only HTTP and HTTPS addresses without login credentials are supported"
        );
        let mut request = self.client.get(url.clone());
        if let Some(tag) = etag {
            request = request.header(header::IF_NONE_MATCH, tag);
        }
        if let Some(date) = modified {
            request = request.header(header::IF_MODIFIED_SINCE, date);
        }
        let mut response = request.send().await.map_err(network_error)?;
        let status = response.status();
        if status != StatusCode::NOT_MODIFIED && !status.is_success() {
            bail!("Server returned HTTP {}", status.as_u16());
        }
        anyhow::ensure!(
            response.content_length().unwrap_or(0) <= max as u64,
            "Response exceeds the download limit"
        );
        let url = response.url().clone();
        let headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= max,
                "Response exceeds the download limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(Document {
            bytes,
            url,
            headers,
            status,
        })
    }
    pub async fn discover(&self, raw: &str) -> Result<Vec<DiscoveredFeed>> {
        let normalized = if raw.contains("://") {
            raw.trim().to_owned()
        } else {
            format!("https://{}", raw.trim())
        };
        let url = Url::parse(&normalized)
            .map_err(|_| anyhow!("Enter a valid feed or website address"))?;
        let doc = self.get(&url, None, None, 8 * 1024 * 1024).await?;
        if let Ok(feed) = feed_rs::parser::parse(doc.bytes.as_slice()) {
            return Ok(vec![DiscoveredFeed {
                url: doc.url.to_string(),
                title: feed
                    .title
                    .map(|s| s.content)
                    .unwrap_or_else(|| url.host_str().unwrap_or("Feed").into()),
            }]);
        }
        let text = String::from_utf8_lossy(&doc.bytes);
        let html = Html::parse_document(&text);
        let mut feeds = Vec::new();
        for el in html.select(&Selector::parse("link[href]").unwrap()) {
            let kind = el.value().attr("type").unwrap_or("").to_ascii_lowercase();
            let alternate = el
                .value()
                .attr("rel")
                .unwrap_or("")
                .split_whitespace()
                .any(|s| s.eq_ignore_ascii_case("alternate"));
            if alternate
                && matches!(
                    kind.as_str(),
                    "application/rss+xml"
                        | "application/atom+xml"
                        | "application/feed+json"
                        | "application/json"
                )
                && let Ok(feed_url) = doc.url.join(el.value().attr("href").unwrap_or(""))
                && content::web_url(&feed_url)
                && !feeds
                    .iter()
                    .any(|f: &DiscoveredFeed| f.url == feed_url.as_str())
            {
                feeds.push(DiscoveredFeed {
                    title: el.value().attr("title").unwrap_or("Website feed").into(),
                    url: feed_url.to_string(),
                });
            }
        }
        anyhow::ensure!(
            !feeds.is_empty(),
            "No feed was advertised by this page. Try its RSS or Atom address."
        );
        feeds.truncate(30);
        Ok(feeds)
    }
    pub async fn refresh(&self, feed: &Feed) -> Result<FeedUpdate> {
        let url = Url::parse(&feed.url).map_err(|_| anyhow!("Invalid subscription address"))?;
        if url.host_str() == Some("ruby-reader.invalid") {
            return Ok(FeedUpdate {
                items: vec![],
                etag: None,
                modified: None,
            });
        }
        let doc = self
            .get(
                &url,
                feed.etag.as_deref(),
                feed.modified.as_deref(),
                8 * 1024 * 1024,
            )
            .await?;
        let etag = doc
            .headers
            .get(header::ETAG)
            .and_then(|s| s.to_str().ok())
            .map(str::to_owned);
        let modified = doc
            .headers
            .get(header::LAST_MODIFIED)
            .and_then(|s| s.to_str().ok())
            .map(str::to_owned);
        let items = if doc.status == StatusCode::NOT_MODIFIED {
            vec![]
        } else {
            parse_feed(&doc.bytes, &doc.url)?
        };
        Ok(FeedUpdate {
            items,
            etag,
            modified,
        })
    }
    pub async fn extract(&self, raw: &str) -> Result<String> {
        let url =
            Url::parse(raw).map_err(|_| anyhow!("This article has no valid original address"))?;
        let doc = self.get(&url, None, None, 16 * 1024 * 1024).await?;
        let text = String::from_utf8_lossy(&doc.bytes).into_owned();
        let base = doc.url;
        tokio::task::spawn_blocking(move || {
            let mut reader = dom_smoothie::Readability::new(text, Some(base.as_str()), None)
                .map_err(|_| anyhow!("Could not prepare this page for extraction"))?;
            let article = reader.parse().map_err(|_| {
                anyhow!("No readable article found. Open the original page to continue.")
            })?;
            let html = content::sanitize(article.content.as_ref(), &base);
            anyhow::ensure!(
                content::plain_text(&html).chars().count() >= 80,
                "The page did not provide enough readable article text"
            );
            Ok(html)
        })
        .await?
    }
    pub fn cached_image(&self, url: &Url) -> Option<Vec<u8>> {
        std::fs::read(self.cache.join(content::hash(url.as_str().as_bytes()))).ok()
    }
    pub async fn image(&self, url: &Url) -> Result<Vec<u8>> {
        if let Some(bytes) = self.cached_image(url) {
            return Ok(bytes);
        }
        let doc = self.get(url, None, None, 12 * 1024 * 1024).await?;
        let path = self.cache.join(content::hash(url.as_str().as_bytes()));
        tokio::fs::write(&path, &doc.bytes).await?;
        let cache = self.cache.clone();
        tokio::task::spawn_blocking(move || trim_cache(&cache, 512 * 1024 * 1024)).await??;
        Ok(doc.bytes)
    }
    pub fn clear_cache(&self) -> Result<()> {
        for entry in std::fs::read_dir(&self.cache)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_file()
                && name.len() == 64
                && name.chars().all(|c| c.is_ascii_hexdigit())
            {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}

pub fn parse_feed(bytes: &[u8], base: &Url) -> Result<Vec<NewArticle>> {
    let feed = feed_rs::parser::parse(bytes)
        .map_err(|_| anyhow!("The response is not a readable RSS, Atom, or JSON feed"))?;
    let mut items = Vec::new();
    for entry in feed.entries.into_iter().take(5000) {
        let title = entry
            .title
            .map(|s| content::plain_text(&s.content))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Untitled article".into());
        let url = entry
            .links
            .iter()
            .find(|l| l.rel.as_deref().is_none_or(|s| s == "alternate"))
            .and_then(|l| base.join(&l.href).ok())
            .filter(content::web_url)
            .unwrap_or_else(|| base.clone());
        let published = entry
            .published
            .or(entry.updated)
            .map(|d| d.timestamp())
            .unwrap_or_else(now);
        let author = entry
            .authors
            .into_iter()
            .map(|a| a.name)
            .collect::<Vec<_>>()
            .join(", ");
        let summary = entry.summary.map(|s| s.content).unwrap_or_default();
        let mut body = entry
            .content
            .and_then(|c| {
                c.body.map(|body| {
                    if c.content_type.as_str() == "text/plain" {
                        format!("<p>{}</p>", content::escape(&body))
                    } else {
                        body
                    }
                })
            })
            .unwrap_or_else(|| summary.clone());
        for link in entry
            .links
            .iter()
            .filter(|l| l.rel.as_deref() == Some("enclosure"))
        {
            if let Ok(target) = base.join(&link.href)
                && content::web_url(&target)
            {
                body.push_str(&format!(
                    "<p><a href=\"{}\">Open attachment / media ↗</a></p>",
                    content::escape(target.as_str())
                ));
            }
        }
        let identity = if !entry.id.is_empty() {
            entry.id
        } else {
            content::hash(format!("{}\n{}", url, title).as_bytes())
        };
        items.push(NewArticle {
            identity,
            title,
            url: url.to_string(),
            author,
            published,
            content: content::sanitize(&body, &url),
            summary: content::plain_text(&summary).chars().take(300).collect(),
        });
    }
    Ok(items)
}

fn network_error(error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        anyhow!("The request timed out. Try refreshing again.")
    } else if error.is_connect() {
        anyhow!("Could not connect. Check the network and feed address.")
    } else {
        anyhow!("The server connection failed or returned an invalid response")
    }
}

fn trim_cache(path: &Path, limit: u64) -> Result<()> {
    let mut files = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.len() != 64
                || !name.chars().all(|c| c.is_ascii_hexdigit())
                || !e.file_type().ok()?.is_file()
            {
                return None;
            }
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok(), meta.len(), e.path()))
        })
        .collect::<Vec<_>>();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, size, file) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(file).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rss_atom_and_json_keep_stable_identity() -> Result<()> {
        let base = Url::parse("https://example.test/rss")?;
        let feeds = [
            r#"<rss version="2.0"><channel><title>A</title><link>https://example.test</link><description>A</description><item><guid>stable</guid><title>Hello</title><link>https://example.test/story</link><description>&lt;p&gt;Text&lt;/p&gt;</description></item></channel></rss>"#,
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><id>feed</id><title>A</title><updated>2026-01-01T00:00:00Z</updated><entry><id>stable</id><title>Hello</title><updated>2026-01-01T00:00:00Z</updated><content type="html">&lt;p&gt;Text&lt;/p&gt;</content></entry></feed>"#,
            r#"{"version":"https://jsonfeed.org/version/1.1","title":"A","items":[{"id":"stable","title":"Hello","content_html":"<p>Text</p>"}]}"#,
        ];
        for feed in feeds {
            let a = parse_feed(feed.as_bytes(), &base)?;
            assert_eq!(a.len(), 1);
            assert_eq!(a[0].identity, "stable");
            assert!(a[0].content.contains("Text"));
        }
        Ok(())
    }
    #[tokio::test]
    async fn conditional_refresh_and_errors_never_expose_tokens() -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = server.local_addr()?;
        let task = tokio::spawn(async move {
            let (mut stream, _) = server.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]);
            assert!(request.to_lowercase().contains("if-none-match: \"v1\""));
            stream
                .write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let temp = tempfile::tempdir()?;
        let net = Network::new(temp.path().to_owned())?;
        let feed = Feed {
            id: 1,
            title: "A".into(),
            url: format!("http://{address}/rss?token=secret"),
            folder_id: None,
            unread: 0,
            error: None,
            last_refresh: None,
            etag: Some("\"v1\"".into()),
            modified: None,
            failures: 0,
            next_refresh: 0,
            initialized: true,
        };
        assert!(net.refresh(&feed).await?.items.is_empty());
        task.await?;
        let error = net.refresh(&feed).await.err().unwrap().to_string();
        assert!(!error.contains("secret"));
        Ok(())
    }

    #[tokio::test]
    async fn website_discovery_and_full_article_extraction() -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = server.local_addr()?;
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = server.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let body = if request.starts_with("GET /site ") {
                    "<html><head><link rel='alternate' type='application/rss+xml' title='Journal' href='/rss'><link rel='alternate' type='application/atom+xml' title='Notes' href='/atom'></head><body>A journal</body></html>".to_owned()
                } else {
                    let paragraph = "This botanical journal follows the spring garden carefully. We gathered detailed observations about flowers, changing weather, and the small creatures that live among the plants. These notes belong to a long article about the natural world and the people who care for it. ";
                    format!(
                        "<html><head><title>Notes from the spring garden</title></head><body><nav>Navigation</nav><article><h1>Notes from the spring garden</h1><p>{}</p><p>{}</p><img src='/flower.jpg'><script>evil()</script></article></body></html>",
                        paragraph.repeat(4),
                        paragraph.repeat(4)
                    )
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let temp = tempfile::tempdir()?;
        let net = Network::new(temp.path().to_owned())?;
        let feeds = net.discover(&format!("http://{address}/site")).await?;
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].url, format!("http://{address}/rss"));
        let article = net.extract(&format!("http://{address}/story")).await?;
        assert!(article.contains("botanical journal"));
        assert!(article.contains(&format!("http://{address}/flower.jpg")));
        assert!(!article.contains("<script"));
        task.await?;
        Ok(())
    }

    #[test]
    fn bounded_cache_only_evicts_its_own_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let owned = temp
            .path()
            .join(content::hash(b"https://example.test/picture"));
        std::fs::write(&owned, [0; 16])?;
        let other = temp.path().join("keep.txt");
        std::fs::write(&other, b"not a cache entry")?;
        trim_cache(temp.path(), 0)?;
        assert!(!owned.exists());
        assert!(other.exists());
        Ok(())
    }
}
