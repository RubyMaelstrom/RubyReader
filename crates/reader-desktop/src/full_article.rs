use reader_core::model::Article;
use tokio::task::AbortHandle;

/// Full text is preferred on each new selection. A pending request belongs to
/// that selection, even if the user leaves and then returns to the same story.
pub struct FullArticle {
    pub show: bool,
    attempted: bool,
    serial: u64,
    pending: Option<(i64, u64, AbortHandle)>,
}

impl Default for FullArticle {
    fn default() -> Self {
        Self {
            show: true,
            attempted: false,
            serial: 0,
            pending: None,
        }
    }
}

impl FullArticle {
    pub fn select(&mut self) {
        self.cancel();
        self.show = true;
        self.attempted = false;
    }

    pub fn request(
        &mut self,
        article: &Article,
        manual: bool,
        fetch: impl FnOnce(i64, String, u64) -> AbortHandle,
    ) -> bool {
        if manual {
            self.show = true;
        }
        if self.pending.is_some() || (!manual && (self.attempted || article.extracted.is_some())) {
            return false;
        }
        let Ok(url) = url::Url::parse(&article.url) else {
            return false;
        };
        // The sample scrapbook must remain entirely local.
        if !reader_core::content::web_url(&url) || url.host_str() == Some("ruby-reader.invalid") {
            return false;
        }
        self.attempted = true;
        self.serial += 1;
        self.pending = Some((
            article.id,
            self.serial,
            fetch(article.id, article.url.clone(), self.serial),
        ));
        true
    }

    pub fn complete(&mut self, id: i64, serial: u64) -> bool {
        if self
            .pending
            .as_ref()
            .is_none_or(|(pending_id, pending_serial, _)| {
                (*pending_id, *pending_serial) != (id, serial)
            })
        {
            return false;
        }
        self.pending = None;
        // Keep both the user's Feed/full choice and the attempt marker. A
        // failed request may be retried explicitly, never in a reload loop.
        true
    }

    fn cancel(&mut self) {
        if let Some((_, _, task)) = self.pending.take() {
            task.abort();
        }
    }
}

impl Drop for FullArticle {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(id: i64) -> Article {
        Article {
            id,
            feed_id: 1,
            feed_title: "Test feed".into(),
            title: "Test story".into(),
            url: format!("https://example.com/story/{id}"),
            author: String::new(),
            published: 0,
            read: false,
            saved: false,
            summary: String::new(),
            content: "<p>Feed text remains readable.</p>".into(),
            extracted: None,
            scroll: 0.0,
        }
    }

    #[tokio::test]
    async fn rapid_selection_cancels_downloads_and_rejects_old_completions() {
        let mut full = FullArticle::default();
        let first = tokio::spawn(std::future::pending::<()>());
        let mut first_serial = 0;
        assert!(full.request(&article(1), false, |id, url, serial| {
            assert_eq!(id, 1);
            assert_eq!(url, "https://example.com/story/1");
            first_serial = serial;
            first.abort_handle()
        }));
        assert!(!full.request(&article(1), true, |_, _, _| panic!("duplicate download")));
        full.show = false;
        full.select();
        assert!(full.show);
        assert!(first.await.unwrap_err().is_cancelled());

        let second = tokio::spawn(std::future::pending::<()>());
        assert!(full.request(&article(2), false, |_, _, _| second.abort_handle()));
        full.select();
        assert!(second.await.unwrap_err().is_cancelled());

        // Returning to the same ID must not accept the earlier visit's result.
        let returning = tokio::spawn(std::future::pending::<()>());
        let mut current_serial = 0;
        assert!(full.request(&article(1), false, |_, _, serial| {
            current_serial = serial;
            returning.abort_handle()
        }));
        assert!(!full.complete(1, first_serial));
        assert!(!full.complete(2, current_serial));
        drop(full);
        assert!(returning.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn full_text_defaults_respect_cache_feed_choice_and_explicit_retries() {
        let mut full = FullArticle::default();
        let mut story = article(1);
        story.extracted = Some("<p>Cached full article.</p>".into());
        assert!(full.show);
        assert!(!full.request(&story, false, |_, _, _| panic!("cached article fetched")));

        full.select();
        story.extracted = None;
        let mut serial = 0;
        assert!(full.request(&story, false, |_, _, request| {
            serial = request;
            tokio::spawn(async {}).abort_handle()
        }));
        full.show = false;
        assert!(full.complete(story.id, serial));
        assert!(
            !full.show,
            "completion must respect choosing Feed while loading"
        );
        assert!(!full.request(&story, false, |_, _, _| panic!("automatic retry loop")));

        assert!(full.request(&story, true, |_, _, request| {
            assert_ne!(request, serial);
            serial = request;
            tokio::spawn(async {}).abort_handle()
        }));
        assert!(full.show, "explicit fetching selects the full version");
        assert!(full.complete(story.id, serial));
        assert!(!full.complete(story.id, serial));
        full.select();
        assert!(full.request(&story, false, |_, _, _| {
            tokio::spawn(async {}).abort_handle()
        }));

        for url in [
            "https://ruby-reader.invalid/sample",
            "file:///local",
            "app:quit",
            "invalid",
        ] {
            full.select();
            story.url = url.into();
            assert!(!full.request(&story, false, |_, _, _| panic!(
                "invalid/sample URL fetched"
            )));
        }
    }
}
