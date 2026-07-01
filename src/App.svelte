<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
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
    createWorkspace,
    updateWorkspace,
    deleteWorkspace,
    getAppSettings,
    setAppSettings,
    DEFAULT_SERVER,
    type MeUser,
    type Service,
    type Workspace,
    type RecipePreview,
    type AppSettings,
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
  let unreadMap = $state<Record<string, number>>({});
  let activeWorkspace = $state<string | null>(null);

  // Vue de la zone droite : un service, réglages d'un service, ajout, réglages app.
  type View = "service" | "svcSettings" | "add" | "appSettings" | "workspaces";
  let view = $state<View>("service");
  let settingsSvc = $state<Service | null>(null);
  let newWorkspaceName = $state("");

  let appSettings = $state<AppSettings>({
    autostart: false,
    startMinimized: false,
    theme: "system",
  });

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
  const visibleServices = $derived.by(() => {
    if (activeWorkspace) {
      const ws = workspaces.find((w) => w.id === activeWorkspace);
      const ids = new Set(ws?.services ?? []);
      return sorted.filter((s) => ids.has(s.id));
    }
    return sorted;
  });

  const darkMq =
    typeof window !== "undefined"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;

  onMount(async () => {
    // Suit l'apparence du système quand le thème est sur « système ».
    darkMq?.addEventListener("change", () => {
      if (appSettings.theme === "system") applyTheme();
    });
    // Non-lus par service (émis par le poller Rust) -> pastilles sidebar.
    listen<Record<string, number>>("unread", (e) => {
      unreadMap = e.payload;
    });
    try {
      appSettings = await getAppSettings();
      applyTheme();
    } catch {
      /* défauts */
    }
    try {
      me = await restoreSession();
      await loadAfterAuth();
    } catch {
      // pas de session valide -> écran de login
    } finally {
      booting = false;
    }
  });

  function applyTheme() {
    const dark =
      appSettings.theme === "dark" ||
      (appSettings.theme === "system" && (darkMq?.matches ?? true));
    document.body.classList.toggle("light", !dark);
  }

  function openAppSettings() {
    view = "appSettings";
    hideServices();
  }

  async function saveAppSetting(key: keyof AppSettings, value: unknown) {
    (appSettings as Record<string, unknown>)[key] = value;
    if (key === "theme") applyTheme();
    try {
      appSettings = await setAppSettings({
        [key]: value,
      } as Partial<AppSettings>);
      applyTheme();
    } catch (err) {
      error = String(err);
    }
  }

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

  function openWorkspaces() {
    view = "workspaces";
    newWorkspaceName = "";
    hideServices();
  }

  async function reloadWorkspaces() {
    workspaces = await getWorkspaces();
  }

  async function handleCreateWorkspace() {
    const name = newWorkspaceName.trim();
    if (!name) return;
    try {
      await createWorkspace(name);
      newWorkspaceName = "";
      await reloadWorkspaces();
    } catch (err) {
      error = String(err);
    }
  }

  async function toggleServiceInWorkspace(
    ws: Workspace,
    serviceId: string,
    member: boolean,
  ) {
    const set = new Set(ws.services);
    if (member) set.add(serviceId);
    else set.delete(serviceId);
    const list = [...set];
    const idx = workspaces.findIndex((w) => w.id === ws.id);
    if (idx >= 0) workspaces[idx].services = list;
    try {
      await updateWorkspace(ws.id, ws.name, list);
    } catch (err) {
      error = String(err);
    }
  }

  async function renameWorkspace(ws: Workspace, name: string) {
    if (!name.trim() || name === ws.name) return;
    try {
      await updateWorkspace(ws.id, name.trim(), ws.services);
      await reloadWorkspaces();
    } catch (err) {
      error = String(err);
    }
  }

  async function handleDeleteWorkspace(ws: Workspace) {
    if (!confirm(`Supprimer le workspace « ${ws.name} » ?`)) return;
    try {
      await deleteWorkspace(ws.id);
      if (activeWorkspace === ws.id) activeWorkspace = null;
      await reloadWorkspaces();
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

      <div class="wspills">
        <button
          class="pill"
          class:on={activeWorkspace === null}
          onclick={() => (activeWorkspace = null)}>Tous</button>
        {#each workspaces as w (w.id)}
          <button
            class="pill"
            class:on={activeWorkspace === w.id}
            onclick={() => (activeWorkspace = w.id)}>{w.name}</button>
        {/each}
        <button class="pill mng" onclick={openWorkspaces} title="Gérer les workspaces">⚙</button>
      </div>

      <div class="svclist">
        {#each visibleServices as s (s.id)}{@render row(s)}{/each}
      </div>

      <button class="appcog" onclick={openAppSettings}>⚙ Réglages</button>
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
      {:else if view === "appSettings"}
        <div class="panel">
          <div class="panel-head">
            <h2>Réglages</h2>
            <button class="link" onclick={backToService}>✕ fermer</button>
          </div>

          <div class="setblock">
            <div class="set-title">Apparence</div>
            <label class="row-toggle">
              <span>Thème</span>
              <select
                class="select"
                value={appSettings.theme}
                onchange={(e) => saveAppSetting("theme", e.currentTarget.value)}
              >
                <option value="system">Système</option>
                <option value="dark">Sombre</option>
                <option value="light">Clair</option>
              </select>
            </label>
          </div>

          <div class="setblock">
            <div class="set-title">Démarrage</div>
            <label class="row-toggle">
              <input
                type="checkbox"
                checked={appSettings.autostart}
                onchange={(e) => saveAppSetting("autostart", e.currentTarget.checked)}
              />
              <span>Lancer au démarrage de la session</span>
            </label>
            <label class="row-toggle">
              <input
                type="checkbox"
                checked={appSettings.startMinimized}
                onchange={(e) => saveAppSetting("startMinimized", e.currentTarget.checked)}
              />
              <span>Démarrer en arrière-plan (fenêtre masquée)</span>
            </label>
          </div>

          <div class="setblock">
            <div class="set-title">Serveur</div>
            <code class="recipe">{server}</code>
            <p class="sub">
              Connecté : {me.email}. Pour changer de serveur, déconnecte-toi.
            </p>
          </div>

          {#if error}<p class="error">{error}</p>{/if}
        </div>
      {:else if view === "workspaces"}
        <div class="panel">
          <div class="panel-head">
            <h2>Workspaces</h2>
            <button class="link" onclick={backToService}>✕ fermer</button>
          </div>

          <div class="searchrow">
            <input bind:value={newWorkspaceName} placeholder="Nom du nouveau workspace" />
            <button class="primary" onclick={handleCreateWorkspace}>Créer</button>
          </div>
          {#if error}<p class="error">{error}</p>{/if}

          {#each workspaces as ws (ws.id)}
            <div class="wsedit">
              <div class="wsedit-head">
                <input
                  class="wsname"
                  value={ws.name}
                  onblur={(e) => renameWorkspace(ws, e.currentTarget.value)}
                />
                <button class="link" onclick={() => handleDeleteWorkspace(ws)}>supprimer</button>
              </div>
              <div class="set-title">Services de ce workspace</div>
              <div class="wsservices">
                {#each sorted as s (s.id)}
                  <label class="row-toggle">
                    <input
                      type="checkbox"
                      checked={ws.services.includes(s.id)}
                      onchange={(e) =>
                        toggleServiceInWorkspace(ws, s.id, e.currentTarget.checked)}
                    />
                    <span>{s.name}</span>
                  </label>
                {/each}
              </div>
            </div>
          {:else}
            <p class="sub">Aucun workspace. Crée-en un ci-dessus.</p>
          {/each}
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
      {#if (unreadMap[s.id] ?? 0) > 0}
        <span class="ubadge" class:muted={s.isMuted === true}>
          {unreadMap[s.id] > 99 ? "99+" : unreadMap[s.id]}
        </span>
      {/if}
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
  :global(:root) {
    --bg: #1f2230; --sidebar: #1b1d28; --card: #282b3a; --panel: #232633;
    --input: #1f2230; --border: #2f3445; --border2: #3a3f55;
    --text: #e8e8ef; --text2: #d6d9e6; --muted: #9aa0b5; --muted2: #6b7193;
    --hover: #262a3a; --accent: #4f46e5; --accent-soft: #b9b2ff; --link: #7a82a8;
  }
  :global(body.light) {
    --bg: #f3f4f8; --sidebar: #e9ebf1; --card: #ffffff; --panel: #ffffff;
    --input: #ffffff; --border: #d6dae6; --border2: #c8cddc;
    --text: #1c2030; --text2: #2a2f40; --muted: #5b6280; --muted2: #818aa6;
    --hover: #e4e7f0; --accent: #4f46e5; --accent-soft: #5b52d6; --link: #6d75a0;
  }
  :global(body) {
    margin: 0;
    font-family: -apple-system, system-ui, sans-serif;
    background: var(--bg);
    color: var(--text);
  }
  .login { display: grid; place-items: center; height: 100vh; }
  .card {
    background: var(--card); padding: 28px; border-radius: 14px; width: 320px;
    display: flex; flex-direction: column; gap: 12px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
  }
  .card h1 { margin: 0; font-size: 24px; }
  .sub { margin: 0 0 6px; color: var(--muted); font-size: 13px; }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 12px; color: var(--muted); }
  input {
    padding: 9px 11px; border-radius: 8px; border: 1px solid var(--border2);
    background: var(--input); color: var(--text); font-size: 14px;
  }
  .primary {
    padding: 10px 14px; border: none; border-radius: 8px; background: var(--accent);
    color: #fff; font-weight: 700; cursor: pointer;
  }
  .primary:disabled { opacity: 0.6; cursor: default; }
  .gear { align-self: flex-start; background: none; border: none; color: var(--muted); cursor: pointer; font-size: 12px; padding: 0; }
  .error { color: #ff8a8a; font-size: 13px; margin: 4px 0; }

  .shell { display: grid; grid-template-columns: 240px 1fr; height: 100vh; }
  .sidebar {
    background: var(--sidebar); padding: 12px; overflow-y: auto;
    display: flex; flex-direction: column; gap: 12px;
  }
  .account { display: flex; justify-content: space-between; align-items: center; font-size: 13px; }
  .acts { display: inline-flex; gap: 8px; align-items: center; }
  .link { background: none; border: none; color: var(--link); cursor: pointer; font-size: 12px; text-decoration: underline; }
  .add {
    background: var(--hover); border: 1px dashed var(--border2); color: var(--accent-soft);
    border-radius: 8px; padding: 8px; cursor: pointer; font-size: 13px;
  }
  .add:hover { filter: brightness(1.1); }
  .ws-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted2); margin-bottom: 6px; }
  .wspills { display: flex; flex-wrap: wrap; gap: 5px; }
  .pill {
    background: var(--hover); border: none; color: var(--muted);
    border-radius: 999px; padding: 3px 10px; cursor: pointer; font-size: 12px;
  }
  .pill.on { background: var(--accent); color: #fff; }
  .svclist { display: flex; flex-direction: column; gap: 2px; }
  .ubadge {
    background: #e23b3b; color: #fff; font-size: 11px; font-weight: 700;
    min-width: 18px; height: 18px; padding: 0 5px; border-radius: 9px; flex: none;
    display: inline-flex; align-items: center; justify-content: center;
  }
  .ubadge.muted { background: var(--muted2); }

  .srow-wrap { display: flex; align-items: center; }
  .srow {
    display: flex; align-items: center; gap: 9px; flex: 1; min-width: 0;
    padding: 7px 8px; border: none; border-radius: 8px; background: none;
    color: var(--text2); cursor: pointer; text-align: left; font-size: 14px;
  }
  .srow:hover { background: var(--hover); }
  .srow.active { background: var(--accent); color: #fff; }
  .srow.disabled { opacity: 0.45; }
  .srow img, .srow .dot { width: 22px; height: 22px; border-radius: 5px; object-fit: cover; flex: none; }
  .srow .dot { display: grid; place-items: center; background: var(--border2); font-size: 12px; font-weight: 700; }
  .srow-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cog { background: none; border: none; color: var(--muted2); cursor: pointer; font-size: 13px; opacity: 0; padding: 4px; }
  .srow-wrap:hover .cog { opacity: 1; }
  .cog:hover { color: var(--accent-soft); }
  .appcog {
    margin-top: auto; background: var(--hover); border: 1px solid var(--border);
    color: var(--text2); border-radius: 8px; padding: 8px; cursor: pointer; font-size: 13px;
  }
  .appcog:hover { filter: brightness(1.1); }
  .count { font-size: 11px; color: var(--muted2); }

  .stage { display: grid; place-items: center; overflow: auto; }
  .placeholder { text-align: center; color: var(--muted); }
  .panel {
    width: min(560px, 90%); align-self: start; margin: 40px auto;
    background: var(--panel); border: 1px solid var(--border); border-radius: 14px; padding: 22px;
    display: flex; flex-direction: column; gap: 14px;
  }
  .panel-head { display: flex; justify-content: space-between; align-items: center; }
  .panel-head h2 { margin: 0; font-size: 18px; }
  .recipe { color: var(--accent-soft); font-size: 12px; }
  .setblock { display: flex; flex-direction: column; gap: 8px; padding-bottom: 8px; border-bottom: 1px solid var(--border); }
  .set-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted2); }
  .row-toggle { flex-direction: row; align-items: center; gap: 10px; color: var(--text2); font-size: 14px; cursor: pointer; }
  .row-toggle input { width: auto; }
  .select {
    margin-left: auto; padding: 6px 9px; border-radius: 8px;
    border: 1px solid var(--border2); background: var(--input); color: var(--text); font-size: 13px;
  }
  .searchrow { display: flex; gap: 8px; }
  .searchrow input { flex: 1; }
  .pill.mng { background: transparent; border: 1px dashed var(--border2); color: var(--muted); }
  .wsedit {
    display: flex; flex-direction: column; gap: 8px; padding: 12px;
    border: 1px solid var(--border); border-radius: 10px; background: var(--input);
  }
  .wsedit-head { display: flex; gap: 10px; align-items: center; }
  .wsname { flex: 1; }
  .wsservices { display: flex; flex-direction: column; gap: 4px; max-height: 30vh; overflow-y: auto; }
  .block { gap: 6px; }
  .danger { margin-top: 6px; background: #3a2330; border: 1px solid #6e2b3e; color: #ff9aa8; border-radius: 8px; padding: 9px; cursor: pointer; }
  .danger:hover { filter: brightness(1.15); }
  .results { display: flex; flex-direction: column; gap: 6px; max-height: 55vh; overflow-y: auto; }
  .result {
    display: flex; justify-content: space-between; align-items: center;
    background: var(--input); border: 1px solid var(--border); border-radius: 8px;
    padding: 9px 11px; cursor: pointer; color: var(--text); text-align: left;
  }
  .result:hover { background: var(--hover); border-color: var(--accent); }
  .result-icon { width: 22px; height: 22px; border-radius: 5px; flex: none; }
  .result-name { flex: 1; }
  .result-id { color: var(--muted2); font-size: 12px; }
</style>
