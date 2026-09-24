//! Caller-owned, script-free document workers. The native loop never waits for
//! parsing, layout, image decoding or find geometry. Replace/resize/hover inputs
//! coalesce; generations reject stale documents and image completions.
use crate::{backend::Event, ui};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use trust::{
    core::{CssPoint, CssSize, PhysicalSize, ScaleFactor, ViewportMetrics},
    embed::{EmbeddedDocument, EmbeddedSnapshot},
    render::{CssRect, ImageResource, ImageStore},
};
use url::Url;
use winit::event_loop::EventLoopProxy;

#[derive(Clone, Debug, Default)]
pub struct FindHighlights {
    pub query: String,
    pub matches: Vec<Vec<CssRect>>,
}

#[derive(Debug)]
pub struct Ready {
    pub generation: u64,
    pub revision: u64,
    pub result: Result<EmbeddedSnapshot, String>,
    pub find: FindHighlights,
}

struct Build {
    html: Arc<str>,
    base: Url,
    size: CssSize,
    resources: ImageStore,
}

struct ImageDelivery {
    source: String,
    image: ImageResource,
    _permit: OwnedSemaphorePermit,
}

#[derive(Default)]
struct Pending {
    build: Option<Build>,
    resize: Option<CssSize>,
    hover: Option<Option<usize>>,
    focus: Option<Option<usize>>,
    toggles: HashSet<usize>,
    scrolls: HashMap<usize, CssPoint>,
    images: Vec<ImageDelivery>,
    find: Option<String>,
    semantics: bool,
    assets: bool,
    wake: bool,
}

struct Shared {
    pending: Mutex<Pending>,
    wake: Condvar,
    ready: Mutex<Option<Ready>>,
    generation: AtomicU64,
    revision: AtomicU64,
    completed: AtomicU64,
    closed: AtomicBool,
    semantic_client: AtomicBool,
    cancelled: watch::Sender<u64>,
    image_slots: Arc<Semaphore>,
    image_requests: Mutex<HashMap<String, ImageStatus>>,
}

enum ImageStatus {
    Pending,
    Complete,
    Failed(Instant),
}

pub struct DocumentWorker {
    shared: Arc<Shared>,
    hover: Cell<Option<usize>>,
    focus: Cell<Option<usize>>,
}

#[derive(Clone)]
pub struct ImageSink {
    shared: Arc<Shared>,
    generation: u64,
}

impl ImageSink {
    pub fn reserve(&self, source: &str, visible: bool) -> bool {
        let mut requests = self.shared.image_requests.lock().unwrap();
        if !self.current() {
            return false;
        }
        let retry = match requests.get(source) {
            None => true,
            Some(ImageStatus::Complete) => visible,
            Some(ImageStatus::Failed(at)) => visible && at.elapsed() >= Duration::from_secs(60),
            Some(ImageStatus::Pending) => false,
        };
        if retry {
            requests.insert(source.to_owned(), ImageStatus::Pending);
        }
        retry
    }
    pub fn failed(&self, source: String) {
        let mut requests = self.shared.image_requests.lock().unwrap();
        if self.current() {
            requests.insert(source, ImageStatus::Failed(Instant::now()));
        }
    }
    pub fn current(&self) -> bool {
        !self.shared.closed.load(Ordering::Acquire)
            && self.shared.generation.load(Ordering::Acquire) == self.generation
    }
    pub fn cancellation(&self) -> watch::Receiver<u64> {
        self.shared.cancelled.subscribe()
    }
    pub async fn supply(&self, source: String, image: ImageResource) {
        let Ok(permit) = self.shared.image_slots.clone().acquire_owned().await else {
            return;
        };
        let mut pending = self.shared.pending.lock().unwrap();
        if !self.current() {
            return;
        }
        pending.images.push(ImageDelivery {
            source,
            image,
            _permit: permit,
        });
        pending.wake = true;
        self.shared.revision.fetch_add(1, Ordering::AcqRel);
        self.shared.wake.notify_one();
    }
}

impl DocumentWorker {
    pub fn new(index: usize, proxy: EventLoopProxy<Event>) -> std::io::Result<Self> {
        Self::start(move || {
            let _ = proxy.send_event(Event::DocumentReady(index));
        })
    }
    fn start(notify: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (cancelled, _) = watch::channel(0);
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
            wake: Condvar::new(),
            ready: Mutex::new(None),
            generation: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            semantic_client: AtomicBool::new(false),
            cancelled,
            image_slots: Arc::new(Semaphore::new(8)),
            image_requests: Mutex::new(HashMap::new()),
        });
        let owner = shared.clone();
        std::thread::Builder::new()
            .name("ruby-reader-document".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || run(owner, notify))?;
        Ok(Self {
            shared,
            hover: Cell::new(None),
            focus: Cell::new(None),
        })
    }
    fn update(&self, change: impl FnOnce(&mut Pending)) {
        let mut pending = self.shared.pending.lock().unwrap();
        change(&mut pending);
        pending.wake = true;
        self.shared.revision.fetch_add(1, Ordering::AcqRel);
        self.shared.wake.notify_one();
    }
    pub fn replace(&self, html: Arc<str>, base: Url, size: CssSize, resources: ImageStore) -> u64 {
        let mut pending = self.shared.pending.lock().unwrap();
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *pending = Pending {
            build: Some(Build {
                html,
                base,
                size,
                resources,
            }),
            wake: true,
            ..Default::default()
        };
        self.shared.revision.fetch_add(1, Ordering::AcqRel);
        self.shared.cancelled.send_replace(generation);
        self.shared.image_requests.lock().unwrap().clear();
        self.hover.set(None);
        self.focus.set(None);
        self.shared.wake.notify_one();
        generation
    }
    pub fn clear(&self) {
        let mut pending = self.shared.pending.lock().unwrap();
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.revision.fetch_add(1, Ordering::AcqRel);
        self.shared
            .cancelled
            .send_replace(self.shared.generation.load(Ordering::Acquire));
        *pending = Pending {
            wake: true,
            ..Default::default()
        };
        self.shared.wake.notify_one();
    }
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }
    pub fn idle(&self) -> bool {
        self.shared.completed.load(Ordering::Acquire)
            >= self.shared.revision.load(Ordering::Acquire)
            && self.shared.ready.lock().unwrap().is_none()
    }
    pub fn resize(&self, size: CssSize) {
        self.update(|p| p.resize = Some(size));
    }
    pub fn hover(&self, node: Option<usize>) {
        if self.hover.replace(node) != node {
            self.update(|p| p.hover = Some(node));
        }
    }
    pub fn focus(&self, node: Option<usize>) {
        if self.focus.replace(node) != node {
            self.update(|p| p.focus = Some(node));
        }
    }
    pub fn toggle(&self, node: usize) {
        self.update(|p| {
            if !p.toggles.insert(node) {
                p.toggles.remove(&node);
            }
        });
    }
    pub fn scroll(&self, node: usize, point: CssPoint) {
        self.update(|p| {
            p.scrolls.insert(node, point);
        });
    }
    pub fn find(&self, query: String) {
        self.update(|p| p.find = Some(query));
    }
    pub fn request_semantics(&self) {
        if !self.shared.semantic_client.swap(true, Ordering::AcqRel) {
            self.update(|p| p.semantics = true);
        }
    }
    pub fn restore_assets(&self) {
        self.update(|p| p.assets = true);
    }
    pub fn images(&self) -> ImageSink {
        ImageSink {
            shared: self.shared.clone(),
            generation: self.generation(),
        }
    }
    pub fn take_ready(&self) -> Option<Ready> {
        self.shared.ready.lock().unwrap().take()
    }
}

impl Drop for DocumentWorker {
    fn drop(&mut self) {
        // Pair the closed predicate with the condition-variable mutex so a
        // worker cannot enter its idle wait just after the shutdown wake.
        let _pending = self.shared.pending.lock().unwrap();
        self.shared.closed.store(true, Ordering::Release);
        self.shared.cancelled.send_replace(u64::MAX);
        self.shared.image_slots.close();
        self.shared.wake.notify_one();
        // Do not join a possibly long layout from the native loop. The owner
        // drops the DOM and recursive layout caches on its own large stack.
    }
}

fn run(shared: Arc<Shared>, notify: impl Fn()) {
    let mut document: Option<EmbeddedDocument> = None;
    let mut generation = 0;
    let mut query = String::new();
    loop {
        let mut pending = shared.pending.lock().unwrap();
        while !pending.wake && !shared.closed.load(Ordering::Acquire) {
            pending = shared.wake.wait(pending).unwrap();
        }
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        // Let concurrent completions form one bounded resource transaction.
        let collect_until = Instant::now() + Duration::from_millis(12);
        while !pending.images.is_empty()
            && pending.images.len() < 8
            && pending.build.is_none()
            && pending.resize.is_none()
            && pending.find.is_none()
            && !shared.closed.load(Ordering::Acquire)
        {
            let remaining = collect_until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            pending = shared.wake.wait_timeout(pending, remaining).unwrap().0;
        }
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        let next_generation = shared.generation.load(Ordering::Acquire);
        let revision = shared.revision.load(Ordering::Acquire);
        let work = std::mem::take(&mut *pending);
        drop(pending);
        if next_generation != generation {
            document = None;
            query.clear();
            generation = next_generation;
        }
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut changed = false;
            if let Some(build) = work.build {
                document = Some(
                    ui::prepare_document(&build.html, build.base, build.size, build.resources)
                        .map_err(str::to_owned)?,
                );
                changed = true;
            }
            let Some(doc) = &mut document else {
                return Ok(None);
            };
            if let Some(size) = work.resize {
                doc.resize(size);
                changed = true;
            }
            if let Some(node) = work.hover {
                changed |= doc.hover(node);
            }
            if let Some(node) = work.focus {
                ui::focus_feed_actions(doc, node);
                changed = true;
            }
            if work.assets {
                changed |= ui::restore_assets(doc);
            }
            for node in work.toggles {
                if doc.dom.is_valid(node) {
                    changed |= ui::toggle_disclosure(doc, node);
                }
            }
            for (node, point) in work.scrolls {
                if let Some(c) = doc
                    .layout
                    .paint
                    .scroll_containers
                    .iter_mut()
                    .find(|c| c.node == node)
                {
                    c.offset = point;
                    changed = true;
                }
            }
            if !work.images.is_empty() {
                let delivered = work
                    .images
                    .iter()
                    .map(|d| d.source.clone())
                    .collect::<Vec<_>>();
                let images = work
                    .images
                    .into_iter()
                    .map(|delivery| (delivery.source, delivery.image))
                    .collect::<Vec<_>>();
                changed |= doc.supply_images(images) != 0;
                let mut requests = shared.image_requests.lock().unwrap();
                if shared.generation.load(Ordering::Acquire) == generation {
                    for source in delivered {
                        requests.insert(source, ImageStatus::Complete);
                    }
                }
            }
            if let Some(find) = work.find {
                query = find;
                changed = true;
            }
            if !changed && !work.semantics {
                return Ok(None);
            }
            let mut find = FindHighlights {
                query: query.clone(),
                matches: vec![],
            };
            if !query.is_empty() {
                let size = doc.viewport();
                let metrics = ViewportMetrics::from_physical(
                    PhysicalSize::new(size.width.ceil() as u32, size.height.ceil() as u32),
                    ScaleFactor::default(),
                );
                let scene = doc.scene(
                    metrics,
                    CssRect::new(0.0, 0.0, size.width, size.height),
                    CssPoint::default(),
                    0.0,
                );
                for selection in scene.find_text(&query) {
                    find.matches.push(scene.selection_rects(selection));
                }
            }
            Ok(Some((
                doc.snapshot(shared.semantic_client.load(Ordering::Acquire)),
                find,
            )))
        }));
        let (result, find) = match prepared {
            Ok(Ok(Some((snapshot, find)))) => (Ok(snapshot), find),
            Ok(Ok(None)) => {
                shared.completed.store(revision, Ordering::Release);
                continue;
            }
            Ok(Err(error)) => (Err(error), FindHighlights::default()),
            Err(_) => {
                document = None;
                (
                    Err("Could not prepare this document. You can open the original page.".into()),
                    FindHighlights::default(),
                )
            }
        };
        if generation != shared.generation.load(Ordering::Acquire)
            || shared.closed.load(Ordering::Acquire)
        {
            shared.completed.store(revision, Ordering::Release);
            continue;
        }
        let mut ready = shared.ready.lock().unwrap();
        let wake = ready.is_none();
        *ready = Some(Ready {
            generation,
            revision,
            result,
            find,
        });
        drop(ready);
        shared.completed.store(revision, Ordering::Release);
        if wake {
            notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    fn worker() -> (DocumentWorker, Receiver<()>) {
        let (sent, received) = mpsc::channel();
        (
            DocumentWorker::start(move || {
                let _ = sent.send(());
            })
            .unwrap(),
            received,
        )
    }
    fn replace(worker: &DocumentWorker, html: &str, store: ImageStore) -> u64 {
        worker.replace(
            html.into(),
            Url::parse("https://example.test/").unwrap(),
            CssSize::new(320.0, 180.0),
            store,
        )
    }
    #[track_caller]
    fn settle(worker: &DocumentWorker, received: &Receiver<()>) -> Ready {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut latest = None;
        loop {
            if let Some(ready) = worker.take_ready() {
                latest = Some(ready);
            }
            if worker.idle()
                && let Some(ready) = latest.take()
            {
                return ready;
            }
            assert!(
                Instant::now() < deadline,
                "document worker did not settle: completed={}, revision={}, latest={}",
                worker.shared.completed.load(Ordering::Acquire),
                worker.shared.revision.load(Ordering::Acquire),
                latest.is_some()
            );
            let _ = received.recv_timeout(Duration::from_millis(20));
        }
    }
    fn by_id(snapshot: &EmbeddedSnapshot, id: &str) -> usize {
        snapshot
            .dom
            .descendants(trust::dom::DOCUMENT)
            .find(|&node| snapshot.dom.attr(node, "id") == Some(id))
            .unwrap()
    }
    fn image() -> ImageResource {
        ImageResource {
            width: 2,
            height: 2,
            rgba: vec![255; 16].into(),
            has_alpha: false,
        }
    }

    #[test]
    fn worker_preserves_disclosures_nested_scroll_and_find_across_resize() {
        let (worker, received) = worker();
        replace(
            &worker,
            "<style>#scroll{width:100px;height:40px;overflow:auto}#wide{width:600px;height:100px}</style><div id='scroll'><div id='wide'>wide</div></div><details><summary id='summary'>More</summary><p id='secret'>Hidden treasure</p></details>",
            ImageStore::default(),
        );
        let first = settle(&worker, &received).result.unwrap();
        let secret = by_id(&first, "secret");
        assert!(!first.layout.boxes.contains_key(&secret));
        worker.toggle(by_id(&first, "summary"));
        let scroll = by_id(&first, "scroll");
        worker.scroll(scroll, CssPoint::new(200.0, 20.0));
        worker.resize(CssSize::new(400.0, 200.0));
        worker.find("Hidden treasure".into());
        worker.request_semantics();
        let ready = settle(&worker, &received);
        assert_eq!(ready.find.matches.len(), 1);
        assert!(!ready.find.matches[0].is_empty());
        let doc = ready.result.unwrap();
        assert!(doc.semantics.is_some());
        assert!(doc.layout.boxes.contains_key(&secret));
        assert_eq!(doc.viewport(), CssSize::new(400.0, 200.0));
        assert_eq!(
            doc.layout
                .paint
                .scroll_containers
                .iter()
                .find(|c| c.node == scroll)
                .unwrap()
                .offset,
            CssPoint::new(200.0, 20.0)
        );
    }

    #[test]
    fn replacement_coalesces_and_cancels_a_bounded_image_inbox_without_waiting() {
        let (sent, received) = mpsc::channel();
        let (resume, gate) = mpsc::channel();
        let blocked = Cell::new(false);
        let worker = DocumentWorker::start(move || {
            let _ = sent.send(());
            if !blocked.replace(true) {
                let _ = gate.recv();
            }
        })
        .unwrap();
        let store = ImageStore::default();
        replace(&worker, "<img src='/old.png'>", store.clone());
        received.recv_timeout(Duration::from_secs(20)).unwrap();
        worker.take_ready().unwrap();
        let old = worker.images();
        let mut cancellation = old.cancellation();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for i in 0..8 {
                old.supply(format!("https://example.test/{i}.png"), image())
                    .await;
            }
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(20),
                    old.supply("https://example.test/old.png".into(), image())
                )
                .await
                .is_err()
            );
        });
        assert_eq!(worker.shared.image_slots.available_permits(), 0);
        for i in 0..100 {
            replace(
                &worker,
                &format!("<p id='latest-{i}'>A new story</p>"),
                store.clone(),
            );
        }
        assert!(!old.current());
        assert!(!old.reserve("https://example.test/old.png", true));
        runtime.block_on(cancellation.changed()).unwrap();
        assert_eq!(worker.shared.image_slots.available_permits(), 8);
        assert!(worker.shared.pending.lock().unwrap().images.is_empty());
        let generation = worker.generation();
        resume.send(()).unwrap();
        let ready = settle(&worker, &received);
        assert_eq!(ready.generation, generation);
        by_id(&ready.result.unwrap(), "latest-99");
        assert!(
            store.is_empty(),
            "obsolete images never reach the resource store"
        );
    }

    #[test]
    fn oversized_nesting_returns_an_error_and_worker_accepts_the_next_story() {
        let (worker, received) = worker();
        replace(
            &worker,
            &format!(
                "{}<p>Deep</p>{}",
                "<div>".repeat(2048),
                "</div>".repeat(2048)
            ),
            ImageStore::default(),
        );
        assert!(
            settle(&worker, &received)
                .result
                .unwrap_err()
                .contains("nested too deeply")
        );
        replace(
            &worker,
            "<p id='healthy'>Still responsive</p>",
            ImageStore::default(),
        );
        by_id(&settle(&worker, &received).result.unwrap(), "healthy");
    }

    #[test]
    fn article_can_load_more_than_256_images_and_retry_evicted_visible_images() {
        let (worker, received) = worker();
        let html: String = (0..260)
            .map(|i| format!("<img src='/{i}.png' width='20' height='20'>"))
            .collect();
        let store = ImageStore::default();
        replace(&worker, &html, store.clone());
        let first = settle(&worker, &received).result.unwrap();
        assert_eq!(first.image_requests().len(), 260);
        let sink = worker.images();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                for request in first.image_requests() {
                    assert!(sink.reserve(&request.source, false));
                    assert!(
                        !sink.reserve(&request.source, true),
                        "pending work is not duplicated"
                    );
                    sink.supply(request.source.clone(), image()).await;
                }
            });
        let doc = settle(&worker, &received).result.unwrap();
        assert_eq!(doc.image_requests().len(), 260);
        assert!(
            doc.image_requests()
                .iter()
                .all(|r| store.contains(r.handle))
        );
        let last = doc.image_requests().last().unwrap();
        store.remove(last.handle);
        assert!(!sink.reserve(&last.source, false));
        assert!(sink.reserve(&last.source, true));
        sink.failed(last.source.clone());
        assert!(
            !sink.reserve(&last.source, true),
            "failed resources back off"
        );
    }
}
