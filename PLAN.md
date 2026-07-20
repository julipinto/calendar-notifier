# Calendar Notifier — Plano de Arquitetura

App desktop (Tauri v2) que conecta 1..N contas Google, sincroniza a agenda
e notifica X minutos antes de cada evento. 100% local, sem servidor.

## Decisões travadas

| Tópico | Escolha |
|---|---|
| Framework | Tauri v2 (back Rust + front web) |
| Front-end | Svelte + TypeScript + Vite |
| Auth | OAuth 2.0 "Desktop / Installed App" com PKCE, loopback em `127.0.0.1` |
| Multi-conta | 1..N contas Google, cada uma com seu token |
| Sync | Polling periódico incremental (sync tokens) + botão "sincronizar agora" |
| Antecedência | Global configurável (default 10 min) |
| UI v1 | Ícone na bandeja (tray) + notificações do sistema + tela de config |
| Persistência | SQLite local + tokens no keychain do SO |
| Servidor | Nenhum |
| Alvo primário | Windows (multiplataforma: Win/macOS/Linux) |
| Dev | WSLg (build Linux) pra iterar; instaladores Windows via CI |

## Fluxo de autenticação (sem servidor)

1. Usuário clica "Conectar conta" → Rust sobe um `TcpListener` numa porta
   aleatória em `127.0.0.1` (listener efêmero, não é servidor exposto).
2. Rust gera `code_verifier`/`code_challenge` (PKCE) e abre o navegador na URL
   de consentimento do Google (`redirect_uri = http://127.0.0.1:<porta>`).
3. Usuário loga e autoriza. Google redireciona pro loopback com o `code`.
4. Rust captura o `code`, fecha o listener, troca `code` + `code_verifier` por
   `access_token` + `refresh_token`.
5. `refresh_token` vai pro keychain do SO; metadados da conta vão pro SQLite.
6. Repetir pra cada conta adicional (multi-conta).

**Pré-requisito único (uma vez):** criar projeto no Google Cloud Console,
ativar a Google Calendar API, criar credencial OAuth tipo "Desktop app" e
configurar a tela de consentimento. O Client ID fica embutido no app.
Com PKCE não é preciso tratar o client secret como sigiloso.

**Escopos:** `calendar.readonly` + `calendar.calendarlist.readonly` (só leitura).

## Sincronização

- Primeira sync de cada calendário: busca eventos numa janela (ex: agora até
  +30 dias) e guarda o `nextSyncToken`.
- Syncs seguintes: incremental via `syncToken` (barato, só o que mudou).
- Se o Google devolver `410 GONE`, o token expirou → full resync do calendário.
- Polling: intervalo configurável (default 5 min) por `tokio::interval`.
- Botão "Sincronizar agora" no tray força uma rodada imediata.
- Refresh de `access_token` automático quando expira (usando o refresh_token).

## Scheduler de notificações

- Após cada sync, recalcula os eventos futuros e o instante de notificação
  `notify_at = event.start - antecedência_global`.
- Um loop leve (tick a cada ~30s) dispara notificações cujo `notify_at` já
  passou e ainda não foram notificadas.
- Cada evento tem flag `notified` (dedup) pra não notificar duas vezes.
- Ignora eventos recusados/cancelados e all-day (configurável depois).

## Modelo de dados (SQLite)

- `accounts(id, email, display_name, created_at)`
- `calendars(id, account_id, google_calendar_id, summary, selected, sync_token)`
- `events(id, calendar_id, google_event_id, title, start_ts, end_ts,
   status, notify_at, notified, updated_at)`
- `settings(key, value)` — ex: `lead_minutes=10`, `poll_minutes=5`
- Tokens **não** ficam no SQLite — vão pro keychain (`keyring` crate).

> Nota WSL2: o keychain via Secret Service não existe no ambiente de dev.
> Em dev usamos fallback de arquivo cifrado (atrás de feature flag); no Windows
> real o `keyring` usa o Credential Manager. Decidido na Fase 1.

## Workflow de desenvolvimento (WSL → Windows)

- Código vive no WSL (`~/personal/calendar-notifier`).
- `tauri dev` roda via WSLg → build Linux (WebKitGTK), pra iterar rápido em
  OAuth, sync, scheduler e UI.
- Tray, notificações e keychain só validam de verdade no Windows → confirmados
  no primeiro build Windows (CI).
- Instaladores Windows (`.msi`/`.exe`) gerados por GitHub Actions (runner
  `windows-latest`) — não cross-compilar de Linux.
- Deps de dev no WSL (apt): `libwebkit2gtk-4.1-dev`, `librsvg2-dev`,
  `libnotify-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, além de Rust
  (rustup) e Node (via nvm).

## Estrutura de pastas

```
calendar-notifier/
  src/                      # front Svelte
    routes/ (config, contas)
    lib/ (chamadas invoke ao Rust)
  src-tauri/
    src/
      auth.rs               # OAuth PKCE + loopback + refresh
      google.rs             # cliente Calendar API (calendars, events, sync)
      store.rs              # SQLite (accounts, calendars, events, settings)
      secrets.rs            # keychain / fallback cifrado
      scheduler.rs          # polling + loop de notificações
      tray.rs               # tray icon + menu
      commands.rs           # comandos expostos ao front (#[tauri::command])
      main.rs
    tauri.conf.json
  PLAN.md
```

## Roadmap por fases

**Fase 0 — Scaffold** ✅
- `create-tauri-app` (Svelte + TS), plugins: notification, sql, opener.
- App abre, tray aparece, "Hello".

**Fase 1 — Auth OAuth (1 conta)** ✅
- Fluxo OAuth PKCE + loopback (`auth.rs`); conclusão automática (Windows) via
  evento `account-connected` + fallback manual (colar URL de redirect) p/ WSL.
- Refresh token em `tokens.json` (0600) — `secrets.rs`; conta em SQLite —
  `store.rs`. Credenciais via `include_str!` (`config.rs`, gitignored).
- UI: conectar / listar / remover / testar (conta a Calendar API real).
- **Gotchas do Google Cloud (para conectar contas):**
  1. Ativar a **Google Calendar API** no projeto (senão 403 `accessNotConfigured`).
  2. Registrar o escopo `calendar.readonly` na tela de consentimento via
     "Adicionar escopos manualmente" (senão 403 `ACCESS_TOKEN_SCOPE_INSUFFICIENT`).
  3. App em "Testing" → refresh token expira em 7 dias; publicar (Production)
     para uso contínuo.
- **WSL:** loopback `127.0.0.1` às vezes funciona (localhost forwarding), às
  vezes dá `ERR_CONNECTION_REFUSED` → usar o fallback de colar a URL.

**Fase 2 — Sync** ✅
- `google.rs`: cliente da Calendar API (calendarList + events, `singleEvents`).
- Tabelas `calendars` e `events` no SQLite; ao conectar, só o calendário
  principal vem marcado (`selected`); escolha do usuário preservada.
- Sync = fetch da janela deslizante (agora → +30d) e substituição dos eventos
  do calendário (transação). Simples e correto p/ janela móvel.
  > Nota: optamos por refetch da janela em vez de sync tokens — tokens não
  > combinam bem com janela deslizante. Otimização futura se necessário.
- UI: expandir calendários + checkbox, "recarregar calendários",
  "sincronizar agora", lista de próximos eventos (dia-inteiro renderizado em UTC).

**Fase 3 — Notificações + Scheduler** ✅
- `scheduler.rs`: loop (tick 30s) que dispara notificação do SO p/ eventos
  entrando na janela `start - lead`, com dedup (`notified`), ignorando dia-inteiro.
- Antecedência global (`lead_minutes`, default 10) configurável na UI.
- `replace_events` preserva o `notified` entre syncs (reseta se o horário muda).
- Botão "testar notificação" + campo de antecedência na UI.
- **Correção importante:** removida a feature `tokio` do zbus (workaround
  redundante da Fase 0). Ela fazia a chamada bloqueante do `notify-rust` rodar
  dentro do runtime tokio → `show()` falhava calado. Com async-io (padrão)
  funciona. Patch do `command.rs` mantido.
- **Dev WSL:** sem daemon de notificação nativo; instalamos `dunst` num D-Bus de
  sessão dedicado. No Windows nativo o toast é nativo.

**Fase 4 — Multi-conta + Tray + polling** ✅
- Polling automático (`scheduler::start_poller`): sincroniza ao subir e a cada
  intervalo escolhido (30min/1h/4h/6h/12h/24h; padrão 1h). Emite `events-updated`.
- `tray.rs`: ícone na bandeja, tooltip com próximo evento, menu (sincronizar /
  abrir / sair). Não-fatal se o WSLg não tiver host de bandeja.
- Fechar a janela esconde na bandeja (`prevent_close`) — roda em background.
- Tratamento de erro (`friendly_err`): sem internet/timeout e 401/403 viram
  mensagens amigáveis; poller offline apenas loga e tenta no próximo ciclo.
- **Removido o `tauri-plugin-single-instance`**: no nosso dev (D-Bus de sessão
  dedicado + muitos kill/relaunch) ele deixava locks obsoletos e a instância
  nova saía (exit 0). Dep mantida no Cargo p/ re-adição fácil no build Windows.
  > TODO Fase 5: re-adicionar single-instance p/ Windows (evita 2 tray icons),
  > com guarda contra lock obsoleto.

**Fase 5 — Polimento** (em andamento)
- ✅ Ícone próprio (calendário índigo) em todos os formatos.
- ✅ Sistema de releases (tag `v*` → publica `.msi`/`.exe` no GitHub Releases).
- ✅ Single-instance re-adicionado (callback foca a janela).
- ✅ Reconexão de conta: detecta token inválido/revogado → badge + botão
  "Reconectar" na conta (não-silencioso; OAuth exige o navegador).
- ✅ Tokens no keychain nativo (Windows/macOS via `keyring`); arquivo 0600 no Linux.
- ✅ Nome do produto "Calendar Notifier" (binário segue `calendar-notifier`).
- ✅ Antecedência por conta (override do lead global, por conta).
- ✅ **Múltiplos avisos** (lista de minutos, ex.: 10 e 2) — global e por conta;
  cada aviso disparado é rastreado por evento (`notified_leads`).
- ✅ **Filtros**: ignorar recusados (attendee self=declined) e ignorar dia-inteiro.
- ✅ **Busca** de evento por título.
- ✅ **Visão de mês** (grade) + toggle Lista/Mês.
- ✅ **Iniciar em segundo plano** (autostart passa `--minimized`; toggle na config).
- ✅ Correção do indicador de origem (mostrava o e-mail 2x no calendário principal).
- ✅ **Testes** (unitários da lógica pura: pkce, parse de avisos/CSV).
- ✅ **UI de avisos** como lista de campos (+ aviso / ×) em vez de texto.
- ✅ **Resumo diário** opcional: no horário configurado, notifica a agenda do
  dia (todos os tipos); só dispara se houver eventos; 1x/dia (`daily_summary_*`).
- **Assinatura de código (Windows):** SmartScreen bloqueia o app não assinado.
  Fix real: certificado (EV) + assinar `.msi`/`.exe` no CI. TODO.
- **Ações na notificação (adiar/abrir no clique):** o plugin do Tauri **não**
  expõe ações/clique no desktop. Precisa de código custom por plataforma
  (notify-rust/WinRT). **Adiado** (risco de quebrar as notificações). TODO.
- Auto-update (`tauri-plugin-updater`), builds macOS/Linux. TODO.

## Problemas conhecidos (a retomar)

- **Abrir navegador automático não funciona no WSL** (dev). `open::that` /
  opener usam `xdg-open`, que não alcança o navegador do Windows. Fallback já
  implementado: o app mostra a URL de autorização copiável na UI.
  - Correção futura p/ dev: detectar WSL e usar `wslview` (pacote `wslu`) ou
    `cmd.exe /c start` / `powershell.exe Start-Process`.
  - **No build Windows nativo isso não é problema** — `open` abre o navegador
    normalmente. É só um incômodo do dev-em-WSL.
- O redirect do OAuth vai pra `127.0.0.1:<porta>` dentro do WSL; o navegador do
  Windows alcança via espelhamento de localhost do WSL2 (a validar no teste).

## Fora do escopo da v1
- Visão de calendário completa (dia/semana/mês).
- Criar/editar eventos (só leitura por enquanto).
- Outros provedores (Outlook, iCloud).
- Push/webhooks em tempo real.
