# Calendar Notifier

App de desktop (Windows / Linux / macOS) que conecta **1 ou mais contas Google**,
sincroniza sua agenda e **avisa X minutos antes** de cada evento — com ícone na
bandeja e execução em segundo plano. Feito em **Tauri v2** (Rust + Svelte).
100% local: **sem servidor**, tokens guardados na sua máquina.

## Recursos

- Conectar **N contas Google** (OAuth, somente leitura da agenda).
- **Múltiplos avisos** por evento (ex.: 10 min e 2 min antes) — global ou por conta.
- **Sincronização automática** (a cada 30 min / 1 h / 4 h / 6 h / 12 h / 24 h) + manual.
- **Notificações do sistema** com som (configurável).
- **Bandeja (tray)** com próximo evento + iniciar **em segundo plano** no login.
- **Visão de lista e de mês**, busca por título, cores por calendário.
- **Filtros**: ignorar eventos recusados / de dia inteiro.
- Tratamento de **offline** e **reconexão** quando o token expira.
- **Atualização automática** (Windows e AppImage): o app detecta uma nova
  versão, baixa e instala sozinho — sem precisar baixar manualmente.

---

## Instalação

Baixe o instalador da sua plataforma na página de **[Releases](../../releases)**
(pegue a versão mais recente).

### Linux — instalação rápida (curl)
Detecta o gerenciador de pacotes e instala o `.deb` (Debian/Ubuntu) ou o
AppImage (Arch, Fedora, etc.) com atalho no menu:
```bash
curl -fsSL https://raw.githubusercontent.com/julipinto/calendar-notifier/main/scripts/install.sh | sh
```

### Windows
1. Baixe o **`.msi`** (recomendado) ou o **`.exe`**.
2. Execute. O **SmartScreen** pode avisar "app não reconhecido" (o app ainda não
   é assinado): clique em **Mais informações → Executar assim mesmo**.

### Linux — Debian / Ubuntu (`.deb`)
```bash
sudo apt install ./Calendar\ Notifier_*_amd64.deb
```
Depois abra pelo menu de aplicativos (**Calendar Notifier**) ou rode
`calendar-notifier` no terminal.

### Linux — Arch e outras distros (`.AppImage`)
O **AppImage** é portável e roda em qualquer distro:
```bash
chmod +x Calendar\ Notifier_*_amd64.AppImage
./Calendar\ Notifier_*_amd64.AppImage
```
Se reclamar de FUSE: no Arch, `sudo pacman -S fuse2`; no Debian/Ubuntu,
`sudo apt install libfuse2`.

> No GNOME, o ícone da bandeja pode exigir a extensão **AppIndicator/KStatusNotifier**.

---

## Primeiro uso

1. Abra o app → **Conectar conta** → autorize no navegador.
   - Como o app ainda **não passou pela verificação do Google**, aparece a tela
     "app não verificado": clique em **Avançado → Acessar Calendar Notifier**.
     (É seguro — é o seu próprio app.)
2. Escolha os calendários a acompanhar (em **Calendários**, na conta).
3. Ajuste os **avisos** e a **sincronização** nas configurações (⚙).

---

## Privacidade

- Comunicação **direta** com a API do Google, **sem servidor intermediário**.
- **Refresh token** guardado localmente: no **Windows/macOS** via keychain nativo
  (Credential Manager / Keychain); no **Linux**, em arquivo com permissão `600`.
- As credenciais OAuth do app (`google_client.json`) **não** vão para o
  repositório.

---

## Desenvolvimento

Requisitos: **Rust** (stable), **Node 20+**, e as libs de sistema do Tauri.

```bash
# libs de sistema (Debian/Ubuntu)
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf xdg-utils

npm install
npm run tauri dev      # rodar em desenvolvimento
npm run tauri build    # gerar instaladores da plataforma atual
cargo test --manifest-path src-tauri/Cargo.toml   # testes
```

É preciso um **`src-tauri/google_client.json`** (credencial OAuth tipo
*Desktop app* do Google Cloud Console). Ele é *gitignored*.

### Releases
Os instaladores são gerados por **GitHub Actions** (`.github/workflows/build.yml`)
ao criar uma tag `vX.Y.Z`:
```bash
# 1. suba a versão em package.json, src-tauri/tauri.conf.json e src-tauri/Cargo.toml
git commit -am "Release vX.Y.Z"
git tag vX.Y.Z
git push origin main && git push origin vX.Y.Z
```
O CI compila Windows + Linux e anexa `.msi`/`.exe`/`.deb`/`.AppImage` à release.
Requer o secret **`GOOGLE_CLIENT_JSON`** no repositório (conteúdo do
`google_client.json`).

### Notas sobre WSL (dev)
No WSL, a bandeja e as notificações não renderizam nativamente (não há host de
bandeja / daemon de notificação). O comportamento completo é validado no build
Windows/Linux nativo. Detalhes e decisões de arquitetura estão em `PLAN.md`.
