# Ruby Reader original artwork

Generated locally with Ideogram V4_TURBO_12 at 1536 × 1024, September 2026.
Source PNGs and returned generation metadata are retained here. Runtime artwork
is prepared under `assets/`. Typography and functional controls are authored
independently so lettering remains crisp, selectable, and accessible.

Art direction: ruby and candy-pink Decora scrapbook, glossy enamel trinkets,
pearl and lace edging, holographic foil, pixel stars, tiny flowers, bows and
heart charms. Transparent-ready isolated ornaments on a flat white backdrop.

## Toy-box polish, 6 September 2026

Two further local Ideogram V4_TURBO_12 generations at 1536 × 1152:

- `decora-toys-01.png`, seed 214551081: plush bunny, cherries, gummy bear,
  lollipop, daisy clip, winged glasses, bracelet, planet, mushroom, pinwheel,
  jelly star. The generated milk carton was rejected because it contained
  unwanted lettering; it is not cropped or shipped in the runtime catalogue.
- `decora-toys-02.png`, seed 1046322328: plush kitten, rhinestone strawberry,
  virtual pet, butterfly clip, raincloud, roller skate, wrapped candy, duck,
  flower safety pin, cassette, seashell, plush axolotl.

Both prompts requested twelve distinct original accessories on an isolated,
plain background in a 4×3 sheet: tactile plush, glitter, resin, translucent
plastic, saturated candy colors, no brands, captions, or franchise characters.
The reviewed boundaries differ between sheets; `prepare-toys.sh` records the
actual crops, removes only edge-connected background, trims, and downsamples
to at most 280 pixels plus a six-pixel transparent margin. Full-resolution
originals remain here. All 23 prepared accessories were reviewed on pink.

Styling inspiration: [Tokyo Fashion's Harajuku street photography](https://tokyofashion.com/kawaii-harajuku-decora-fashion-6dokidoki/),
particularly layered hair clips, toys, candy, and beads. No source photographs
or branded designs are bundled. The beaded garland and four-point jeweled
sparkles are authored in CSS and Rust, not raster images or textual stickers.

## Transparency QC and fidget pass

Reviewed all 29 runtime decorations on plum as well as the actual pastel UI.
`prepare-toys.sh` now retains an explicit source-resolution alpha-mask pipeline:
interior background seeds for the bracelet, cherry stems, metal key/safety rings,
planet and mushroom loops; separate lower-background cleanup for the glasses;
foreground guards restoring the original lollipop stick and white jelly-star
pearls. The generator originals and the colored artwork are not regenerated.
The old metallic/beveled sticker borders, reflective lenses, white plush and
physical pearls remain part of the objects, not background holes.

`artifacts/transparency-before.png` and `artifacts/transparency-after.png` record
the review. Intermediate source crops and grayscale masks are retained under
`artifacts/alpha-qc-*`. Pixel tests cover the bracelet hole and require visible
RGB artwork as well as alpha, so an accidental mask-only export fails validation.

## App theme wallpapers

`gingham.svg` is hand-authored vector artwork. `prepare-wallpapers.py` derives
warm sepia and deep plum variants by replacing its six colors, preserving the
checks and lace geometry. Run it with Python 3 from any directory to regenerate
`assets/gingham-sepia.svg` and `assets/gingham-dark.svg`. Existing Ideogram toys,
their source RGB, and their transparency masks are unchanged across themes.
