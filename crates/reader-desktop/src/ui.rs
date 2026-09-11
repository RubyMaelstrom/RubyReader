use crate::decora::{ARTWORK, Decora};
use reader_core::{content::escape as e, model::*};
use trust::{
    core::{CssSize, ViewportMetrics},
    embed::EmbeddedDocument,
    render::{CssRect, ImageStore},
};
use url::Url;

const CSS: &str = include_str!("../../../assets/theme.css");

#[cfg(test)]
mod tests {
    use super::*;
    use trust::core::{CssPoint, PhysicalSize, ScaleFactor};

    #[test]
    fn article_is_largest_pane_at_supported_window_sizes() {
        for (w, h) in [(860, 650), (1024, 768), (1360, 920), (1920, 1080)] {
            let m = ViewportMetrics::from_physical(PhysicalSize::new(w, h), ScaleFactor::default());
            let g = Geometry::new(m, &Settings::default(), false);
            assert!(g.article.height > g.headlines.height, "{w}×{h}");
            assert!(g.article.x + g.article.width <= w as f32);
        }
    }

    #[test]
    fn table_layers_keep_separated_cell_gaps_transparent() {
        let doc = EmbeddedDocument::new(
            "<style>body{margin:0;background:white}table{width:200px;table-layout:fixed;background:yellow;border-collapse:separate;border-spacing:10px}td{padding:0;height:40px}tbody{background:blue}</style><table><tbody><tr style='background:red'><td></td><td></td></tr></tbody></table>",
            Url::parse("https://example.test/").unwrap(),
            CssSize::new(220.0, 120.0),
            ImageStore::default(),
        );
        let m = ViewportMetrics::from_physical(PhysicalSize::new(220, 120), ScaleFactor::default());
        let scene = doc.scene(
            m,
            CssRect::new(0.0, 0.0, 220.0, 120.0),
            CssPoint::default(),
            0.0,
        );
        let frame = trust::render::vello_cpu::VelloCpuRenderer::new()
            .render_rgba(&scene)
            .unwrap();
        assert_eq!(
            &frame.pixels[(15 * 220 + 100) * 4..(15 * 220 + 100) * 4 + 3],
            &[255, 255, 0]
        );
        assert_eq!(
            &frame.pixels[(15 * 220 + 15) * 4..(15 * 220 + 15) * 4 + 3],
            &[255, 0, 0]
        );
    }

    #[test]
    fn article_disclosures_expand_without_javascript() {
        let mut doc = EmbeddedDocument::new(
            "<details><summary><b id='label'>More</b></summary><p id='secret'>Hidden detail</p></details>",
            Url::parse("https://example.test/").unwrap(),
            CssSize::new(400.0, 300.0),
            ImageStore::default(),
        );
        let label = doc.dom.get_by_id("label").unwrap();
        let secret = doc.dom.get_by_id("secret").unwrap();
        assert!(!doc.layout.boxes.contains_key(&secret));
        assert!(toggle_disclosure(&mut doc, label));
        assert!(doc.layout.boxes.contains_key(&secret));
        assert!(toggle_disclosure(&mut doc, label));
        assert!(!doc.layout.boxes.contains_key(&secret));
    }

    #[test]
    fn feed_actions_reveal_only_for_hover_or_keyboard_without_layout_shift() {
        let snap = Snapshot {
            feeds: (1..=2)
                .map(|id| Feed {
                    id,
                    title: format!("Sample feed {id}"),
                    url: format!("https://example.test/{id}"),
                    folder_id: None,
                    unread: 4,
                    error: (id == 2).then(|| "Offline".into()),
                    last_refresh: None,
                    etag: None,
                    modified: None,
                    failures: 0,
                    next_refresh: 0,
                    initialized: true,
                })
                .collect(),
            folders: vec![],
            articles: vec![],
            total: 8,
            all: 8,
            unread: 8,
            saved: 0,
            settings: Settings::default(),
        };
        let mut doc = document(
            &sidebar(Some(&snap), &View::Feed(1), &Decora::seeded(42)),
            "",
            Url::parse(ASSET_BASE).unwrap(),
            CssSize::new(244.0, 700.0),
            ImageStore::default(),
        );
        let link = |href: &str| {
            doc.dom
                .descendants(trust::dom::DOCUMENT)
                .find(|&node| doc.dom.attr(node, "href") == Some(href))
                .unwrap()
        };
        let feed = link("app:feed:1");
        let edit = link("app:feed-edit:1");
        let refresh = link("app:feed-refresh:1");
        let retry = link("app:feed-refresh:2");
        let actions = doc.dom.node(edit).parent.unwrap();
        let other_actions = doc.dom.node(retry).parent.unwrap();
        let geometry = doc.layout.boxes.clone();
        let opacity =
            |doc: &EmbeddedDocument, node| doc.dom.computed_value(node, "opacity").unwrap();
        assert_eq!(opacity(&doc, actions), "0", "Selection is not hover");
        assert_eq!(opacity(&doc, other_actions), "0");
        assert_eq!(
            doc.dom.computed_value(edit, "pointer-events").as_deref(),
            Some("none")
        );
        assert!(
            doc.semantics(None)
                .nodes
                .iter()
                .any(|n| n.dom_node == Some(edit)),
            "Unrevealed controls remain available to assistive technology"
        );

        let render = |doc: &EmbeddedDocument| {
            let metrics =
                ViewportMetrics::from_physical(PhysicalSize::new(244, 700), ScaleFactor::default());
            trust::render::vello_cpu::VelloCpuRenderer::new()
                .render_rgba(&doc.scene(
                    metrics,
                    CssRect::new(0.0, 0.0, 244.0, 700.0),
                    CssPoint::default(),
                    0.0,
                ))
                .unwrap()
                .pixels
        };
        let hidden = render(&doc);
        doc.hover(Some(feed));
        assert_eq!(opacity(&doc, actions), "1");
        assert_eq!(opacity(&doc, other_actions), "0");
        assert_ne!(hidden, render(&doc), "Hover feedback is actually painted");
        for node in [edit, refresh] {
            doc.hover(Some(node));
            assert_eq!(
                opacity(&doc, actions),
                "1",
                "Moving onto controls keeps them visible"
            );
            assert_eq!(
                doc.dom.computed_value(node, "pointer-events").as_deref(),
                Some("auto")
            );
        }
        doc.hover(None);
        assert_eq!(render(&doc), hidden, "Leaving restores the uncluttered row");
        for node in [feed, edit, refresh] {
            focus_feed_actions(&mut doc, Some(node));
            assert_eq!(opacity(&doc, actions), "1");
            assert_eq!(opacity(&doc, other_actions), "0");
        }
        focus_feed_actions(&mut doc, Some(retry));
        assert_eq!(opacity(&doc, actions), "0");
        assert_eq!(opacity(&doc, other_actions), "1");
        assert_ne!(
            render(&doc),
            hidden,
            "Keyboard focus visibly reveals controls"
        );
        focus_feed_actions(&mut doc, None);
        assert_eq!(render(&doc), hidden);
        assert_eq!(
            doc.layout.boxes, geometry,
            "Revealing controls never moves rows"
        );
    }

    #[test]
    fn all_visible_decora_documents_share_a_non_repeating_collection() {
        for seed in 0..24 {
            let decora = Decora::seeded(seed);
            let settings = Settings::default();
            let m = ViewportMetrics::from_physical(
                PhysicalSize::new(1360, 920),
                ScaleFactor::default(),
            );
            let g = Geometry::new(m, &settings, false);
            let html = background(g, None, &Query::default(), None, "Ready", false, &decora)
                + &sidebar(None, &View::All, &decora)
                + &article_html(None, false, &decora);
            let mut images = std::collections::HashSet::new();
            for tail in html.split(ASSET_BASE).skip(1) {
                let name = tail.split('\'').next().unwrap();
                assert!(images.insert(name), "Duplicate visible asset: {name}");
                assert!(asset_bytes(name).is_some(), "Missing image: {name}");
            }
            assert!(images.len() >= 20);
            for slogan in [
                "personal web club",
                "ALIVE",
                "thousand tiny",
                "web of your own",
                "good taste",
            ] {
                assert!(!html.contains(slogan));
            }
        }
    }

    #[test]
    fn accessory_motion_changes_paint_but_reduced_motion_is_static() {
        let decora = Decora::seeded(42);
        let doc = document(
            &art("cherries", "sway", "width:75px;height:75px"),
            "",
            Url::parse(ASSET_BASE).unwrap(),
            CssSize::new(100.0, 100.0),
            ImageStore::default(),
        );
        let m = ViewportMetrics::from_physical(PhysicalSize::new(100, 100), ScaleFactor::default());
        let rect = CssRect::new(0.0, 0.0, 100.0, 100.0);
        let original = doc.scene(m, rect, CssPoint::default(), 0.0);
        let mut moving = original.clone();
        decora.animate(&mut moving, &doc, 0.0, false);
        let mut later = original.clone();
        decora.animate(&mut later, &doc, 0.8, false);
        assert_ne!(moving.primitives, later.primitives);
        let mut still = original.clone();
        decora.animate(&mut still, &doc, 0.8, true);
        assert_eq!(still.primitives, original.primitives);
        let pixels_a = trust::render::vello_cpu::VelloCpuRenderer::new()
            .render_rgba(&moving)
            .unwrap();
        let pixels_b = trust::render::vello_cpu::VelloCpuRenderer::new()
            .render_rgba(&later)
            .unwrap();
        let changed = pixels_a
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(pixels_b.pixels.as_chunks::<4>().0.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 500,
            "Accessory movement must be visibly present: {changed} changed pixels"
        );
    }

    #[test]
    fn sparkles_freeze_with_reduced_motion_and_disappear_in_focus() {
        let decora = Decora::seeded(7);
        let doc = document(
            "",
            "",
            Url::parse(ASSET_BASE).unwrap(),
            CssSize::new(860.0, 650.0),
            ImageStore::default(),
        );
        let m = ViewportMetrics::from_physical(PhysicalSize::new(860, 650), ScaleFactor::default());
        let original = doc.scene(
            m,
            CssRect::new(0.0, 0.0, 860.0, 650.0),
            CssPoint::default(),
            0.0,
        );
        let mut first = original.clone();
        decora.paint(&mut first, 860.0, 650.0, 0.0, true, false);
        let mut later = original.clone();
        decora.paint(&mut later, 860.0, 650.0, 5.0, true, false);
        assert_eq!(first.primitives, later.primitives);
        let mut moving = original.clone();
        decora.paint(&mut moving, 860.0, 650.0, 0.8, false, false);
        assert_ne!(first.primitives, moving.primitives);
        let mut focus = original.clone();
        decora.paint(&mut focus, 860.0, 650.0, 0.8, false, true);
        assert_eq!(original.primitives, focus.primitives);
    }

    #[test]
    fn charm_hits_follow_painted_pixels_transforms_clips_and_holes() {
        let decora = Decora::seeded(42);
        let html = format!(
            "<div style='position:absolute;left:40px;top:20px;width:100px;height:100px;overflow:hidden'>{}</div>",
            art(
                "bracelet",
                "sway",
                "width:100px;height:100px;transform:rotate(17deg)"
            )
        );
        let doc = document(
            &html,
            "",
            Url::parse(ASSET_BASE).unwrap(),
            CssSize::new(220.0, 160.0),
            ImageStore::default(),
        );
        let m = ViewportMetrics::from_physical(PhysicalSize::new(220, 160), ScaleFactor::default());
        let mut scene = doc.scene(
            m,
            CssRect::new(0.0, 0.0, 220.0, 160.0),
            CssPoint::default(),
            0.0,
        );
        decora.animate(&mut scene, &doc, 0.4, false);
        assert!(
            Decora::hit(&scene, &doc, CssPoint::new(90.0, 70.0)).is_none(),
            "Bracelet center is not a button"
        );
        assert!(
            Decora::hit(&scene, &doc, CssPoint::new(38.0, 65.0)).is_none(),
            "Parent clipping limits hits"
        );
        let count = (40..140)
            .flat_map(|x| (20..120).map(move |y| CssPoint::new(x as f32, y as f32)))
            .filter(|point| Decora::hit(&scene, &doc, *point).is_some())
            .count();
        assert!(
            (800..5_000).contains(&count),
            "Only the painted bead ring should hit: {count}"
        );
    }

    #[test]
    fn a_fidget_affects_only_its_charm_and_reduced_motion_never_shakes() {
        let mut decora = Decora::seeded(42);
        let m = ViewportMetrics::from_physical(PhysicalSize::new(120, 120), ScaleFactor::default());
        let rect = CssRect::new(0.0, 0.0, 120.0, 120.0);
        let doc = document(
            &art("cherries", "sway", "width:80px;height:80px"),
            "",
            Url::parse(ASSET_BASE).unwrap(),
            CssSize::new(120.0, 120.0),
            ImageStore::default(),
        );
        let original = doc.scene(m, rect, CssPoint::default(), 0.0);
        let mut ambient = original.clone();
        decora.animate(&mut ambient, &doc, 0.3, false);
        decora.poke("bracelet", 0.0);
        let mut untouched = original.clone();
        decora.animate(&mut untouched, &doc, 0.3, false);
        assert_eq!(
            ambient.primitives, untouched.primitives,
            "Other charms don't shake"
        );
        decora.poke("cherries", 0.0);
        let mut burst = original.clone();
        decora.animate(&mut burst, &doc, 0.3, false);
        assert_ne!(ambient.primitives, burst.primitives);
        let mut still = original.clone();
        decora.animate(&mut still, &doc, 0.3, true);
        let mut later = original.clone();
        decora.animate(&mut later, &doc, 0.9, true);
        assert_eq!(
            still.primitives, later.primitives,
            "Reduced Motion gets static glints"
        );
        let mut expired = original.clone();
        decora.animate(&mut expired, &doc, 2.0, true);
        assert_eq!(expired.primitives, original.primitives);
    }
}
pub fn install_fonts() {
    trust::embed::install_application_fonts(vec![
        (
            "Ruby Sans".into(),
            include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec(),
        ),
        (
            "Ruby Sans".into(),
            include_bytes!("../../../assets/fonts/DejaVuSans-Bold.ttf").to_vec(),
        ),
        (
            "Ruby Serif".into(),
            include_bytes!("../../../assets/fonts/DejaVuSerif.ttf").to_vec(),
        ),
        (
            "Ruby Serif".into(),
            include_bytes!("../../../assets/fonts/DejaVuSerif-Bold.ttf").to_vec(),
        ),
        (
            "Ruby Serif".into(),
            include_bytes!("../../../assets/fonts/DejaVuSerif-Italic.ttf").to_vec(),
        ),
    ]);
}
pub const ASSET_BASE: &str = "https://ruby-reader.invalid/assets/";

/// Script-free HTML summary activation (HTML §4.11.2). Sanitized articles
/// contain ordinary light DOM; only the first summary of a details toggles.
pub fn toggle_disclosure(doc: &mut EmbeddedDocument, mut node: usize) -> bool {
    loop {
        if doc.dom.tag_name(node) == Some("summary") {
            break;
        }
        let Some(parent) = doc.dom.node(node).parent else {
            return false;
        };
        node = parent;
    }
    let Some(details) = doc.dom.node(node).parent else {
        return false;
    };
    if doc.dom.tag_name(details) != Some("details")
        || doc
            .dom
            .children(details)
            .into_iter()
            .find(|&n| doc.dom.tag_name(n) == Some("summary"))
            != Some(node)
    {
        return false;
    }
    if doc.dom.attr(details, "open").is_some() {
        doc.dom.remove_attr(details, "open");
    } else {
        doc.dom.set_attr(details, "open", "");
    }
    doc.relayout();
    true
}

#[derive(Clone, Debug)]
pub enum Dialog {
    AddFeed,
    Folder(Option<i64>),
    EditFeed(i64),
    Settings,
    DeleteFeed(i64),
    DeleteFolder(i64),
    Help,
    Discover(Vec<DiscoveredFeed>),
    Image(String),
    Export,
    Search,
    Find,
}

#[derive(Clone, Copy)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    pub sidebar: CssRect,
    pub headlines: CssRect,
    pub article: CssRect,
    pub vertical: CssRect,
    pub horizontal: CssRect,
    pub modal: CssRect,
}
impl Geometry {
    pub fn new(metrics: ViewportMetrics, settings: &Settings, focus: bool) -> Self {
        let width = metrics.css.width;
        let height = metrics.css.height;
        let left = settings
            .sidebar_width
            .clamp(170.0, (width * 0.36).max(170.0));
        let top = 148.0;
        let list = settings
            .headline_height
            .clamp(110.0, ((height - top - 100.0) * 0.38).max(110.0));
        let sx = if focus { 18.0 } else { left + 24.0 };
        let ay = if focus { top + 42.0 } else { top + list + 56.0 };
        Self {
            width,
            height,
            sidebar: CssRect::new(
                18.0,
                top + 42.0,
                left - 12.0,
                (height - top - 90.0).max(1.0),
            ),
            headlines: CssRect::new(sx, top + 38.0, (width - sx - 22.0).max(1.0), list),
            article: CssRect::new(
                sx,
                ay + 36.0,
                (width - sx - 22.0).max(1.0),
                (height - ay - 76.0).max(1.0),
            ),
            vertical: CssRect::new(left + 8.0, top, 12.0, height - top - 38.0),
            horizontal: CssRect::new(sx, top + list + 40.0, width - sx - 22.0, 12.0),
            modal: CssRect::new(
                ((width - 620.0) / 2.0).max(14.0),
                ((height - 580.0) / 2.0).max(20.0),
                (width - 28.0).min(620.0),
                (height - 40.0).min(580.0),
            ),
        }
    }
}

pub fn document(
    body: &str,
    extra: &str,
    base: Url,
    size: CssSize,
    store: ImageStore,
) -> EmbeddedDocument {
    let html = format!(
        "<!doctype html><html lang='en'><head><meta charset='utf-8'><style>{CSS}\n{extra}</style></head><body>{body}</body></html>"
    );
    let mut doc = EmbeddedDocument::new(&html, base, size, store);
    let requests = doc.image_requests().to_vec();
    for request in requests {
        if let Some(name) = request.source.strip_prefix(ASSET_BASE)
            && let Some(bytes) = asset_bytes(name)
            && let Ok(image) = trust::img::decode_graphical(bytes)
        {
            doc.supply_image(&request.source, image);
        }
    }
    doc
}

pub fn asset_bytes(name: &str) -> Option<&'static [u8]> {
    if let Some((_, bytes)) = ARTWORK
        .iter()
        .find(|(stem, _)| name.strip_suffix(".png") == Some(stem))
    {
        return Some(bytes);
    }
    Some(match name {
        "bow.png" => include_bytes!("../../../assets/bow.png"),
        "heart.png" => include_bytes!("../../../assets/heart.png"),
        "comet.png" => include_bytes!("../../../assets/comet.png"),
        "butterfly.png" => include_bytes!("../../../assets/butterfly.png"),
        "letter.png" => include_bytes!("../../../assets/letter.png"),
        "mouse.png" => include_bytes!("../../../assets/mouse.png"),
        "rose.png" => include_bytes!("../../../assets/rose.png"),
        "icon.png" => include_bytes!("../../../assets/icon.png"),
        "gingham.svg" => include_bytes!("../../../assets/gingham.svg"),
        _ => return None,
    })
}
fn art(name: &str, class: &str, style: &str) -> String {
    format!(
        "<img alt='' aria-hidden='true' data-charm='{name}' data-decora='{class}' class='art {class}' src='{ASSET_BASE}{name}.png' style='{style}'>"
    )
}
fn a(action: &str, text: &str, class: &str) -> String {
    format!("<a href='app:{action}' class='{class}'>{text}</a>")
}
fn pos(rect: CssRect) -> String {
    format!(
        "left:{}px;top:{}px;width:{}px;height:{}px",
        rect.x, rect.y, rect.width, rect.height
    )
}

pub fn background(
    g: Geometry,
    snap: Option<&Snapshot>,
    query: &Query,
    article: Option<&Article>,
    status: &str,
    focus: bool,
    decora: &Decora,
) -> String {
    let unread = snap.map_or(0, |s| s.unread);
    let title = view_name(&query.view, snap);
    let mut out=format!("<img class='wallpaper' alt='' src='{ASSET_BASE}gingham.svg'><div class='topline'>♡ YOUR OWN LITTLE CORNER OF THE INTERNET ♡ <span style='float:right'>EST. 2026 · MADE WITH FEELING</span></div>
        <div class='masthead'><div class='logo'><span class='logo-ruby'>Ruby</span> <span class='logo-reader'>Reader</span></div><div class='tagline'>follow your curiosity. keep the lovely bits.</div></div>");
    out.push_str(&art(
        "bow",
        "sway",
        "left:12px;top:39px;width:76px;transform:rotate(-14deg)",
    ));
    if !focus {
        out.push_str("<div class='header-charms' aria-hidden='true'><div class='bead-garland'>");
        let colors = ["#fa4e96", "#80ddd0", "#fff09b", "#bda0ed", "#f99791"];
        for i in 0..40 {
            out.push_str(&format!(
                "<i style='left:{}%;top:{}px;background:{}'></i>",
                i as f32 * 2.5,
                6.0 + (i as f32 * 0.24).sin().abs() * 15.0,
                colors[i % colors.len()]
            ));
        }
        out.push_str("</div>");
        let available = g.width - 428.0;
        let count = ((available / 76.0) as usize).clamp(4, 11);
        let cell = available / count as f32;
        for i in 0..count {
            let size = (cell * if i % 3 == 0 { 1.05 } else { 0.88 }).min(92.0);
            let x = 6.0 + cell * i as f32 + (cell - size) * 0.5;
            let y = if i % 2 == 0 { 3.0 } else { 14.0 };
            let angle = [-14, 11, -8, 17, -5][i % 5];
            out.push_str(&format!("<div class='charm' style='left:{x}px;top:{y}px;width:{size}px;height:65px;transform:rotate({angle}deg)'>{}</div>", art(decora.slot(i), ["sway", "bounce", "drift"][i % 3], "width:100%;height:100%")));
        }
        out.push_str("</div>");
    }
    out.push_str(&format!("<div class='toolbar' style='top:109px'>{}{}{}{}{}{}<span class='unread-pill'>{unread} unread little things</span></div>",
        a("add","＋ Add feed","button primary"),a("refresh","↻ Refresh","button"),a("search","⌕ Search","button"),a("import","Import OPML","button"),a("settings","⚙ Settings","button"),a("help","?","button")));
    if !focus {
        out.push_str(&format!("<div class='pane-frame side-frame' style='{}'></div><div class='ribbon' style='left:18px;top:151px;width:{}px'>♡ MY COLLECTION {}</div>",pos(CssRect::new(14.0,148.0,g.sidebar.width+8.0,g.sidebar.height+48.0)),g.sidebar.width,a("folder","＋","small-action")));
        out.push_str(&format!("<div class='pane-frame list-frame' style='{}'></div><div class='list-title' style='left:{}px;top:150px;width:{}px'><b>{}</b> <span class='quiet'>{} stories</span><span style='float:right'>{} {} {}</span></div>",pos(CssRect::new(g.headlines.x-4.0,148.0,g.headlines.width+8.0,g.headlines.height+44.0)),g.headlines.x+10.0,g.headlines.width-20.0,e(&title),snap.map_or(0,|s|s.total),a("sort","↕ Sort","small-action"),a("page-prev","‹","small-action"),a("page-next","›","small-action")));
    }
    let toolbar_y = g.article.y - 34.0;
    out.push_str(&format!("<div class='pane-frame reader-frame' style='{}'></div><div class='reading-tools' style='left:{}px;top:{toolbar_y}px;width:{}px'>{}{}{}{}{}<span style='float:right'>{}{}{}</span></div>",
        pos(CssRect::new(g.article.x-4.0,g.article.y-38.0,g.article.width+8.0,g.article.height+42.0)),g.article.x+8.0,g.article.width-16.0,
        a("save",if article.is_some_and(|a|a.saved){"♥ Saved"}else{"♡ Save"},"tool"),
        a("read",if article.is_some_and(|a|a.read){"○ Unread"}else{"✓ Read"},"tool"),
        a("extract","Fetch full article","tool"),a("original","Original ↗","tool"),a("source","Feed / full","tool"),
        a("find","Find","tool"),a("theme","◐","tool"),a("focus",if focus{"↙ Restore"}else{"↗ Focus"},"tool")));
    out.push_str(&format!("<div class='statusbar'><span class='status-heart'>♥</span> {} <span style='float:right'>{} · {} · {}</span></div>",e(status),a("mark-all","Mark view read","status-link"),a("export","Export OPML","status-link"),a("quit","Quit","status-link")));
    out
}

pub fn view_name(view: &View, snap: Option<&Snapshot>) -> String {
    match view {
        View::All => "The latest lovely things".into(),
        View::Unread => "Waiting to be discovered".into(),
        View::Saved => "Kept close to the heart".into(),
        View::Feed(id) => snap
            .and_then(|s| s.feeds.iter().find(|f| f.id == *id))
            .map(|f| f.title.clone())
            .unwrap_or_else(|| "Feed".into()),
        View::Folder(id) => snap
            .and_then(|s| s.folders.iter().find(|f| f.id == *id))
            .map(|f| f.title.clone())
            .unwrap_or_else(|| "Folder".into()),
    }
}

pub fn sidebar(snap: Option<&Snapshot>, view: &View, decora: &Decora) -> String {
    let mut out = String::from("<div class='sidebar-paper'>");
    for (kind, label, active, count) in [
        (
            "all",
            "✧ All articles",
            matches!(view, View::All),
            snap.map_or(0, |s| s.all),
        ),
        (
            "unread",
            "● Unread",
            matches!(view, View::Unread),
            snap.map_or(0, |s| s.unread),
        ),
        (
            "saved",
            "♥ Saved",
            matches!(view, View::Saved),
            snap.map_or(0, |s| s.saved),
        ),
    ] {
        out.push_str(&a(
            kind,
            &format!("{label}<span class='count'>{count}</span>"),
            if active { "nav selected" } else { "nav" },
        ));
    }
    out.push_str("<div class='section-label'>SUBSCRIPTIONS</div>");
    if let Some(snap) = snap {
        fn tree(out: &mut String, snap: &Snapshot, view: &View, parent: Option<i64>, depth: usize) {
            for folder in snap.folders.iter().filter(|f| f.parent_id == parent) {
                out.push_str(&format!(
                    "<div class='folder-row' style='padding-left:{}px'>{}{}</div>",
                    depth * 10,
                    a(
                        &format!("folder-view:{}", folder.id),
                        &format!("▾ {}", e(&folder.title)),
                        if *view == View::Folder(folder.id) {
                            "folder selected"
                        } else {
                            "folder"
                        }
                    ),
                    a(&format!("folder-edit:{}", folder.id), "⋯", "small-action")
                ));
                tree(out, snap, view, Some(folder.id), depth + 1);
            }
            for feed in snap.feeds.iter().filter(|f| f.folder_id == parent) {
                let label = format!(
                    "<span class='feed-dot'>{}</span><span class='feed-name'>{}</span><span class='count'>{}</span>",
                    if feed.error.is_some() { "!" } else { "✿" },
                    e(&feed.title),
                    feed.unread
                );
                out.push_str(&format!(
                    "<div class='feed-row' style='padding-left:{}px'>{}<div class='feed-actions'>{} {}</div></div>",
                    depth * 8,
                    a(
                        &format!("feed:{}", feed.id),
                        &label,
                        if *view == View::Feed(feed.id) {
                            "feed selected"
                        } else {
                            "feed"
                        }
                    ),
                    a(&format!("feed-edit:{}", feed.id), "edit", "small-action"),
                    a(
                        &format!("feed-refresh:{}", feed.id),
                        if feed.error.is_some() {
                            "retry !"
                        } else {
                            "↻"
                        },
                        "small-action"
                    )
                ));
            }
        }
        tree(&mut out, snap, view, None, 0);
        if snap.feeds.is_empty() {
            out.push_str(
                "<div class='empty-subscriptions'>A collection starts<br>with one good find.</div>",
            );
            out.push_str(&a("add", "＋ Add your first feed", "button primary wide"));
        }
    }
    out.push_str("<div class='charm-board' aria-hidden='true'><div class='charm-chain'></div>");
    for (i, (x, y, size, angle)) in [
        (2, 8, 76, -18),
        (55, 7, 80, 14),
        (24, 55, 108, -7),
        (0, 124, 70, 12),
        (62, 111, 69, -15),
        (26, 167, 87, 15),
        (0, 226, 69, -11),
        (62, 223, 73, 14),
    ]
    .iter()
    .enumerate()
    {
        out.push_str(&format!("<div class='charm' style='left:{x}%;top:{y}px;width:{size}px;height:{size}px;transform:rotate({angle}deg)'>{}</div>", art(decora.slot(11 + i), ["sway", "drift", "bounce"][i % 3], "width:100%;height:100%")));
    }
    out.push_str(&format!("</div><p class='keepsake-caption'>Collected with curiosity.<br>Read at your own pace.</p>{}</div>", a("sample", "Try the sample scrapbook", "sample-link")));
    out
}

/// TRust's embedded surface has native focus, not CSS :focus-within state.
/// Reveal the keyboard-focused row without changing layout or hiding its links
/// from Tab/assistive technology. Pointer hover remains independent.
pub fn focus_feed_actions(doc: &mut EmbeddedDocument, focused: Option<usize>) {
    let mut row = focused.filter(|node| doc.layout.boxes.contains_key(node));
    while let Some(node) = row {
        if doc.dom.attr(node, "class") == Some("feed-row") {
            break;
        }
        row = doc.dom.node(node).parent;
    }
    let previous = doc.dom.get_by_id("keyboard-feed-row");
    if previous == row {
        return;
    }
    if let Some(node) = previous {
        doc.dom.remove_attr(node, "id");
    }
    if let Some(node) = row {
        doc.dom.set_attr(node, "id", "keyboard-feed-row");
    }
    doc.relayout();
}

pub fn headlines(snap: Option<&Snapshot>, selected: Option<i64>, query: &Query) -> String {
    let Some(snap) = snap else {
        return "<div class='empty-list'>Opening your collection…</div>".into();
    };
    if snap.articles.is_empty() {
        return format!(
            "<div class='empty-list'><span style='font-size:34px;color:#d34176'>✧</span><h3>{}</h3><p>{}</p></div>",
            if query.search.is_empty() {
                "A little room for discovery"
            } else {
                "No matching stories"
            },
            if query.search.is_empty() {
                "Add a feed, import your subscriptions, or explore the sample scrapbook."
            } else {
                "Try another word or a different collection."
            }
        );
    }
    let mut out = String::from(
        "<table class='headlines'><thead><tr><th style='width:38px'>♡</th><th>STORY</th><th style='width:150px'>SOURCE</th><th style='width:90px'>DATE</th></tr></thead><tbody>",
    );
    for article in &snap.articles {
        let action = format!("article:{}", article.id);
        let class = if selected == Some(article.id) {
            "chosen"
        } else if !article.read {
            "fresh"
        } else {
            ""
        };
        out.push_str(&format!(
            "<tr class='{class}'><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            a(
                &format!("save-id:{}", article.id),
                if article.saved {
                    "♥"
                } else if !article.read {
                    "●"
                } else {
                    "♡"
                },
                "row-heart"
            ),
            a(&action, &e(&article.title), "headline-link"),
            a(&action, &e(&article.feed_title), "source-link"),
            a(&action, &date(article.published), "source-link")
        ));
    }
    out.push_str("</tbody></table>");
    out
}

pub fn article_html(article: Option<&Article>, full: bool, decora: &Decora) -> String {
    match article {
        Some(article) => format!(
            "<main class='article'><div class='article-eyebrow'>{} <span> / {} </span></div><h1>{}</h1><div class='byline'>{}{} <span class='article-heart'>♡</span></div><div class='article-rule'></div><div class='prose'>{}</div><div class='endmark'>✧ &nbsp; end of this little story &nbsp; ✧</div></main>",
            e(&article.feed_title),
            date(article.published),
            e(&article.title),
            e(&article.author),
            if full && article.extracted.is_some() {
                " · full article"
            } else {
                " · from the feed"
            },
            if full {
                article.extracted.as_deref().unwrap_or(&article.content)
            } else {
                &article.content
            }
        ),
        None => format!(
            "<div class='welcome'>{}<div class='welcome-kicker'>WELCOME TO YOUR READING ROOM</div><h1>The internet is lovely.<br>Make a little collection.</h1><p>Good stories, curious corners, and voices worth following.<br>All together in a place that's unmistakably yours.</p><div class='welcome-note'>Select a story above and settle in.</div><div class='welcome-stars'>✧ &nbsp; ♡ &nbsp; ✧</div></div>",
            art(decora.welcome, "welcome-mascot", "width:135px;height:125px")
        ),
    }
}

pub fn article_css(s: &Settings) -> String {
    let (paper, ink, muted, accent) = match s.reading_theme.as_str() {
        "dark" => ("#241c2a", "#eee5ed", "#b9a9ba", "#ffa9ce"),
        "sepia" => ("#f6ead3", "#4b382e", "#8e7261", "#ad355b"),
        _ => ("#fffdf9", "#302935", "#8a7385", "#bd285f"),
    };
    format!(
        "body{{background:{paper};color:{ink}}}.article,.welcome{{background:{paper};color:{ink}}}.prose{{font-size:{}px;line-height:{}}}.prose a{{color:{accent}}}.byline,.article-eyebrow,.endmark{{color:{muted}}}.prose pre,.prose blockquote,.prose th{{background:{};color:{ink}}}",
        s.font_size,
        s.line_height,
        if s.reading_theme == "dark" {
            "#37283e"
        } else {
            "#faedf1"
        }
    )
}

pub fn dialog_html(
    dialog: &Dialog,
    fields: &[(String, String)],
    snap: Option<&Snapshot>,
    s: &Settings,
    folder: Option<i64>,
) -> String {
    let (title, description) = match dialog {
        Dialog::AddFeed => (
            "Invite a new voice",
            "Paste an RSS, Atom, JSON feed, or website address.",
        ),
        Dialog::Folder(id) => (
            if id.is_some() {
                "Arrange this little collection"
            } else {
                "A home for your discoveries"
            },
            "Give your folder a name.",
        ),
        Dialog::EditFeed(_) => (
            "A little subscription care",
            "Edit its name, address, or folder.",
        ),
        Dialog::Settings => (
            "Make yourself at home",
            "A few little adjustments to your reading room.",
        ),
        Dialog::DeleteFeed(_) => (
            "Remove this subscription?",
            "Its unsaved articles will be removed. Your saved stories stay in Saved.",
        ),
        Dialog::DeleteFolder(_) => (
            "Remove this folder?",
            "Its subscriptions and child folders will move to the top level.",
        ),
        Dialog::Help => (
            "A few helpful little shortcuts",
            "Everything important is a keypress away.",
        ),
        Dialog::Discover(_) => (
            "Which voice would you like?",
            "This website offers more than one feed.",
        ),
        Dialog::Image(_) => ("A closer look", ""),
        Dialog::Export => (
            "Take your collection with you",
            "OPML exports include your subscription addresses, including any private feed tokens. Keep the exported file private.",
        ),
        Dialog::Search => (
            "Find a little something",
            "Search titles and the full text stored in this collection. Leave blank to clear.",
        ),
        Dialog::Find => (
            "Between the lines",
            "Find text in the current article. Enter searches; F3 moves to the next match.",
        ),
    };
    let mut out = format!(
        "<div class='dialog'><div class='dialog-top'>RUBY READER · A LITTLE WINDOW {}</div><div class='dialog-body'><h1>{title}</h1><p class='dialog-description'>{description}</p>",
        a("cancel", "×", "close")
    );
    for (i, (label, _)) in fields.iter().enumerate() {
        out.push_str(&format!("<label class='field-label'>{}</label><a class='field' id='field-{i}' href='app:field:{i}' aria-label='{}'>&nbsp;</a>",e(label),e(label)));
    }
    match dialog {
        Dialog::AddFeed|Dialog::EditFeed(_)=> {let folder_name=folder.and_then(|id|snap.and_then(|s|s.folders.iter().find(|f|f.id==id))).map(|f|f.title.as_str()).unwrap_or("Top level");out.push_str(&format!("<p>Folder: {}</p>",a("cycle-folder",&format!("{} ▾",e(folder_name)),"button")));}
        Dialog::Folder(Some(id))=>out.push_str(&a(&format!("folder-delete:{id}"),"Remove folder…","tool")),
        Dialog::Settings=>{
            for (label,value,minus,plus) in [
                ("Refresh interval",format!("{} minutes",s.refresh_minutes),"interval-down","interval-up"),
                ("Reading size",format!("{} px",s.font_size),"font-down","font-up"),
                ("Line spacing",format!("{:.2}",s.line_height),"line-down","line-up"),
                ("Interface scale",format!("{}%",(s.ui_scale*100.0).round()),"scale-down","scale-up"),
            ] {out.push_str(&format!("<div class='setting-row'><span>{label}</span><span>{} <b>{value}</b> {}</span></div>",a(minus,"−","button"),a(plus,"＋","button")));}
            out.push_str(&format!("<div class='setting-row'>Reading colors {}</div><div class='setting-row'>Decorative motion {}</div><div class='setting-row'>Desktop notifications {}</div><p class='quiet'>Text stays in your library. Viewed images use a 512 MiB cache.</p>{}",a("theme",&s.reading_theme,"button"),a("motion",if s.reduced_motion{"Reduced"}else{"Very lively ✧"},"button"),a("notifications",if s.notifications{"On"}else{"Off"},"button"),a("clear-cache","Clear image cache","tool")));
        }
        Dialog::Discover(feeds)=>for(i,feed)in feeds.iter().enumerate(){out.push_str(&a(&format!("choose-feed:{i}"),&e(&feed.title),"discovered"));},
        Dialog::Help=>out.push_str("<table class='shortcuts'><tr><td>Ctrl + N</td><td>Add a feed</td></tr><tr><td>Ctrl + K</td><td>Search your collection</td></tr><tr><td>Ctrl + F</td><td>Find in the article</td></tr><tr><td>J / K, ↓ / ↑</td><td>Next / previous story</td></tr><tr><td>S / M</td><td>Save / toggle read</td></tr><tr><td>F / O / R</td><td>Focus / original / refresh</td></tr><tr><td>Space / Shift + Space</td><td>Scroll article down / up</td></tr><tr><td>Tab / Shift + Tab</td><td>Move keyboard focus</td></tr><tr><td>Ctrl + C</td><td>Copy selected text</td></tr><tr><td>Ctrl + Q</td><td>Quit completely</td></tr><tr><td>Escape</td><td>Close dialog / leave focus mode</td></tr></table>"),
        Dialog::Image(source)=>out.push_str(&format!("<img class='enlarged' src='{}' alt='Enlarged article image'>",e(source))),
        _=>{}
    }
    out.push_str("<div class='dialog-footer'>");
    if let Dialog::EditFeed(id) = dialog {
        out.push_str(&a(&format!("feed-delete:{id}"), "Remove…", "button"));
    }
    out.push_str(&a(
        "cancel",
        if matches!(dialog, Dialog::Settings | Dialog::Help | Dialog::Image(_)) {
            "Done ♥"
        } else {
            "Cancel"
        },
        "button",
    ));
    if !matches!(
        dialog,
        Dialog::Settings | Dialog::Help | Dialog::Discover(_) | Dialog::Image(_)
    ) {
        out.push_str(&a(
            "submit",
            if matches!(dialog, Dialog::DeleteFeed(_) | Dialog::DeleteFolder(_)) {
                "Remove"
            } else if matches!(dialog, Dialog::AddFeed) {
                "Find feed →"
            } else if matches!(dialog, Dialog::Export) {
                "Export OPML"
            } else {
                "Save ♥"
            },
            "button primary",
        ));
    }
    out.push_str("</div></div></div>");
    out
}

pub fn date(timestamp: i64) -> String {
    // UTC ISO date needs no locale runtime; relative recency remains unambiguous.
    let days = timestamp.div_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    format!("{:04}-{m:02}-{d:02}", y + if m <= 2 { 1 } else { 0 })
}
