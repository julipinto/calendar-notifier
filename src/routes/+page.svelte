<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";

  type Account = { email: string; display_name: string };

  let accounts = $state<Account[]>([]);
  let busy = $state(false);
  let status = $state("");
  let authUrl = $state("");
  let manualUrl = $state("");

  async function refresh() {
    accounts = await invoke<Account[]>("list_accounts");
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
    // A conclusão chega por evento (automático) ou via finishManual (colar URL).
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
    await refresh();
  }

  async function remove(email: string) {
    await invoke("remove_account", { email });
    status = `Removida: ${email}`;
    await refresh();
  }

  async function test(email: string) {
    status = `Testando ${email}…`;
    try {
      const r = await invoke<string>("test_account", { email });
      status = `${email}: ${r}`;
    } catch (e) {
      status = `Erro: ${e}`;
    }
  }

  onMount(() => {
    refresh();
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
  <h1>Calendar Notifier</h1>
  <p class="subtitle">Contas Google conectadas</p>

  <button onclick={connect} disabled={busy}>
    {busy ? "Conectando…" : "+ Conectar conta"}
  </button>

  {#if authUrl}
    <div class="auth-flow">
      <p class="step"><b>1.</b> Abra o link e autorize no navegador:</p>
      <button class="ghost" onclick={() => openUrl(authUrl)}>Abrir link de autorização</button>
      <textarea class="url-box" readonly rows="2">{authUrl}</textarea>

      <p class="step">
        <b>2.</b> Após autorizar, se o navegador mostrar erro em
        <code>127.0.0.1</code> (normal no WSL), copie a URL da barra de endereço
        e cole aqui:
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

  {#if accounts.length === 0}
    <p class="empty">Nenhuma conta conectada ainda.</p>
  {:else}
    <ul class="accounts">
      {#each accounts as acc (acc.email)}
        <li>
          <div class="acc-info">
            <span class="name">{acc.display_name}</span>
            <span class="email">{acc.email}</span>
          </div>
          <div class="acc-actions">
            <button class="ghost" onclick={() => test(acc.email)}>Testar</button>
            <button class="danger" onclick={() => remove(acc.email)}>Remover</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}

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
    max-width: 560px;
    margin: 0 auto;
    padding: 2rem 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  h1 {
    margin: 0;
    font-size: 1.6rem;
  }
  .subtitle {
    margin: 0;
    opacity: 0.7;
  }
  button {
    border-radius: 8px;
    border: 1px solid transparent;
    padding: 0.5em 1em;
    font-size: 0.95em;
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
    opacity: 0.6;
    cursor: default;
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
  .accounts {
    list-style: none;
    padding: 0;
    margin: 0.5rem 0 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .accounts li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.6rem 0.8rem;
    background: #fff;
    border-radius: 10px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
  }
  .acc-info {
    display: flex;
    flex-direction: column;
  }
  .name {
    font-weight: 600;
  }
  .email {
    font-size: 0.85em;
    opacity: 0.65;
  }
  .acc-actions {
    display: flex;
    gap: 0.4rem;
  }
  .empty,
  .hint,
  .status {
    font-size: 0.9em;
    opacity: 0.8;
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
  .auth-flow code {
    font-size: 0.85em;
  }
  .status {
    margin-top: 0.5rem;
    padding: 0.5rem 0.75rem;
    background: #eef;
    border-radius: 8px;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }
    .accounts li {
      background: #3a3a3a;
      box-shadow: none;
    }
    .status {
      background: #33384d;
    }
  }
</style>
