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
    hideServices,
    inspectService,
    setServiceFlags,
    updateService,
    createService,
    deleteService,
    listRecipes,
    DEFAULT_SERVER,
    type MeUser,
    type Service,
    type Workspace,
    type RecipePreview,
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

  // Vue de la zone droite : un service, les réglages d'un service, ou l'ajout.
  type View = "service" | "svcSettings" | "add";
  let view = $state<View>("service");
  let settingsSvc = $state<Service | null>(null);

  // Ajout de service : catalogue complet chargé une fois, filtré en live.
  let recipeQuery = $state("");
  let allRecipes = $state<RecipePreview[]>([]);
  let recipesLoading = $state(false);
  let newServiceName = $state("");

  const filteredRecipes = $derived.by(() => {
    const q = recipeQuery.trim().toLowerCase();
    const list = q
      ? allRecipes.filter(
          (r) =>
            r.name.toLowerCase().includes(q) || r.id.toLowerCase().includes(q),
        )
      : allRecipes;
    return [...list].sort((a, b) => a.name.localeCompare(b.name));
  });

  const activeService = $derived(
    services.find((s) => s.id === activeId) ?? null,
  );
  const sorted = $derived(
    [...services].sort((a, b) => (a.order ?? 0) - (b.order ?? 0)),
  );
  const unassigned = $derived(
    sorted.filter((s) => !workspaces.some((w) => w.services.includes(s.id))),
  );

  onMount(async () => {
    try {
      me = await restoreSession();
      await loadAfterAuth();
    } catch {
      // pas de session valide -> écran de login
    } finally {
      booting = false;
    }
  });

  async function loadAfterAuth() {
    [services, workspaces] = await Promise.all([getServices(), getWorkspaces()]);
    // Pousse les réglages (notif/mute/badge) au backend pour qu'il les respecte.
    await Promise.all(services.map((s) => setServiceFlags(s).catch(() => {})));
    const first = sorted.find((s) => s.isEnabled) ?? sorted[0] ?? null;
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
    view = "service";
    activeId = s.id;
    showService(s).catch((err) => {
      error = `Service « ${s.name} » : ${err}`;
    });
  }

  function openServiceSettings(s: Service) {
    settingsSvc = s;
    view = "svcSettings";
    hideServices();
  }

  async function saveSetting(key: keyof Service, value: boolean) {
    if (!settingsSvc) return;
    (settingsSvc as Record<string, unknown>)[key] = value;
    const idx = services.findIndex((s) => s.id === settingsSvc!.id);
    if (idx >= 0) (services[idx] as Record<string, unknown>)[key] = value;
    const s = settingsSvc;
    try {
      await updateService(s.id, {
        name: s.name,
        isEnabled: s.isEnabled,
        isNotificationEnabled: s.isNotificationEnabled,
        isMuted: s.isMuted,
        isBadgeEnabled: s.isBadgeEnabled,
      });
      await setServiceFlags(s);
    } catch (err) {
      error = String(err);
    }
  }

  async function handleDelete(s: Service) {
    if (!confirm(`Supprimer le service « ${s.name} » ?`)) return;
    try {
      await deleteService(s.id);
      services = services.filter((x) => x.id !== s.id);
      backToService();
    } catch (err) {
      error = String(err);
    }
  }

  async function openAdd() {
    view = "add";
    recipeQuery = "";
    newServiceName = "";
    hideServices();
    if (allRecipes.length === 0) {
      recipesLoading = true;
      error = null;
      try {
        allRecipes = await listRecipes();
      } catch (err) {
        error = String(err);
      } finally {
        recipesLoading = false;
      }
    }
  }

  async function pickRecipe(r: RecipePreview) {
    try {
      await createService(newServiceName.trim() || r.name, r.id);
      [services, workspaces] = await Promise.all([
        getServices(),
        getWorkspaces(),
      ]);
      await Promise.all(services.map((s) => setServiceFlags(s).catch(() => {})));
      const created =
        sorted.find((s) => s.recipeId === r.id) ?? sorted.at(-1) ?? null;
      if (created) selectService(created);
      else view = "service";
    } catch (err) {
      error = String(err);
    }
  }

  function backToService() {
    const target = activeService ?? sorted.find((s) => s.isEnabled) ?? sorted[0];
    if (target) selectService(target);
    else view = "service";
  }

  async function handleLogout() {
    await closeServices();
    await logout();
    me = null;
    services = [];
    workspaces = [];
    activeId = null;
    view = "service";
    error = null;
  }
</script>

{#if booting}
  <main class="login">
    <div class="card"><p class="sub">Restauration de la session…</p></div>
  </main>
{:else if !me}
  <main class="login">
    <form class="card" onsubmit={handleLogin}>
      <h1>Tauridium</h1>
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
      <button type="button" class="gear" onclick={() => (showServer = !showServer)}>
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
  <div class="shell">
    <aside class="sidebar">
      <div class="account">
        <strong>{me.firstname || me.email}</strong>
        <span class="acts">
          <button class="link" onclick={() => inspectService()} title="Inspecter (devtools)">🐞</button>
          <button class="link" onclick={handleLogout}>déconnexion</button>
        </span>
      </div>

      <button class="add" onclick={openAdd}>＋ Ajouter un service</button>

      {#each workspaces as ws (ws.id)}
        {@const list = ws.services
          .map((sid) => sorted.find((s) => s.id === sid))
          .filter((s): s is Service => !!s)}
        {#if list.length}
          <div class="ws">
            <div class="ws-title">{ws.name}</div>
            {#each list as s (s.id)}{@render row(s)}{/each}
          </div>
        {/if}
      {/each}

      {#if unassigned.length}
        <div class="ws">
          <div class="ws-title">Sans workspace</div>
          {#each unassigned as s (s.id)}{@render row(s)}{/each}
        </div>
      {/if}

      <div class="count">{services.length} services · {workspaces.length} workspaces</div>
    </aside>

    <section class="stage">
      {#if view === "service"}
        {#if activeService}
          <div class="placeholder">
            <h2>{activeService.name}</h2>
            <p>Chargement de la webview…</p>
          </div>
        {:else}
          <div class="placeholder"><p>Aucun service sélectionné.</p></div>
        {/if}
      {:else if view === "svcSettings" && settingsSvc}
        <div class="panel">
          <div class="panel-head">
            <h2>Réglages — {settingsSvc.name}</h2>
            <button class="link" onclick={backToService}>✕ fermer</button>
          </div>
          <code class="recipe">recipe : {settingsSvc.recipeId}</code>

          {@render toggle("Activé", "isEnabled", settingsSvc.isEnabled !== false)}
          {@render toggle("Notifications", "isNotificationEnabled", settingsSvc.isNotificationEnabled !== false)}
          {@render toggle("En sourdine (mute)", "isMuted", settingsSvc.isMuted === true)}
          {@render toggle("Badge non-lus", "isBadgeEnabled", settingsSvc.isBadgeEnabled !== false)}

          {#if error}<p class="error">{error}</p>{/if}
          <button class="danger" onclick={() => settingsSvc && handleDelete(settingsSvc)}>
            Supprimer ce service
          </button>
        </div>
      {:else if view === "add"}
        <div class="panel">
          <div class="panel-head">
            <h2>Ajouter un service</h2>
            <button class="link" onclick={backToService}>✕ fermer</button>
          </div>
          <label class="block">
            Nom (optionnel)
            <input bind:value={newServiceName} placeholder="laisser vide = nom du recipe" />
          </label>
          <input
            class="filter"
            bind:value={recipeQuery}
            placeholder="Filtrer parmi {allRecipes.length} services…"
          />
          {#if error}<p class="error">{error}</p>{/if}
          {#if recipesLoading}
            <p class="sub">Chargement du catalogue…</p>
          {:else}
            <div class="results">
              {#each filteredRecipes as r (r.id)}
                <button class="result" onclick={() => pickRecipe(r)}>
                  {#if r.icons?.svg}
                    <img class="result-icon" src={r.icons.svg} alt="" />
                  {/if}
                  <span class="result-name">{r.name}</span>
                  <span class="result-id">{r.id}</span>
                </button>
              {:else}
                <p class="sub">Aucun service ne correspond.</p>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </section>
  </div>
{/if}

{#snippet row(s: Service)}
  <div class="srow-wrap">
    <button
      class="srow"
      class:active={s.id === activeId && view === "service"}
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
    <button class="cog" title="Réglages" onclick={() => openServiceSettings(s)}>⚙</button>
  </div>
{/snippet}

{#snippet toggle(label: string, key: keyof Service, checked: boolean)}
  <label class="row-toggle">
    <input
      type="checkbox"
      {checked}
      onchange={(e) => saveSetting(key, e.currentTarget.checked)}
    />
    <span>{label}</span>
  </label>
{/snippet}

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, system-ui, sans-serif;
    background: #1f2230;
    color: #e8e8ef;
  }
  .login { display: grid; place-items: center; height: 100vh; }
  .card {
    background: #282b3a; padding: 28px; border-radius: 14px; width: 320px;
    display: flex; flex-direction: column; gap: 12px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
  }
  .card h1 { margin: 0; font-size: 24px; }
  .sub { margin: 0 0 6px; color: #9aa0b5; font-size: 13px; }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 12px; color: #9aa0b5; }
  input {
    padding: 9px 11px; border-radius: 8px; border: 1px solid #3a3f55;
    background: #1f2230; color: #e8e8ef; font-size: 14px;
  }
  .primary {
    padding: 10px 14px; border: none; border-radius: 8px; background: #4f46e5;
    color: #fff; font-weight: 700; cursor: pointer;
  }
  .primary:disabled { opacity: 0.6; cursor: default; }
  .gear { align-self: flex-start; background: none; border: none; color: #9aa0b5; cursor: pointer; font-size: 12px; padding: 0; }
  .error { color: #ff8a8a; font-size: 13px; margin: 4px 0; }

  .shell { display: grid; grid-template-columns: 240px 1fr; height: 100vh; }
  .sidebar {
    background: #1b1d28; padding: 12px; overflow-y: auto;
    display: flex; flex-direction: column; gap: 12px;
  }
  .account { display: flex; justify-content: space-between; align-items: center; font-size: 13px; }
  .acts { display: inline-flex; gap: 8px; align-items: center; }
  .link { background: none; border: none; color: #7a82a8; cursor: pointer; font-size: 12px; text-decoration: underline; }
  .add {
    background: #262a3a; border: 1px dashed #3a3f55; color: #b9b2ff;
    border-radius: 8px; padding: 8px; cursor: pointer; font-size: 13px;
  }
  .add:hover { background: #2c3046; }
  .ws-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: #6b7193; margin-bottom: 6px; }

  .srow-wrap { display: flex; align-items: center; }
  .srow {
    display: flex; align-items: center; gap: 9px; flex: 1; min-width: 0;
    padding: 7px 8px; border: none; border-radius: 8px; background: none;
    color: #d6d9e6; cursor: pointer; text-align: left; font-size: 14px;
  }
  .srow:hover { background: #262a3a; }
  .srow.active { background: #4f46e5; color: #fff; }
  .srow.disabled { opacity: 0.45; }
  .srow img, .srow .dot { width: 22px; height: 22px; border-radius: 5px; object-fit: cover; flex: none; }
  .srow .dot { display: grid; place-items: center; background: #3a3f55; font-size: 12px; font-weight: 700; }
  .srow-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cog { background: none; border: none; color: #5b628a; cursor: pointer; font-size: 13px; opacity: 0; padding: 4px; }
  .srow-wrap:hover .cog { opacity: 1; }
  .cog:hover { color: #b9b2ff; }
  .count { margin-top: auto; font-size: 11px; color: #6b7193; }

  .stage { display: grid; place-items: center; overflow: auto; }
  .placeholder { text-align: center; color: #9aa0b5; }
  .panel {
    width: min(560px, 90%); align-self: start; margin: 40px auto;
    background: #232633; border: 1px solid #2f3445; border-radius: 14px; padding: 22px;
    display: flex; flex-direction: column; gap: 14px;
  }
  .panel-head { display: flex; justify-content: space-between; align-items: center; }
  .panel-head h2 { margin: 0; font-size: 18px; }
  .recipe { color: #b9b2ff; font-size: 12px; }
  .row-toggle { flex-direction: row; align-items: center; gap: 10px; color: #d6d9e6; font-size: 14px; cursor: pointer; }
  .row-toggle input { width: auto; }
  .block { gap: 6px; }
  .danger { margin-top: 6px; background: #3a2330; border: 1px solid #6e2b3e; color: #ff9aa8; border-radius: 8px; padding: 9px; cursor: pointer; }
  .danger:hover { background: #46283a; }
  .results { display: flex; flex-direction: column; gap: 6px; max-height: 55vh; overflow-y: auto; }
  .result {
    display: flex; justify-content: space-between; align-items: center;
    background: #1f2230; border: 1px solid #2f3445; border-radius: 8px;
    padding: 9px 11px; cursor: pointer; color: #e8e8ef; text-align: left;
  }
  .result:hover { background: #262a3a; border-color: #4f46e5; }
  .result-icon { width: 22px; height: 22px; border-radius: 5px; flex: none; }
  .result-name { flex: 1; }
  .result-id { color: #6b7193; font-size: 12px; }
</style>
