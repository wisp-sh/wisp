#!/usr/bin/env bash
# Run only in a disposable builder. This replaces its Debian apt sources.
set -euo pipefail

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 || "$EUID" != 0 ]]; then
  echo 'The builder bootstrap requires root in a disposable Linux amd64 container.' >&2
  exit 1
fi

source /etc/os-release
if [[ "$ID" != debian || "$VERSION_ID" != 13 ]]; then
  echo 'The builder bootstrap requires Debian 13.' >&2
  exit 1
fi

rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list
printf '%s\n' \
  'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260503T000000Z trixie main' \
  'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260503T000000Z trixie-updates main' \
  'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260503T000000Z trixie-security main' \
  > /etc/apt/sources.list
apt-get -o Acquire::Check-Valid-Until=false update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  binutils=2.44-3 ca-certificates curl gcc-14=14.2.0-19 git gzip \
  libc6-dev=2.41-12+deb13u2 rustup=1.27.1-3+b1 xz-utils \
  python3 python3-venv

test "$(/usr/bin/x86_64-linux-gnu-gcc-14 --version | sed -n '1p')" = \
  'x86_64-linux-gnu-gcc-14 (Debian 14.2.0-19) 14.2.0'
test "$(/usr/bin/x86_64-linux-gnu-ar --version | sed -n '1p')" = \
  'GNU ar (GNU Binutils for Debian) 2.44'

rustup toolchain install 1.98.1 --profile minimal
rustup default 1.98.1
rustup target add --toolchain 1.98.1 wasm32-unknown-unknown
"$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin/rustc" --version
"$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin/cargo" --version
