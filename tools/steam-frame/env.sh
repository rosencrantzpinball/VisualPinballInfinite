# Shared paths for the Steam Frame tools (sourced by the other scripts).
#
# Everything they build or download stays under WORK (the folder holding the repository, unless VPI_FRAME_WORK
# names another), on the data volume: the third-party sources (src/),
# their builds (build/), the Cargo home (cargo/) and the test runs (runs/). The repository's own git-ignored
# folders keep what the player finds in a development build: target/ and vpx-build/linux-aarch64/.
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
WORK=${VPI_FRAME_WORK:-$(dirname "$REPO")}
IMAGE=${VPI_FRAME_IMAGE:-vpi-steamrt4-arm64}
BASE_IMAGE=vpinball-steamrt4-arm64:latest
FRAME=${VPI_FRAME_HOST:-steamos@frame.local}

# Runs a command in the image, with WORK and the repository mounted at their own paths and Cargo's home on WORK.
in_image() {
   docker run --rm --init ${DOCKER_TTY:-} --platform linux/arm64 \
      -v "$WORK:$WORK" -v "$REPO:$REPO" \
      -e CARGO_HOME="$WORK/cargo" -e HOME="$WORK/home" \
      -w "$REPO" "${IMAGE_FOR_RUN:-$IMAGE}" "$@"
}
