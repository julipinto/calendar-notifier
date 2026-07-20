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

# Chave pública do updater (mesma do tauri.conf.json). Base64 do arquivo .pub
# do minisign. Usada para verificar a assinatura .sig de cada release.
PUBKEY="dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDhERjE5MUY5NkM2OTZENTMKUldSVGJXbHMrWkh4amNIamo3MlNzWVhKdWYxUkZzc1M5WC9CMk8wbzk2Q0x6K3hrT0ovMEFXeTYK"

err() { printf '\033[31merro:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[36m::\033[0m %s\n' "$1"; }
warn() { printf '\033[33maviso:\033[0m %s\n' "$1" >&2; }

# Verifica a assinatura minisign de um arquivo baixado (defesa em profundidade).
# O .sig do release (formato Tauri) é base64 de uma assinatura minisign normal.
# Melhor-esforço: se 'minisign' não estiver instalado, avisa e segue — o download
# já é HTTPS a partir do repo oficial.
verify_sig() {
  file="$1"; sig_url="$2"
  if ! command -v minisign >/dev/null 2>&1; then
    warn "minisign não encontrado — assinatura NÃO verificada."
    warn "Para validar, instale 'minisign' (Arch: 'sudo pacman -S minisign', Debian: 'sudo apt install minisign') e rode de novo."
    return 0
  fi
  sig_b64="${tmp}/asset.sig.b64"
  sig="${tmp}/asset.sig"
  if ! curl -fsSL --retry 3 "$sig_url" -o "$sig_b64"; then
    err "não achei o .sig do asset — abortando (não dá pra verificar a assinatura)."
  fi
  base64 -d "$sig_b64" > "$sig" 2>/dev/null || err "arquivo .sig inválido."
  key="$(printf '%s' "$PUBKEY" | base64 -d 2>/dev/null | tail -n1)"
  [ -n "$key" ] || err "não consegui decodificar a chave pública."
  info "Verificando assinatura (minisign)..."
  minisign -V -P "$key" -x "$sig" -m "$file" >/dev/null 2>&1 \
    || err "assinatura inválida — abortando (arquivo corrompido ou adulterado)."
  info "Assinatura OK."
}

# Dependências mínimas.
command -v curl >/dev/null 2>&1 || err "curl não encontrado."

# Só amd64/x86_64 é publicado hoje.
arch="$(uname -m)"
[ "$arch" = "x86_64" ] || [ "$arch" = "amd64" ] || \
  err "arquitetura '$arch' não suportada (só x86_64)."

api="https://api.github.com/repos/${REPO}/releases/latest"
info "Buscando último release..."
json="$(curl -fsSL --retry 3 "$api")" || err "não consegui consultar o GitHub."

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
  curl -fSL --retry 3 --progress-bar "$url" -o "$out"
  verify_sig "$out" "${url}.sig"
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

  # FUSE não é preciso pra extrair, mas é pra ABRIR o AppImage. Avisa antes.
  if ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    warn "libfuse2 parece ausente — o app não abre sem ela."
    warn "Instale: Arch 'sudo pacman -S fuse2', Debian/Ubuntu 'sudo apt install libfuse2'."
  fi

  mkdir -p "$BIN_DIR" "$DESKTOP_DIR" "${ICON_DIR}/hicolor"
  target="${BIN_DIR}/calendar-notifier.AppImage"
  info "Baixando AppImage..."
  curl -fSL --retry 3 --progress-bar "$url" -o "$target"
  verify_sig "$target" "${url}.sig"
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

  # No terminal só funciona sem caminho absoluto se ~/.local/bin estiver no PATH.
  case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) warn "${BIN_DIR} não está no PATH — pelo terminal rode com o caminho completo, ou adicione ao PATH." ;;
  esac
}

if command -v dpkg >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
  install_deb
else
  install_appimage
fi
