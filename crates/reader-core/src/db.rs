use crate::{
    content::{escape, plain_text},
    model::*,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Self::initialize(conn)
    }
    pub fn memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }
    fn initialize(conn: Connection) -> Result<Self> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        anyhow::ensure!(
            version <= 1,
            "This library was created by a newer Ruby Reader version"
        );
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS folders(id INTEGER PRIMARY KEY,title TEXT NOT NULL,parent_id INTEGER REFERENCES folders(id) ON DELETE SET NULL);
        CREATE TABLE IF NOT EXISTS feeds(id INTEGER PRIMARY KEY,url TEXT NOT NULL UNIQUE,title TEXT NOT NULL,folder_id INTEGER REFERENCES folders(id) ON DELETE SET NULL,active INTEGER NOT NULL DEFAULT 1,etag TEXT,modified TEXT,last_refresh INTEGER,error TEXT,failures INTEGER NOT NULL DEFAULT 0,next_refresh INTEGER NOT NULL DEFAULT 0,initialized INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS articles(id INTEGER PRIMARY KEY,feed_id INTEGER NOT NULL REFERENCES feeds(id),identity TEXT NOT NULL,title TEXT NOT NULL,url TEXT NOT NULL,author TEXT NOT NULL,published INTEGER NOT NULL,read INTEGER NOT NULL DEFAULT 0,saved INTEGER NOT NULL DEFAULT 0,summary TEXT NOT NULL,content TEXT NOT NULL,extracted TEXT,search_text TEXT NOT NULL,scroll REAL NOT NULL DEFAULT 0,UNIQUE(feed_id,identity));
        CREATE INDEX IF NOT EXISTS articles_feed_date ON articles(feed_id,published DESC,id DESC);
        CREATE INDEX IF NOT EXISTS articles_feed_read ON articles(feed_id,read);
        CREATE INDEX IF NOT EXISTS articles_read_date ON articles(read,published DESC,id DESC);
        CREATE INDEX IF NOT EXISTS articles_saved_date ON articles(saved,published DESC,id DESC);
        CREATE VIRTUAL TABLE IF NOT EXISTS article_fts USING fts5(title,search_text,content=articles,content_rowid=id,tokenize='unicode61');
        CREATE TRIGGER IF NOT EXISTS articles_ai AFTER INSERT ON articles BEGIN INSERT INTO article_fts(rowid,title,search_text) VALUES(new.id,new.title,new.search_text); END;
        CREATE TRIGGER IF NOT EXISTS articles_ad AFTER DELETE ON articles BEGIN INSERT INTO article_fts(article_fts,rowid,title,search_text) VALUES('delete',old.id,old.title,old.search_text); END;
        CREATE TRIGGER IF NOT EXISTS articles_au AFTER UPDATE OF title,search_text ON articles BEGIN INSERT INTO article_fts(article_fts,rowid,title,search_text) VALUES('delete',old.id,old.title,old.search_text); INSERT INTO article_fts(rowid,title,search_text) VALUES(new.id,new.title,new.search_text); END;
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        PRAGMA user_version=1;")?;
        Ok(Self { conn })
    }
    pub fn settings(&self) -> Result<Settings> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key='preferences'",
                [],
                |r| r.get(0),
            )
            .ok();
        Ok(value
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES('preferences',?1)",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }
    pub fn add_feed(&self, url: &str, title: &str, folder: Option<i64>) -> Result<i64> {
        self.conn.execute("INSERT INTO feeds(url,title,folder_id) VALUES(?1,?2,?3) ON CONFLICT(url) DO UPDATE SET active=1", params![url,title,folder])?;
        Ok(self
            .conn
            .query_row("SELECT id FROM feeds WHERE url=?1", [url], |r| r.get(0))?)
    }
    pub fn edit_feed(&self, id: i64, title: &str, url: &str, folder: Option<i64>) -> Result<()> {
        self.conn.execute("UPDATE feeds SET title=?2,etag=CASE WHEN url=?3 THEN etag END,modified=CASE WHEN url=?3 THEN modified END,url=?3,folder_id=?4 WHERE id=?1", params![id,title,url,folder])?;
        Ok(())
    }
    pub fn feed_is_current(&self, id: i64, url: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM feeds WHERE id=?1 AND url=?2 AND active=1)",
            params![id, url],
            |r| r.get(0),
        )?)
    }
    pub fn add_folder(&self, title: &str, parent: Option<i64>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO folders(title,parent_id) VALUES(?1,?2)",
            params![title, parent],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn rename_folder(&self, id: i64, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET title=?2 WHERE id=?1",
            params![id, title],
        )?;
        Ok(())
    }
    pub fn remove_folder(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM folders WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn remove_feed(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM articles WHERE feed_id=?1 AND saved=0", [id])?;
        tx.execute("UPDATE feeds SET active=0 WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn folders(&self) -> Result<Vec<Folder>> {
        Ok(self
            .conn
            .prepare("SELECT id,title,parent_id FROM folders ORDER BY title COLLATE NOCASE")?
            .query_map([], |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    parent_id: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn feeds(&self) -> Result<Vec<Feed>> {
        Ok(self.conn.prepare("SELECT f.id,f.title,f.url,f.folder_id,(SELECT count(*) FROM articles a WHERE a.feed_id=f.id AND a.read=0),f.error,f.last_refresh,f.etag,f.modified,f.failures,f.next_refresh,f.initialized FROM feeds f WHERE active=1 ORDER BY f.title COLLATE NOCASE")?
            .query_map([],|r| Ok(Feed{id:r.get(0)?,title:r.get(1)?,url:r.get(2)?,folder_id:r.get(3)?,unread:r.get(4)?,error:r.get(5)?,last_refresh:r.get(6)?,etag:r.get(7)?,modified:r.get(8)?,failures:r.get(9)?,next_refresh:r.get(10)?,initialized:r.get(11)?}))?.collect::<rusqlite::Result<_>>()?)
    }
    fn filter(query: &Query) -> (String, Vec<rusqlite::types::Value>) {
        let mut args = vec![];
        let mut sql = match query.view {
            View::All => "f.active=1".to_owned(),
            View::Unread => "f.active=1 AND a.read=0".into(),
            View::Saved => "a.saved=1".into(),
            View::Feed(id) => {
                args.push(id.into());
                "f.active=1 AND a.feed_id=?".into()
            }
            View::Folder(id) => {
                args.push(id.into());
                "f.active=1 AND f.folder_id IN (WITH RECURSIVE tree(id) AS (SELECT ? UNION ALL SELECT folders.id FROM folders JOIN tree ON folders.parent_id=tree.id) SELECT id FROM tree)".into()
            }
        };
        let terms: Vec<_> = query
            .search
            .split_whitespace()
            .take(32)
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect();
        if !terms.is_empty() {
            sql.push_str(" AND a.id IN (SELECT rowid FROM article_fts WHERE article_fts MATCH ?)");
            args.push(terms.join(" AND ").into());
        }
        (sql, args)
    }
    pub fn snapshot(&self, query: &Query) -> Result<Snapshot> {
        let (filter, args) = Self::filter(query);
        let order = match query.sort {
            Sort::Newest => "a.published DESC,a.id DESC",
            Sort::Oldest => "a.published,a.id",
            Sort::Title => "a.title COLLATE NOCASE,a.id DESC",
            Sort::Feed => "f.title COLLATE NOCASE,a.published DESC,a.id DESC",
        };
        let total = self.conn.query_row(
            &format!(
                "SELECT count(*) FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE {filter}"
            ),
            rusqlite::params_from_iter(args.iter()),
            |r| r.get(0),
        )?;
        let articles = self.conn.prepare(&format!("SELECT a.id,a.feed_id,f.title,a.title,a.url,a.author,a.published,a.read,a.saved,a.summary,'',NULL,a.scroll FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE {filter} ORDER BY {order} LIMIT 150 OFFSET {}",query.offset))?
            .query_map(rusqlite::params_from_iter(args.iter()), article_row)?.collect::<rusqlite::Result<_>>()?;
        let unread = self.conn.query_row("SELECT count(*) FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.read=0 AND f.active=1",[],|r|r.get(0))?;
        let all = self.conn.query_row(
            "SELECT count(*) FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE f.active=1",
            [],
            |r| r.get(0),
        )?;
        let saved =
            self.conn
                .query_row("SELECT count(*) FROM articles WHERE saved=1", [], |r| {
                    r.get(0)
                })?;
        Ok(Snapshot {
            feeds: self.feeds()?,
            folders: self.folders()?,
            articles,
            total,
            all,
            unread,
            saved,
            settings: self.settings()?,
        })
    }
    pub fn article(&self, id: i64) -> Result<Article> {
        Ok(self.conn.query_row("SELECT a.id,a.feed_id,f.title,a.title,a.url,a.author,a.published,a.read,a.saved,a.summary,a.content,a.extracted,a.scroll FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.id=?1",[id],article_row)?)
    }
    pub fn update_articles(&mut self, feed: i64, items: &[NewArticle]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut added = 0;
        for item in items {
            let existed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM articles WHERE feed_id=?1 AND identity=?2)",
                params![feed, item.identity],
                |r| r.get(0),
            )?;
            let text = plain_text(&item.content);
            tx.execute("INSERT INTO articles(feed_id,identity,title,url,author,published,summary,content,search_text) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(feed_id,identity) DO UPDATE SET title=excluded.title,url=excluded.url,author=excluded.author,summary=excluded.summary,content=excluded.content,search_text=excluded.search_text || ' ' || coalesce(articles.extracted,'')",params![feed,item.identity,item.title,item.url,item.author,item.published,item.summary,item.content,text])?;
            if !existed {
                added += 1;
            }
        }
        tx.commit()?;
        Ok(added)
    }
    pub fn refreshed(
        &self,
        id: i64,
        etag: Option<&str>,
        modified: Option<&str>,
        minutes: u64,
    ) -> Result<()> {
        self.conn.execute("UPDATE feeds SET etag=coalesce(?2,etag),modified=coalesce(?3,modified),last_refresh=?4,next_refresh=?5,error=NULL,failures=0,initialized=1 WHERE id=?1",params![id,etag,modified,now(),now()+minutes as i64*60])?;
        Ok(())
    }
    pub fn failed(&self, id: i64, error: &str, failures: u32) -> Result<()> {
        let delay = (60_i64 * 2_i64.pow(failures.min(8))).min(6 * 3600);
        self.conn.execute(
            "UPDATE feeds SET error=?2,failures=failures+1,next_refresh=?3 WHERE id=?1",
            params![id, error, now() + delay],
        )?;
        Ok(())
    }
    pub fn read(&self, id: i64, read: bool) -> Result<()> {
        self.conn
            .execute("UPDATE articles SET read=?2 WHERE id=?1", params![id, read])?;
        Ok(())
    }
    pub fn saved(&self, id: i64, saved: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE articles SET saved=?2 WHERE id=?1",
            params![id, saved],
        )?;
        Ok(())
    }
    pub fn scroll(&self, id: i64, scroll: f32) -> Result<()> {
        self.conn.execute(
            "UPDATE articles SET scroll=?2 WHERE id=?1",
            params![id, scroll],
        )?;
        Ok(())
    }
    pub fn mark_view_read(&self, query: &Query) -> Result<()> {
        let (filter, args) = Self::filter(query);
        self.conn.execute(&format!("UPDATE articles SET read=1 WHERE id IN (SELECT a.id FROM articles a JOIN feeds f ON a.feed_id=f.id WHERE {filter})"),rusqlite::params_from_iter(args.iter()))?;
        Ok(())
    }
    pub fn extracted(&self, id: i64, html: &str) -> Result<()> {
        let original = self.article(id)?;
        let search = format!("{} {}", plain_text(&original.content), plain_text(html));
        self.conn.execute(
            "UPDATE articles SET extracted=?2,search_text=?3 WHERE id=?1",
            params![id, html, search],
        )?;
        Ok(())
    }
    pub fn export_opml(&self) -> Result<String> {
        let feeds = self.feeds()?;
        let folders = self.folders()?;
        fn outlines(parent: Option<i64>, folders: &[Folder], feeds: &[Feed]) -> String {
            let mut out = String::new();
            for folder in folders.iter().filter(|f| f.parent_id == parent) {
                out.push_str(&format!(
                    "<outline text=\"{}\">{}</outline>\n",
                    escape(&folder.title),
                    outlines(Some(folder.id), folders, feeds)
                ));
            }
            for feed in feeds.iter().filter(|f| f.folder_id == parent) {
                out.push_str(&format!(
                    "<outline type=\"rss\" text=\"{}\" title=\"{}\" xmlUrl=\"{}\"/>\n",
                    escape(&feed.title),
                    escape(&feed.title),
                    escape(&feed.url)
                ));
            }
            out
        }
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><opml version=\"2.0\"><head><title>Ruby Reader subscriptions</title></head><body>{}</body></opml>",
            outlines(None, &folders, &feeds)
        ))
    }
    pub fn import_opml(&mut self, xml: &str) -> Result<(usize, usize)> {
        anyhow::ensure!(xml.len() <= 8 * 1024 * 1024, "OPML file exceeds 8 MiB");
        let doc = roxmltree::Document::parse(xml).context("Invalid OPML document")?;
        anyhow::ensure!(
            !doc.descendants()
                .any(|n| n.ancestors().take(66).count() > 65),
            "OPML folder nesting exceeds 64 levels"
        );
        anyhow::ensure!(
            doc.root_element().tag_name().name() == "opml",
            "Expected an OPML document"
        );
        let mut mapping = std::collections::HashMap::new();
        let mut added = 0;
        let mut skipped = 0;
        // One transaction ensures malformed imports never leave half a library.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            for node in doc.descendants().filter(|n| n.has_tag_name("outline")) {
                let parent = node
                    .ancestors()
                    .skip(1)
                    .find_map(|p| mapping.get(&p.id()).copied());
                let title = node
                    .attribute("text")
                    .or_else(|| node.attribute("title"))
                    .unwrap_or("Untitled");
                if let Some(raw) = node.attribute("xmlUrl") {
                    match url::Url::parse(raw) {
                        Ok(url) if crate::content::web_url(&url) => {
                            self.add_feed(url.as_str(), title, parent)?;
                            added += 1;
                        }
                        _ => skipped += 1,
                    }
                } else {
                    let existing = self
                        .folders()?
                        .into_iter()
                        .find(|f| f.title == title && f.parent_id == parent)
                        .map(|f| f.id);
                    let id = match existing {
                        Some(id) => id,
                        None => self.add_folder(title, parent)?,
                    };
                    mapping.insert(node.id(), id);
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(e) => {
                self.conn.execute_batch("ROLLBACK")?;
                return Err(e);
            }
        }
        Ok((added, skipped))
    }
    pub fn sample(&mut self) -> Result<()> {
        let folder = match self
            .folders()?
            .into_iter()
            .find(|f| f.title == "The sample scrapbook")
        {
            Some(folder) => folder.id,
            None => self.add_folder("The sample scrapbook", None)?,
        };
        let id = self.add_feed(
            "https://ruby-reader.invalid/sample",
            "Ruby Reader · sample collection",
            Some(folder),
        )?;
        let items = [
            (
                "A little corner of the internet, entirely yours",
                "Welcome to Ruby Reader",
                "<p>Welcome to your new reading room. A place for interesting things, tiny discoveries, and the joy of following your own curiosity.</p><h2>Collect what you love</h2><p>Add a feed or website address with <strong>Add feed</strong>, or bring your subscriptions along with <strong>Import OPML</strong>. Your library lives on this computer.</p><blockquote>The web is still full of wonderful little places. Make yourself a collection.</blockquote><h2>A reader with room to breathe</h2><p>Drag the dividers to make your space. Save a story with the heart, search your collection, or press <kbd>F</kbd> for an uninterrupted reading view.</p><p>This is a local sample article. The sample subscription never contacts a server.</p>",
            ),
            (
                "Field notes: a small garden of big ideas",
                "The personal web is alive",
                "<p>A collection doesn't have to be comprehensive to be meaningful. Sometimes five thoughtful voices are better company than an endless timeline.</p><h2>Three things to look for</h2><ul><li>A perspective that surprises you</li><li>Someone making things with care</li><li>A little delight you weren't expecting</li></ul><p>Use folders to arrange your reading by curiosity, not obligation.</p>",
            ),
            (
                "The beautiful details: a typography specimen",
                "Reading, rendered with care",
                "<h2>Words with structure</h2><p>Regular text, <em>emphasis</em>, <strong>strong emphasis</strong>, H<sub>2</sub>O, and x<sup>2</sup>. A <a href=\"#note\">footnote</a> belongs to the story, too.</p><figure><figcaption>A figure caption gives an image context.</figcaption></figure><pre><code>fn main() {\n    println!(\"Hello, lovely world!\");\n}</code></pre><table><thead><tr><th>Element</th><th>Purpose</th></tr></thead><tbody><tr><td>Headings</td><td>A clear path through a story</td></tr><tr><td>Tables</td><td>Details that belong together</td></tr></tbody></table><blockquote>Elegance is making room for the thing that matters.</blockquote><p dir=\"rtl\" lang=\"ar\">مرحبا بالعالم</p><p lang=\"ja\">小さな発見を大切に。</p><p id=\"note\"><sup>1</sup> Footnotes and in-article links work here.</p>",
            ),
        ];
        let articles = items
            .iter()
            .enumerate()
            .map(|(i, (title, summary, body))| NewArticle {
                identity: format!("sample-{i}"),
                title: (*title).into(),
                summary: (*summary).into(),
                url: format!("https://ruby-reader.invalid/sample/{i}"),
                author: "Ruby Reader".into(),
                published: now() - i as i64 * 3600,
                content: (*body).into(),
            })
            .collect::<Vec<_>>();
        self.update_articles(id, &articles)?;
        self.refreshed(id, None, None, 525600)?;
        Ok(())
    }
}

fn article_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Article> {
    Ok(Article {
        id: r.get(0)?,
        feed_id: r.get(1)?,
        feed_title: r.get(2)?,
        title: r.get(3)?,
        url: r.get(4)?,
        author: r.get(5)?,
        published: r.get(6)?,
        read: r.get(7)?,
        saved: r.get(8)?,
        summary: r.get(9)?,
        content: r.get(10)?,
        extracted: r.get(11)?,
        scroll: r.get(12)?,
    })
}

pub fn data_directory() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("ruby-reader")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item() -> NewArticle {
        NewArticle {
            identity: "stable-id".into(),
            title: "Orchid news".into(),
            url: "https://example.test/1".into(),
            author: "A".into(),
            published: 1,
            content: "<p>Violet flowers</p>".into(),
            summary: "Flowers".into(),
        }
    }
    #[test]
    fn refresh_preserves_state_and_search_tracks_updated_content() -> Result<()> {
        let mut db = Store::memory()?;
        let f = db.add_feed("https://example.test/feed", "Test", None)?;
        assert_eq!(db.update_articles(f, &[item()])?, 1);
        let a = db.snapshot(&Query::default())?.articles[0].id;
        db.read(a, true)?;
        db.saved(a, true)?;
        db.extracted(a, "<p>Emerald collection</p>")?;
        let mut next = item();
        next.title = "Updated orchid".into();
        assert_eq!(db.update_articles(f, &[next])?, 0);
        assert!(db.article(a)?.read && db.article(a)?.saved);
        assert_eq!(
            db.snapshot(&Query {
                search: "Emerald".into(),
                ..Default::default()
            })?
            .total,
            1
        );
        db.remove_feed(f)?;
        assert_eq!(db.snapshot(&Query::default())?.total, 0);
        assert_eq!(
            db.snapshot(&Query {
                view: View::Saved,
                ..Default::default()
            })?
            .total,
            1
        );
        Ok(())
    }
    #[test]
    fn nested_opml_roundtrip_and_restart() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("reader.sqlite");
        {
            let mut db = Store::open(&path)?;
            db.import_opml(r#"<opml><body><outline text="Art &amp; craft"><outline text="Blogs"><outline text="One" xmlUrl="https://example.test/rss?token=secret&amp;x=1"/></outline></outline></body></opml>"#)?;
        }
        let db = Store::open(&path)?;
        let xml = db.export_opml()?;
        let mut second = Store::memory()?;
        second.import_opml(&xml)?;
        assert_eq!(second.feeds()?.len(), 1);
        assert_eq!(second.folders()?.len(), 2);
        assert!(second.feeds()?[0].url.contains("token=secret&x=1"));
        Ok(())
    }

    #[test]
    fn stale_refresh_is_identified_after_edit_or_removal() -> Result<()> {
        let mut db = Store::memory()?;
        let old = "https://example.test/old";
        let new = "https://example.test/new";
        let id = db.add_feed(old, "Example", None)?;
        assert!(db.feed_is_current(id, old)?);
        db.edit_feed(id, "Example", new, None)?;
        assert!(!db.feed_is_current(id, old)?);
        assert!(db.feed_is_current(id, new)?);
        db.remove_feed(id)?;
        assert!(!db.feed_is_current(id, new)?);
        Ok(())
    }
}
