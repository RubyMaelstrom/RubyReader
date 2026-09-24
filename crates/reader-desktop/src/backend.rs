use reader_core::{db::Store, model::*, network::Network};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use winit::event_loop::EventLoopProxy;

type Mutation = Box<dyn FnOnce(&mut Store) -> anyhow::Result<String> + Send>;

#[derive(Debug)]
pub enum Event {
    Snapshot(u64, Result<Snapshot, String>),
    Article(i64, u64, Result<Article, String>),
    Extracted(i64, u64, Result<String, String>),
    Changed(Result<String, String>),
    Subscribed(Result<i64, String>),
    Discovered(Result<Vec<DiscoveredFeed>, String>),
    RefreshDone { new: usize, failed: usize },
    DocumentReady(usize),
    Open,
    Quit,
    Refresh,
    TrayStatus(bool),
    Access(accesskit_winit::Event),
}
impl From<accesskit_winit::Event> for Event {
    fn from(e: accesskit_winit::Event) -> Self {
        Self::Access(e)
    }
}

#[derive(Clone)]
pub struct Backend {
    pub data_dir: PathBuf,
    changes: std::sync::mpsc::Sender<Mutation>,
    pub rt: Arc<tokio::runtime::Runtime>,
    pub store: Arc<Mutex<Store>>,
    pub net: Network,
    pub proxy: EventLoopProxy<Event>,
    pub refreshing: Arc<AtomicBool>,
    pending_refresh: Arc<AtomicBool>,
    image_jobs: Arc<tokio::sync::Semaphore>,
}
impl Backend {
    pub fn new(path: PathBuf, proxy: EventLoopProxy<Event>) -> anyhow::Result<Self> {
        let store = Arc::new(Mutex::new(Store::open(&path.join("library.sqlite"))?));
        let (changes, incoming) = std::sync::mpsc::channel::<Mutation>();
        let mutation_store = store.clone();
        let mutation_proxy = proxy.clone();
        std::thread::Builder::new()
            .name("ruby-reader-storage".into())
            .spawn(move || {
                for job in incoming {
                    let result =
                        job(&mut mutation_store.lock().unwrap()).map_err(|e| e.to_string());
                    let _ = mutation_proxy.send_event(Event::Changed(result));
                }
            })?;
        Ok(Self {
            rt: Arc::new(tokio::runtime::Runtime::new()?),
            store,
            changes,
            net: Network::new(path.join("images"))?,
            data_dir: path,
            proxy,
            refreshing: Arc::new(AtomicBool::new(false)),
            pending_refresh: Arc::new(AtomicBool::new(false)),
            image_jobs: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }
    pub fn snapshot(&self, serial: u64, query: Query) {
        let this = self.clone();
        self.rt.spawn_blocking(move || {
            let result = this
                .store
                .lock()
                .unwrap()
                .snapshot(&query)
                .map_err(|e| e.to_string());
            let _ = this.proxy.send_event(Event::Snapshot(serial, result));
        });
    }
    pub fn article(&self, id: i64, serial: u64) {
        let this = self.clone();
        self.rt.spawn_blocking(move || {
            let result = this
                .store
                .lock()
                .unwrap()
                .article(id)
                .map_err(|e| e.to_string());
            let _ = this.proxy.send_event(Event::Article(id, serial, result));
        });
    }
    pub fn change(&self, job: impl FnOnce(&mut Store) -> anyhow::Result<String> + Send + 'static) {
        // Preserve user intent when rapid toggles queue several disk writes.
        // A mutex around independent pool jobs does not guarantee FIFO order.
        let _ = self.changes.send(Box::new(job));
    }
    pub fn flush(&self) {
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        self.change(move |_| {
            let _ = sent.send(());
            Ok(String::new())
        });
        let _ = received.recv();
    }
    pub fn discover(&self, address: String) {
        let this = self.clone();
        self.rt.spawn(async move {
            let result = this.net.discover(&address).await.map_err(|e| e.to_string());
            let _ = this.proxy.send_event(Event::Discovered(result));
        });
    }
    pub fn subscribe(&self, feed: DiscoveredFeed, folder: Option<i64>) {
        let this = self.clone();
        self.rt.spawn_blocking(move || {
            let result = this
                .store
                .lock()
                .unwrap()
                .add_feed(&feed.url, &feed.title, folder)
                .map_err(|e| e.to_string());
            let _ = this.proxy.send_event(Event::Subscribed(result));
        });
    }
    pub fn extract(&self, id: i64, address: String, serial: u64) -> tokio::task::AbortHandle {
        let this = self.clone();
        self.rt
            .spawn(async move {
                match this.net.extract(&address).await {
                    Ok(html) => {
                        let proxy = this.proxy.clone();
                        // Cache through the same FIFO worker as read/save/scroll
                        // writes. Deliver only the new text, not an old snapshot
                        // of article state that could undo intervening actions.
                        this.change(move |db| {
                            let result = db
                                .extracted(id, &html)
                                .map(|()| html)
                                .map_err(|e| e.to_string());
                            let _ = proxy.send_event(Event::Extracted(id, serial, result));
                            Ok(String::new())
                        });
                    }
                    Err(e) => {
                        let _ =
                            this.proxy
                                .send_event(Event::Extracted(id, serial, Err(e.to_string())));
                    }
                }
            })
            .abort_handle()
    }
    pub fn images(&self, sink: crate::documents::ImageSink, sources: Vec<String>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let mut cancelled = sink.cancellation();
            if !sink.current() {
                return;
            }
            let load = async {
                let mut jobs = tokio::task::JoinSet::new();
                for source in sources {
                    if !sink.current() {
                        break;
                    }
                    let Ok(url) = url::Url::parse(&source) else {
                        sink.failed(source);
                        continue;
                    };
                    if !reader_core::content::web_url(&url) {
                        sink.failed(source);
                        continue;
                    }
                    while jobs.len() >= 4 {
                        jobs.join_next().await;
                    }
                    let Ok(permit) = this.image_jobs.clone().acquire_owned().await else {
                        break;
                    };
                    let net = this.net.clone();
                    let sink = sink.clone();
                    jobs.spawn(async move {
                        if !sink.current() {
                            return;
                        }
                        let Ok(bytes) = net.image(&url).await else {
                            sink.failed(source);
                            return;
                        };
                        if !sink.current() {
                            return;
                        }
                        let decoded = tokio::task::spawn_blocking(move || {
                            // Blocking decoders cannot be aborted. Keep their
                            // concurrency slot even if navigation drops the
                            // async task waiting for this result.
                            (trust::img::decode_graphical(&bytes), permit)
                        })
                        .await;
                        if let Ok((Ok(image), _permit)) = decoded
                            && sink.current()
                        {
                            sink.supply(source, image).await;
                        } else {
                            sink.failed(source);
                        }
                    });
                }
                while jobs.join_next().await.is_some() {}
            };
            tokio::select! {
                _ = cancelled.changed() => {},
                _ = load => {},
            }
        });
    }
    pub fn refresh(&self, only: Option<i64>, due_only: bool) {
        if self.refreshing.swap(true, Ordering::SeqCst) {
            self.pending_refresh.store(true, Ordering::SeqCst);
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move {
            let feeds = this.store.lock().unwrap().feeds().unwrap_or_default();
            let minutes = this
                .store
                .lock()
                .unwrap()
                .settings()
                .unwrap_or_default()
                .refresh_minutes;
            let semaphore = Arc::new(tokio::sync::Semaphore::new(6));
            let mut jobs = tokio::task::JoinSet::new();
            for feed in feeds.into_iter().filter(|f| {
                only.is_none_or(|id| f.id == id) && (!due_only || f.next_refresh <= now())
            }) {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let net = this.net.clone();
                let store = this.store.clone();
                jobs.spawn(async move {
                    let _permit = permit;
                    let result = net.refresh(&feed).await;
                    let mut db = store.lock().unwrap();
                    // An in-flight response must not repopulate a removed
                    // subscription or write data from an old feed address.
                    if !db.feed_is_current(feed.id, &feed.url).unwrap_or(false) {
                        return (0, 0);
                    }
                    match result {
                        Ok(update) => match db.update_articles(feed.id, &update.items) {
                            Ok(count) => {
                                let _ = db.refreshed(
                                    feed.id,
                                    update.etag.as_deref(),
                                    update.modified.as_deref(),
                                    minutes,
                                );
                                (if feed.initialized { count } else { 0 }, 0)
                            }
                            Err(_) => {
                                let _ = db.failed(
                                    feed.id,
                                    "Could not save downloaded articles",
                                    feed.failures,
                                );
                                (0, 1)
                            }
                        },
                        Err(e) => {
                            let _ = db.failed(feed.id, &e.to_string(), feed.failures);
                            (0, 1)
                        }
                    }
                });
            }
            let (mut new, mut failed) = (0, 0);
            while let Some(result) = jobs.join_next().await {
                match result {
                    Ok((n, f)) => {
                        new += n;
                        failed += f
                    }
                    Err(_) => failed += 1,
                }
            }
            this.refreshing.store(false, Ordering::SeqCst);
            let _ = this.proxy.send_event(Event::RefreshDone { new, failed });
            if this.pending_refresh.swap(false, Ordering::SeqCst) {
                this.refresh(None, true);
            }
        });
    }
}
