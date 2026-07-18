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

**Fase 3 — Notificações + Scheduler**
- Antecedência global nas configs. Loop de tick + dedup.
- Notificação do sistema com título/horário do evento.

**Fase 4 — Multi-conta + Tray**
- Conectar N contas; agregação de eventos.
- Tray mostra próximo evento; menu (sync now, config, sair).
- Polling periódico automático.

**Fase 5 — Polimento**
- Reconexão/refresh robusto, tratamento de 410, start-on-login, ícones,
  filtros (ignorar all-day, recusados), testes.

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
