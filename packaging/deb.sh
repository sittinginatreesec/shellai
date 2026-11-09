#!/usr/bin/env bash
set -euo pipefail
binary=${1:?binary path required}
version=${2:?version required}
arch=${3:?Debian architecture required}
case "$arch" in amd64|arm64) ;; *) exit 1 ;; esac
[[ "$version" =~ ^[0-9][0-9A-Za-z.+:~-]*$ ]]
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
install -Dm755 "$binary" "$root/usr/bin/shellai"
install -Dm644 shellai.plugin.zsh "$root/usr/share/zsh/plugins/shellai/shellai.plugin.zsh"
install -Dm644 LICENSE "$root/usr/share/doc/shellai/copyright"
install -Dm644 README.md "$root/usr/share/doc/shellai/README.md"
mkdir -p "$root/DEBIAN" "$root/etc/zsh/zshrc.d" dist
cat > "$root/etc/zsh/zshrc.d/shellai.zsh" <<'ZSH'
# Debian's global zshrc does not load zshrc.d automatically; postinst enables it.
source /usr/share/zsh/plugins/shellai/shellai.plugin.zsh
ZSH
# Use a dpkg-managed conffile and an idempotent, removable global source line.
cat > "$root/DEBIAN/postinst" <<'SH'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    line='[ ! -r /etc/zsh/zshrc.d/shellai.zsh ] || source /etc/zsh/zshrc.d/shellai.zsh # shellai package'
    grep -qxF "$line" /etc/zsh/zshrc || printf '\n%s\n' "$line" >> /etc/zsh/zshrc
fi
SH
cat > "$root/DEBIAN/postrm" <<'SH'
#!/bin/sh
set -e
case "$1" in remove|purge)
    if [ -f /etc/zsh/zshrc ]; then
        sed -i '\@^\[ ! -r /etc/zsh/zshrc\.d/shellai\.zsh \] || source /etc/zsh/zshrc\.d/shellai\.zsh # shellai package$@d' /etc/zsh/zshrc
    fi
;; esac
SH
chmod 755 "$root/DEBIAN/postinst" "$root/DEBIAN/postrm"
printf '/etc/zsh/zshrc.d/shellai.zsh\n' > "$root/DEBIAN/conffiles"
mkdir -p "$root/debian"
printf 'Source: shellai\n\nPackage: shellai\nArchitecture: any\n' > "$root/debian/control"
binary_path=$(realpath "$root/usr/bin/shellai")
deps=$(cd "$root" && dpkg-shlibdeps -O -e"$binary_path" | sed -n 's/^shlibs:Depends=//p')
rm -rf "$root/debian"
cat > "$root/DEBIAN/control" <<CONTROL
Package: shellai
Version: $version
Architecture: $arch
Maintainer: Shellai maintainers <maintainers@users.noreply.github.com>
Depends: zsh, $deps
Section: shells
Priority: optional
Description: Local AI command generation for ZSH
 Native command generation with an interactive ZSH plugin.
CONTROL
dpkg-deb --root-owner-group --build "$root" "dist/shellai_${version}_${arch}.deb"
