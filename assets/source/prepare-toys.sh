#!/usr/bin/env bash
# Reviewed Ideogram sheets. Explicit interior seeds remove enclosed background;
# foreground guards preserve pale physical details. Originals stay untouched.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
mkdir -p artifacts
reader_masks="$(mktemp -d "$PWD/artifacts/alpha-qc-XXXXXX")"
prepare_sheet() {
    local sheet="$1"
    shift
    local -a xs ys
    if [[ "$sheet" == *01.png ]]; then
        xs=(55 460 785 1120 1470)
        ys=(55 404 754 1095)
    else
        xs=(180 490 770 1030 1310)
        ys=(95 422 752 1060)
    fi
    local index=0
    for name in "$@"; do
        if [[ "$name" != skip ]]; then
            local col=$((index % 4)) row=$((index / 4))
            local gaps='color 0,0 floodfill' guards='' fuzz='5%'
            # Coordinates are in the untrimmed source crop, before resampling.
            case "$name" in
                bracelet) gaps+=' color 175,187 floodfill' ;;
                cherries) gaps+=' color 160,158 floodfill' ;;
                gummy-bear) gaps+=' color 177,75 floodfill color 177,112 floodfill' ;;
                strawberry) gaps+=' color 145,82 floodfill' ;;
                planet) gaps+=' color 177,126 floodfill' ;;
                mushroom) gaps+=' color 155,119 floodfill' ;;
                flower-pin) gaps+=' color 216,117 floodfill color 204,137 floodfill' ;;
                # The original flood ate these real white objects where they
                # meet the white backdrop. Restore their original RGB/alpha.
                lollipop) guards='polygon 181,161 190,161 190,273 181,273' ;;
                jelly-star) guards='ellipse 250,189 12,13 0,360 ellipse 236,214 12,13 0,360 ellipse 221,237 12,14 0,360' ;;
            esac
            local -a guard_args=()
            if [[ -n "$guards" ]]; then guard_args=(-fill white -draw "$guards"); fi
            magick "$sheet" -crop "$((xs[col+1]-xs[col]))x$((ys[row+1]-ys[row]))+${xs[col]}+${ys[row]}" +repage "$reader_masks/$name-source.png"
            magick "$reader_masks/$name-source.png" -alpha on -fuzz "$fuzz" -fill none -draw "$gaps" \
                -alpha extract "${guard_args[@]}" "$reader_masks/$name-mask.png"
            if [[ "$name" == heart-glasses ]]; then
                # Remove the floor visible under/between the lenses, without
                # mistaking the pale iridescent wings for background.
                magick "$reader_masks/$name-source.png" -alpha on -fuzz 12% -fill none \
                    -draw 'color 0,0 floodfill' -alpha extract \
                    -crop "$((xs[col+1]-xs[col]))x$((ys[row+1]-ys[row]-190))+0+190" +repage "$reader_masks/$name-lower.png"
                magick "$reader_masks/$name-mask.png" "$reader_masks/$name-lower.png" \
                    -geometry +0+190 -compose Over -composite "$reader_masks/$name-mask.png"
            fi
            magick "$reader_masks/$name-source.png" "$reader_masks/$name-mask.png" -alpha off -compose CopyOpacity -composite \
                -trim +repage -resize '280x280>' \
                -compose Over -bordercolor none -border 6 "assets/$name.png"
        fi
        index=$((index + 1))
    done
}
prepare_sheet assets/source/decora-toys-01.png \
    bunny cherries gummy-bear lollipop daisy-clip heart-glasses bracelet planet \
    skip mushroom pinwheel jelly-star
prepare_sheet assets/source/decora-toys-02.png \
    kitten strawberry virtual-pet butterfly-clip raincloud roller-skate candy duck \
    flower-pin cassette seashell axolotl
