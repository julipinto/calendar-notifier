<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";

  type Account = { email: string; display_name: string };
  type Calendar = {
    id: string;
    account_email: string;
    summary: string;
    selected: boolean;
    is_primary: boolean;
  };
  type CalEvent = {
    id: string;
    account_email: string;
    title: string;
    start_ts: number;
    all_day: boolean;
    html_link: string;
  };

  let accounts = $state<Account[]>([]);
  let calendars = $state<Record<string, Calendar[]>>({});
  let expanded = $state<Record<string, boolean>>({});
  let events = $state<CalEvent[]>([]);
  let busy = $state(false);
  let syncing = $state(false);
  let status = $state("");
  let authUrl = $state("");
  let manualUrl = $state("");
  let leadMinutes = $state(10);

  async function loadAccounts() {
    accounts = await invoke<Account[]>("list_accounts");
    for (const a of accounts) await loadCalendars(a.email);
  }

  async function loadCalendars(email: string) {
    calendars[email] = await invoke<Calendar[]>("account_calendars", { email });
  }

  async function loadEvents() {
    events = await invoke<CalEvent[]>("list_events");
  }

  async function loadLead() {
    leadMinutes = await invoke<number>("get_lead_minutes");
  }

  async function saveLead() {
    await invoke("set_lead_minutes", { minutes: Number(leadMinutes) });
    status = `Antecedência salva: ${leadMinutes} min antes.`;
  }

  async function testNotif() {
    try {
      await invoke("test_notification");
      status = "Notificação de teste enviada (veja no seu SO).";
    } catch (e) {
      status = `Erro na notificação: ${e}`;
    }
  }

  async function connect() {
    busy = true;
    authUrl = "";
    manualUrl = "";
    status = "Autorize no navegador. No WSL, cole a URL de redirect abaixo.";
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
    status = `Conta conectada: ${acc.email}. Sincronizando…`;
    authUrl = "";
    manualUrl = "";
    busy = false;
    await loadAccounts();
    await syncNow();
  }

  async function remove(email: string) {
    await invoke("remove_account", { email });
    status = `Removida: ${email}`;
    await loadAccounts();
    await loadEvents();
  }

  async function reloadCalendars(email: string) {
    status = `Atualizando calendários de ${email}…`;
    try {
      calendars[email] = await invoke<Calendar[]>("refresh_calendars", { email });
      status = "Calendários atualizados.";
    } catch (e) {
      status = `Erro: ${e}`;
    }
  }

  async function toggleCalendar(cal: Calendar) {
    await invoke("set_calendar_selected", {
      email: cal.account_email,
      calendarId: cal.id,
      selected: !cal.selected,
    });
    await loadCalendars(cal.account_email);
  }

  async function syncNow() {
    syncing = true;
    status = "Sincronizando eventos…";
    try {
      const n = await invoke<number>("sync_now");
      status = `${n} evento(s) sincronizado(s).`;
      await loadEvents();
    } catch (e) {
      status = `Erro na sincronização: ${e}`;
    } finally {
      syncing = false;
    }
  }

  function fmtWhen(e: CalEvent): string {
    const d = new Date(e.start_ts * 1000);
    if (e.all_day)
      // dia inteiro é guardado como meia-noite UTC — renderiza em UTC p/ não
      // "voltar" um dia no fuso local.
      return (
        d.toLocaleDateString("pt-BR", {
          weekday: "short",
          day: "2-digit",
          month: "short",
          timeZone: "UTC",
        }) + " · dia inteiro"
      );
    return d.toLocaleString("pt-BR", {
      weekday: "short",
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  onMount(() => {
    loadAccounts();
    loadEvents();
    loadLead();
    const un1 = listen<Account>("account-connected", (e) => onConnected(e.payload));
    const un2 = listen<string>("auth-error", (e) => {
      status = `Erro: ${e.payload}`;
      busy = false;
    });
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
    };
  });
</script>

<main class="container">
  <header>
    <h1>Calendar Notifier</h1>
    <button class="sync" onclick={syncNow} disabled={syncing || accounts.length === 0}>
      {syncing ? "Sincronizando…" : "↻ Sincronizar agora"}
    </button>
  </header>

  <button onclick={connect} disabled={busy}>
    {busy ? "Conectando…" : "+ Conectar conta"}
  </button>

  {#if authUrl}
    <div class="auth-flow">
      <p class="step"><b>1.</b> Abra o link e autorize no navegador:</p>
      <button class="ghost" onclick={() => openUrl(authUrl)}>Abrir link de autorização</button>
      <textarea class="url-box" readonly rows="2">{authUrl}</textarea>
      <p class="step">
        <b>2.</b> Se o navegador der erro em <code>127.0.0.1</code> (normal no WSL),
        copie a URL da barra de endereço e cole aqui:
      </p>
      <textarea
        class="url-box"
        bind:value={manualUrl}
        rows="2"
        placeholder="http://127.0.0.1:PORTA/?state=...&code=..."
      ></textarea>
      <button onclick={finishManual} disabled={!manualUrl.trim()}>Concluir conexão</button>
    </div>
  {/if}

  <section class="settings">
    <h2>Notificações</h2>
    <div class="lead-row">
      <label>
        Avisar
        <input type="number" min="0" max="1440" bind:value={leadMinutes} onchange={saveLead} />
        minutos antes
      </label>
      <button class="ghost" onclick={testNotif}>Testar notificação</button>
    </div>
  </section>

  {#if accounts.length > 0}
    <section>
      <h2>Contas</h2>
      <ul class="accounts">
        {#each accounts as acc (acc.email)}
          <li>
            <div class="acc-row">
              <div class="acc-info">
                <span class="name">{acc.display_name}</span>
                <span class="email">{acc.email}</span>
              </div>
              <div class="acc-actions">
                <button class="ghost" onclick={() => (expanded[acc.email] = !expanded[acc.email])}>
                  {expanded[acc.email] ? "▾" : "▸"} Calendários
                </button>
                <button class="danger" onclick={() => remove(acc.email)}>Remover</button>
              </div>
            </div>

            {#if expanded[acc.email]}
              <div class="cals">
                {#each calendars[acc.email] ?? [] as cal (cal.id)}
                  <label class="cal">
                    <input
                      type="checkbox"
                      checked={cal.selected}
                      onchange={() => toggleCalendar(cal)}
                    />
                    <span>{cal.summary || cal.id}</span>
                    {#if cal.is_primary}<span class="badge">principal</span>{/if}
                  </label>
                {/each}
                <button class="ghost tiny" onclick={() => reloadCalendars(acc.email)}>
                  Recarregar calendários
                </button>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section>
    <h2>Próximos eventos</h2>
    {#if events.length === 0}
      <p class="empty">Nenhum evento em cache. Conecte uma conta e sincronize.</p>
    {:else}
      <ul class="events">
        {#each events as ev (ev.account_email + ev.id)}
          <li>
            <span class="when">{fmtWhen(ev)}</span>
            <span class="title">
              {#if ev.html_link}
                <a href={ev.html_link} onclick={(e) => { e.preventDefault(); openUrl(ev.html_link); }}>{ev.title}</a>
              {:else}{ev.title}{/if}
            </span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  {#if status}
    <p class="status">{status}</p>
  {/if}
</main>

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    color: #0f0f0f;
    background-color: #f6f6f6;
  }
  .container {
    max-width: 620px;
    margin: 0 auto;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h1 {
    margin: 0;
    font-size: 1.5rem;
  }
  h2 {
    margin: 0.5rem 0 0.25rem;
    font-size: 1rem;
    opacity: 0.7;
  }
  button {
    border-radius: 8px;
    border: 1px solid transparent;
    padding: 0.5em 1em;
    font-size: 0.9em;
    font-weight: 500;
    font-family: inherit;
    color: #fff;
    background-color: #396cd8;
    cursor: pointer;
    transition: filter 0.2s;
  }
  button:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  button:disabled {
    opacity: 0.55;
    cursor: default;
  }
  button.sync {
    background: #2e8b57;
  }
  button.ghost {
    background: transparent;
    color: #396cd8;
    border-color: #396cd8;
  }
  button.danger {
    background: transparent;
    color: #c0392b;
    border-color: #c0392b;
  }
  button.tiny {
    font-size: 0.8em;
    padding: 0.3em 0.6em;
    align-self: flex-start;
  }
  .accounts,
  .events {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .accounts li {
    padding: 0.6rem 0.8rem;
    background: #fff;
    border-radius: 10px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
  }
  .acc-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .acc-info {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .name {
    font-weight: 600;
  }
  .email {
    font-size: 0.82em;
    opacity: 0.65;
  }
  .acc-actions {
    display: flex;
    gap: 0.4rem;
    flex-shrink: 0;
  }
  .cals {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 0.6rem;
    padding-top: 0.6rem;
    border-top: 1px solid #eee;
    max-height: 220px;
    overflow-y: auto;
  }
  .cal {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.9em;
  }
  .badge {
    font-size: 0.7em;
    background: #396cd8;
    color: #fff;
    padding: 0.05em 0.4em;
    border-radius: 6px;
  }
  .events li {
    display: flex;
    flex-direction: column;
    padding: 0.5rem 0.7rem;
    background: #fff;
    border-radius: 8px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
  }
  .when {
    font-size: 0.78em;
    opacity: 0.6;
    text-transform: capitalize;
  }
  .title {
    font-weight: 500;
  }
  .title a {
    color: inherit;
    text-decoration: none;
  }
  .title a:hover {
    text-decoration: underline;
  }
  .empty,
  .status {
    font-size: 0.9em;
    opacity: 0.85;
  }
  .status {
    margin-top: 0.5rem;
    padding: 0.5rem 0.75rem;
    background: #eef;
    border-radius: 8px;
  }
  .auth-flow {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.75rem;
    border: 1px dashed #888;
    border-radius: 10px;
  }
  .auth-flow .step {
    margin: 0;
    font-size: 0.9em;
  }
  .url-box {
    width: 100%;
    font-size: 0.75em;
    font-family: monospace;
    resize: vertical;
    border-radius: 6px;
    padding: 0.4rem;
    box-sizing: border-box;
  }
  .lead-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .lead-row label {
    font-size: 0.95em;
  }
  .lead-row input[type="number"] {
    width: 4rem;
    padding: 0.3em 0.4em;
    border-radius: 6px;
    border: 1px solid #bbb;
    font-size: 0.95em;
    text-align: center;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }
    .accounts li,
    .events li {
      background: #3a3a3a;
      box-shadow: none;
    }
    .cals {
      border-top-color: #4a4a4a;
    }
    .status {
      background: #33384d;
    }
  }
</style>
