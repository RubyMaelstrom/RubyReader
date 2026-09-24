use crate::{
    backend::{Backend, Event},
    decora::Decora,
    documents::{DocumentWorker, FindHighlights},
    full_article::FullArticle,
    platform,
    theme::Theme,
    ui::{self, Dialog, Geometry},
};
use reader_core::{db::Store, model::*};
use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use trust::{
    core::{
        CssPoint, CssSize, ImeAction, Key, KeyInput, KeyState, Modifiers, PhysicalSize,
        ScaleFactor, ViewportMetrics,
    },
    embed::EmbeddedSnapshot,
    render::{
        CssRect, DisplayCommand as Paint, EditorVisual, ImageStore, RasterBackend, Scene,
        TextSelection,
    },
    text::{TextEditor, TextStyle},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{Key as NativeKey, ModifiersState, NamedKey},
    window::{CursorIcon, Window, WindowId},
};

struct Surface {
    generation: u64,
    revision: u64,
    doc: EmbeddedSnapshot,
    rect: CssRect,
    scroll: CssPoint,
    scene: Option<Scene>,
    cached: Option<(CssRect, CssPoint, Scene)>,
}
struct DocumentRequest {
    source: String,
    base: url::Url,
    rect: CssRect,
    generation: u64,
    scroll: CssPoint,
    failed: bool,
}
enum Presenter {
    Gpu(Box<trust::render::vello_hybrid::VelloHybridRenderer>),
    Cpu(Box<CpuPresenter>),
}
struct CpuPresenter {
    renderer: trust::render::vello_cpu::VelloCpuRenderer,
    _context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
}
impl Presenter {
    fn cpu(window: Arc<Window>) -> Result<Self, String> {
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = softbuffer::Surface::new(&context, window).map_err(|e| e.to_string())?;
        Ok(Self::Cpu(Box::new(CpuPresenter {
            renderer: trust::render::vello_cpu::VelloCpuRenderer::new(),
            _context: context,
            surface,
        })))
    }
    fn present(&mut self, scene: &Scene) -> Result<(), String> {
        match self {
            Self::Gpu(gpu) => {
                gpu.resize(scene.viewport.physical)?;
                gpu.present(scene)?;
            }
            Self::Cpu(cpu) => {
                let CpuPresenter {
                    renderer, surface, ..
                } = &mut **cpu;
                let p = scene.viewport.physical;
                if p.width == 0 || p.height == 0 {
                    return Ok(());
                }
                surface
                    .resize(
                        NonZeroU32::new(p.width).unwrap(),
                        NonZeroU32::new(p.height).unwrap(),
                    )
                    .map_err(|e| e.to_string())?;
                let frame = renderer.render(scene)?;
                let mut buffer = surface.buffer_mut().map_err(|e| e.to_string())?;
                buffer.copy_from_slice(frame.pixels);
                buffer.present().map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

pub struct App {
    backend: Backend,
    renderer_preference: String,
    window: Option<Arc<Window>>,
    presenter: Option<Presenter>,
    accessibility: Option<accesskit_winit::Adapter>,
    tray: platform::TrayHandle,
    tray_online: bool,
    settings: Settings,
    decora: Decora,
    snapshot: Option<Snapshot>,
    query: Query,
    serial: u64,
    selected: Option<i64>,
    article: Option<Article>,
    article_serial: u64,
    full_article: FullArticle,
    surfaces: [Option<Surface>; 5],
    documents: [DocumentWorker; 5],
    requests: [Option<DocumentRequest>; 5],
    store: ImageStore,
    geometry: Geometry,
    metrics: ViewportMetrics,
    pointer: CssPoint,
    modifiers: ModifiersState,
    mouse_down: bool,
    press: Option<(usize, usize, CssPoint)>,
    charm_press: Option<(&'static str, CssPoint)>,
    drag: Option<u8>,
    selection: Option<(usize, TextSelection)>,
    focus_target: Option<(usize, usize)>,
    keyboard_focus: bool,
    dialog: Option<Dialog>,
    fields: Vec<(String, String)>,
    editor: Option<(usize, TextEditor)>,
    folder: Option<i64>,
    status: String,
    focus_mode: bool,
    visible: bool,
    occluded: bool,
    initialized: bool,
    focused: bool,
    dirty: bool,
    started: Instant,
    last_frame: Instant,
    last_refresh: Instant,
    read_since: Option<Instant>,
    manually_unread: Option<i64>,
    image_scroll: Option<(u64, CssPoint)>,
    find: String,
    find_index: usize,
    find_highlights: FindHighlights,
    find_pending: Option<bool>,
    article_scroll: Option<CssPoint>,
    pending_pointer: Option<PhysicalPosition<f64>>,
    last_pointer: Instant,
    accessibility_dirty: bool,
    smoke: bool,
    smoke_step: usize,
    smoke_time: Instant,
}

impl App {
    pub fn new(backend: Backend, renderer_preference: String, smoke: bool) -> anyhow::Result<Self> {
        let settings = backend.store.lock().unwrap().settings().unwrap_or_default();
        let metrics = ViewportMetrics::from_physical(
            PhysicalSize::new(settings.window_width, settings.window_height),
            ScaleFactor::default(),
        );
        let documents = (0..5)
            .map(|i| DocumentWorker::new(i, backend.proxy.clone()))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .ok()
            .unwrap();
        Ok(Self {
            backend,
            renderer_preference,
            window: None,
            presenter: None,
            accessibility: None,
            tray: Arc::default(),
            tray_online: false,
            geometry: Geometry::new(metrics, &settings, false),
            metrics,
            settings,
            decora: Decora::fresh(),
            snapshot: None,
            query: Query::default(),
            serial: 0,
            selected: None,
            article: None,
            article_serial: 0,
            full_article: FullArticle::default(),
            surfaces: std::array::from_fn(|_| None),
            documents,
            requests: std::array::from_fn(|_| None),
            store: ImageStore::default(),
            pointer: CssPoint::default(),
            modifiers: ModifiersState::empty(),
            mouse_down: false,
            press: None,
            charm_press: None,
            drag: None,
            selection: None,
            focus_target: None,
            keyboard_focus: false,
            dialog: None,
            fields: vec![],
            editor: None,
            folder: None,
            status: "A little room for discovery".into(),
            focus_mode: false,
            visible: true,
            occluded: false,
            initialized: false,
            focused: true,
            dirty: true,
            started: Instant::now(),
            last_frame: Instant::now(),
            last_refresh: Instant::now(),
            read_since: None,
            manually_unread: None,
            image_scroll: None,
            find: String::new(),
            find_index: 0,
            find_highlights: FindHighlights::default(),
            find_pending: None,
            article_scroll: None,
            pending_pointer: None,
            last_pointer: Instant::now(),
            accessibility_dirty: true,
            smoke,
            smoke_step: 0,
            smoke_time: Instant::now(),
        })
    }
    fn redraw(&self) {
        if self.visible
            && !self.occluded
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }
    fn reload(&mut self) {
        self.serial += 1;
        self.backend.snapshot(self.serial, self.query.clone());
    }
    fn message(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.dirty = true;
        self.redraw();
    }
    fn persist_settings(&self) {
        let s = self.settings.clone();
        self.backend.change(move |db| {
            db.save_settings(&s)?;
            Ok(String::new())
        });
    }
    fn persist_scroll(&self) {
        if let (Some(a), Some(s)) = (&self.article, &self.surfaces[3]) {
            let (id, y) = (a.id, s.scroll.y);
            self.backend.change(move |db| {
                db.scroll(id, y)?;
                Ok(String::new())
            });
        }
    }
    fn metrics_for_window(&mut self) {
        if let Some(w) = &self.window {
            let size = w.inner_size();
            self.metrics = ViewportMetrics::from_physical(
                PhysicalSize::new(size.width.max(1), size.height.max(1)),
                ScaleFactor::new(w.scale_factor() * self.settings.ui_scale as f64),
            );
            self.geometry = Geometry::new(self.metrics, &self.settings, self.focus_mode);
        }
    }
    fn set_surface(
        &mut self,
        index: usize,
        body: String,
        extra: String,
        rect: CssRect,
        base: url::Url,
    ) {
        let source = format!("{extra}\n{body}");
        let size = CssSize::new(rect.width, rect.height);
        if let Some(request) = &mut self.requests[index]
            && request.source == source
            && request.base == base
        {
            if request.rect.width != rect.width || request.rect.height != rect.height {
                self.documents[index].resize(size);
            }
            request.rect = rect;
            if let Some(surface) = &mut self.surfaces[index] {
                surface.rect = rect;
            }
            return;
        }
        let scroll = self.surfaces[index]
            .as_ref()
            .map_or(CssPoint::default(), |s| s.scroll);
        let generation = self.documents[index].replace(
            ui::document_html(&body, &extra).into(),
            base.clone(),
            size,
            self.store.clone(),
        );
        self.requests[index] = Some(DocumentRequest {
            source,
            base,
            rect,
            generation,
            scroll,
            failed: false,
        });
        if self.press.is_some_and(|(i, _, _)| i == index) {
            self.press = None;
        }
        if index == 3 {
            self.image_scroll = None;
            self.find_highlights = FindHighlights::default();
            if !self.find.is_empty() {
                self.documents[index].find(self.find.clone());
            }
        }
        if index == 4 {
            self.surfaces[index] = None;
        }
    }
    fn clear_surface(&mut self, index: usize) {
        if self.requests[index].take().is_some() {
            self.documents[index].clear();
        }
        self.surfaces[index] = None;
    }
    fn current_surface(&self, index: usize) -> bool {
        self.surfaces[index].as_ref().is_some_and(|s| {
            self.requests[index]
                .as_ref()
                .is_some_and(|r| r.generation == s.generation)
        })
    }
    fn document_ready(&mut self, index: usize) {
        let Some(ready) = self.documents[index].take_ready() else {
            return;
        };
        let Some(request) = &self.requests[index] else {
            return;
        };
        if request.generation != ready.generation {
            return;
        }
        if self.surfaces[index]
            .as_ref()
            .is_some_and(|s| s.generation == ready.generation && s.revision >= ready.revision)
        {
            return;
        }
        let mut doc = match ready.result {
            Ok(doc) => doc,
            Err(error) => {
                self.surfaces[index] = None;
                if index == 3 && !request.failed {
                    let html = ui::document_html(
                        &format!(
                            "<main class='article'><h1>This story could not be displayed</h1><p>{}</p><p>You can still open its original page using the toolbar above.</p></main>",
                            reader_core::content::escape(&error)
                        ),
                        &ui::article_css(&self.settings),
                    );
                    let generation = self.documents[index].replace(
                        html.into(),
                        request.base.clone(),
                        CssSize::new(request.rect.width, request.rect.height),
                        self.store.clone(),
                    );
                    if let Some(request) = &mut self.requests[index] {
                        request.generation = generation;
                        request.failed = true;
                    }
                }
                self.message(error);
                return;
            }
        };
        if doc.viewport() != CssSize::new(request.rect.width, request.rect.height) {
            return;
        }
        let rect = request.rect;
        let old = self.surfaces[index].take();
        let mut scroll = old.as_ref().map_or(request.scroll, |s| s.scroll);
        if index == 3
            && let Some(saved) = self.article_scroll.take()
        {
            scroll = saved;
        }
        if let Some(previous) = &old
            && previous.generation == ready.generation
        {
            for container in &mut doc.layout.paint.scroll_containers {
                if let Some(old) = previous
                    .doc
                    .layout
                    .paint
                    .scroll_containers
                    .iter()
                    .find(|c| c.node == container.node)
                {
                    container.offset = CssPoint::new(
                        old.offset.x.clamp(
                            0.0,
                            (container.content.width - container.viewport.width).max(0.0),
                        ),
                        old.offset.y.clamp(
                            0.0,
                            (container.content.height - container.viewport.height).max(0.0),
                        ),
                    );
                }
            }
        }
        if let Some((i, node)) = self.focus_target
            && i == index
            && old
                .as_ref()
                .is_none_or(|s| s.generation != ready.generation)
        {
            let href = old.as_ref().and_then(|s| s.doc.dom.attr(node, "href"));
            self.focus_target = href
                .and_then(|href| {
                    doc.dom
                        .descendants(trust::dom::DOCUMENT)
                        .find(|&node| doc.dom.attr(node, "href") == Some(href))
                })
                .map(|node| (index, node));
        }
        if old.as_ref().is_some_and(|s| {
            s.generation != ready.generation || s.doc.geometry_revision != doc.geometry_revision
        }) && self.selection.is_some_and(|(i, _)| i == index)
        {
            self.selection = None;
        }
        scroll = doc.clamp_scroll(scroll);
        self.surfaces[index] = Some(Surface {
            generation: ready.generation,
            revision: ready.revision,
            doc,
            rect,
            scroll,
            scene: None,
            cached: None,
        });
        if index == 3 {
            if ready.find.query == self.find {
                self.find_highlights = ready.find;
                if let Some(next) = self.find_pending.take() {
                    self.jump_find(next);
                }
            }
            self.schedule_images();
        }
        self.accessibility_dirty = true;
        if self.pointer.x >= 0.0 && self.pointer.y >= 0.0 {
            let scale = self.metrics.scale_factor.get();
            self.pending_pointer = Some(PhysicalPosition::new(
                self.pointer.x as f64 * scale,
                self.pointer.y as f64 * scale,
            ));
        }
        self.redraw();
    }
    fn rebuild(&mut self) {
        self.metrics_for_window();
        let g = self.geometry;
        let palette = Theme::from_name(&self.settings.reading_theme).css();
        let base = url::Url::parse("https://ruby-reader.invalid/").unwrap();
        self.set_surface(
            0,
            ui::background(
                g,
                self.snapshot.as_ref(),
                &self.query,
                self.article.as_ref(),
                &self.status,
                self.focus_mode,
                &self.decora,
            ),
            palette.clone(),
            CssRect::new(0.0, 0.0, g.width, g.height),
            base.clone(),
        );
        if !self.focus_mode {
            self.set_surface(
                1,
                ui::sidebar(self.snapshot.as_ref(), &self.query.view, &self.decora),
                palette.clone(),
                g.sidebar,
                base.clone(),
            );
            self.set_surface(
                2,
                ui::headlines(self.snapshot.as_ref(), self.selected, &self.query),
                palette.clone(),
                g.headlines,
                base.clone(),
            );
        } else {
            self.clear_surface(1);
            self.clear_surface(2);
        }
        let article_base = self
            .article
            .as_ref()
            .and_then(|a| url::Url::parse(&a.url).ok())
            .unwrap_or(base.clone());
        self.set_surface(
            3,
            ui::article_html(self.article.as_ref(), self.full_article.show, &self.decora),
            ui::article_css(&self.settings),
            g.article,
            article_base,
        );
        if let Some(dialog) = &self.dialog {
            let html = ui::dialog_html(
                dialog,
                &self.fields,
                self.snapshot.as_ref(),
                &self.settings,
                self.folder,
            );
            let desired: f32 = match dialog {
                Dialog::AddFeed => 340.0,
                Dialog::Folder(_) | Dialog::Search | Dialog::Find => 310.0,
                Dialog::EditFeed(_) => 470.0,
                Dialog::DeleteFeed(_) | Dialog::DeleteFolder(_) | Dialog::Export => 350.0,
                Dialog::Discover(feeds) => (200.0 + feeds.len() as f32 * 55.0).min(580.0),
                _ => 580.0,
            };
            let height = desired.min(g.modal.height);
            let rect = CssRect::new(g.modal.x, (g.height - height) * 0.5, g.modal.width, height);
            self.set_surface(
                4,
                html,
                format!("{palette}.dialog{{min-height:{height}px}}"),
                rect,
                base,
            );
        } else {
            self.clear_surface(4);
        }
        self.dirty = false;
    }
    fn compose(&mut self) -> Scene {
        if self.dirty {
            self.rebuild();
        }
        let theme = Theme::from_name(&self.settings.reading_theme);
        let mut scene = empty_scene(self.metrics, self.store.clone(), theme);
        let seconds = self.started.elapsed().as_secs_f32();
        for index in 0..5 {
            let Some(surface) = &mut self.surfaces[index] else {
                continue;
            };
            if index == 1
                && self.requests[1]
                    .as_ref()
                    .is_some_and(|r| r.generation == surface.generation)
            {
                self.documents[1].focus(
                    self.focus_target
                        .filter(|(i, _)| *i == 1 && self.keyboard_focus && self.dialog.is_none())
                        .map(|(_, node)| node),
                );
            }
            if index == 4 {
                scene.primitives.push(Paint::FillRect {
                    rect: CssRect::new(0.0, 0.0, self.geometry.width, self.geometry.height),
                    color: theme.paint("overlay", 155),
                });
            }
            if surface
                .cached
                .as_ref()
                .is_none_or(|(rect, scroll, _)| *rect != surface.rect || *scroll != surface.scroll)
            {
                surface.cached = Some((
                    surface.rect,
                    surface.scroll,
                    surface
                        .doc
                        .scene(self.metrics, surface.rect, surface.scroll, seconds),
                ));
            }
            let mut part = surface.cached.as_ref().unwrap().2.clone();
            if index == 0 || index == 1 || (index == 3 && self.article.is_none()) {
                self.decora.animate(
                    &mut part,
                    &surface.doc,
                    seconds,
                    self.settings.reduced_motion || self.focus_mode || self.dialog.is_some(),
                );
            }
            if let Some((selected_index, selection)) = self.selection
                && selected_index == index
            {
                for rect in part.selection_rects(selection) {
                    part.primitives.push(Paint::FillRect {
                        rect,
                        color: theme.paint("accent", 65),
                    });
                }
            }
            if index == 3 && !self.find.is_empty() && self.find_highlights.query == self.find {
                part.primitives
                    .push(Paint::PushClip(trust::render::PaintShape::Rect(
                        surface.rect,
                    )));
                for (i, rectangles) in self.find_highlights.matches.iter().enumerate() {
                    for rectangle in rectangles {
                        let rect = rectangle.translate(
                            surface.rect.x - surface.scroll.x,
                            surface.rect.y - surface.scroll.y,
                        );
                        if rect.y + rect.height < surface.rect.y
                            || rect.y > surface.rect.y + surface.rect.height
                        {
                            continue;
                        }
                        part.primitives.push(Paint::FillRect {
                            rect,
                            color: if i == self.find_index {
                                theme.paint("find", 110)
                            } else {
                                theme.paint("find-soft", 60)
                            },
                        });
                    }
                }
                part.primitives.push(Paint::PopClip);
            }
            if let Some((fi, node)) = self.focus_target
                && self.keyboard_focus
                && fi == index
                && let Some(rect) = node_rect(surface, node)
            {
                // Keyboard-only underline, never an arbitrary clicked box.
                let mut ancestor = Some(node);
                let mut focus_color = theme.paint("accent", 255);
                while let Some(current) = ancestor {
                    if surface
                        .doc
                        .dom
                        .attr(current, "class")
                        .is_some_and(|classes| {
                            classes
                                .split_ascii_whitespace()
                                .any(|class| matches!(class, "primary" | "ribbon" | "dialog-top"))
                        })
                    {
                        focus_color = theme.paint("primary-ink", 255);
                        break;
                    }
                    ancestor = surface.doc.dom.node(current).parent;
                }
                part.primitives.push(Paint::FillRect {
                    rect: CssRect::new(
                        rect.x + 3.0,
                        rect.y + rect.height - 2.0,
                        (rect.width - 6.0).max(1.0),
                        2.0,
                    ),
                    color: focus_color,
                });
            }
            if index == 4 {
                for (i, (_, value)) in self.fields.iter().enumerate() {
                    if let Some(node) = surface
                        .doc
                        .dom
                        .descendants(trust::dom::DOCUMENT)
                        .find(|&n| surface.doc.dom.attr(n, "id") == Some(&format!("field-{i}")))
                        && let Some(rect) = node_rect(surface, node)
                    {
                        let visual = if let Some((ei, editor)) = &mut self.editor {
                            if *ei == i {
                                editor.set_width((rect.width - 18.0).max(1.0));
                                let (selection, caret, _) = editor.geometry();
                                EditorVisual {
                                    text: editor.text(),
                                    selection: selection.into_iter().map(editor_rect).collect(),
                                    caret: caret.map(editor_rect),
                                }
                            } else {
                                EditorVisual {
                                    text: value.clone(),
                                    ..Default::default()
                                }
                            }
                        } else {
                            EditorVisual {
                                text: value.clone(),
                                ..Default::default()
                            }
                        };
                        trust::render::paint_text_editor(
                            &mut part.primitives,
                            &visual,
                            rect,
                            theme.paint("ink", 255),
                        );
                    }
                }
            }
            // Native scrollbar decorations operate on the retained surface, including horizontal overflow.
            for (horizontal, extent, viewport, offset) in [
                (
                    false,
                    surface.doc.layout.paint.height,
                    surface.rect.height,
                    surface.scroll.y,
                ),
                (
                    true,
                    surface.doc.layout.paint.width,
                    surface.rect.width,
                    surface.scroll.x,
                ),
            ] {
                if extent > viewport + 1.0 && index != 0 {
                    let length = (viewport * viewport / extent).max(24.0);
                    let position = offset / (extent - viewport) * (viewport - length);
                    let rect = if horizontal {
                        CssRect::new(
                            surface.rect.x + position,
                            surface.rect.y + surface.rect.height - 5.0,
                            length,
                            4.0,
                        )
                    } else {
                        CssRect::new(
                            surface.rect.x + surface.rect.width - 5.0,
                            surface.rect.y + position,
                            4.0,
                            length,
                        )
                    };
                    part.primitives.push(Paint::FillRect {
                        rect,
                        color: theme.paint("accent", 180),
                    });
                }
            }
            scene.primitives.extend(part.primitives.iter().cloned());
            surface.scene = Some(part);
        }
        if self.dialog.is_none() {
            self.decora.paint(
                &mut scene,
                self.geometry.width,
                self.geometry.height,
                seconds,
                self.settings.reduced_motion,
                self.focus_mode,
            );
        }
        scene
    }
    fn render(&mut self) {
        if !self.visible || self.occluded || self.window.is_none() {
            return;
        }
        let scene = self.compose();
        if self.surfaces[3]
            .as_ref()
            .is_some_and(|s| self.image_scroll != Some((s.generation, s.scroll)))
        {
            self.schedule_images();
        }
        if let Some(presenter) = &mut self.presenter
            && let Err(error) = presenter.present(&scene)
        {
            eprintln!("Ruby Reader renderer: {error}");
            if let Some(window) = &self.window {
                match Presenter::cpu(window.clone()) {
                    Ok(mut cpu) => {
                        let _ = cpu.present(&scene);
                        self.presenter = Some(cpu);
                        self.message("Using the software renderer");
                    }
                    Err(e) => self.message(e),
                }
            }
        }
        self.last_frame = Instant::now();
        self.update_accessibility();
    }
    fn schedule_images(&mut self) {
        if self.selected.is_none() || self.article.is_none() || !self.current_surface(3) {
            return;
        }
        if self.requests[3].as_ref().is_some_and(|r| r.failed) {
            return;
        }
        let Some(surface) = &self.surfaces[3] else {
            return;
        };
        self.image_scroll = Some((surface.generation, surface.scroll));
        let top = surface.scroll.y;
        let bottom = top + surface.rect.height;
        let distances: HashMap<_, _> = surface
            .doc
            .layout
            .paint
            .primitives
            .iter()
            .filter_map(|paint| {
                if let Paint::Image { handle, rect, .. } = paint {
                    let distance = if rect.y + rect.height < top {
                        top - rect.y - rect.height
                    } else if rect.y > bottom {
                        rect.y - bottom
                    } else {
                        0.0
                    };
                    Some((*handle, distance))
                } else {
                    None
                }
            })
            .collect();
        let sink = self.documents[3].images();
        let mut requests: Vec<_> = surface
            .doc
            .image_requests()
            .iter()
            .filter(|r| !r.source.starts_with(ui::ASSET_BASE) && !r.source.starts_with("data:"))
            .filter(|r| !surface.doc.resources.contains(r.handle))
            .filter(|r| sink.reserve(&r.source, distances.get(&r.handle) == Some(&0.0)))
            .collect();
        requests.sort_by(|a, b| {
            distances
                .get(&a.handle)
                .unwrap_or(&f32::MAX)
                .total_cmp(distances.get(&b.handle).unwrap_or(&f32::MAX))
        });
        let sources = requests
            .into_iter()
            .map(|r| r.source.clone())
            .collect::<Vec<_>>();
        if !sources.is_empty() {
            self.backend.images(sink, sources);
        }
    }
    fn select(&mut self, id: i64) {
        if self.selected == Some(id) {
            self.manually_unread = None;
            self.read_since = None;
            return;
        }
        self.persist_scroll();
        self.full_article.select();
        self.article_serial += 1;
        self.selected = Some(id);
        self.manually_unread = None;
        self.article = None;
        self.read_since = None;
        self.selection = None;
        self.image_scroll = None;
        self.find.clear();
        self.find_highlights = FindHighlights::default();
        self.find_pending = None;
        self.article_scroll = None;
        self.clear_surface(3);
        self.backend.article(id, self.article_serial);
        self.dirty = true;
        self.redraw();
    }
    fn fetch_full_article(&mut self, manual: bool) {
        let Some(article) = &self.article else {
            return;
        };
        if self
            .full_article
            .request(article, manual, |id, url, serial| {
                self.backend.extract(id, url, serial)
            })
        {
            self.message("Retrieving the full article…");
        }
        if manual {
            self.dirty = true;
        }
    }
    fn choose_view(&mut self, view: View) {
        self.persist_scroll();
        self.query.view = view;
        self.query.offset = 0;
        self.clear_surface(2);
        self.reload();
    }
    fn next_article(&mut self, delta: isize) {
        if let Some(snap) = &self.snapshot {
            let pos = snap
                .articles
                .iter()
                .position(|a| Some(a.id) == self.selected)
                .map(|p| p as isize)
                .unwrap_or(if delta > 0 {
                    -1
                } else {
                    snap.articles.len() as isize
                });
            if let Some(a) = snap.articles.get((pos + delta).max(0) as usize) {
                self.select(a.id);
            }
        }
    }
    fn mouse_navigation(&mut self, button: MouseButton, state: ElementState) {
        if state != ElementState::Pressed || self.dialog.is_some() || self.editor.is_some() {
            return;
        }
        let Some(delta) = (match button {
            MouseButton::Back => Some(1),
            MouseButton::Forward => Some(-1),
            _ => None,
        }) else {
            return;
        };
        self.next_article(delta);
    }
    fn open_dialog(&mut self, dialog: Dialog, fields: Vec<(String, String)>) {
        self.commit_editor();
        self.dialog = Some(dialog);
        self.fields = fields;
        self.clear_surface(4);
        self.focus_target = None;
        self.selection = None;
        self.dirty = true;
        self.rebuild();
        if !self.fields.is_empty() {
            self.focus_field(0);
        }
        self.redraw();
    }
    fn commit_editor(&mut self) {
        if let Some((i, editor)) = self.editor.take()
            && let Some(field) = self.fields.get_mut(i)
        {
            field.1 = editor.text();
        }
    }
    fn focus_field(&mut self, index: usize) {
        self.commit_editor();
        if let Some((_, value)) = self.fields.get(index) {
            self.editor = Some((
                index,
                TextEditor::new(
                    value,
                    &TextStyle {
                        size: 15.0,
                        ..Default::default()
                    },
                    480.0,
                    false,
                ),
            ));
            if let Some((_, editor)) = &mut self.editor {
                editor.select_all();
            }
            if let Some(window) = &self.window {
                window.set_ime_allowed(true);
                window.set_ime_cursor_area(
                    PhysicalPosition::new(
                        self.geometry.modal.x as i32,
                        self.geometry.modal.y as i32,
                    ),
                    winit::dpi::PhysicalSize::new(480, 38),
                );
            }
        }
        self.redraw();
    }
    fn close_dialog(&mut self) {
        self.commit_editor();
        self.dialog = None;
        self.fields.clear();
        self.clear_surface(4);
        self.focus_target = None;
        self.dirty = true;
        if let Some(w) = &self.window {
            w.set_ime_allowed(false);
        }
        self.redraw();
    }
    fn show(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.create_window(event_loop);
        }
        self.visible = true;
        self.occluded = false;
        if let Some(w) = &self.window {
            w.set_visible(true);
            w.set_minimized(false);
            w.focus_window();
        }
        self.read_since = None;
        self.redraw();
    }
    fn close_window(&mut self) {
        if self.tray_online {
            self.persist_scroll();
            self.persist_settings();
            self.visible = false;
            self.focused = false;
            self.occluded = false;
            self.read_since = None;
            self.mouse_down = false;
            self.press = None;
            self.charm_press = None;
            self.drag = None;
            self.modifiers = ModifiersState::empty();
            // Wayland's Window::set_visible is a no-op. Release every native
            // window owner so the compositor really removes the toplevel and
            // its taskbar entry. Documents, scroll, library, and tray survive.
            self.accessibility = None;
            self.presenter = None;
            self.window = None;
        } else {
            self.message("The system tray is unavailable. Use Quit to exit Ruby Reader.");
        }
    }
    /// Exercise the real retained hit test, not an action-only shortcut.
    fn smoke_click(&mut self, action: &str, event_loop: &ActiveEventLoop) {
        self.smoke_settle_documents();
        if self.dirty {
            self.rebuild();
        }
        let href = format!("app:{action}");
        let mut point = None;
        for i in (0..5).rev() {
            if self.dialog.is_some() && i != 4 {
                continue;
            }
            let Some(s) = &self.surfaces[i] else {
                continue;
            };
            for node in s.doc.dom.descendants(trust::dom::DOCUMENT) {
                if s.doc.dom.attr(node, "href") == Some(href.as_str())
                    && let Some(rect) = node_rect(s, node)
                {
                    point = Some(CssPoint::new(
                        rect.x + rect.width.min(40.0) * 0.5,
                        rect.y + rect.height * 0.5,
                    ));
                    break;
                }
            }
            if point.is_some() {
                break;
            }
        }
        let point = point.unwrap_or_else(|| panic!("Smoke: no geometry for {action}"));
        let (index, node) = self
            .hit(point)
            .unwrap_or_else(|| panic!("Smoke: no hit for {action} at {point:?}"));
        assert_eq!(
            self.ancestor_attr(index, node, "href").as_deref(),
            Some(href.as_str()),
            "Smoke: wrong hit for {action}"
        );
        self.activate(index, node, event_loop);
    }
    fn smoke_capture(&mut self, name: &str) {
        self.smoke_settle_documents();
        let scene = self.compose();
        let frame = trust::render::vello_cpu::VelloCpuRenderer::new()
            .render_rgba(&scene)
            .expect("Smoke rasterization");
        let path = self.backend.data_dir.join(format!("smoke-{name}.png"));
        trust::render::headless::write_png(&frame, &path).expect("Smoke screenshot");
    }
    /// Only the scripted acceptance driver waits for a specific document state;
    /// production input/rendering never waits on a document worker.
    fn smoke_settle_documents(&mut self) {
        assert!(self.smoke);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.dirty {
                self.rebuild();
            }
            self.documents[1].focus(
                self.focus_target
                    .filter(|(i, _)| *i == 1 && self.keyboard_focus && self.dialog.is_none())
                    .map(|(_, node)| node),
            );
            for index in 0..5 {
                self.document_ready(index);
            }
            if self.documents.iter().all(DocumentWorker::idle) && !self.dirty {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Smoke: document preparation timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn empty_scene(metrics: ViewportMetrics, store: ImageStore, theme: Theme) -> Scene {
    Scene {
        viewport: metrics,
        primitives: vec![Paint::FillRect {
            rect: CssRect::new(0.0, 0.0, metrics.css.width, metrics.css.height),
            color: theme.paint("canvas", 255),
        }],
        controls: vec![],
        content_viewport: CssRect::new(0.0, 0.0, metrics.css.width, metrics.css.height),
        image_store: store,
        canvas_images: Default::default(),
        page_scroll_containers: vec![],
        page_size: CssSize::default(),
    }
}
fn node_rect(surface: &Surface, node: usize) -> Option<CssRect> {
    surface.doc.layout.boxes.get(&node).map(|r| {
        CssRect::new(
            r.left as f32 + surface.rect.x - surface.scroll.x,
            r.top as f32 + surface.rect.y - surface.scroll.y,
            r.width as f32,
            r.height as f32,
        )
    })
}
fn editor_rect(rect: trust::text::EditorRect) -> CssRect {
    CssRect::new(rect.x, rect.y, rect.width, rect.height)
}

impl App {
    fn action(&mut self, action: &str, event_loop: &ActiveEventLoop) {
        let (kind, arg) = action.split_once(':').unwrap_or((action, ""));
        let id = arg.parse::<i64>().ok();
        match kind {
            "all" => self.choose_view(View::All),
            "unread" => self.choose_view(View::Unread),
            "saved" => self.choose_view(View::Saved),
            "feed" => {
                if let Some(id) = id {
                    self.choose_view(View::Feed(id))
                }
            }
            "folder-view" => {
                if let Some(id) = id {
                    self.choose_view(View::Folder(id))
                }
            }
            "article" => {
                if let Some(id) = id {
                    self.select(id)
                }
            }
            "add" => {
                self.folder = match self.query.view {
                    View::Folder(id) => Some(id),
                    _ => None,
                };
                self.open_dialog(
                    Dialog::AddFeed,
                    vec![("Feed or website address".into(), String::new())],
                );
            }
            "folder" => self.open_dialog(
                Dialog::Folder(None),
                vec![("Folder name".into(), String::new())],
            ),
            "folder-edit" => {
                if let Some(folder) = id
                    .and_then(|id| self.snapshot.as_ref()?.folders.iter().find(|f| f.id == id))
                    .cloned()
                {
                    self.open_dialog(
                        Dialog::Folder(Some(folder.id)),
                        vec![("Folder name".into(), folder.title)],
                    );
                }
            }
            "feed-edit" => {
                if let Some(feed) = id
                    .and_then(|id| self.snapshot.as_ref()?.feeds.iter().find(|f| f.id == id))
                    .cloned()
                {
                    self.folder = feed.folder_id;
                    self.open_dialog(
                        Dialog::EditFeed(feed.id),
                        vec![
                            ("Name".into(), feed.title),
                            ("Feed address".into(), feed.url),
                        ],
                    );
                }
            }
            "feed-delete" => {
                if let Some(id) = id {
                    self.open_dialog(Dialog::DeleteFeed(id), vec![])
                }
            }
            "folder-delete" => {
                if let Some(id) = id {
                    self.open_dialog(Dialog::DeleteFolder(id), vec![])
                }
            }
            "settings" => self.open_dialog(Dialog::Settings, vec![]),
            "help" => self.open_dialog(Dialog::Help, vec![]),
            "search" => self.open_dialog(
                Dialog::Search,
                vec![("Search this collection".into(), self.query.search.clone())],
            ),
            "find" => self.open_dialog(
                Dialog::Find,
                vec![("Find in this article".into(), self.find.clone())],
            ),
            "field" => {
                if let Some(i) = id {
                    self.focus_field(i as usize)
                }
            }
            "cancel" => self.close_dialog(),
            "submit" => self.submit(),
            "cycle-folder" => {
                self.commit_editor();
                let choices = std::iter::once(None)
                    .chain(
                        self.snapshot
                            .as_ref()
                            .into_iter()
                            .flat_map(|s| s.folders.iter().map(|f| Some(f.id))),
                    )
                    .collect::<Vec<_>>();
                let i = choices
                    .iter()
                    .position(|id| *id == self.folder)
                    .unwrap_or(0);
                self.folder = choices[(i + 1) % choices.len()];
                self.dirty = true;
            }
            "choose-feed" => {
                if let (Some(i), Some(Dialog::Discover(feeds))) = (id, &self.dialog)
                    && let Some(feed) = feeds.get(i as usize).cloned()
                {
                    self.backend.subscribe(feed, self.folder);
                    self.close_dialog();
                }
            }
            "refresh" | "feed-refresh" => {
                self.backend
                    .refresh(if kind == "feed-refresh" { id } else { None }, false);
                self.message("Gathering new stories…");
            }
            "save" => {
                if let Some(a) = &mut self.article {
                    a.saved = !a.saved;
                    let (id, saved) = (a.id, a.saved);
                    self.backend.change(move |db| {
                        db.saved(id, saved)?;
                        Ok(String::new())
                    });
                    self.dirty = true;
                }
            }
            "save-id" => {
                if let Some(a) = id
                    .and_then(|id| self.snapshot.as_ref()?.articles.iter().find(|a| a.id == id))
                    .cloned()
                {
                    let saved = !a.saved;
                    self.backend.change(move |db| {
                        db.saved(a.id, saved)?;
                        Ok(String::new())
                    });
                    if let Some(current) = &mut self.article
                        && current.id == a.id
                    {
                        current.saved = saved;
                    }
                    self.dirty = true;
                }
            }
            "read" => {
                if let Some(a) = &mut self.article {
                    a.read = !a.read;
                    self.manually_unread = if a.read { None } else { Some(a.id) };
                    let (id, read) = (a.id, a.read);
                    self.read_since = None;
                    self.backend.change(move |db| {
                        db.read(id, read)?;
                        Ok(String::new())
                    });
                    self.dirty = true;
                }
            }
            "mark-all" => {
                let query = self.query.clone();
                self.backend.change(move |db| {
                    db.mark_view_read(&query)?;
                    Ok("This view is marked as read".into())
                });
                if let Some(a) = &mut self.article {
                    a.read = true;
                }
            }
            "extract" => {
                self.fetch_full_article(true);
            }
            "original" => {
                if let Some(a) = &self.article {
                    platform::open_url(&a.url)
                }
            }
            "source" => {
                self.full_article.show = !self.full_article.show;
                if self.full_article.show
                    && self.article.as_ref().is_some_and(|a| a.extracted.is_none())
                {
                    self.fetch_full_article(true);
                }
                self.dirty = true;
            }
            "focus" => {
                self.focus_mode = !self.focus_mode;
                self.dirty = true;
            }
            "theme" => {
                self.settings.reading_theme = match self.settings.reading_theme.as_str() {
                    "light" => "sepia",
                    "sepia" => "dark",
                    _ => "light",
                }
                .into();
                self.settings_changed();
            }
            "motion" => {
                self.settings.reduced_motion = !self.settings.reduced_motion;
                self.settings_changed();
            }
            "notifications" => {
                self.settings.notifications = !self.settings.notifications;
                self.settings_changed();
            }
            "interval-down" => {
                self.settings.refresh_minutes =
                    self.settings.refresh_minutes.saturating_sub(5).max(5);
                self.settings_changed();
            }
            "interval-up" => {
                self.settings.refresh_minutes = (self.settings.refresh_minutes + 5).min(1440);
                self.settings_changed();
            }
            "font-down" => {
                self.settings.font_size = (self.settings.font_size - 1.0).max(12.0);
                self.settings_changed();
            }
            "font-up" => {
                self.settings.font_size = (self.settings.font_size + 1.0).min(32.0);
                self.settings_changed();
            }
            "line-down" => {
                self.settings.line_height = (self.settings.line_height - 0.1).max(1.2);
                self.settings_changed();
            }
            "line-up" => {
                self.settings.line_height = (self.settings.line_height + 0.1).min(2.2);
                self.settings_changed();
            }
            "scale-down" => {
                self.settings.ui_scale = (self.settings.ui_scale - 0.1).max(0.8);
                self.settings_changed();
            }
            "scale-up" => {
                self.settings.ui_scale = (self.settings.ui_scale + 0.1).min(1.5);
                self.settings_changed();
            }
            "clear-cache" => {
                let net = self.backend.net.clone();
                self.backend.change(move |_| {
                    net.clear_cache()?;
                    Ok("The image cache has been cleared".into())
                });
            }
            "sample" => {
                self.backend.change(|db| {
                    db.sample()?;
                    Ok("Welcome to the sample scrapbook".into())
                });
            }
            "sort" => {
                self.query.sort = match self.query.sort {
                    Sort::Newest => Sort::Oldest,
                    Sort::Oldest => Sort::Title,
                    Sort::Title => Sort::Feed,
                    Sort::Feed => Sort::Newest,
                };
                self.query.offset = 0;
                self.clear_surface(2);
                self.message(format!("Sort: {:?}", self.query.sort));
                self.reload();
            }
            "page-prev" => {
                self.query.offset = self.query.offset.saturating_sub(150);
                self.clear_surface(2);
                self.reload();
            }
            "page-next" => {
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| (self.query.offset + 150) < s.total as usize)
                {
                    self.query.offset += 150;
                    self.clear_surface(2);
                    self.reload();
                }
            }
            "import" => self.import_file(),
            "export" => self.open_dialog(Dialog::Export, vec![]),
            "quit" => {
                self.persist_scroll();
                self.persist_settings();
                self.backend.flush();
                event_loop.exit();
            }
            _ => {}
        }
        self.redraw();
    }
    fn settings_changed(&mut self) {
        if let Some(window) = &self.window {
            window.set_theme(Some(
                Theme::from_name(&self.settings.reading_theme).window_theme(),
            ));
            window.set_min_inner_size(Some(LogicalSize::new(
                860.0 * self.settings.ui_scale as f64,
                650.0 * self.settings.ui_scale as f64,
            )));
        }
        self.dirty = true;
        self.persist_settings();
    }
    fn submit(&mut self) {
        self.commit_editor();
        let values = self
            .fields
            .iter()
            .map(|f| f.1.trim().to_owned())
            .collect::<Vec<_>>();
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        match dialog {
            Dialog::AddFeed => {
                let Some(address) = values.first().filter(|s| !s.is_empty()) else {
                    self.message("Enter a feed or website address");
                    return;
                };
                self.backend.discover(address.clone());
                self.close_dialog();
                self.message("Looking for a feed…");
            }
            Dialog::Folder(id) => {
                let Some(title) = values.first().filter(|s| !s.is_empty()).cloned() else {
                    self.message("Give the folder a name");
                    return;
                };
                let parent = match self.query.view {
                    View::Folder(id) => Some(id),
                    _ => None,
                };
                self.backend.change(move |db| {
                    match id {
                        Some(id) => db.rename_folder(id, &title)?,
                        None => {
                            db.add_folder(&title, parent)?;
                        }
                    }
                    Ok("Folder saved".into())
                });
                self.close_dialog();
            }
            Dialog::EditFeed(id) => {
                if values.len() < 2 || values[0].is_empty() {
                    self.message("A feed needs a name and address");
                    return;
                }
                let Ok(url) = url::Url::parse(&values[1]) else {
                    self.message("Enter a valid HTTP or HTTPS feed address");
                    return;
                };
                if !reader_core::content::web_url(&url) {
                    self.message("Use an HTTP or HTTPS feed address without login credentials");
                    return;
                }
                let title = values[0].clone();
                let folder = self.folder;
                self.backend.change(move |db| {
                    db.edit_feed(id, &title, url.as_str(), folder)?;
                    Ok("Subscription saved".into())
                });
                self.close_dialog();
            }
            Dialog::DeleteFeed(id) => {
                self.backend.change(move |db| {
                    db.remove_feed(id)?;
                    Ok("Subscription removed; saved articles were retained".into())
                });
                self.close_dialog();
                if self.query.view == View::Feed(id) {
                    self.choose_view(View::All);
                }
                self.selected = None;
                self.article = None;
                self.full_article.select();
            }
            Dialog::DeleteFolder(id) => {
                self.backend.change(move |db| {
                    db.remove_folder(id)?;
                    Ok("Folder removed; subscriptions were retained".into())
                });
                self.close_dialog();
                if self.query.view == View::Folder(id) {
                    self.choose_view(View::All);
                }
            }
            Dialog::Search => {
                self.query.search = values.first().cloned().unwrap_or_default();
                self.query.offset = 0;
                self.clear_surface(2);
                self.close_dialog();
                self.reload();
            }
            Dialog::Find => {
                self.find = values.first().cloned().unwrap_or_default();
                self.find_index = 0;
                self.close_dialog();
                self.jump_find(false);
            }
            Dialog::Export => {
                self.close_dialog();
                self.export_file();
            }
            _ => {}
        }
    }
    fn import_file(&self) {
        let backend = self.backend.clone();
        self.backend.rt.spawn(async move {
        if let Some(file)=rfd::AsyncFileDialog::new().add_filter("OPML subscriptions",&["opml","xml"]).pick_file().await {
            let path=file.path().to_owned();backend.change(move|db|{let meta=std::fs::metadata(&path)?;anyhow::ensure!(meta.len()<=8*1024*1024,"OPML file exceeds 8 MiB");let xml=std::fs::read_to_string(path)?;let(added,skipped)=db.import_opml(&xml)?;Ok(format!("Imported {added} subscriptions; skipped {skipped} invalid addresses. Refresh to collect stories."))});
        }
    });
    }
    fn export_file(&self) {
        let backend = self.backend.clone();
        self.backend.rt.spawn(async move {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("OPML subscriptions", &["opml"])
                .set_file_name("ruby-reader.opml")
                .save_file()
                .await
            {
                let path = file.path().to_owned();
                backend.change(move |db| {
                    let xml = db.export_opml()?;
                    std::fs::write(&path, xml)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                    }
                    Ok("Your subscriptions have been exported".into())
                });
            }
        });
    }
    fn jump_find(&mut self, next: bool) {
        if self.find_highlights.query != self.find {
            self.find_pending = Some(next);
            self.documents[3].find(self.find.clone());
            return;
        }
        let matches = &self.find_highlights.matches;
        if matches.is_empty() {
            self.message("No matching text in this article");
            return;
        }
        self.find_index = if next {
            (self.find_index + 1) % matches.len()
        } else {
            0
        };
        if let Some(surface) = &mut self.surfaces[3]
            && let Some(rect) = matches[self.find_index].first()
        {
            surface.scroll = surface
                .doc
                .clamp_scroll(CssPoint::new(surface.scroll.x, (rect.y - 40.0).max(0.0)));
        }
        self.message(format!(
            "Match {} of {}",
            self.find_index + 1,
            matches.len()
        ));
    }
    fn ancestor_attr(&self, index: usize, node: usize, name: &str) -> Option<String> {
        self.surfaces[index]
            .as_ref()?
            .doc
            .ancestor_attribute(node, name)
            .map(str::to_owned)
    }
    fn interactive_target(&self, index: usize, mut node: usize) -> Option<usize> {
        let doc = &self.surfaces[index].as_ref()?.doc;
        loop {
            if doc.dom.attr(node, "href").is_some() || doc.dom.tag_name(node) == Some("summary") {
                return Some(node);
            }
            node = doc.dom.node(node).parent?;
        }
    }
    fn charm_hit(&self, point: CssPoint) -> Option<crate::decora::CharmHit> {
        self.charm_hit_with_target(point, self.hit(point))
    }
    fn charm_hit_with_target(
        &self,
        point: CssPoint,
        target: Option<(usize, usize)>,
    ) -> Option<crate::decora::CharmHit> {
        if self.dialog.is_some() {
            return None;
        }
        if target.is_some_and(|(i, node)| self.interactive_target(i, node).is_some()) {
            return None; // Functional controls always take priority.
        }
        for i in (0..4).rev() {
            if let Some(surface) = &self.surfaces[i]
                && surface.rect.contains(point)
            {
                if i == 2 || (i == 3 && self.article.is_some()) {
                    return None;
                }
                return Decora::hit(surface.scene.as_ref()?, &surface.doc, point);
            }
        }
        None
    }
    fn hit(&self, point: CssPoint) -> Option<(usize, usize)> {
        for i in (0..5).rev() {
            if self.dialog.is_some() && i != 4 {
                continue;
            }
            if let Some(s) = &self.surfaces[i]
                && (i != 3 || self.current_surface(i))
                && s.rect.contains(point)
                && let Some(hit) = s.doc.hit(
                    CssPoint::new(point.x - s.rect.x, point.y - s.rect.y),
                    s.scroll,
                )
            {
                return Some((i, hit.node));
            }
        }
        None
    }
    fn activate(&mut self, index: usize, node: usize, event_loop: &ActiveEventLoop) {
        // App controls dispatch the trusted href in the visible snapshot,
        // including while a new status/layout is being prepared. Only article
        // mutations require node IDs from the current document generation.
        if (index == 3 && !self.current_surface(index))
            || !self.surfaces[index]
                .as_ref()
                .is_some_and(|s| s.doc.dom.is_valid(node))
        {
            return;
        }
        if index == 3 {
            if self.article.is_none() {
                return;
            } // Welcome art uses pixel fidget hits only.
            if let Some(href) = self.ancestor_attr(index, node, "href") {
                let href = self.surfaces[index]
                    .as_ref()
                    .and_then(|s| s.doc.base().join(&href).ok())
                    .map(|url| url.to_string())
                    .unwrap_or(href);
                if let Ok(url) = url::Url::parse(&href) {
                    let base = self
                        .article
                        .as_ref()
                        .and_then(|a| url::Url::parse(&a.url).ok());
                    let mut plain = url.clone();
                    plain.set_fragment(None);
                    let same = base.is_some_and(|mut b| {
                        b.set_fragment(None);
                        b == plain
                    });
                    if same && url.fragment().is_some() {
                        if let Some(surface) = &mut self.surfaces[3] {
                            let target = surface
                                .doc
                                .dom
                                .descendants(trust::dom::DOCUMENT)
                                .find(|&n| surface.doc.dom.attr(n, "id") == url.fragment());
                            if let Some(rect) =
                                target.and_then(|n| surface.doc.layout.boxes.get(&n))
                            {
                                surface.scroll = surface
                                    .doc
                                    .clamp_scroll(CssPoint::new(0.0, rect.top as f32));
                                self.redraw();
                            }
                        }
                        return;
                    }
                }
                platform::open_url(&href);
            } else if let Some(src) = self.ancestor_attr(index, node, "src") {
                self.open_dialog(Dialog::Image(src), vec![]);
            } else {
                self.documents[3].toggle(node);
            }
        } else if let Some(href) = self.ancestor_attr(index, node, "href")
            && let Some(action) = href.strip_prefix("app:")
        {
            self.action(action, event_loop);
        }
    }
    fn wheel(&mut self, dx: f32, dy: f32) {
        let index = if self.dialog.is_some() {
            4
        } else if self.geometry.headlines.contains(self.pointer) && !self.focus_mode {
            2
        } else if self.geometry.sidebar.contains(self.pointer) && !self.focus_mode {
            1
        } else {
            3
        };
        if let Some(s) = &mut self.surfaces[index] {
            let (mut dx, mut dy) = (dx, dy);
            if let Some(container) = s
                .scene
                .as_ref()
                .and_then(|scene| scene.scroll_container_at(self.pointer))
                .cloned()
                && let Some(c) = s
                    .doc
                    .layout
                    .paint
                    .scroll_containers
                    .iter_mut()
                    .find(|c| c.node == container.node)
            {
                let next = CssPoint::new(
                    (c.offset.x + dx).clamp(0.0, (c.content.width - c.viewport.width).max(0.0)),
                    (c.offset.y + dy).clamp(0.0, (c.content.height - c.viewport.height).max(0.0)),
                );
                dx -= next.x - c.offset.x;
                dy -= next.y - c.offset.y;
                c.offset = next;
                s.cached = None;
                self.documents[index].scroll(c.node, next);
            }
            s.scroll = s
                .doc
                .clamp_scroll(CssPoint::new(s.scroll.x + dx, s.scroll.y + dy));
        }
        self.redraw();
    }
    fn focus_next(&mut self, reverse: bool) {
        let mut targets = Vec::new();
        for i in 0..5 {
            if self.dialog.is_some() && i != 4 {
                continue;
            }
            if i == 3 && !self.current_surface(i) {
                continue;
            }
            if let Some(surface) = &self.surfaces[i] {
                for node in surface.doc.dom.descendants(trust::dom::DOCUMENT) {
                    if (surface.doc.dom.attr(node, "href").is_some()
                        || surface.doc.dom.tag_name(node) == Some("summary"))
                        && surface.doc.layout.boxes.contains_key(&node)
                    {
                        targets.push((i, node));
                    }
                }
            }
        }
        if targets.is_empty() {
            return;
        }
        let current = self
            .focus_target
            .and_then(|t| targets.iter().position(|n| *n == t));
        let i = match (current, reverse) {
            (Some(i), false) => (i + 1) % targets.len(),
            (Some(i), true) => (i + targets.len() - 1) % targets.len(),
            (None, false) => 0,
            (None, true) => targets.len() - 1,
        };
        self.commit_editor();
        self.focus_target = Some(targets[i]);
        self.keyboard_focus = true;
        let (index, node) = targets[i];
        if let Some(s) = &mut self.surfaces[index]
            && let Some(rect) = s.doc.layout.boxes.get(&node)
        {
            let (top, height) = (rect.top as f32, rect.height as f32);
            if top < s.scroll.y {
                s.scroll.y = top;
            } else if top + height > s.scroll.y + s.rect.height {
                s.scroll.y = top + height - s.rect.height;
            }
            s.scroll = s.doc.clamp_scroll(s.scroll);
        }
        if let Some(href) = self.ancestor_attr(index, node, "href")
            && let Some(i) = href
                .strip_prefix("app:field:")
                .and_then(|s| s.parse::<usize>().ok())
        {
            self.focus_field(i);
        }
        self.redraw();
    }
    fn copy(&mut self, cut: bool) {
        let text = if let Some((_, editor)) = &mut self.editor {
            let text = editor.selected_text().map(str::to_owned);
            if cut {
                editor.delete_selection();
            }
            text
        } else {
            self.selection.and_then(|(i, selection)| {
                self.surfaces[i]
                    .as_ref()?
                    .scene
                    .as_ref()
                    .map(|scene| scene.selected_text(selection))
            })
        };
        if let Some(text) = text
            && let Ok(mut clipboard) = arboard::Clipboard::new()
        {
            let _ = clipboard.set_text(text);
        }
        self.redraw();
    }
    fn keyboard(&mut self, event: winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        let control = self.modifiers.control_key() || self.modifiers.super_key();
        let character = match &event.logical_key {
            NativeKey::Character(s) => s.to_lowercase(),
            _ => String::new(),
        };
        if control {
            match character.as_str() {
                "q" => {
                    self.action("quit", event_loop);
                    return;
                }
                "c" => {
                    self.copy(false);
                    return;
                }
                "x" => {
                    self.copy(true);
                    return;
                }
                "v" => {
                    if let Some((_, editor)) = &mut self.editor
                        && let Ok(mut clip) = arboard::Clipboard::new()
                        && let Ok(text) = clip.get_text()
                    {
                        editor.replace_selection(&text);
                    }
                    self.redraw();
                    return;
                }
                "a" => {
                    if let Some((_, editor)) = &mut self.editor {
                        editor.select_all();
                    }
                    self.redraw();
                    return;
                }
                "n" => {
                    self.action("add", event_loop);
                    return;
                }
                "k" => {
                    self.action("search", event_loop);
                    return;
                }
                "f" => {
                    self.action("find", event_loop);
                    return;
                }
                _ => {}
            }
        }
        if event.logical_key == NativeKey::Named(NamedKey::Escape) {
            if self.dialog.is_some() {
                self.close_dialog();
            } else {
                self.focus_mode = false;
                self.find.clear();
                self.selection = None;
                self.dirty = true;
                self.redraw();
            }
            return;
        }
        if event.logical_key == NativeKey::Named(NamedKey::Tab) {
            self.focus_next(self.modifiers.shift_key());
            return;
        }
        if event.logical_key == NativeKey::Named(NamedKey::Enter) {
            if self.editor.is_some() {
                self.submit();
            } else if let Some((i, node)) = self.focus_target {
                self.activate(i, node, event_loop);
            } else {
                self.submit();
            }
            return;
        }
        if let Some((_, editor)) = &mut self.editor {
            let input = KeyInput {
                key: translate_key(&event.logical_key),
                code: match event.physical_key {
                    winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::SuperLeft) => {
                        String::from("MetaLeft")
                    }
                    winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::SuperRight) => {
                        String::from("MetaRight")
                    }
                    winit::keyboard::PhysicalKey::Code(code) => format!("{code:?}"),
                    _ => String::new(),
                },
                location: match event.location {
                    winit::keyboard::KeyLocation::Standard => 0,
                    winit::keyboard::KeyLocation::Left => 1,
                    winit::keyboard::KeyLocation::Right => 2,
                    winit::keyboard::KeyLocation::Numpad => 3,
                },
                state: KeyState::Pressed,
                modifiers: Modifiers {
                    shift: self.modifiers.shift_key(),
                    control,
                    alt: self.modifiers.alt_key(),
                    meta: self.modifiers.super_key(),
                },
                repeat: event.repeat,
                composing: editor.is_composing(),
            };
            if !editor.handle_key(&input)
                && !control
                && !self.modifiers.alt_key()
                && let Some(text) = event.text
                && !text.chars().any(char::is_control)
            {
                editor.replace_selection(&text);
            }
            self.redraw();
            return;
        }
        if self.dialog.is_some() {
            return;
        }
        match &event.logical_key {
            NativeKey::Named(NamedKey::ArrowDown) => self.next_article(1),
            NativeKey::Named(NamedKey::ArrowUp) => self.next_article(-1),
            NativeKey::Named(NamedKey::PageDown) | NativeKey::Named(NamedKey::Space) => {
                let d = if self.modifiers.shift_key() {
                    -1.0
                } else {
                    1.0
                };
                if let Some(s) = &mut self.surfaces[3] {
                    s.scroll = s.doc.clamp_scroll(CssPoint::new(
                        s.scroll.x,
                        s.scroll.y + d * s.rect.height * 0.85,
                    ));
                }
                self.redraw();
            }
            NativeKey::Named(NamedKey::PageUp) => {
                if let Some(s) = &mut self.surfaces[3] {
                    s.scroll = s
                        .doc
                        .clamp_scroll(CssPoint::new(s.scroll.x, s.scroll.y - s.rect.height * 0.85));
                }
                self.redraw();
            }
            NativeKey::Named(NamedKey::F3) => self.jump_find(true),
            NativeKey::Named(NamedKey::Home) => {
                if let Some(s) = &mut self.surfaces[3] {
                    s.scroll.y = 0.0;
                }
                self.redraw();
            }
            NativeKey::Named(NamedKey::End) => {
                if let Some(s) = &mut self.surfaces[3] {
                    s.scroll = s.doc.clamp_scroll(CssPoint::new(s.scroll.x, f32::MAX));
                }
                self.redraw();
            }
            _ => match character.as_str() {
                "j" => self.next_article(1),
                "k" => self.next_article(-1),
                "s" => self.action("save", event_loop),
                "m" => self.action("read", event_loop),
                "f" => self.action("focus", event_loop),
                "o" => self.action("original", event_loop),
                "r" => self.action("refresh", event_loop),
                _ => {}
            },
        }
    }
}

fn translate_key(key: &NativeKey) -> Key {
    match key {
        NativeKey::Character(s) => Key::Character(s.to_string()),
        NativeKey::Named(n) => match n {
            NamedKey::Enter => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::Tab => Key::Tab,
            NamedKey::ArrowLeft => Key::ArrowLeft,
            NamedKey::ArrowRight => Key::ArrowRight,
            NamedKey::ArrowUp => Key::ArrowUp,
            NamedKey::ArrowDown => Key::ArrowDown,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            _ => Key::Other(format!("{n:?}")),
        },
        _ => Key::Other(String::new()),
    }
}

impl App {
    fn update_accessibility(&mut self) {
        use accesskit::{Action, Node, NodeId, Rect, Role, Tree, TreeUpdate};
        if !self.accessibility_dirty {
            return;
        }
        self.accessibility_dirty = false;
        let Some(mut adapter) = self.accessibility.take() else {
            return;
        };
        // Building the semantic tree is only useful when an assistive client
        // is listening. Ambient sparkle frames must not walk every article.
        adapter.update_if_active(|| {
            let mut nodes = Vec::new();
            let mut root = Node::new(Role::Window);
            root.set_label("Ruby Reader");
            let mut children = Vec::new();
            for i in 0..5 {
                if self.dialog.is_some() && i != 4 {
                    continue;
                }
                let Some(surface) = &self.surfaces[i] else {
                    continue;
                };
                let Some(semantics) = &surface.doc.semantics else {
                    self.documents[i].request_semantics();
                    continue;
                };
                let offset = (i as u64 + 1) * 1_000_000;
                for semantic in &semantics.nodes {
                    let sid = NodeId(offset + semantic.id);
                    let role = match semantic.role {
                        trust::accessibility::Role::Heading => Role::Heading,
                        trust::accessibility::Role::Paragraph => Role::Paragraph,
                        trust::accessibility::Role::Link => Role::Link,
                        trust::accessibility::Role::Button => Role::Button,
                        trust::accessibility::Role::Image => Role::Image,
                        trust::accessibility::Role::Table => Role::Table,
                        trust::accessibility::Role::Row => Role::Row,
                        trust::accessibility::Role::Cell => Role::Cell,
                        trust::accessibility::Role::List => Role::List,
                        trust::accessibility::Role::ListItem => Role::ListItem,
                        _ => Role::GenericContainer,
                    };
                    let mut node = Node::new(role);
                    if !semantic.name.is_empty() {
                        node.set_label(semantic.name.as_str());
                    }
                    let r = semantic.bounds;
                    node.set_bounds(Rect::new(
                        (r.x + surface.rect.x - surface.scroll.x) as f64,
                        (r.y + surface.rect.y - surface.scroll.y) as f64,
                        (r.x + r.width + surface.rect.x - surface.scroll.x) as f64,
                        (r.y + r.height + surface.rect.y - surface.scroll.y) as f64,
                    ));
                    if let Some(dom_node) = semantic.dom_node {
                        if surface.doc.dom.attr(dom_node, "href").is_some()
                            || surface.doc.dom.tag_name(dom_node) == Some("summary")
                        {
                            node.add_action(Action::Click);
                            node.add_action(Action::Focus);
                        }
                        if let Some(index) = surface
                            .doc
                            .dom
                            .attr(dom_node, "id")
                            .and_then(|id| id.strip_prefix("field-"))
                            .and_then(|s| s.parse::<usize>().ok())
                        {
                            node.set_role(Role::TextInput);
                            if let Some((label, value)) = self.fields.get(index) {
                                node.set_label(label.as_str());
                                node.set_value(value.as_str());
                            }
                            node.add_action(Action::SetValue);
                        }
                    }
                    node.set_children(
                        semantic
                            .children
                            .iter()
                            .map(|id| NodeId(offset + id))
                            .collect::<Vec<_>>(),
                    );
                    nodes.push((sid, node));
                }
                children.push(NodeId(offset + 1));
            }
            root.set_children(children);
            nodes.push((NodeId(0), root));
            let focus = self
                .focus_target
                .map(|(i, node)| {
                    NodeId(
                        (i as u64 + 1) * 1_000_000
                            + trust::accessibility::SemanticTree::DOM_BASE
                            + node as u64,
                    )
                })
                .unwrap_or(NodeId(0));
            let focus = if nodes.iter().any(|(id, _)| *id == focus) {
                focus
            } else {
                NodeId(0)
            };
            TreeUpdate {
                nodes,
                tree: Some(Tree::new(NodeId(0))),
                focus,
                tree_id: accesskit::TreeId::ROOT,
            }
        });
        self.accessibility = Some(adapter);
    }
    fn pointer_move(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        self.pending_pointer = None;
        self.pointer = self.metrics.physical_to_css(position.x, position.y);
        if let Some(drag) = self.drag {
            if drag == 0 {
                self.settings.sidebar_width = self.pointer.x - 14.0;
                self.dirty = true;
            } else if drag == 1 {
                self.settings.headline_height = self.pointer.y - 188.0;
                self.dirty = true;
            } else {
                let index = ((drag - 10) / 2) as usize;
                let horizontal = (drag - 10) % 2 == 1;
                if let Some(s) = &mut self.surfaces[index] {
                    if horizontal {
                        s.scroll.x = ((self.pointer.x - s.rect.x) / s.rect.width)
                            * (s.doc.layout.paint.width - s.rect.width).max(0.0);
                    } else {
                        s.scroll.y = ((self.pointer.y - s.rect.y) / s.rect.height)
                            * (s.doc.layout.paint.height - s.rect.height).max(0.0);
                    }
                    s.scroll = s.doc.clamp_scroll(s.scroll);
                }
            }
            self.redraw();
            return;
        }
        if self.mouse_down
            && let Some((index, selection)) = &mut self.selection
            && let Some(position) = self.surfaces[*index]
                .as_ref()
                .and_then(|s| s.scene.as_ref()?.text_position_at(self.pointer))
        {
            selection.focus = position;
            self.redraw();
        }
        let hit = self.hit(self.pointer);
        for i in 0..5 {
            if self.surfaces[i].is_some() {
                let target = hit.filter(|(index, _)| *index == i).map(|(_, node)| node);
                if i != 3 && self.current_surface(i) {
                    self.documents[i].hover(target);
                }
            }
        }
        let cursor = if self.dialog.is_none()
            && !self.focus_mode
            && self.geometry.vertical.contains(self.pointer)
        {
            CursorIcon::ColResize
        } else if self.dialog.is_none()
            && !self.focus_mode
            && self.geometry.horizontal.contains(self.pointer)
        {
            CursorIcon::RowResize
        } else if self.charm_hit_with_target(self.pointer, hit).is_some()
            || hit.is_some_and(|(i, node)| self.ancestor_attr(i, node, "href").is_some())
        {
            CursorIcon::Pointer
        } else if self.geometry.article.contains(self.pointer) {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        if let Some(w) = &self.window {
            w.set_cursor(cursor);
        }
    }
    fn mouse(&mut self, state: ElementState, event_loop: &ActiveEventLoop) {
        if let Some(position) = self.pending_pointer.take() {
            self.pointer_move(position);
        }
        if state == ElementState::Pressed {
            self.keyboard_focus = false;
            self.mouse_down = true;
            self.press = self
                .hit(self.pointer)
                .map(|(i, node)| (i, node, self.pointer));
            if self.dialog.is_none() && !self.focus_mode {
                if self.geometry.vertical.contains(self.pointer) {
                    self.drag = Some(0);
                    return;
                }
                if self.geometry.horizontal.contains(self.pointer) {
                    self.drag = Some(1);
                    return;
                }
            }
            for i in (1..5).rev() {
                if self.dialog.is_some() && i != 4 {
                    continue;
                }
                if let Some(s) = &self.surfaces[i]
                    && s.rect.contains(self.pointer)
                {
                    if self.pointer.x > s.rect.x + s.rect.width - 10.0
                        && s.doc.layout.paint.height > s.rect.height
                    {
                        self.drag = Some(10 + i as u8 * 2);
                        return;
                    }
                    if self.pointer.y > s.rect.y + s.rect.height - 10.0
                        && s.doc.layout.paint.width > s.rect.width
                    {
                        self.drag = Some(11 + i as u8 * 2);
                        return;
                    }
                }
            }
            self.selection = None;
            self.charm_press = self
                .charm_hit(self.pointer)
                .map(|hit| (hit.name, self.pointer));
            if self.charm_press.is_some() {
                self.press = None;
                self.focus_target = None;
                self.redraw();
                return;
            }
            if let Some((i, node, _)) = self.press {
                self.focus_target = self.interactive_target(i, node).map(|node| (i, node));
                if i == 3
                    && let Some(position) = self.surfaces[i]
                        .as_ref()
                        .and_then(|s| s.scene.as_ref()?.text_position_at(self.pointer))
                {
                    self.selection = Some((
                        i,
                        TextSelection {
                            anchor: position,
                            focus: position,
                        },
                    ));
                }
            }
        } else {
            self.mouse_down = false;
            if let Some((name, start)) = self.charm_press.take() {
                if (start.x - self.pointer.x).abs() < 5.0 && (start.y - self.pointer.y).abs() < 5.0
                {
                    self.decora.poke(name, self.started.elapsed().as_secs_f32());
                }
                self.redraw();
                return;
            }
            if let Some(drag) = self.drag.take() {
                if drag < 2 {
                    self.persist_settings();
                }
                return;
            }
            if let Some((index, node, start)) = self.press.take()
                && (start.x - self.pointer.x).abs() < 5.0
                && (start.y - self.pointer.y).abs() < 5.0
            {
                self.activate(index, node, event_loop);
            }
        }
        self.redraw();
    }
}

impl App {
    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let icon = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
            .ok()
            .and_then(|i| {
                let image = i.to_rgba8();
                winit::window::Icon::from_rgba(
                    image.as_raw().clone(),
                    image.width(),
                    image.height(),
                )
                .ok()
            });
        let title = if self.smoke {
            format!("Ruby Reader test {}", std::process::id())
        } else {
            "Ruby Reader ♡".into()
        };
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_theme(Some(
                Theme::from_name(&self.settings.reading_theme).window_theme(),
            ))
            .with_inner_size(LogicalSize::new(
                self.settings.window_width,
                self.settings.window_height,
            ))
            .with_min_inner_size(LogicalSize::new(
                860.0 * self.settings.ui_scale as f64,
                650.0 * self.settings.ui_scale as f64,
            ))
            .with_window_icon(icon)
            .with_visible(false);
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("Could not open Ruby Reader: {e}");
                event_loop.exit();
                return;
            }
        };
        {
            let _guard = self.backend.rt.enter();
            self.accessibility = Some(accesskit_winit::Adapter::with_event_loop_proxy(
                event_loop,
                &window,
                self.backend.proxy.clone(),
            ));
        }
        self.presenter = if self.renderer_preference != "cpu" {
            match self.backend.rt.block_on(
                trust::render::vello_hybrid::VelloHybridRenderer::new_window(window.clone()),
            ) {
                Ok(gpu) => {
                    eprintln!("Ruby Reader renderer: {}", gpu.adapter_name());
                    Some(Presenter::Gpu(Box::new(gpu)))
                }
                Err(e) => {
                    if self.renderer_preference == "hybrid" {
                        eprintln!("Hybrid unavailable: {e}");
                        event_loop.exit();
                        return;
                    }
                    Presenter::cpu(window.clone()).ok()
                }
            }
        } else {
            Presenter::cpu(window.clone()).ok()
        };
        if self.presenter.is_none() {
            eprintln!("No native presenter available");
            event_loop.exit();
            return;
        }
        self.window = Some(window.clone());
        self.visible = true;
        self.occluded = false;
        self.dirty = true;
        self.metrics_for_window();
        for document in &self.documents {
            document.restore_assets();
        }
        window.set_ime_allowed(self.editor.is_some());
        window.set_visible(true);
        self.redraw();
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // A hidden-to-tray app must not reopen on redundant resume callbacks.
        if self.initialized {
            return;
        }
        self.initialized = true;
        self.tray = platform::start_tray(&self.backend);
        self.create_window(event_loop);
        self.reload();
        self.backend.refresh(None, false);
        event_loop.set_control_flow(ControlFlow::Wait);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Snapshot(serial, result) => {
                if serial != self.serial {
                    return;
                }
                match result {
                    Ok(snap) => {
                        platform::update_tray(
                            &self.backend,
                            &self.tray,
                            snap.unread,
                            if self
                                .backend
                                .refreshing
                                .load(std::sync::atomic::Ordering::SeqCst)
                            {
                                "Refreshing"
                            } else {
                                "Ready"
                            }
                            .into(),
                        );
                        self.snapshot = Some(snap);
                        self.dirty = true;
                    }
                    Err(e) => self.message(e),
                }
            }
            Event::Article(id, serial, result) => {
                if self.selected != Some(id) || self.article_serial != serial {
                    return;
                }
                match result {
                    Ok(article) => {
                        let scroll = article.scroll;
                        self.article = Some(article);
                        self.read_since = None;
                        self.dirty = true;
                        self.article_scroll = Some(CssPoint::new(0.0, scroll));
                        self.fetch_full_article(false);
                        self.rebuild();
                    }
                    Err(e) => self.message(e),
                }
            }
            Event::Extracted(id, serial, result) => {
                if !self.full_article.complete(id, serial) || self.selected != Some(id) {
                    return;
                }
                match result {
                    Ok(html) => {
                        if let Some(article) = &mut self.article {
                            article.extracted = Some(html);
                        }
                        self.message("Full article retrieved");
                    }
                    Err(e) => self.message(format!("Couldn’t load the full article: {e}")),
                }
            }
            Event::Changed(result) => {
                match result {
                    Ok(message) => {
                        if !message.is_empty() {
                            self.message(message)
                        }
                    }
                    Err(e) => self.message(e),
                }
                self.reload();
            }
            Event::Discovered(result) => match result {
                Ok(feeds) => {
                    if feeds.len() == 1 {
                        let feed = feeds[0].clone();
                        self.backend.subscribe(feed, self.folder);
                    } else {
                        self.open_dialog(Dialog::Discover(feeds), vec![])
                    }
                }
                Err(e) => self.message(e),
            },
            Event::Subscribed(result) => match result {
                Ok(id) => {
                    self.choose_view(View::Feed(id));
                    self.backend.refresh(Some(id), false);
                    self.message("Subscription added · gathering its stories…");
                    // Also pick up this feed promptly if another refresh is
                    // already running and coalesced the targeted request.
                    self.last_refresh = Instant::now() - Duration::from_secs(59);
                }
                Err(e) => self.message(e),
            },
            Event::RefreshDone { new, failed } => {
                self.message(if failed > 0 {
                    format!("Refresh complete · {new} new stories · {failed} feeds need attention")
                } else if new > 0 {
                    format!("{new} new little discoveries are waiting for you")
                } else {
                    "Your collection is up to date".into()
                });
                if new > 0 && self.settings.notifications {
                    platform::notify(&self.backend, new);
                }
                self.reload();
            }
            Event::DocumentReady(index) => self.document_ready(index),
            Event::Open => self.show(event_loop),
            Event::Quit => self.action("quit", event_loop),
            Event::Refresh => self.action("refresh", event_loop),
            Event::TrayStatus(online) => {
                self.tray_online = online;
                if !online && self.window.is_none() {
                    self.show(event_loop);
                    self.message("The tray disappeared; your reading window has been restored");
                }
            }
            Event::Access(event)
                if self
                    .window
                    .as_ref()
                    .is_some_and(|w| w.id() == event.window_id) =>
            {
                match event.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        self.accessibility_dirty = true;
                        self.update_accessibility()
                    }
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        let raw = request.target_node.0;
                        let index = (raw / 1_000_000).saturating_sub(1) as usize;
                        let node = raw % 1_000_000;
                        if index < 5 && node >= trust::accessibility::SemanticTree::DOM_BASE {
                            let node =
                                (node - trust::accessibility::SemanticTree::DOM_BASE) as usize;
                            match request.action {
                                accesskit::Action::Click => self.activate(index, node, event_loop),
                                accesskit::Action::Focus => {
                                    self.focus_target = Some((index, node));
                                    self.keyboard_focus = true;
                                    if let Some(href) = self.ancestor_attr(index, node, "href")
                                        && let Some(i) = href
                                            .strip_prefix("app:field:")
                                            .and_then(|s| s.parse::<usize>().ok())
                                    {
                                        self.focus_field(i);
                                    }
                                }
                                accesskit::Action::SetValue => {
                                    if let Some(accesskit::ActionData::Value(value)) = request.data
                                        && let Some(href) = self.ancestor_attr(index, node, "href")
                                        && let Some(i) = href
                                            .strip_prefix("app:field:")
                                            .and_then(|s| s.parse::<usize>().ok())
                                    {
                                        if let Some(f) = self.fields.get_mut(i) {
                                            f.1 = value.to_string();
                                        }
                                        self.focus_field(i);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Access(_) => {}
        }
        self.redraw();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Destruction can leave queued events for the previous native window.
        if self.window.as_ref().is_none_or(|window| window.id() != id) {
            return;
        }
        if let (Some(adapter), Some(window)) = (&mut self.accessibility, &self.window) {
            adapter.process_event(window, &event);
        }
        if !matches!(
            &event,
            WindowEvent::RedrawRequested | WindowEvent::CursorMoved { .. }
        ) {
            self.accessibility_dirty = true;
        }
        match event {
            WindowEvent::CloseRequested => self.close_window(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(w) = &self.window {
                        let scale = w.scale_factor();
                        self.settings.window_width = (size.width as f64 / scale) as u32;
                        self.settings.window_height = (size.height as f64 / scale) as u32;
                    }
                    self.dirty = true;
                    self.redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.dirty = true;
                self.redraw();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.read_since = None;
            }
            WindowEvent::Occluded(hidden) => {
                self.occluded = hidden;
                self.read_since = None;
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = self.metrics.physical_to_css(position.x, position.y);
                self.pending_pointer = Some(position);
            }
            WindowEvent::CursorLeft { .. } => {
                self.pending_pointer = None;
                self.pointer = CssPoint::new(-1.0, -1.0);
                for document in &self.documents {
                    document.hover(None);
                }
                self.redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.mouse(state, event_loop),
            WindowEvent::MouseInput { state, button, .. } => self.mouse_navigation(button, state),
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * 36.0, -y * 36.0),
                    MouseScrollDelta::PixelDelta(p) => {
                        let s = self.metrics.scale_factor.get();
                        ((-p.x / s) as f32, (-p.y / s) as f32)
                    }
                };
                if self.modifiers.shift_key() {
                    self.wheel(dy + dx, 0.0)
                } else {
                    self.wheel(dx, dy)
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.keyboard(event, event_loop),
            WindowEvent::Ime(ime) => {
                if let Some((_, editor)) = &mut self.editor {
                    editor.handle_ime(&match ime {
                        Ime::Enabled => ImeAction::Enabled,
                        Ime::Disabled => ImeAction::Disabled,
                        Ime::Commit(text) => ImeAction::Commit(text),
                        Ime::Preedit(text, cursor) => ImeAction::Preedit { text, cursor },
                    });
                    self.redraw();
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.last_pointer.elapsed() >= Duration::from_millis(16)
            && let Some(position) = self.pending_pointer.take()
        {
            self.pointer_move(position);
            self.last_pointer = now;
        }
        if self.last_refresh.elapsed() >= Duration::from_secs(60) {
            self.backend.refresh(None, true);
            self.last_refresh = now;
        }
        let can_read = self.visible
            && !self.occluded
            && self.focused
            && self.dialog.is_none()
            && self.manually_unread != self.selected
            && self.current_surface(3)
            && self.requests[3].as_ref().is_none_or(|r| !r.failed)
            && self.article.as_ref().is_some_and(|a| !a.read);
        if can_read {
            if let Some(since) = self.read_since {
                if since.elapsed() >= Duration::from_secs(2)
                    && let Some(a) = &mut self.article
                {
                    a.read = true;
                    let id = a.id;
                    self.backend.change(move |db| {
                        db.read(id, true)?;
                        Ok(String::new())
                    });
                    self.dirty = true;
                    self.read_since = None;
                    self.redraw();
                }
            } else {
                self.read_since = Some(now);
            }
        } else {
            self.read_since = None;
        }
        let can_animate = self.visible
            && !self.occluded
            && self.dialog.is_none()
            && ((!self.settings.reduced_motion && !self.focus_mode)
                || self.decora.active(self.started.elapsed().as_secs_f32()));
        if can_animate && self.last_frame.elapsed() >= Duration::from_millis(33) {
            self.redraw();
        }
        let delay = if can_animate {
            Duration::from_millis(33)
        } else if can_read {
            Duration::from_millis(100)
        } else {
            if self.smoke {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(60)
                    .saturating_sub(self.last_refresh.elapsed())
                    .max(Duration::from_millis(100))
            }
        };
        let delay = if self.pending_pointer.is_some() {
            delay.min(Duration::from_millis(16).saturating_sub(self.last_pointer.elapsed()))
        } else {
            delay
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + delay));
        if self.smoke && self.smoke_time.elapsed() > Duration::from_secs(2) {
            match self.smoke_step {
                0 => {
                    self.smoke_capture("welcome");
                    // Feed controls reveal through the actual pointer path,
                    // remain clickable, and can also be reached with Tab.
                    let surface = self.surfaces[1].as_ref().unwrap();
                    let feed = surface
                        .doc
                        .dom
                        .descendants(trust::dom::DOCUMENT)
                        .find(|&n| {
                            surface
                                .doc
                                .dom
                                .attr(n, "href")
                                .is_some_and(|h| h.starts_with("app:feed:"))
                        })
                        .expect("Smoke: sample subscription");
                    let edit = surface
                        .doc
                        .dom
                        .descendants(trust::dom::DOCUMENT)
                        .find(|&n| {
                            surface
                                .doc
                                .dom
                                .attr(n, "href")
                                .is_some_and(|h| h.starts_with("app:feed-edit:"))
                        })
                        .unwrap();
                    let actions = surface.doc.dom.node(edit).parent.unwrap();
                    assert_eq!(surface.doc.dom.node(actions).opacity, 0.0);
                    let feed_rect = node_rect(surface, feed).unwrap();
                    let scale = self.metrics.scale_factor.get();
                    self.pointer_move(PhysicalPosition::new(
                        (feed_rect.x + 30.0) as f64 * scale,
                        (feed_rect.y + feed_rect.height * 0.5) as f64 * scale,
                    ));
                    self.smoke_capture("feed-hover");
                    let surface = self.surfaces[1].as_ref().unwrap();
                    assert_eq!(surface.doc.dom.node(actions).opacity, 1.0);
                    let edit_rect = node_rect(surface, edit).unwrap();
                    self.pointer_move(PhysicalPosition::new(
                        (edit_rect.x + edit_rect.width * 0.5) as f64 * scale,
                        (edit_rect.y + edit_rect.height * 0.5) as f64 * scale,
                    ));
                    self.mouse(ElementState::Pressed, event_loop);
                    self.mouse(ElementState::Released, event_loop);
                    assert!(
                        matches!(self.dialog, Some(Dialog::EditFeed(_))),
                        "Smoke: hovered edit is clickable"
                    );
                    self.smoke_click("cancel", event_loop);
                    self.pointer_move(PhysicalPosition::new(420.0 * scale, 105.0 * scale));
                    self.focus_target = Some((1, feed));
                    self.focus_next(false);
                    self.smoke_settle_documents();
                    self.compose();
                    assert_eq!(
                        self.focus_target,
                        Some((1, edit)),
                        "Smoke: Tab reaches edit"
                    );
                    assert_eq!(
                        self.surfaces[1]
                            .as_ref()
                            .unwrap()
                            .doc
                            .dom
                            .node(actions)
                            .opacity,
                        1.0
                    );
                    self.smoke_capture("feed-keyboard");
                    self.focus_target = None;
                    self.keyboard_focus = false;
                    self.smoke_settle_documents();
                    self.compose();
                    assert_eq!(
                        self.surfaces[1]
                            .as_ref()
                            .unwrap()
                            .doc
                            .dom
                            .node(actions)
                            .opacity,
                        0.0
                    );
                    // Actual pointer path: blank decoration cannot become a
                    // focus box, and a visible charm is a local fidget, not a
                    // link, article selection, or image enlargement.
                    self.pointer_move(PhysicalPosition::new(420.0 * scale, 105.0 * scale));
                    self.mouse(ElementState::Pressed, event_loop);
                    self.mouse(ElementState::Released, event_loop);
                    assert!(self.focus_target.is_none());
                    for class in ["charm-board", "charm-chain"] {
                        let surface = self.surfaces[1].as_ref().unwrap();
                        let node = surface
                            .doc
                            .dom
                            .descendants(trust::dom::DOCUMENT)
                            .find(|n| surface.doc.dom.attr(*n, "class") == Some(class))
                            .unwrap();
                        let rect = node_rect(surface, node).unwrap();
                        self.pointer_move(PhysicalPosition::new(
                            (rect.x + 3.0) as f64 * scale,
                            (rect.y + 3.0) as f64 * scale,
                        ));
                        self.mouse(ElementState::Pressed, event_loop);
                        self.mouse(ElementState::Released, event_loop);
                        assert!(
                            self.focus_target.is_none(),
                            "Decorative containers never take focus"
                        );
                    }
                    let selected = self.selected;
                    let point = (430..(self.geometry.width as i32 - 20))
                        .step_by(5)
                        .flat_map(|x| {
                            (35..95)
                                .step_by(5)
                                .map(move |y| CssPoint::new(x as f32, y as f32))
                        })
                        .find(|p| self.charm_hit(*p).is_some())
                        .expect("Smoke: visible charm pixels");
                    self.pointer_move(PhysicalPosition::new(
                        point.x as f64 * scale,
                        point.y as f64 * scale,
                    ));
                    self.mouse(ElementState::Pressed, event_loop);
                    self.mouse(ElementState::Released, event_loop);
                    assert!(self.decora.active(self.started.elapsed().as_secs_f32()));
                    assert_eq!(self.selected, selected);
                    assert!(self.dialog.is_none() && self.focus_target.is_none());
                    self.smoke_capture("fidget");
                    if let Some(id) = self
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.articles.first())
                        .map(|a| a.id)
                    {
                        self.smoke_click(&format!("article:{id}"), event_loop);
                    } else {
                        panic!("Smoke needs sample articles; use --sample");
                    }
                    self.message("Native smoke: article selection");
                }
                1 => {
                    assert!(self.article.is_some(), "Smoke: article loaded");
                    self.smoke_capture("reader");
                    self.smoke_click("settings", event_loop);
                }
                2 => {
                    assert!(matches!(self.dialog, Some(Dialog::Settings)));
                    let selected = self.selected;
                    let scroll = self.surfaces[3].as_ref().unwrap().scroll;
                    // Exercise both palettes through the real settings control,
                    // including every live document and the modal itself.
                    for theme in ["sepia", "dark", "light", "sepia"] {
                        let generations: Vec<_> = self
                            .surfaces
                            .iter()
                            .map(|s| s.as_ref().unwrap().generation)
                            .collect();
                        self.smoke_click("theme", event_loop);
                        assert_eq!(self.settings.reading_theme, theme);
                        self.smoke_capture(&format!("settings-{theme}"));
                        for (i, before) in generations.into_iter().enumerate() {
                            assert!(self.current_surface(i));
                            assert_ne!(
                                self.surfaces[i].as_ref().unwrap().generation,
                                before,
                                "Theme changes must reach surface {i}"
                            );
                        }
                        assert_eq!(self.selected, selected);
                        assert_eq!(self.surfaces[3].as_ref().unwrap().scroll, scroll);
                        assert!(matches!(self.dialog, Some(Dialog::Settings)));
                    }
                    self.smoke_click("cancel", event_loop);
                    self.smoke_capture("sepia");
                }
                3 => {
                    assert_eq!(
                        self.backend
                            .store
                            .lock()
                            .unwrap()
                            .settings()
                            .unwrap()
                            .reading_theme,
                        "sepia",
                        "App theme persists through the storage worker"
                    );
                    self.smoke_click("add", event_loop);
                }
                4 => {
                    let (_, editor) = self.editor.as_mut().expect("Smoke: field focused");
                    editor.replace_selection("https://example.test/feed?label=花&note=café");
                    assert!(editor.text().contains("花"));
                    self.smoke_capture("field");
                    self.redraw();
                }
                5 => {
                    self.commit_editor();
                    assert!(self.fields[0].1.contains("café"));
                    self.smoke_click("cancel", event_loop);
                    self.smoke_click("focus", event_loop);
                    assert!(self.focus_mode);
                    self.smoke_capture("focus");
                }
                6 => {
                    let window = Arc::downgrade(self.window.as_ref().unwrap());
                    self.close_window();
                    assert_eq!(self.visible, !self.tray_online);
                    if self.tray_online {
                        assert!(self.window.is_none());
                        assert!(self.presenter.is_none());
                        assert!(self.accessibility.is_none());
                        assert!(
                            window.upgrade().is_none(),
                            "Native window must actually be dropped"
                        );
                    }
                }
                7 => {
                    self.show(event_loop);
                    assert!(self.visible);
                    assert!(self.window.is_some() && self.presenter.is_some());
                    assert!(
                        self.article.is_some() && self.focus_mode,
                        "Reading state survives close"
                    );
                    eprintln!("Smoke: tray window recreated");
                }
                8 => {
                    if !self.article.as_ref().unwrap().read {
                        self.action("read", event_loop);
                    }
                    self.action("read", event_loop);
                    assert_eq!(self.manually_unread, self.selected);
                    self.action("save", event_loop);
                }
                9 => {
                    let article = self.article.as_ref().unwrap();
                    assert!(!article.read, "Smoke: manual unread must persist");
                    let stored = self
                        .backend
                        .store
                        .lock()
                        .unwrap()
                        .article(article.id)
                        .unwrap();
                    assert!(!stored.read);
                    assert_eq!(stored.saved, article.saved);
                    self.find = "curiosity".into();
                    self.jump_find(false);
                    self.smoke_capture("find");
                    self.query.search = "curiosity".into();
                    self.reload();
                }
                10 => {
                    assert!(
                        self.snapshot.as_ref().unwrap().total > 0,
                        "Smoke: full-text search"
                    );
                    self.query.search.clear();
                    self.focus_mode = false;
                    self.settings.reduced_motion = true;
                    self.dirty = true;
                    self.smoke_click("theme", event_loop);
                    assert_eq!(self.settings.reading_theme, "dark");
                    self.smoke_capture("dark");
                    self.smoke_click("settings", event_loop);
                    self.smoke_capture("dark-settings");
                    self.smoke_click("cancel", event_loop);
                    self.smoke_click("add", event_loop);
                    let (_, editor) = self.editor.as_mut().expect("Smoke: dark field focused");
                    editor.replace_selection("https://example.test/feed?label=花&note=café");
                    self.smoke_capture("dark-field");
                    self.smoke_click("cancel", event_loop);
                    self.smoke_click("focus", event_loop);
                    self.smoke_capture("dark-focus");
                    self.smoke_click("focus", event_loop);
                }
                11 => {
                    let sample = self
                        .backend
                        .store
                        .lock()
                        .unwrap()
                        .snapshot(&Query::default())
                        .unwrap()
                        .articles
                        .into_iter()
                        .find(|a| a.title.contains("typography specimen"))
                        .expect("Smoke: rich article fixture");
                    self.smoke_click("theme", event_loop);
                    assert_eq!(self.settings.reading_theme, "light");
                    self.select(sample.id);
                }
                12 => {
                    assert!(
                        self.article
                            .as_ref()
                            .unwrap()
                            .title
                            .contains("typography specimen")
                    );
                    self.compose();
                    let surface = self.surfaces[3].as_ref().unwrap();
                    assert!(
                        !surface
                            .scene
                            .as_ref()
                            .unwrap()
                            .find_text("Headings")
                            .is_empty(),
                        "Smoke: table text renders"
                    );
                    let note = surface
                        .doc
                        .dom
                        .descendants(trust::dom::DOCUMENT)
                        .find(|&n| surface.doc.dom.attr(n, "href") == Some("#note"))
                        .unwrap();
                    self.activate(3, note, event_loop);
                    assert!(
                        self.surfaces[3].as_ref().unwrap().scroll.y > 0.0,
                        "Smoke: local footnote scrolls"
                    );
                    let surface = self.surfaces[3].as_mut().unwrap();
                    surface.scroll = surface.doc.clamp_scroll(CssPoint::new(0.0, 250.0));
                    self.smoke_capture("typography");
                }
                13 => {
                    let article = self.article.as_mut().unwrap();
                    article.extracted = None;
                    article.content = format!(
                        "{}<p>Deep nesting fixture</p>{}",
                        "<div>".repeat(2048),
                        "</div>".repeat(2048)
                    );
                    self.dirty = true;
                    self.redraw();
                }
                14 => {
                    self.smoke_settle_documents();
                    assert!(self.requests[3].as_ref().unwrap().failed);
                    assert!(
                        self.current_surface(3),
                        "Rejected articles leave an explanatory reading surface"
                    );
                    self.smoke_capture("article-limit");
                    let add = self.surfaces[0]
                        .as_ref()
                        .unwrap()
                        .doc
                        .dom
                        .descendants(trust::dom::DOCUMENT)
                        .find(|&n| {
                            self.surfaces[0].as_ref().unwrap().doc.dom.attr(n, "href")
                                == Some("app:add")
                        })
                        .unwrap();
                    self.message("Smoke: controls stay active during document replacement");
                    self.rebuild();
                    assert!(!self.current_surface(0));
                    self.activate(0, add, event_loop);
                    assert!(matches!(self.dialog, Some(Dialog::AddFeed)));
                    // Close/reopen before a frame has consumed the previous
                    // request: the modal must still acquire a fresh document.
                    self.action("cancel", event_loop);
                    self.action("add", event_loop);
                    self.smoke_settle_documents();
                    assert!(self.current_surface(4));
                    assert!(self.editor.is_some());
                    self.smoke_click("cancel", event_loop);
                }
                _ => {
                    eprintln!(
                        "Native smoke passed: rendered control hits, charm fidgets, no decorative focus boxes, article loading, Unicode editing, search, find, manual unread, saved persistence, themes, focus, tray close/reopen, document limits, rapid modal replacement"
                    );
                    event_loop.exit();
                }
            }
            self.smoke_step += 1;
            self.smoke_time = now;
        }
    }
}

pub fn snapshot(
    path: &Path,
    output: &Path,
    sample: bool,
    width: u32,
    height: u32,
    renderer: &str,
) -> anyhow::Result<()> {
    let mut db = Store::open(&path.join("library.sqlite"))?;
    if sample {
        db.sample()?;
    }
    let snap = db.snapshot(&Query::default())?;
    let article = snap.articles.first().and_then(|a| db.article(a.id).ok());
    let settings = db.settings()?;
    let decora = Decora::fresh();
    let metrics =
        ViewportMetrics::from_physical(PhysicalSize::new(width, height), ScaleFactor::default());
    let g = Geometry::new(metrics, &settings, false);
    let store = ImageStore::default();
    let base = url::Url::parse("https://ruby-reader.invalid/")?;
    let theme = Theme::from_name(&settings.reading_theme);
    let mut scene = empty_scene(metrics, store.clone(), theme);
    let surfaces = [
        (
            ui::background(
                g,
                Some(&snap),
                &Query::default(),
                article.as_ref(),
                "Your own little corner of the internet",
                false,
                &decora,
            ),
            theme.css(),
            CssRect::new(0.0, 0.0, g.width, g.height),
        ),
        (
            ui::sidebar(Some(&snap), &View::All, &decora),
            theme.css(),
            g.sidebar,
        ),
        (
            ui::headlines(
                Some(&snap),
                article.as_ref().map(|a| a.id),
                &Query::default(),
            ),
            theme.css(),
            g.headlines,
        ),
        (
            ui::article_html(article.as_ref(), true, &decora),
            ui::article_css(&settings),
            g.article,
        ),
    ];
    for (body, css, rect) in surfaces {
        let doc = ui::document(
            &body,
            &css,
            base.clone(),
            CssSize::new(rect.width, rect.height),
            store.clone(),
        );
        let mut part = doc.scene(metrics, rect, CssPoint::default(), 0.0);
        if rect != g.article || article.is_none() {
            decora.animate(&mut part, &doc, 0.0, settings.reduced_motion);
        }
        scene.primitives.extend(part.primitives);
    }
    decora.paint(
        &mut scene,
        g.width,
        g.height,
        0.0,
        settings.reduced_motion,
        false,
    );
    let frame = if renderer == "hybrid" {
        let rt = tokio::runtime::Runtime::new()?;
        let mut gpu = rt
            .block_on(trust::render::vello_hybrid::VelloHybridRenderer::new_headless())
            .map_err(anyhow::Error::msg)?;
        gpu.render_rgba(&scene).map_err(anyhow::Error::msg)?
    } else {
        trust::render::vello_cpu::VelloCpuRenderer::new()
            .render_rgba(&scene)
            .map_err(anyhow::Error::msg)?
    };
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    trust::render::headless::write_png(&frame, output).map_err(anyhow::Error::msg)?;
    println!("Rendered {} × {} to {}", width, height, output.display());
    Ok(())
}
