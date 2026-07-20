#!/usr/bin/env sh
# Instalador do Calendar Notifier para Linux.
#
# Uso:
#   curl -fsSL https://raw.githubusercontent.com/julipinto/calendar-notifier/main/scripts/install.sh | sh
#
# O que faz:
#   - Descobre o último release no GitHub.
#   - Se o sistema tem apt/dpkg (Debian/Ubuntu): baixa e instala o .deb.
#   - Caso contrário (Arch, Fedora, etc.): baixa o AppImage para ~/.local/bin
#     e cria um atalho no menu de aplicativos.
set -eu

REPO="julipinto/calendar-notifier"
APP_NAME="Calendar Notifier"
BIN_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons"

err() { printf '\033[31merro:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[36m::\033[0m %s\n' "$1"; }

# Dependências mínimas.
command -v curl >/dev/null 2>&1 || err "curl não encontrado."

# Só amd64/x86_64 é publicado hoje.
arch="$(uname -m)"
[ "$arch" = "x86_64" ] || [ "$arch" = "amd64" ] || \
  err "arquitetura '$arch' não suportada (só x86_64)."

api="https://api.github.com/repos/${REPO}/releases/latest"
info "Buscando último release..."
json="$(curl -fsSL "$api")" || err "não consegui consultar o GitHub."

# Extrai a URL de download de um asset pelo sufixo do nome (sem jq).
asset_url() {
  suffix="$1"
  printf '%s\n' "$json" \
    | grep -o '"browser_download_url": *"[^"]*"' \
    | sed 's/.*"browser_download_url": *"//; s/"$//' \
    | grep -E "${suffix}\$" \
    | head -n1
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

install_deb() {
  url="$(asset_url '_amd64\.deb')"
  [ -n "$url" ] || err "asset .deb não encontrado no release."
  out="${tmp}/pkg.deb"
  info "Baixando .deb..."
  curl -fSL --progress-bar "$url" -o "$out"
  info "Instalando (pode pedir sua senha)..."
  if command -v apt >/dev/null 2>&1; then
    sudo apt install -y "$out"
  else
    sudo dpkg -i "$out" || sudo apt-get -f install -y
  fi
  info "Pronto. Abra pelo menu ou rode: calendar-notifier"
}

install_appimage() {
  url="$(asset_url '_amd64\.AppImage')"
  [ -n "$url" ] || err "asset .AppImage não encontrado no release."
  mkdir -p "$BIN_DIR" "$DESKTOP_DIR" "${ICON_DIR}/hicolor"
  target="${BIN_DIR}/calendar-notifier.AppImage"
  info "Baixando AppImage..."
  curl -fSL --progress-bar "$url" -o "$target"
  chmod +x "$target"

  # Extrai o .desktop e os ícones que o próprio Tauri embutiu no AppImage —
  # assim o nome, categorias e ícone ficam idênticos ao build oficial.
  info "Registrando no menu de aplicativos..."
  (
    cd "$tmp"
    "$target" --appimage-extract 'usr/share/applications/*' >/dev/null 2>&1 || true
    "$target" --appimage-extract 'usr/share/icons/*' >/dev/null 2>&1 || true
    "$target" --appimage-extract '*.png' >/dev/null 2>&1 || true
  )
  root="${tmp}/squashfs-root"

  # Copia a árvore de ícones (hicolor) — os search engines resolvem por nome.
  if [ -d "${root}/usr/share/icons/hicolor" ]; then
    cp -r "${root}/usr/share/icons/hicolor/." "${ICON_DIR}/hicolor/" 2>/dev/null || true
  fi

  # O .desktop real fica em usr/share/applications (na raiz é só um symlink).
  src_desktop="$(find "${root}/usr/share/applications" -maxdepth 1 -name '*.desktop' 2>/dev/null | head -n1 || true)"
  # Nome do arquivo = app_id (StartupWMClass) p/ o ícone agrupar no Wayland.
  dst_desktop="${DESKTOP_DIR}/calendar-notifier.desktop"

  if [ -n "$src_desktop" ]; then
    # Reaproveita o .desktop oficial: Exec absoluto e Categories preenchido
    # (o do build vem vazio, o que atrapalha a categorização no menu).
    sed -E \
      -e "s#^Exec=.*#Exec=${target}#" \
      -e "s#^TryExec=.*#TryExec=${target}#" \
      -e "s#^Categories=\$#Categories=Office;Calendar;#" \
      "$src_desktop" > "$dst_desktop"
  else
    # Fallback: monta um .desktop mínimo.
    icon_name="calendar-notifier"
    fallback_icon="$(find "$root" -maxdepth 3 -name '*.png' 2>/dev/null | head -n1 || true)"
    [ -n "$fallback_icon" ] && cp "$fallback_icon" "${ICON_DIR}/${icon_name}.png"
    cat > "$dst_desktop" <<EOF
[Desktop Entry]
Name=${APP_NAME}
Exec=${target}
Icon=${icon_name}
Type=Application
Categories=Office;Calendar;
Terminal=false
StartupWMClass=calendar-notifier
EOF
  fi

  # Garante StartupWMClass (agrupa a janela com o ícone do launcher).
  grep -qi '^StartupWMClass=' "$dst_desktop" || \
    printf 'StartupWMClass=calendar-notifier\n' >> "$dst_desktop"

  # Atualiza os caches para aparecer na busca sem precisar relogar.
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && \
    gtk-update-icon-cache -f -t "${ICON_DIR}/hicolor" >/dev/null 2>&1 || true

  info "Instalado em: $target"
  info "Aparece no menu/busca como \"${APP_NAME}\" (pode levar alguns segundos)."
  info "Precisa de FUSE: Arch 'sudo pacman -S fuse2', Debian 'sudo apt install libfuse2'."
}

if command -v dpkg >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
  install_deb
else
  install_appimage
fi
