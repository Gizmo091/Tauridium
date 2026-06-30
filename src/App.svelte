<script lang="ts">
  import { onMount } from "svelte";
  import {
    login,
    restoreSession,
    getServices,
    getWorkspaces,
    logout,
    showService,
    closeServices,
    inspectService,
    DEFAULT_SERVER,
    type MeUser,
    type Service,
    type Workspace,
  } from "./lib/api";

  let server = $state(DEFAULT_SERVER);
  let email = $state("");
  let password = $state("");
  let showServer = $state(false);

  let booting = $state(true);
  let loading = $state(false);
  let error = $state<string | null>(null);

  let me = $state<MeUser | null>(null);
  let services = $state<Service[]>([]);
  let workspaces = $state<Workspace[]>([]);
  let activeId = $state<string | null>(null);

  const activeService = $derived(
    services.find((s) => s.id === activeId) ?? null,
  );

  // Services hors de tout workspace (affichés à part).
  const unassigned = $derived(
    services.filter(
      (s) => !workspaces.some((w) => w.services.includes(s.id)),
    ),
  );

  // Au démarrage : tente de restaurer une session enregistrée.
  onMount(async () => {
    try {
      me = await restoreSession();
      await loadAfterAuth();
    } catch {
      // pas de session valide -> on affiche l'écran de login
    } finally {
      booting = false;
    }
  });

  async function loadAfterAuth() {
    [services, workspaces] = await Promise.all([getServices(), getWorkspaces()]);
    const first = services.find((s) => s.isEnabled) ?? services[0] ?? null;
    if (first) selectService(first);
  }

  async function handleLogin(e: Event) {
    e.preventDefault();
    loading = true;
    error = null;
    try {
      me = await login(server, email, password);
      password = "";
      await loadAfterAuth();
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  function selectService(s: Service) {
    activeId = s.id;
    showService(s).catch((err) => {
      error = `Service « ${s.name} » : ${err}`;
    });
  }

  async function handleLogout() {
    await closeServices();
    await logout();
    me = null;
    services = [];
    workspaces = [];
    activeId = null;
    error = null;
  }
</script>

{#if booting}
  <main class="login">
    <div class="card"><p class="sub">Restauration de la session…</p></div>
  </main>
{:else if !me}
  <!-- ÉCRAN DE CONNEXION -->
  <main class="login">
    <form class="card" onsubmit={handleLogin}>
      <h1>pakeFerdium</h1>
      <p class="sub">Client Ferdium léger — connexion à ton serveur</p>

      <label>
        E-mail
        <input type="email" bind:value={email} autocomplete="username" required />
      </label>
      <label>
        Mot de passe
        <input
          type="password"
          bind:value={password}
          autocomplete="current-password"
          required
        />
      </label>

      <button
        type="button"
        class="gear"
        onclick={() => (showServer = !showServer)}
        title="Réglages du serveur"
      >
        ⚙︎ Serveur {showServer ? "▲" : "▼"}
      </button>
      {#if showServer}
        <label>
          URL du serveur
          <input type="url" bind:value={server} placeholder={DEFAULT_SERVER} />
        </label>
      {/if}

      {#if error}<p class="error">{error}</p>{/if}

      <button class="primary" type="submit" disabled={loading}>
        {loading ? "Connexion…" : "Se connecter"}
      </button>
    </form>
  </main>
{:else}
  <!-- COQUILLE : sidebar + zone service (rendu en Phase 2) -->
  <div class="shell">
    <aside class="sidebar">
      <div class="account">
        <strong>{me.firstname || me.email}</strong>
        <span class="acts">
          <button
            class="link"
            onclick={() => inspectService()}
            title="Inspecter le service actif (devtools)">🐞</button>
          <button class="link" onclick={handleLogout}>déconnexion</button>
        </span>
      </div>

      {#each workspaces as ws (ws.id)}
        <div class="ws">
          <div class="ws-title">{ws.name}</div>
          {#each ws.services
            .map((sid) => services.find((s) => s.id === sid))
            .filter((s): s is Service => !!s) as s (s.id)}
            {@render serviceRow(s)}
          {/each}
        </div>
      {/each}

      {#if unassigned.length}
        <div class="ws">
          <div class="ws-title">Sans workspace</div>
          {#each unassigned as s (s.id)}
            {@render serviceRow(s)}
          {/each}
        </div>
      {/if}

      <div class="count">{services.length} services · {workspaces.length} workspaces</div>
    </aside>

    <section class="stage">
      {#if activeService}
        <div class="placeholder">
          <h2>{activeService.name}</h2>
          <code>recipe : {activeService.recipeId}</code>
          <p>Chargement de la webview…</p>
        </div>
      {:else}
        <div class="placeholder"><p>Aucun service sélectionné.</p></div>
      {/if}
    </section>
  </div>
{/if}

{#snippet serviceRow(s: Service)}
  <button
    class="srow"
    class:active={s.id === activeId}
    class:disabled={!s.isEnabled}
    onclick={() => selectService(s)}
  >
    {#if s.iconUrl}
      <img src={s.iconUrl} alt="" />
    {:else}
      <span class="dot">{s.name.slice(0, 1)}</span>
    {/if}
    <span class="srow-name">{s.name}</span>
  </button>
{/snippet}

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, system-ui, sans-serif;
    background: #1f2230;
    color: #e8e8ef;
  }
  /* Login */
  .login {
    display: grid;
    place-items: center;
    height: 100vh;
  }
  .card {
    background: #282b3a;
    padding: 28px;
    border-radius: 14px;
    width: 320px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
  }
  .card h1 {
    margin: 0;
    font-size: 24px;
  }
  .sub {
    margin: 0 0 6px;
    color: #9aa0b5;
    font-size: 13px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: #9aa0b5;
  }
  input {
    padding: 9px 11px;
    border-radius: 8px;
    border: 1px solid #3a3f55;
    background: #1f2230;
    color: #e8e8ef;
    font-size: 14px;
  }
  .primary {
    margin-top: 6px;
    padding: 11px;
    border: none;
    border-radius: 8px;
    background: #4f46e5;
    color: #fff;
    font-weight: 700;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .gear {
    align-self: flex-start;
    background: none;
    border: none;
    color: #9aa0b5;
    cursor: pointer;
    font-size: 12px;
    padding: 0;
  }
  .error {
    color: #ff8a8a;
    font-size: 13px;
    margin: 0;
  }
  /* Shell */
  .shell {
    display: grid;
    grid-template-columns: 240px 1fr;
    height: 100vh;
  }
  .sidebar {
    background: #23263305;
    background: #1b1d28;
    padding: 12px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .account {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 13px;
  }
  .acts {
    display: inline-flex;
    gap: 8px;
    align-items: center;
  }
  .link {
    background: none;
    border: none;
    color: #7a82a8;
    cursor: pointer;
    font-size: 12px;
    text-decoration: underline;
  }
  .ws-title {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #6b7193;
    margin-bottom: 6px;
  }
  .srow {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 8px;
    border: none;
    border-radius: 8px;
    background: none;
    color: #d6d9e6;
    cursor: pointer;
    text-align: left;
    font-size: 14px;
  }
  .srow:hover {
    background: #262a3a;
  }
  .srow.active {
    background: #4f46e5;
    color: #fff;
  }
  .srow.disabled {
    opacity: 0.45;
  }
  .srow img,
  .srow .dot {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    object-fit: cover;
    flex: none;
  }
  .srow .dot {
    display: grid;
    place-items: center;
    background: #3a3f55;
    font-size: 12px;
    font-weight: 700;
  }
  .srow-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    margin-top: auto;
    font-size: 11px;
    color: #6b7193;
  }
  .stage {
    display: grid;
    place-items: center;
  }
  .placeholder {
    text-align: center;
    color: #9aa0b5;
  }
  .placeholder code {
    color: #b9b2ff;
  }
</style>
