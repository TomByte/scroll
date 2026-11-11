#!/usr/bin/env bash
# Records the scripted tour of the demo example (crates/scroll-app/examples/demo) and writes
# assets/demo/scroll-demo.gif. macOS only. Needs screen recording permission for the terminal,
# and `brew install ffmpeg gifski imagemagick`. Keep the mouse pointer away from the window
# while it records.
#
#   scripts/record-demo.sh [WORK_DIR]
#
# WORK_DIR (default: a new temporary directory) keeps the workspace, the log and the video.

set -euo pipefail
cd "$(dirname "$0")/.."

# The window's frame in points, as WINDOW in the example sets it.
readonly WINDOW_RECT="284,160,1160,740"
# Longer than the tour; the GIF is cut where the tour ends.
readonly RECORD_SECONDS=45
# One GIF pixel per point of the window.
readonly GIF_WIDTH=1160
readonly GIF_FPS=12
# A little more than the window's corner radius, so nothing behind the window shows.
readonly CORNER_RADIUS=14
readonly OUTPUT=assets/demo/scroll-demo.gif

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target-ui}"
work="${1:-$(mktemp -d)}"
mkdir -p "$work"
log="$work/demo.log"
video="$work/demo.mov"
rm -rf "$work/data" "$video"

now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time'; }

cargo build --release -p scroll-app --example demo
"$CARGO_TARGET_DIR/release/examples/demo" "$work/data" >"$log" 2>&1 &
demo=$!
until grep -q "demo: window open" "$log"; do sleep 0.05; done

recording_started=$(now)
screencapture -x -v -V"$RECORD_SECONDS" -R"$WINDOW_RECT" "$video" &
recorder=$!
wait "$demo"
wait "$recorder"

tour_finished=$(sed -n 's/^demo: tour finished at unix //p' "$log")
# Stops a little before the app quits, so the GIF never shows the desktop.
length=$(echo "$tour_finished - $recording_started - 0.3" | bc)

IFS=, read -r _ _ window_width window_height <<<"$WINDOW_RECT"
gif_height=$((GIF_WIDTH * window_height / window_width / 2 * 2))
mask="$work/mask.png"
magick -size "${GIF_WIDTH}x$gif_height" xc:black +antialias -fill white \
    -draw "roundrectangle 0,0,$((GIF_WIDTH - 1)),$((gif_height - 1)),$CORNER_RADIUS,$CORNER_RADIUS" "$mask"

# Frames with the corners cut out, then gifski for a palette per frame. Full motion quality, or
# faint text of an earlier page lingers on the dark background.
frames="$work/frames"
rm -rf "$frames"
mkdir -p "$frames" "$(dirname "$OUTPUT")"
ffmpeg -v error -y -t "$length" -i "$video" -loop 1 -t "$length" -i "$mask" \
    -filter_complex "[0:v]fps=$GIF_FPS,scale=$GIF_WIDTH:$gif_height:flags=lanczos[scaled];[scaled][1:v]alphamerge" \
    "$frames/%04d.png"
gifski --quiet --fps "$GIF_FPS" --quality 90 --motion-quality 100 --width "$GIF_WIDTH" -o "$OUTPUT" "$frames"/*.png
echo "wrote $OUTPUT ($(du -h "$OUTPUT" | cut -f1)); recording in $work"
