//! One accessory collection per process. Slots are allocated without replacement
//! across all documents, including the empty reader; tray reopen keeps the set.
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};
use trust::{
    core::CssPoint,
    embed::EmbeddedAttributes,
    render::{
        Affine2d, CssRect, DisplayCommand as Paint, ImageFit, ImageResource, PaintBrush,
        PaintColor, PaintShape, PathElement, Scene, StrokeStyle,
    },
};

macro_rules! artwork {
    ($($name:literal),+ $(,)?) => {
        pub const ARTWORK: &[(&str, &[u8])] = &[$(($name, include_bytes!(concat!("../../../assets/", $name, ".png")))),+];
    };
}
artwork!(
    "bow",
    "comet",
    "butterfly",
    "letter",
    "mouse",
    "rose",
    "bunny",
    "cherries",
    "gummy-bear",
    "lollipop",
    "daisy-clip",
    "heart-glasses",
    "bracelet",
    "planet",
    "mushroom",
    "pinwheel",
    "jelly-star",
    "kitten",
    "strawberry",
    "virtual-pet",
    "butterfly-clip",
    "raincloud",
    "roller-skate",
    "candy",
    "duck",
    "flower-pin",
    "cassette",
    "seashell",
    "axolotl",
);

pub struct Decora {
    pub welcome: &'static str,
    slots: Vec<&'static str>,
    phase: f32,
    bursts: HashMap<&'static str, f32>,
}
const FIDGET_SECONDS: f32 = 1.6;

#[derive(Clone, Copy, Debug)]
pub struct CharmHit {
    pub name: &'static str,
}
impl Decora {
    pub fn fresh() -> Self {
        let seed = std::env::var("RUBY_READER_DECORA_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64
                    ^ u64::from(std::process::id())
            });
        Self::seeded(seed)
    }
    pub fn seeded(mut seed: u64) -> Self {
        fn next(seed: &mut u64) -> u64 {
            *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = *seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^ (z >> 31)
        }
        let welcome = ["bunny", "kitten", "axolotl"][(next(&mut seed) % 3) as usize];
        // The bow belongs to the wordmark; the ruby heart belongs to the tray.
        let mut slots: Vec<_> = ARTWORK
            .iter()
            .map(|a| a.0)
            .filter(|n| *n != "bow" && *n != welcome)
            .collect();
        for i in (1..slots.len()).rev() {
            let j = (next(&mut seed) % (i as u64 + 1)) as usize;
            slots.swap(i, j);
        }
        Self {
            welcome,
            slots,
            phase: (next(&mut seed) % 628) as f32 / 100.0,
            bursts: HashMap::new(),
        }
    }
    pub fn slot(&self, index: usize) -> &'static str {
        self.slots[index]
    }

    pub fn poke(&mut self, name: &'static str, seconds: f32) {
        self.bursts
            .retain(|_, start| seconds - *start < FIDGET_SECONDS);
        self.bursts.insert(name, seconds);
    }
    pub fn active(&self, seconds: f32) -> bool {
        self.bursts
            .values()
            .any(|start| (0.0..FIDGET_SECONDS).contains(&(seconds - start)))
    }

    /// Hits the pixels actually presented, including native wiggles, CSS
    /// rotations, scroll and clipping. Transparent holes click through.
    pub fn hit(scene: &Scene, doc: &impl EmbeddedAttributes, point: CssPoint) -> Option<CharmHit> {
        let mut transform = Affine2d::IDENTITY;
        let mut transforms = Vec::new();
        let mut clips = Vec::new();
        let mut found = None;
        for command in &scene.primitives {
            match command {
                Paint::PushTransform(next) => {
                    transforms.push(transform);
                    transform = transform.then(*next);
                }
                Paint::PopTransform => transform = transforms.pop().unwrap_or_default(),
                Paint::PushClip(shape) => clips.push(transform.inverse().is_some_and(|inverse| {
                    let local = inverse.map_point(point);
                    match shape {
                        PaintShape::Rect(rect) | PaintShape::RoundedRect { rect, .. } => {
                            rect.contains(local)
                        }
                        _ => false, // Artwork only uses rectangular viewport clips.
                    }
                })),
                Paint::PopClip => {
                    clips.pop();
                }
                Paint::Image {
                    node,
                    rect,
                    handle,
                    fit,
                    clip,
                    ..
                } if clips.iter().all(|inside| *inside) => {
                    if let Some(name) = doc
                        .attribute(*node, "data-charm")
                        .and_then(|name| ARTWORK.iter().find(|(id, _)| *id == name).map(|a| a.0))
                        && let Some(local) =
                            transform.inverse().map(|inverse| inverse.map_point(point))
                        && clip.is_none_or(|clip| clip.contains(local))
                        && let Some(image) = scene.image_store.get(*handle)
                        && opaque_pixel(&image, *rect, *fit, local)
                    {
                        found = Some(CharmHit { name });
                    }
                }
                _ => {}
            }
        }
        found
    }

    /// Animate only explicitly marked, application-owned accessory images.
    /// A local display-list transform needs no layout invalidation or JS timer.
    pub fn animate(
        &self,
        scene: &mut Scene,
        doc: &impl EmbeddedAttributes,
        seconds: f32,
        reduced: bool,
    ) {
        if reduced && !self.active(seconds) {
            return;
        }
        let mut painted = Vec::with_capacity(scene.primitives.len() + 40);
        for command in scene.primitives.drain(..) {
            let mut burst = None;
            let motion = if let Paint::Image { node, rect, .. } = &command {
                doc.attribute(*node, "data-decora")
                    .filter(|kind| !kind.is_empty())
                    .map(|kind| {
                        let t = seconds * 2.0 + *node as f32 * 1.7 + self.phase;
                        let (mut dx, mut dy, mut angle) = match kind {
                            "bounce" => (0.0, -9.0 * (t.sin() * 0.5 + 0.5), t.sin() * 0.08),
                            "drift" => (t.cos() * 5.0, t.sin() * 5.0, t.sin() * 0.15),
                            "welcome-mascot" => (0.0, -4.0 * t.sin(), t.sin() * 0.07),
                            _ => (0.0, 0.0, t.sin() * 0.20),
                        };
                        if reduced {
                            dx = 0.0;
                            dy = 0.0;
                            angle = 0.0;
                        }
                        let age = doc
                            .attribute(*node, "data-charm")
                            .and_then(|name| self.bursts.get(name))
                            .map(|start| seconds - start)
                            .filter(|age| (0.0..FIDGET_SECONDS).contains(age));
                        let mut scale = 1.0;
                        if let Some(age) = age {
                            burst = Some((*rect, age));
                            if !reduced {
                                let strength = (1.0 - age / FIDGET_SECONDS).powi(2);
                                dx += (age * 39.0).sin() * 5.0 * strength;
                                dy -= (age * 13.0).sin().abs() * 8.0 * strength;
                                angle += (age * 32.0).sin() * 0.25 * strength;
                                scale += (age * 11.0).sin().abs() * 0.12 * strength;
                            }
                        }
                        let cx = rect.x + rect.width * 0.5;
                        let cy = rect.y + rect.height * 0.5;
                        let (sin, cos) = angle.sin_cos();
                        Affine2d::translate(cx + dx, cy + dy)
                            .then(Affine2d([cos, sin, -sin, cos, 0.0, 0.0]))
                            .then(Affine2d::scale(scale, scale))
                            .then(Affine2d::translate(-cx, -cy))
                    })
            } else {
                None
            };
            if let Some(transform) = motion {
                painted.push(Paint::PushTransform(transform));
            }
            painted.push(command);
            if let Some((rect, age)) = burst {
                fidget_sparkles(&mut painted, rect, age, reduced);
            }
            if motion.is_some() {
                painted.push(Paint::PopTransform);
            }
        }
        scene.primitives = painted;
    }

    /// Colorful, slow-breathing jeweled stars confined to decorative chrome.
    /// No animation or particle ever crosses article text or toolbar hit areas.
    pub fn paint(
        &self,
        scene: &mut Scene,
        width: f32,
        height: f32,
        seconds: f32,
        reduced: bool,
        focus: bool,
    ) {
        if focus {
            return;
        }
        let time = if reduced { 0.0 } else { seconds };
        scene
            .primitives
            .push(Paint::PushClip(PaintShape::Rect(CssRect::new(
                0.0, 26.0, width, 80.0,
            ))));
        let colors = [
            (229, 32, 117),
            (3, 165, 169),
            (224, 139, 10),
            (143, 75, 215),
        ];
        // Leave the wordmark and tagline completely unobstructed.
        let count = ((width - 414.0) / 36.0).max(6.0) as usize;
        for i in 0..count {
            let phase = self.phase + i as f32 * 2.399;
            let pulse = 0.5 + 0.5 * (time * 2.3 + phase).sin();
            let x = 422.0 + i as f32 * (width - 444.0) / count as f32;
            let y = 36.0 + ((i * 31) % 62) as f32 + (time * 1.4 + phase).sin() * 4.0;
            let r = 3.5 + pulse * if i % 3 == 0 { 7.5 } else { 4.0 };
            let (red, green, blue) = colors[i % colors.len()];
            star(scene, x, y, r, PaintColor::Rgba(red, green, blue, 255));
        }
        scene.primitives.push(Paint::PopClip);
        // Little jewel rivets on the outside rails, not on the reading paper.
        for i in 0..6 {
            let y = 210.0 + i as f32 * ((height - 275.0) / 6.0);
            let x = if i % 2 == 0 { 6.0 } else { width - 6.0 };
            let r = 3.0 + (time * 1.8 + i as f32).sin() * 1.0;
            let (red, green, blue) = colors[i % colors.len()];
            star(scene, x, y, r, PaintColor::Rgba(red, green, blue, 255));
        }
    }
}

fn opaque_pixel(image: &ImageResource, rect: CssRect, fit: ImageFit, point: CssPoint) -> bool {
    if !rect.contains(point) || image.width == 0 || image.height == 0 {
        return false;
    }
    let (iw, ih) = (image.width as f32, image.height as f32);
    let scale = match fit {
        ImageFit::Contain => (rect.width / iw).min(rect.height / ih),
        ImageFit::Cover => (rect.width / iw).max(rect.height / ih),
        ImageFit::ScaleDown => (rect.width / iw).min(rect.height / ih).min(1.0),
        ImageFit::None => 1.0,
        ImageFit::Fill => 1.0,
    };
    let (w, h) = if matches!(fit, ImageFit::Fill) {
        (rect.width, rect.height)
    } else {
        (iw * scale, ih * scale)
    };
    let u = (point.x - rect.x - (rect.width - w) * 0.5) / w;
    let v = (point.y - rect.y - (rect.height - h) * 0.5) / h;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return false;
    }
    let x = (u * iw) as usize;
    let y = (v * ih) as usize;
    image.rgba[(y * image.width as usize + x) * 4 + 3] > 48
}

fn fidget_sparkles(painted: &mut Vec<Paint>, rect: CssRect, age: f32, reduced: bool) {
    let progress = if reduced { 0.2 } else { age / FIDGET_SECONDS };
    let alpha = if reduced {
        240
    } else {
        ((1.0 - progress).sqrt() * 255.0) as u8
    };
    let colors = [
        (238, 37, 121),
        (5, 172, 184),
        (245, 165, 22),
        (155, 85, 225),
    ];
    // Local to the clicked toy. Parent viewport clips still prevent overflow
    // onto feed controls, toolbar buttons, or the reading paper.
    for i in 0..9 {
        let angle = i as f32 * std::f32::consts::TAU / 9.0 + 0.35;
        let radius = rect.width.min(rect.height) * 0.38 + 6.0 + progress * 18.0;
        let x = rect.x + rect.width * 0.5 + angle.cos() * radius;
        let y = rect.y + rect.height * 0.5 + angle.sin() * radius;
        let size = (if i % 2 == 0 { 7.0 } else { 4.5 }) * (1.0 - progress * 0.55);
        let (r, g, b) = colors[i % colors.len()];
        star_commands(painted, x, y, size, PaintColor::Rgba(r, g, b, alpha), alpha);
    }
}

fn star(scene: &mut Scene, x: f32, y: f32, r: f32, color: PaintColor) {
    star_commands(&mut scene.primitives, x, y, r, color, 255);
}
fn star_commands(painted: &mut Vec<Paint>, x: f32, y: f32, r: f32, color: PaintColor, alpha: u8) {
    let mut path = Vec::with_capacity(9);
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::FRAC_PI_4;
        let radius = if i % 2 == 0 { r } else { r * 0.23 };
        let p = CssPoint::new(x + angle.sin() * radius, y - angle.cos() * radius);
        path.push(if i == 0 {
            PathElement::MoveTo(p)
        } else {
            PathElement::LineTo(p)
        });
    }
    path.push(PathElement::Close);
    let shape = PaintShape::Path(path);
    painted.push(Paint::Stroke {
        shape: shape.clone(),
        brush: PaintBrush::Solid(PaintColor::Rgba(255, 253, 244, alpha)),
        style: StrokeStyle::solid(3.0),
    });
    painted.push(Paint::Fill {
        shape,
        brush: PaintBrush::Solid(color),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn collections_are_unique_stable_and_varied() {
        for seed in 0..128 {
            let a = Decora::seeded(seed);
            let b = Decora::seeded(seed);
            assert_eq!(a.slots, b.slots);
            assert_eq!(a.welcome, b.welcome);
            assert!(!a.slots.contains(&a.welcome));
            assert!(!a.slots.contains(&"bow"));
            assert_eq!(a.slots.len(), a.slots.iter().collect::<HashSet<_>>().len());
            assert!(a.slots.len() >= 20);
        }
        assert_ne!(Decora::seeded(1).slots, Decora::seeded(2).slots);
    }
    #[test]
    fn every_accessory_is_a_transparent_runtime_image() {
        for (name, bytes) in ARTWORK {
            let image = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(image.color().has_alpha(), "{name}");
            assert!(image.to_rgba8().pixels().any(|p| p.0[3] == 0), "{name}");
            assert!(
                image
                    .to_rgba8()
                    .pixels()
                    .filter(|p| p.0[3] > 200 && p.0[..3].iter().any(|c| *c > 150))
                    .count()
                    > 100,
                "{name}: the mask must not replace the original RGB artwork"
            );
        }
    }

    #[test]
    fn fidgets_are_local_bounded_and_retriggerable() {
        let mut decora = Decora::seeded(42);
        decora.poke("bow", 0.0);
        assert!(decora.active(1.0));
        assert!(!decora.active(FIDGET_SECONDS));
        decora.poke("bow", 1.2);
        assert!(decora.active(2.0));
        assert_eq!(decora.bursts.len(), 1);
        decora.poke("bracelet", 2.0);
        assert_eq!(decora.bursts.len(), 2);
        decora.poke("cherries", 5.0);
        assert_eq!(decora.bursts.len(), 1);
    }

    #[test]
    fn bracelet_hole_is_really_transparent() {
        let image = image::load_from_memory(ARTWORK.iter().find(|a| a.0 == "bracelet").unwrap().1)
            .unwrap()
            .to_rgba8();
        let center = image.get_pixel(image.width() / 2, image.height() / 2);
        assert_eq!(center.0[3], 0);
        // Don't silently turn the whole accessory into an empty mask.
        assert!(image.pixels().filter(|p| p.0[3] > 200).count() > 2_000);
    }
}
