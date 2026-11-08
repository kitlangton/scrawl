#!/usr/bin/env bash
# Ubuntu 24.04 build/packaging dependencies, shared by check and release jobs.
set -euo pipefail
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config cmake ninja-build clang \
  libxkbcommon-dev libxkbcommon-x11-dev libvulkan-dev \
  libx11-dev libx11-xcb-dev libxcb1-dev libxcb-shm0-dev libxcb-xfixes0-dev \
  libxcb-shape0-dev libxcb-randr0-dev libxcb-composite0-dev libxcb-xinput-dev \
  libxcb-xtest0-dev libasound2-dev libfontconfig-dev libfreetype-dev libssl-dev \
  libdbus-1-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  patchelf file ffmpeg wl-clipboard gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav gstreamer1.0-pipewire
