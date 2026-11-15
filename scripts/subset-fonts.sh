#!/usr/bin/env sh
# Makes the stand-in for Inter that a browser shows its first frame in: the same face with only the
# characters `letters_fonts::FIRST_COVERS` lists, a third of the download. Run it when Inter.ttf or the
# list changes. Needs fonttools (`pip install fonttools`).
#
# The glyphs keep their numbers, their advances at every weight and size, and every layout feature, so
# text set in the stand-in is set the same in the whole face: the crate's tests hold the two together.
set -eu
cd "$(dirname "$0")/../crates/letters-fonts/fonts"

# Latin of western and central Europe, punctuation, the euro and trade mark signs, arrows, the minus sign,
# the signs for keys, the tick, and the marks a text may carry unseen.
covers="U+0020-007E,U+00A0-017F,U+2000-206F,U+20AC,U+2122,U+2190-21FF,U+2212,U+2300-23FF,U+2713,U+FEFF,U+FFFD"

pyftsubset Inter.ttf --unicodes="$covers" --layout-features='*' --name-IDs='*' --retain-gids --notdef-outline \
  --output-file=Inter-Latin.ttf
ls -l Inter.ttf Inter-Latin.ttf
