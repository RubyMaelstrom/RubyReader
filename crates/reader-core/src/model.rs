use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub refresh_minutes: u64,
    pub notifications: bool,
    pub reduced_motion: bool,
    pub reading_theme: String,
    pub font_size: f32,
    pub line_height: f32,
    pub ui_scale: f32,
    pub sidebar_width: f32,
    pub headline_height: f32,
    pub window_width: u32,
    pub window_height: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_minutes: 30,
            notifications: true,
            reduced_motion: false,
            reading_theme: "light".into(),
            font_size: 19.0,
            line_height: 1.65,
            ui_scale: 1.0,
            sidebar_width: 244.0,
            headline_height: 220.0,
            window_width: 1360,
            window_height: 920,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub id: i64,
    pub title: String,
    pub parent_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Feed {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub folder_id: Option<i64>,
    pub unread: i64,
    pub error: Option<String>,
    pub last_refresh: Option<i64>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub failures: u32,
    pub next_refresh: i64,
    pub initialized: bool,
}

#[derive(Clone, Debug)]
pub struct Article {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub title: String,
    pub url: String,
    pub author: String,
    pub published: i64,
    pub read: bool,
    pub saved: bool,
    pub summary: String,
    pub content: String,
    pub extracted: Option<String>,
    pub scroll: f32,
}

#[derive(Clone, Debug)]
pub struct NewArticle {
    pub identity: String,
    pub title: String,
    pub url: String,
    pub author: String,
    pub published: i64,
    pub content: String,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum View {
    #[default]
    All,
    Unread,
    Saved,
    Feed(i64),
    Folder(i64),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Title,
    Feed,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub view: View,
    pub search: String,
    pub sort: Sort,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub feeds: Vec<Feed>,
    pub folders: Vec<Folder>,
    pub articles: Vec<Article>,
    pub total: i64,
    pub all: i64,
    pub unread: i64,
    pub saved: i64,
    pub settings: Settings,
}

#[derive(Clone, Debug)]
pub struct DiscoveredFeed {
    pub url: String,
    pub title: String,
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
