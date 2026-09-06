use ammonia::{Builder, UrlRelative};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use url::Url;

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn plain_text(html: &str) -> String {
    Html::parse_fragment(html)
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Separate document + no script actor is the primary isolation boundary.
/// Sanitization also removes author CSS, active content, and non-web links.
pub fn sanitize(html: &str, base: &Url) -> String {
    let document = Html::parse_fragment(html);
    let mut media = String::new();
    for node in document.select(&Selector::parse("video, audio, iframe, embed, object").unwrap()) {
        if let Some(raw) = node
            .value()
            .attr("src")
            .or_else(|| node.value().attr("data"))
            && let Ok(url) = base.join(raw)
            && web_url(&url)
        {
            media.push_str(&format!(
                "<p><a href=\"{}\">Open {} externally ↗</a></p>",
                escape(url.as_str()),
                node.value().name()
            ));
        }
    }
    for node in document.select(&Selector::parse("video source, audio source").unwrap()) {
        if let Some(raw) = node.value().attr("src")
            && let Ok(url) = base.join(raw)
            && web_url(&url)
        {
            media.push_str(&format!(
                "<p><a href=\"{}\">Play media externally ↗</a></p>",
                escape(url.as_str())
            ));
        }
    }
    let mut cleaner = Builder::default();
    cleaner
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .url_relative(UrlRelative::RewriteWithBase(base.clone()))
        .add_tags(
            [
                "figure",
                "figcaption",
                "details",
                "summary",
                "picture",
                "source",
                "ruby",
                "rt",
                "rp",
            ]
            .iter()
            .copied(),
        )
        .add_generic_attributes(["id", "dir", "lang"].iter().copied())
        .add_tag_attributes("img", ["width", "height", "alt", "title"].iter().copied())
        .add_tag_attributes("details", ["open"].iter().copied())
        .add_tag_attributes("td", ["colspan", "rowspan"].iter().copied())
        .add_tag_attributes("th", ["colspan", "rowspan", "scope"].iter().copied());
    format!("{}{}", cleaner.clean(html), media)
}

pub fn web_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

pub fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub fn image_urls(html: &str, base: &Url) -> Vec<Url> {
    Html::parse_fragment(html)
        .select(&Selector::parse("img[src]").unwrap())
        .filter_map(|node| base.join(node.value().attr("src")?).ok())
        .filter(web_url)
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_article_retains_reading_content_but_not_actions() {
        let base = Url::parse("https://example.test/posts/story").unwrap();
        let html = sanitize(
            r#"<script>alert(1)</script><style>body{display:none}</style><h2 id="a">Hello</h2><img src="../photo.jpg" onerror="evil()"><a href="reader:delete">bad</a><a href="javascript:evil()">bad</a><table><tr><td colspan="2">OK</td></tr></table><iframe src="https://video.test/watch"></iframe>"#,
            &base,
        );
        assert!(!html.contains("<script"));
        assert!(!html.contains("onerror"));
        assert!(!html.contains("reader:"));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("<style"));
        assert!(html.contains("https://example.test/photo.jpg"));
        assert!(html.contains("colspan=\"2\""));
        assert!(html.contains("Open iframe externally"));
    }
}
