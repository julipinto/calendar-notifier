<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";

  type Account = { email: string; display_name: string; needs_reauth: boolean };
  type Calendar = {
    id: string;
    account_email: string;
    summary: string;
    selected: boolean;
    is_primary: boolean;
    color: string;
  };
  type CalEvent = {
    id: string;
    account_email: string;
    title: string;
    start_ts: number;
    end_ts: number;
    all_day: boolean;
    html_link: string;
    color: string;
    calendar_summary: string;
  };

  let accounts = $state<Account[]>([]);
  let calendars = $state<Record<string, Calendar[]>>({});
  let accountLead = $state<Record<string, number | null>>({});
  let expanded = $state<Record<string, boolean>>({});
  let events = $state<CalEvent[]>([]);
  let busy = $state(false);
  let syncing = $state(false);
  let status = $state("");
  let offline = $state(false);
  let lastSync = $state(0);
  let authUrl = $state("");
  let manualUrl = $state("");
  let leadMinutes = $state(10);
  let pollMinutes = $state(60);
  let soundEnabled = $state(true);
  let autostart = $state(false);
  let view = $state<"events" | "settings">("events");
  let nowTick = $state(0); // incrementa periodicamente p/ atualizar "há X min"

  const ACCENT = "#6366f1";

  async function loadAccounts() {
    accounts = await invoke<Account[]>("list_accounts");
    for (const a of accounts) {
      await loadCalendars(a.email);
      accountLead[a.email] = await invoke<number | null>("get_account_lead", { email: a.email });
    }
  }

  async function saveAccountLead(email: string) {
    const v = accountLead[email];
    const minutes = v === null || v === undefined || (v as any) === "" ? null : Number(v);
    await invoke("set_account_lead", { email, minutes });
    status = minutes === null ? "Antecedência da conta: padrão global." : `Antecedência da conta: ${minutes} min.`;
  }
  async function loadCalendars(email: string) {
    calendars[email] = await invoke<Calendar[]>("account_calendars", { email });
  }
  // atualiza só a lista de contas (p/ refletir o badge "reconectar") sem recarregar calendários
  async function refreshAccountFlags() {
    accounts = await invoke<Account[]>("list_accounts");
  }
  async function loadEvents() {
    events = await invoke<CalEvent[]>("list_events");
  }
  async function loadLead() {
    leadMinutes = await invoke<number>("get_lead_minutes");
  }
  async function loadPoll() {
    const v = await invoke<number>("get_poll_minutes");
    pollMinutes = [30, 60, 240, 360, 720, 1440].includes(v) ? v : 60;
  }
  async function loadSound() {
    soundEnabled = await invoke<boolean>("get_sound_enabled");
  }
  async function loadLastSync() {
    try {
      lastSync = await invoke<number>("get_last_sync");
    } catch {
      lastSync = 0;
    }
  }

  async function saveLead() {
    await invoke("set_lead_minutes", { minutes: Number(leadMinutes) });
  }
  async function savePoll() {
    await invoke("set_poll_minutes", { minutes: Number(pollMinutes) });
  }
  async function saveSound() {
    await invoke("set_sound_enabled", { enabled: soundEnabled });
  }
  async function loadAutostart() {
    try {
      autostart = await invoke<boolean>("get_autostart");
    } catch {
      autostart = false;
    }
  }
  async function saveAutostart() {
    try {
      await invoke("set_autostart", { enabled: autostart });
      status = autostart ? "Vai iniciar com o sistema." : "Não inicia mais com o sistema.";
    } catch (e) {
      status = `Erro: ${e}`;
      autostart = !autostart;
    }
  }

  async function connect() {
    busy = true;
    authUrl = "";
    manualUrl = "";
    status = "Autorize no navegador…";
    try {
      authUrl = await invoke<string>("start_auth");
    } catch (e) {
      status = `Erro ao iniciar: ${e}`;
      busy = false;
    }
  }
  async function finishManual() {
    if (!manualUrl.trim()) return;
    status = "Concluindo…";
    try {
      const acc = await invoke<Account>("finish_auth_manual", { redirectUrl: manualUrl });
      onConnected(acc);
    } catch (e) {
      status = `Erro: ${e}`;
    }
  }
  async function onConnected(acc: Account) {
    status = `Conta conectada: ${acc.email}`;
    authUrl = "";
    manualUrl = "";
    busy = false;
    await loadAccounts();
    await syncNow();
  }
  async function remove(email: string) {
    await invoke("remove_account", { email });
    await loadAccounts();
    await loadEvents();
  }
  async function reloadCalendars(email: string) {
    status = "Atualizando calendários…";
    try {
      calendars[email] = await invoke<Calendar[]>("refresh_calendars", { email });
      status = "Calendários atualizados.";
    } catch (e) {
      status = `Erro: ${e}`;
    }
  }
  async function toggleCalendar(cal: Calendar) {
    const nowSelected = !cal.selected;
    await invoke("set_calendar_selected", {
      email: cal.account_email,
      calendarId: cal.id,
      selected: nowSelected,
    });
    await loadCalendars(cal.account_email);
    if (nowSelected) {
      await syncNow(); // marcou: busca os eventos do calendário na hora
    } else {
      await loadEvents(); // desmarcou: eventos já removidos no backend, só atualiza
    }
  }
  async function syncNow() {
    syncing = true;
    status = "";
    try {
      const n = await invoke<number>("sync_now");
      offline = false;
      await loadEvents();
      await loadLastSync();
      status = `${n} evento(s) sincronizado(s).`;
    } catch (e) {
      offline = String(e).includes("internet");
      status = `${e}`;
    } finally {
      syncing = false;
      await refreshAccountFlags(); // reflete "reconectar" se o token falhou
    }
  }
  async function testNotif() {
    try {
      await invoke("test_notification");
      status = "Notificação de teste enviada.";
    } catch (e) {
      status = `Erro: ${e}`;
    }
  }

  function fmtTime(e: CalEvent): string {
    if (e.all_day) return "dia inteiro";
    return new Date(e.start_ts * 1000).toLocaleTimeString("pt-BR", {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  function dayInfo(e: CalEvent): { key: string; label: string } {
    const d = new Date(e.start_ts * 1000);
    const y = e.all_day ? d.getUTCFullYear() : d.getFullYear();
    const m = e.all_day ? d.getUTCMonth() : d.getMonth();
    const day = e.all_day ? d.getUTCDate() : d.getDate();
    const dateOnly = new Date(y, m, day);
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const diff = Math.round((dateOnly.getTime() - today.getTime()) / 86400000);
    let label: string;
    if (diff === 0) label = "Hoje";
    else if (diff === 1) label = "Amanhã";
    else
      label = dateOnly.toLocaleDateString("pt-BR", {
        weekday: "long",
        day: "2-digit",
        month: "long",
      });
    return { key: `${y}-${m}-${day}`, label };
  }
  function relTime(ts: number): string {
    if (!ts) return "nunca sincronizado";
    const mins = Math.round((Date.now() / 1000 - ts) / 60);
    if (mins < 1) return "sincronizado agora";
    if (mins < 60) return `sincronizado há ${mins} min`;
    const h = Math.round(mins / 60);
    if (h < 24) return `sincronizado há ${h} h`;
    return `sincronizado há ${Math.round(h / 24)} d`;
  }
  function dotColor(c: string): string {
    return c && c.startsWith("#") ? c : ACCENT;
  }

  const groups = $derived.by(() => {
    const out: { key: string; label: string; items: CalEvent[] }[] = [];
    let cur: { key: string; label: string; items: CalEvent[] } | null = null;
    for (const e of events) {
      const { key, label } = dayInfo(e);
      if (!cur || cur.key !== key) {
        cur = { key, label, items: [] };
        out.push(cur);
      }
      cur.items.push(e);
    }
    return out;
  });

  // mostra a origem (calendário/conta) nos eventos quando há mais de uma fonte
  const multiSource = $derived(
    new Set(events.map((e) => e.account_email + "|" + e.calendar_summary)).size > 1,
  );
  const multiAccount = $derived(new Set(events.map((e) => e.account_email)).size > 1);

  // "sincronizado há X min" — recalcula quando nowTick muda (a cada 30s)
  const syncLabel = $derived.by(() => {
    nowTick;
    return relTime(lastSync);
  });

  // some com a mensagem de status depois de alguns segundos
  $effect(() => {
    if (!status) return;
    const isErr = status.startsWith("Erro") || offline;
    const t = setTimeout(() => (status = ""), isErr ? 7000 : 4000);
    return () => clearTimeout(t);
  });

  onMount(() => {
    loadAccounts();
    loadEvents();
    loadLead();
    loadPoll();
    loadSound();
    loadAutostart();
    loadLastSync();
    const uns = [
      listen<Account>("account-connected", (e) => onConnected(e.payload)),
      listen<string>("auth-error", (e) => {
        status = `Erro: ${e.payload}`;
        busy = false;
      }),
      listen<number>("events-updated", () => {
        offline = false;
        loadEvents();
        loadLastSync();
        refreshAccountFlags();
      }),
      listen<string>("sync-error", (e) => {
        offline = String(e.payload).includes("internet");
        refreshAccountFlags();
      }),
    ];
    const tick = setInterval(() => (nowTick += 1), 30000);
    return () => {
      uns.forEach((u) => u.then((f) => f()));
      clearInterval(tick);
    };
  });
</script>

<div class="app">
  <header class="topbar">
    <div class="brand">
      <div class="logo">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="4" width="18" height="18" rx="2" />
          <line x1="16" y1="2" x2="16" y2="6" />
          <line x1="8" y1="2" x2="8" y2="6" />
          <line x1="3" y1="10" x2="21" y2="10" />
        </svg>
      </div>
      <div>
        <h1>Calendar Notifier</h1>
        <span class="sub">
          {#if offline}<span class="offline">● offline</span> · {/if}{syncLabel}
        </span>
      </div>
    </div>
    <div class="actions">
      <button
        class="icon-btn"
        class:active={view === "settings"}
        title="Configurações"
        aria-label="Configurações"
        onclick={() => (view = view === "settings" ? "events" : "settings")}
      >⚙</button>
      <button class="btn primary" onclick={syncNow} disabled={syncing || accounts.length === 0}>
        {syncing ? "Sincronizando…" : "↻ Sincronizar"}
      </button>
    </div>
  </header>

  <div class="content">
    {#if view === "events"}
      {#if events.length === 0}
        <div class="empty">
          <div class="empty-emoji">🗓️</div>
          <p>Nenhum evento nos próximos 30 dias.</p>
          {#if accounts.length === 0}
            <button class="btn primary" onclick={() => (view = "settings")}>Conectar uma conta</button>
          {/if}
        </div>
      {:else}
        {#each groups as g (g.key)}
          <div class="day-group">
            <h2 class="day-label">{g.label}</h2>
            {#each g.items as ev (ev.account_email + ev.id)}
              <button
                class="event"
                onclick={() => ev.html_link && openUrl(ev.html_link)}
                title={ev.calendar_summary}
              >
                <span class="time">{fmtTime(ev)}</span>
                <span class="dot" style="background:{dotColor(ev.color)}"></span>
                <span class="ev-main">
                  <span class="ev-title">{ev.title}</span>
                  {#if multiSource}
                    <span class="ev-source">
                      {ev.calendar_summary || "calendário"}{#if multiAccount} · {ev.account_email}{/if}
                    </span>
                  {/if}
                </span>
              </button>
            {/each}
          </div>
        {/each}
      {/if}
    {:else}
      <!-- Configurações -->
      <section class="card">
        <div class="card-head">
          <h3>Contas</h3>
          <button class="btn ghost sm" onclick={connect} disabled={busy}>
            {busy ? "Conectando…" : "+ Conectar"}
          </button>
        </div>

        {#if authUrl}
          <div class="auth-flow">
            <p class="step"><b>1.</b> Autorize no navegador:</p>
            <button class="btn ghost sm" onclick={() => openUrl(authUrl)}>Abrir link</button>
            <p class="step">
              <b>2.</b> Se der erro em <code>127.0.0.1</code> (normal no WSL), cole a URL da barra
              de endereço:
            </p>
            <textarea
              class="url-box"
              bind:value={manualUrl}
              rows="2"
              placeholder="http://127.0.0.1:PORTA/?state=...&code=..."
            ></textarea>
            <button class="btn primary sm" onclick={finishManual} disabled={!manualUrl.trim()}>
              Concluir conexão
            </button>
          </div>
        {/if}

        {#if accounts.length === 0}
          <p class="muted small">Nenhuma conta conectada.</p>
        {:else}
          {#each accounts as acc (acc.email)}
            <div class="account">
              <div class="acc-row">
                <div class="acc-id">
                  <span class="acc-name">
                    {acc.display_name}
                    {#if acc.needs_reauth}<span class="badge warn">reconectar</span>{/if}
                  </span>
                  {#if acc.display_name !== acc.email}<span class="acc-mail">{acc.email}</span>{/if}
                </div>
                <div class="acc-actions">
                  {#if acc.needs_reauth}
                    <button class="btn primary sm" onclick={connect} disabled={busy}>Reconectar</button>
                  {/if}
                  <button class="btn ghost sm" onclick={() => (expanded[acc.email] = !expanded[acc.email])}>
                    {expanded[acc.email] ? "▾" : "▸"} Calendários
                  </button>
                  <button class="btn danger sm" onclick={() => remove(acc.email)}>Remover</button>
                </div>
              </div>
              {#if acc.needs_reauth}
                <p class="reauth-hint">Autorização com o Google expirou — clique em "Reconectar" e entre com <b>{acc.email}</b>.</p>
              {/if}
              {#if expanded[acc.email]}
                <div class="cals">
                  <div class="acc-lead">
                    Avisar
                    <input
                      type="number"
                      min="0"
                      max="1440"
                      placeholder={String(leadMinutes)}
                      bind:value={accountLead[acc.email]}
                      onchange={() => saveAccountLead(acc.email)}
                    />
                    min antes <span class="muted">(vazio = padrão global)</span>
                  </div>
                  {#each calendars[acc.email] ?? [] as cal (cal.id)}
                    <label class="cal">
                      <input type="checkbox" checked={cal.selected} onchange={() => toggleCalendar(cal)} />
                      <span class="dot" style="background:{dotColor(cal.color)}"></span>
                      <span class="cal-name">{cal.summary || cal.id}</span>
                      {#if cal.is_primary}<span class="badge">principal</span>{/if}
                    </label>
                  {/each}
                  <button class="btn ghost xs" onclick={() => reloadCalendars(acc.email)}>
                    Recarregar calendários
                  </button>
                </div>
              {/if}
            </div>
          {/each}
        {/if}
      </section>

      <section class="card">
        <div class="card-head"><h3>Notificações & Sincronização</h3></div>
        <div class="settings">
          <div class="set-row">
            <span>Avisar</span>
            <input class="num" type="number" min="0" max="1440" bind:value={leadMinutes} onchange={saveLead} />
            <span>minutos antes</span>
          </div>
          <label class="set-row check">
            <input type="checkbox" bind:checked={soundEnabled} onchange={saveSound} />
            <span>Tocar som na notificação</span>
          </label>
          <label class="set-row check">
            <input type="checkbox" bind:checked={autostart} onchange={saveAutostart} />
            <span>Iniciar com o sistema</span>
          </label>
          <div class="set-row">
            <span>Sincronizar automaticamente</span>
            <select bind:value={pollMinutes} onchange={savePoll}>
              <option value={30}>a cada 30 minutos</option>
              <option value={60}>a cada 1 hora</option>
              <option value={240}>a cada 4 horas</option>
              <option value={360}>a cada 6 horas</option>
              <option value={720}>a cada 12 horas</option>
              <option value={1440}>a cada 24 horas</option>
            </select>
          </div>
          <button class="btn ghost sm" onclick={testNotif}>Testar notificação</button>
        </div>
      </section>
    {/if}
  </div>

  {#if status}
    <div class="status" class:err={status.startsWith("Erro") || offline}>{status}</div>
  {/if}
</div>

<style>
  :global(body) { margin: 0; }
  :global(html, body) { height: 100%; }

  .app {
    --accent: #6366f1;
    --bg: #f5f5f7;
    --card: #ffffff;
    --text: #1a1a1e;
    --muted: #71717a;
    --border: #ececf0;
    font-family: Inter, system-ui, Avenir, Helvetica, Arial, sans-serif;
    color: var(--text);
    background: var(--bg);
    height: 100vh;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
  }

  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.9rem 1.1rem;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .brand { display: flex; align-items: center; gap: 0.7rem; }
  .logo {
    width: 2.3rem; height: 2.3rem; display: grid; place-items: center;
    background: linear-gradient(135deg, var(--accent), #8b5cf6);
    border-radius: 11px;
  }
  h1 { margin: 0; font-size: 1.15rem; line-height: 1.1; }
  .sub { font-size: 0.76rem; color: var(--muted); }
  .offline { color: #e0483d; font-weight: 600; }
  .actions { display: flex; align-items: center; gap: 0.5rem; }

  .btn {
    border: 1px solid transparent; border-radius: 9px; padding: 0.5em 0.9em;
    font: inherit; font-weight: 600; font-size: 0.86rem; cursor: pointer;
    transition: filter 0.15s, background 0.15s; white-space: nowrap;
  }
  .btn.sm { font-size: 0.8rem; padding: 0.4em 0.7em; }
  .btn.xs { font-size: 0.75rem; padding: 0.3em 0.55em; align-self: flex-start; }
  .btn.primary { background: var(--accent); color: #fff; }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn.ghost { background: transparent; color: var(--accent); border-color: var(--border); }
  .btn.ghost:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .btn.danger { background: transparent; color: #e0483d; }
  .btn.danger:hover:not(:disabled) { background: color-mix(in srgb, #e0483d 10%, transparent); }
  .btn:disabled { opacity: 0.5; cursor: default; }

  .icon-btn {
    width: 2.1rem; height: 2.1rem; border-radius: 9px; border: 1px solid var(--border);
    background: transparent; color: var(--text); cursor: pointer; font-size: 1rem;
    display: grid; place-items: center; transition: background 0.15s;
  }
  .icon-btn:hover { background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .icon-btn.active { background: var(--accent); color: #fff; border-color: transparent; }

  .content {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  /* eventos */
  .day-group { display: flex; flex-direction: column; gap: 0.25rem; }
  .day-label {
    margin: 0 0 0.15rem; font-size: 0.78rem; font-weight: 700;
    text-transform: capitalize; color: var(--muted);
  }
  .event {
    display: flex; align-items: center; gap: 0.7rem; width: 100%; text-align: left;
    background: var(--card); border: 1px solid var(--border); border-radius: 10px;
    padding: 0.55rem 0.8rem; cursor: pointer; font: inherit; color: inherit;
    transition: border-color 0.15s;
  }
  .event:hover { border-color: var(--accent); }
  .time {
    font-variant-numeric: tabular-nums; font-size: 0.8rem; color: var(--muted); min-width: 4rem;
  }
  .dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .ev-main { display: flex; flex-direction: column; min-width: 0; gap: 0.05rem; }
  .ev-title { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ev-source {
    font-size: 0.72rem; color: var(--muted);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }

  .empty { text-align: center; padding: 2.5rem 1rem; color: var(--muted); margin: auto 0; }
  .empty-emoji { font-size: 2rem; margin-bottom: 0.3rem; }
  .empty p { margin: 0.3rem 0 0.8rem; }

  /* cards (settings) */
  .card {
    background: var(--card); border: 1px solid var(--border); border-radius: 14px;
    padding: 0.9rem 1rem; display: flex; flex-direction: column; gap: 0.6rem;
  }
  .card-head { display: flex; align-items: center; justify-content: space-between; }
  .card h3 { margin: 0; font-size: 0.95rem; }

  .account { border-top: 1px solid var(--border); padding-top: 0.6rem; }
  .account:first-of-type { border-top: none; padding-top: 0; }
  .acc-row { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; }
  .acc-id { display: flex; flex-direction: column; overflow: hidden; }
  .acc-name { font-weight: 600; }
  .acc-mail { font-size: 0.78rem; color: var(--muted); }
  .acc-actions { display: flex; gap: 0.3rem; flex-shrink: 0; }

  .cals {
    display: flex; flex-direction: column; gap: 0.4rem; margin-top: 0.6rem;
    max-height: 220px; overflow-y: auto; padding-right: 0.2rem;
  }
  .acc-lead {
    display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap;
    font-size: 0.82rem; padding-bottom: 0.5rem; margin-bottom: 0.2rem;
    border-bottom: 1px solid var(--border);
  }
  .acc-lead input {
    width: 3.2rem; padding: 0.2em 0.35em; border-radius: 6px;
    border: 1px solid var(--border); font: inherit; text-align: center;
    background: var(--bg); color: var(--text);
  }
  .cal { display: flex; align-items: center; gap: 0.5rem; font-size: 0.88rem; cursor: pointer; }
  .cal-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge {
    font-size: 0.68rem; background: var(--accent); color: #fff;
    padding: 0.05em 0.45em; border-radius: 6px;
  }
  .badge.warn { background: #e0483d; }
  .reauth-hint { margin: 0.4rem 0 0; font-size: 0.8rem; color: #e0483d; }

  .settings { display: flex; flex-direction: column; gap: 0.6rem; }
  .set-row { display: flex; align-items: center; gap: 0.5rem; font-size: 0.9rem; flex-wrap: wrap; }
  .set-row.check { cursor: pointer; }
  .num {
    width: 3.5rem; padding: 0.3em 0.4em; border-radius: 7px; border: 1px solid var(--border);
    font: inherit; text-align: center; background: var(--bg); color: var(--text);
  }
  select {
    padding: 0.35em 0.5em; border-radius: 7px; border: 1px solid var(--border);
    font: inherit; background: var(--bg); color: var(--text);
  }

  .auth-flow {
    display: flex; flex-direction: column; gap: 0.4rem; padding: 0.7rem;
    border: 1px dashed var(--muted); border-radius: 10px;
  }
  .step { margin: 0; font-size: 0.85rem; }
  .url-box {
    width: 100%; box-sizing: border-box; font-family: monospace; font-size: 0.75rem;
    border-radius: 7px; padding: 0.4rem; border: 1px solid var(--border);
    background: var(--bg); color: var(--text); resize: vertical;
  }

  .muted { color: var(--muted); }
  .small { font-size: 0.85rem; }
  .status {
    flex-shrink: 0; margin: 0; font-size: 0.83rem; padding: 0.5rem 1.1rem;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .status.err { background: color-mix(in srgb, #e0483d 12%, transparent); }

  @media (prefers-color-scheme: dark) {
    .app {
      --bg: #17171b; --card: #232329; --text: #ececf0; --muted: #9b9ba4; --border: #33333c;
    }
  }
</style>
