# Maintainer: Akshay Bapat <https://github.com/akshaybapat6365>
pkgname=omarchy-spatial-audio
pkgver=0.1.0
pkgrel=1
pkgdesc='Pure-Rust Choueiri FDTR-XTC 3D Spatial Audio DSP, LADSPA plugin, and PipeWire filter-chain for laptop speakers'
arch=('x86_64' 'aarch64')
url='https://github.com/akshaybapat6365/omarchy-spatial-audio'
license=('MIT')
depends=('pipewire' 'wireplumber' 'libmysofa' 'lsp-plugins-lv2' 'python')
makedepends=('cargo' 'gcc')
source=()

build() {
  cd "${startdir}"
  cargo build --release --locked
  gcc -O2 -Wall -Wextra tests/verify_ladspa.c -ldl -lm -o target/release/verify_ladspa
}

check() {
  cd "${startdir}"
  cargo test --release --locked
  target/release/omarchy-spatial-dsp verify
  target/release/verify_ladspa target/release/libomarchy_spatial_fdtr.so
}

package() {
  local srcroot="${startdir}"
  local share="$pkgdir/usr/share/$pkgname"

  install -Dm755 "$srcroot/target/release/omarchy-spatial-dsp" "$pkgdir/usr/bin/omarchy-spatial-dsp"
  install -Dm755 "$srcroot/target/release/verify_ladspa" "$pkgdir/usr/lib/$pkgname/verify_ladspa"
  install -Dm755 "$srcroot/target/release/libomarchy_spatial_fdtr.so" "$pkgdir/usr/lib/ladspa/libomarchy_spatial_fdtr.so"
  install -Dm755 "$srcroot/bin/omarchy-audio-spatial" "$pkgdir/usr/bin/omarchy-audio-spatial"
  install -Dm644 "$srcroot/config/91-spatial-fdtr.conf.in" "$share/91-spatial-fdtr.conf.in"
  install -Dm755 "$srcroot/scripts/verify_pipewire_config.py" "$share/verify_pipewire_config.py"
  install -Dm644 "$srcroot/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

  install -dm755 "$share/kernels"
  "$srcroot/target/release/omarchy-spatial-dsp" generate-kernels --out-dir "$share/kernels"
}
