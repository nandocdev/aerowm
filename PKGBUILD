# Maintainer: Fernando Castillo <fdocst@gmail.com>
pkgname=aerowm-git
pkgver=0.1.0
pkgrel=1
pkgdesc="High-performance Wayland dynamic tiling window manager in Rust + Luau"
arch=('x86_64' 'aarch64')
url="https://github.com/nandocdev/aerowm"
license=('MIT')
depends=('wayland' 'libxkbcommon' 'systemd' 'pixman' 'libinput' 'seatd')
makedepends=('cargo' 'git')
provides=('aerowm')
conflicts=('aerowm')
source=("git+https://github.com/nandocdev/aerowm.git")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/aerowm"
  printf "0.1.0.r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

build() {
  cd "$srcdir/aerowm"
  # Build aerowm, aerowm-ctl, and aerowm-bar
  cargo build --release --locked --all-targets --features xwayland
}

package() {
  cd "$srcdir/aerowm"
  
  install -Dm755 "target/release/aerowm" -t "$pkgdir/usr/bin/"
  install -Dm755 "target/release/aerowm-ctl" -t "$pkgdir/usr/bin/"
  install -Dm755 "target/release/aerowm-bar" -t "$pkgdir/usr/bin/"
  
  install -Dm644 "assets/aerowm.desktop" -t "$pkgdir/usr/share/wayland-sessions/"
  
  install -Dm644 "examples/config.luau" -t "$pkgdir/usr/share/doc/aerowm/examples/"
}
