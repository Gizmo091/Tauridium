<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { accentFg, iconSrc, filterRecipes, snapIconSize } from "./lib/ui";
  import { appVersion, checkForUpdate, installUpdate, type Update } from "./lib/updater";
  import { ask } from "@tauri-apps/plugin-dialog";

  // window.confirm() ne fonctionne pas dans WKWebView (wry n'implémente pas le panel JS)
  // -> on passe par le dialogue natif du plugin dialog.
  function confirmAsk(message: string): Promise<boolean> {
    return ask(message, { title: "Tauridium", kind: "warning" });
  }
  import {
    login,
    restoreSession,
    getServices,
    getWorkspaces,
    logout,
    showService,
    preloadService,
    closeService,
    closeServices,
    hideServices,
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
    setSidebarWidth,
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

  // Reconnexion auto quand le serveur est injoignable (panne Ferdium, réseau…).
  const RECONNECT_SECS = 30;
  let reconnecting = $state(false);
  let reconnectIn = $state(RECONNECT_SECS);
  let pendingCreds = $state<{ server: string; email: string; password: string } | null>(
    null,
  );
  let reconnectAttempt: (() => Promise<boolean>) | null = null;
  let reconnectTimer: ReturnType<typeof setInterval> | null = null;

  let me = $state<MeUser | null>(null);
  let services = $state<Service[]>([]);
  let workspaces = $state<Workspace[]>([]);
  let activeId = $state<string | null>(null);
  let unreadMap = $state<Record<string, number>>({});
  let failedIcons = $state<Set<string>>(new Set());
  let activeWorkspace = $state<string | null>(null);

  type View = "service" | "svcSettings" | "add" | "appSettings" | "workspaces";
  let view = $state<View>("service");
  let settingsSvc = $state<Service | null>(null);
  let svcDirty = $state(false); // réglages service modifiés mais pas encore sauvés
  let svcReload = $state(false); // un champ nécessitant un reload (URL/team/UA) a changé
  let newWorkspaceName = $state("");

  type Tab = "general" | "services" | "appearance" | "privacy" | "advanced" | "updates";
  let settingsTab = $state<Tab>("general");

  // Mises à jour (auto-updater).
  let appVer = $state("");
  let updateInfo = $state<Update | null>(null);
  let updChecking = $state(false);
  let updInstalling = $state(false);
  let updStatus = $state("");

  let appSettings = $state<AppSettings>({
    autostart: false,
    startMinimized: false,
    theme: "system",
    accentColor: "#ffc131",
    closeToSystemTray: true,
    privateNotifications: false,
    showDisabledServices: true,
    showServiceName: true,
    showMessageBadgeWhenMuted: true,
    userAgentPref: "",
    sidebarWidth: 240,
    iconSize: 24,
    grayscaleServices: false,
    grayscaleDim: 50,
    sidebarServicesLocation: "top",
    hibernationTimer: 0,
    preloadServices: true,
  });

  // Hibernation : services mis en veille (webview fermée, session conservée).
  let hibernated = $state<Set<string>>(new Set());
  const hibTimers = new Map<string, ReturnType<typeof setTimeout>>();

  // Add service: full catalog loaded once, filtered live.
  let recipeQuery = $state("");
  let allRecipes = $state<RecipePreview[]>([]);
  let recipesLoading = $state(false);
  let newServiceName = $state("");

  const activeService = $derived(
    services.find((s) => s.id === activeId) ?? null,
  );
  const sorted = $derived(
    [...services].sort((a, b) => (a.order ?? 0) - (b.order ?? 0)),
  );
  const visibleServices = $derived.by(() => {
    let list = sorted;
    if (activeWorkspace) {
      const ws = workspaces.find((w) => w.id === activeWorkspace);
      const ids = new Set(ws?.services ?? []);
      list = list.filter((s) => ids.has(s.id));
    }
    if (!appSettings.showDisabledServices) {
      list = list.filter((s) => s.isEnabled);
    }
    return list;
  });

  const darkMq =
    typeof window !== "undefined"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;

  // Formulations adaptées à l'OS (les descriptions de réglages en dépendent).
  const osKind: "mac" | "win" | "linux" = /Mac|iPhone|iPad/.test(
    navigator.userAgent,
  )
    ? "mac"
    : /Win/.test(navigator.userAgent)
      ? "win"
      : "linux";
  const trayWord = osKind === "mac" ? "menu bar" : "system tray";
  const dockWord = osKind === "mac" ? "Dock" : "taskbar";
  const loginText =
    osKind === "mac"
      ? "when you sign in to your Mac"
      : osKind === "win"
        ? "when you sign in to Windows"
        : "when you log in";

  onMount(async () => {
    darkMq?.addEventListener("change", () => {
      if (appSettings.theme === "system") applyTheme();
    });
    listen<Record<string, number>>("unread", (e) => {
      unreadMap = e.payload;
    });
    try {
      appSettings = await getAppSettings();
      // Snap iconSize sur un niveau valide (anciennes valeurs improvisées).
      const snapped = snapIconSize(appSettings.iconSize);
      if (snapped !== appSettings.iconSize) {
        appSettings.iconSize = snapped;
        setAppSettings({ iconSize: snapped }).catch(() => {});
      }
      applyTheme();
      applyLayout();
    } catch {
      /* defaults */
    }
    // Restaure la session. Si le serveur est injoignable (panne Ferdium, réseau…), on
    // n'affiche PAS le login : un écran « reconnexion » retente tout seul toutes les 30s.
    const restored = await attemptRestore();
    booting = false;
    if (!restored) startReconnect(attemptRestore);
    appVersion()
      .then((v) => (appVer = v))
      .catch(() => {});
    checkUpdates(true); // vérif silencieuse au démarrage
  });

  function applyTheme() {
    const dark =
      appSettings.theme === "dark" ||
      (appSettings.theme === "system" && (darkMq?.matches ?? true));
    document.body.classList.toggle("light", !dark);
    document.body.style.setProperty("--accent", appSettings.accentColor);
    document.body.style.setProperty("--accent-fg", accentFg(appSettings.accentColor));
  }

  // Customisation sidebar : largeur, taille des icônes, gris + dim, position.
  function applyLayout() {
    const b = document.body;
    b.style.setProperty("--sidebar-w", `${appSettings.sidebarWidth}px`);
    b.style.setProperty("--icon-size", `${appSettings.iconSize}px`);
    b.classList.toggle("grayscale", !!appSettings.grayscaleServices);
    // dim 0..100 -> opacité des icônes en gris (100 = très estompé)
    const op = Math.max(0.2, 1 - (appSettings.grayscaleDim ?? 50) / 130);
    b.style.setProperty("--gray-op", String(op));
    b.dataset.svcloc = appSettings.sidebarServicesLocation ?? "top";
  }

  function markIconFailed(id: string) {
    failedIcons = new Set(failedIcons).add(id);
  }

  async function loadAfterAuth() {
    [services, workspaces] = await Promise.all([getServices(), getWorkspaces()]);
    await Promise.all(services.map((s) => setServiceFlags(s).catch(() => {})));
    const first = sorted.find((s) => s.isEnabled) ?? sorted[0] ?? null;
    if (first) selectService(first);
    preloadRest(first?.id);
  }

  // Précharge (webviews hors-écran) les autres services actifs, en douceur (échelonné),
  // pour que la bascule vers l'un d'eux soit quasi instantanée. On saute ceux voués à
  // l'hibernation (ils seraient déchargés) et on respecte le réglage.
  function preloadRest(firstId: string | undefined) {
    if (!appSettings.preloadServices) return;
    const list = sorted.filter(
      (s) =>
        s.isEnabled &&
        s.id !== firstId &&
        !(appSettings.hibernationTimer > 0 && s.isHibernationEnabled === true),
    );
    let i = 0;
    const step = () => {
      const s = list[i++];
      if (!s) return;
      preloadService(s)
        .catch(() => {})
        .finally(() => setTimeout(step, 700));
    };
    setTimeout(step, 1500); // laisse le service actif se charger d'abord
  }

  function stopReconnect() {
    if (reconnectTimer) {
      clearInterval(reconnectTimer);
      reconnectTimer = null;
    }
    reconnectAttempt = null;
    reconnecting = false;
  }

  // Lance une boucle de reconnexion : rappelle `attempt` toutes les RECONNECT_SECS.
  // `attempt` renvoie true quand c'est terminé (succès OU erreur définitive) -> stop ;
  // false -> serveur toujours injoignable -> on retentera.
  function startReconnect(attempt: () => Promise<boolean>) {
    stopReconnect();
    reconnectAttempt = attempt;
    reconnecting = true;
    reconnectIn = RECONNECT_SECS;
    reconnectTimer = setInterval(async () => {
      reconnectIn -= 1;
      if (reconnectIn <= 0) {
        reconnectIn = RECONNECT_SECS;
        if (await attempt()) stopReconnect();
      }
    }, 1000);
  }

  async function retryNow() {
    const fn = reconnectAttempt;
    if (!fn) return;
    reconnectIn = RECONNECT_SECS;
    if (await fn()) stopReconnect();
  }

  function cancelReconnect() {
    pendingCreds = null;
    stopReconnect();
  }

  // Tente de restaurer la session. true = terminé (connecté OU session expirée ->
  // écran login) ; false = serveur injoignable (on retentera).
  async function attemptRestore(): Promise<boolean> {
    try {
      me = await restoreSession();
      await loadAfterAuth();
      return true;
    } catch (e) {
      return !String(e).startsWith("transient:");
    }
  }

  // Tente le login avec les identifiants en attente. false = serveur injoignable.
  async function attemptLogin(): Promise<boolean> {
    if (!pendingCreds) return true;
    try {
      me = await login(pendingCreds.server, pendingCreds.email, pendingCreds.password);
      pendingCreds = null;
      password = "";
      error = null;
      await loadAfterAuth();
      return true;
    } catch (e) {
      if (String(e).startsWith("transient:")) return false;
      error = String(e); // identifiants refusés -> on arrête et on affiche l'erreur
      pendingCreds = null;
      return true;
    }
  }

  async function handleLogin(e: Event) {
    e.preventDefault();
    loading = true;
    error = null;
    pendingCreds = { server, email, password };
    const done = await attemptLogin();
    loading = false;
    if (!done) startReconnect(attemptLogin); // serveur injoignable -> reconnexion auto
  }

  function clearHibTimer(sid: string) {
    const t = hibTimers.get(sid);
    if (t) {
      clearTimeout(t);
      hibTimers.delete(sid);
    }
  }

  // Programme la mise en veille d'un service inactif éligible.
  function scheduleHibernation(sid: string) {
    clearHibTimer(sid);
    const secs = appSettings.hibernationTimer;
    const svc = services.find((s) => s.id === sid);
    if (!secs || secs <= 0 || svc?.isHibernationEnabled !== true) return;
    hibTimers.set(
      sid,
      setTimeout(() => {
        hibTimers.delete(sid);
        if (activeId === sid) return; // redevenu actif entre-temps
        closeService(sid)
          .then(() => {
            hibernated = new Set(hibernated).add(sid);
          })
          .catch(() => {});
      }, secs * 1000),
    );
  }

  function selectService(s: Service) {
    const prev = activeId;
    view = "service";
    activeId = s.id;
    clearHibTimer(s.id);
    if (hibernated.has(s.id)) {
      const next = new Set(hibernated);
      next.delete(s.id);
      hibernated = next;
    }
    if (prev && prev !== s.id) scheduleHibernation(prev);
    showService(s).catch((err) => {
      error = `Service "${s.name}": ${err}`;
    });
  }

  function openServiceSettings(s: Service) {
    settingsSvc = { ...s }; // copie éditable ; appliquée au serveur au Save
    svcDirty = false;
    svcReload = false;
    view = "svcSettings";
    hideServices();
  }

  async function persistService(reload = false) {
    if (!settingsSvc) return;
    const s = settingsSvc;
    const idx = services.findIndex((x) => x.id === s.id);
    if (idx >= 0) services[idx] = { ...s };
    try {
      await updateService(s.id, {
        name: s.name,
        isEnabled: s.isEnabled,
        isNotificationEnabled: s.isNotificationEnabled,
        isMuted: s.isMuted,
        isBadgeEnabled: s.isBadgeEnabled,
        isMediaBadgeEnabled: s.isMediaBadgeEnabled,
        isIndirectMessageBadgeEnabled: s.isIndirectMessageBadgeEnabled,
        isHibernationEnabled: s.isHibernationEnabled,
        isWakeUpEnabled: s.isWakeUpEnabled,
        trapLinkClicks: s.trapLinkClicks,
        useFavicon: s.useFavicon,
        isDarkModeEnabled: s.isDarkModeEnabled,
        isProgressbarEnabled: s.isProgressbarEnabled,
        onlyShowFavoritesInUnreadCount: s.onlyShowFavoritesInUnreadCount,
        darkReaderBrightness: s.darkReaderBrightness,
        darkReaderContrast: s.darkReaderContrast,
        darkReaderSepia: s.darkReaderSepia,
        isProxyFeatureEnabled: s.isProxyFeatureEnabled,
        proxyHost: s.proxyHost ?? "",
        proxyPort: s.proxyPort ?? "",
        proxyUser: s.proxyUser ?? "",
        proxyPassword: s.proxyPassword ?? "",
        customUrl: s.customUrl ?? "",
        team: s.team ?? "",
        userAgentPref: s.userAgentPref ?? "",
      });
      await setServiceFlags(s);
      if (reload) await closeService(s.id); // recreated on next open with new params
    } catch (err) {
      error = String(err);
    }
  }

  // Les handlers ne modifient QUE l'état local ; le Save persiste tout d'un coup.
  function saveSetting(key: keyof Service, value: boolean) {
    if (!settingsSvc) return;
    (settingsSvc as Record<string, unknown>)[key] = value;
    svcDirty = true;
  }

  function saveText(key: keyof Service, value: string, reload = false) {
    if (!settingsSvc) return;
    (settingsSvc as Record<string, unknown>)[key] = value;
    svcDirty = true;
    if (reload) svcReload = true;
  }

  function saveNum(key: keyof Service, value: string) {
    if (!settingsSvc) return;
    const n = Number.parseInt(value, 10);
    (settingsSvc as Record<string, unknown>)[key] = Number.isNaN(n) ? undefined : n;
    svcDirty = true;
  }

  async function saveServiceSettings() {
    await persistService(svcReload);
    svcDirty = false;
    svcReload = false;
  }

  async function handleDelete(s: Service) {
    if (!(await confirmAsk(`Delete service "${s.name}"?`))) return;
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
    if (!(await confirmAsk(`Delete workspace "${ws.name}"?`))) return;
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

  const filteredRecipes = $derived(filterRecipes(allRecipes, recipeQuery));

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

  function openAppSettings() {
    view = "appSettings";
    hideServices();
  }

  async function checkUpdates(silent = false) {
    updChecking = true;
    if (!silent) updStatus = "";
    try {
      updateInfo = await checkForUpdate();
      if (!silent && !updateInfo) updStatus = "You're on the latest version.";
    } catch (e) {
      if (!silent) updStatus = `Update check failed: ${e}`;
    } finally {
      updChecking = false;
    }
  }

  async function doInstall() {
    if (!updateInfo) return;
    updInstalling = true;
    updStatus = "Downloading…";
    try {
      await installUpdate(updateInfo); // télécharge, installe, relance
    } catch (e) {
      updStatus = `Update failed: ${e}`;
      updInstalling = false;
    }
  }

  async function saveAppSetting(key: keyof AppSettings, value: unknown) {
    (appSettings as Record<string, unknown>)[key] = value;
    if (key === "theme" || key === "accentColor") applyTheme();
    applyLayout();
    if (key === "sidebarWidth") setSidebarWidth(value as number).catch(() => {});
    try {
      appSettings = await setAppSettings({
        [key]: value,
      } as Partial<AppSettings>);
      applyTheme();
      applyLayout();
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
    <div class="card"><p class="sub">Restoring session…</p></div>
  </main>
{:else if reconnecting}
  <main class="login">
    <div class="card">
      <h1>Tauridium</h1>
      <p class="notice">⚠️ Can't reach the server — it may be temporarily down.</p>
      <p class="sub">Retrying automatically in {reconnectIn}s…</p>
      <button class="primary" onclick={retryNow}>Retry now</button>
      <button class="link" onclick={cancelReconnect}>
        {pendingCreds ? "Cancel" : "Sign in with a different account"}
      </button>
    </div>
  </main>
{:else if !me}
  <main class="login">
    <form class="card" onsubmit={handleLogin}>
      <h1>Tauridium</h1>
      <p class="sub">Lightweight Ferdium client — sign in to your server</p>
      <label>
        Email
        <input type="email" bind:value={email} autocomplete="username" required />
      </label>
      <label>
        Password
        <input
          type="password"
          bind:value={password}
          autocomplete="current-password"
          required
        />
      </label>
      <button type="button" class="gear" onclick={() => (showServer = !showServer)}>
        ⚙︎ Server {showServer ? "▲" : "▼"}
      </button>
      {#if showServer}
        <label>
          Server URL
          <input type="url" bind:value={server} placeholder={DEFAULT_SERVER} />
        </label>
      {/if}
      {#if error}<p class="error">{error}</p>{/if}
      <button class="primary" type="submit" disabled={loading}>
        {loading ? "Signing in…" : "Sign in"}
      </button>
    </form>
  </main>
{:else}
  <div class="shell">
    <aside class="sidebar">
      <div class="account">
        <strong>{me.firstname || me.email}</strong>
        <button class="link" onclick={handleLogout}>sign out</button>
      </div>

      <button class="add" onclick={openAdd}>＋ Add a service</button>

      <div class="wspills">
        <button
          class="pill"
          class:on={activeWorkspace === null}
          onclick={() => (activeWorkspace = null)}>All</button>
        {#each workspaces as w (w.id)}
          <button
            class="pill"
            class:on={activeWorkspace === w.id}
            onclick={() => (activeWorkspace = w.id)}>{w.name}</button>
        {/each}
        <button class="pill mng" onclick={openWorkspaces} title="Manage workspaces">⚙</button>
      </div>

      <div class="svcarea">
        <div class="svclist">
          {#each visibleServices as s (s.id)}{@render row(s)}{/each}
        </div>
      </div>

      <button class="appcog" onclick={openAppSettings}>
        <span class="ic">⚙</span> Settings{#if updateInfo}<span class="upddot" title="Update available"></span>{/if}
      </button>
      <div class="count">
        {services.length} services · {workspaces.length} workspaces{#if appVer} · <span class="ver">v{appVer}</span>{/if}
      </div>
    </aside>

    <section class="stage">
      {#if view === "service"}
        {#if activeService}
          <div class="placeholder">
            <h2>{activeService.name}</h2>
            <p>Loading webview…</p>
          </div>
        {:else}
          <div class="placeholder"><p>No service selected.</p></div>
        {/if}
      {:else if view === "svcSettings" && settingsSvc}
        <div class="panel">
          <div class="panel-head">
            <h2>Settings — {settingsSvc.name}</h2>
            <span class="head-actions">
              <button class="primary sm" disabled={!svcDirty} onclick={saveServiceSettings}>
                {svcDirty ? "Save changes" : "Saved"}
              </button>
              <button class="link" onclick={backToService}>✕ close</button>
            </span>
          </div>
          <code class="recipe">recipe: {settingsSvc.recipeId}</code>

          <div class="set-title">General</div>
          <label class="block">
            Name
            <input value={settingsSvc.name} onchange={(e) => saveText("name", e.currentTarget.value)} />
          </label>
          <div class="setrow">
            <label class="block">
              Custom URL
              <input value={settingsSvc.customUrl ?? ""} placeholder="https://… (for services that support it)" onchange={(e) => saveText("customUrl", e.currentTarget.value, true)} />
            </label>
            <p class="desc">Override the service URL (self-hosted instances, custom domains). Reloads the service.</p>
          </div>
          <div class="setrow">
            <label class="block">
              Team / workspace ID
              <input value={settingsSvc.team ?? ""} placeholder="e.g. Slack team" onchange={(e) => saveText("team", e.currentTarget.value, true)} />
            </label>
            <p class="desc">For services whose URL includes a team ID (Slack, etc.). Reloads the service.</p>
          </div>

          {@render toggle("Enabled", "Load this service. Disabled services stay listed but aren't loaded.", "isEnabled", settingsSvc.isEnabled !== false)}
          {@render toggle("Notifications", "Show system notifications for new messages in this service.", "isNotificationEnabled", settingsSvc.isNotificationEnabled !== false)}
          {@render toggle("Muted", "Silence this service — no notifications at all.", "isMuted", settingsSvc.isMuted === true)}
          {@render toggle("Unread badge", `Count this service's unread messages in the ${dockWord} badge.`, "isBadgeEnabled", settingsSvc.isBadgeEnabled !== false)}
          {@render toggle("Indirect message badge", "Also count indirect (group / channel) messages in the badge.", "isIndirectMessageBadgeEnabled", settingsSvc.isIndirectMessageBadgeEnabled === true)}
          {@render toggle("Media badge", "Count calls / media activity in the badge.", "isMediaBadgeEnabled", settingsSvc.isMediaBadgeEnabled === true)}
          {@render toggle("Allow hibernation", "Let this service sleep when inactive to save memory.", "isHibernationEnabled", settingsSvc.isHibernationEnabled === true)}
          {@render toggle("Open links externally", "Open clicked links in your default browser instead of inside the service.", "trapLinkClicks", settingsSvc.trapLinkClicks === true)}
          {@render toggle("Allow wake up", "Wake this service from hibernation on new activity.", "isWakeUpEnabled", settingsSvc.isWakeUpEnabled === true)}
          {@render toggle("Only favorites in unread count", "Count unread messages only from favorite chats in this service.", "onlyShowFavoritesInUnreadCount", settingsSvc.onlyShowFavoritesInUnreadCount === true)}

          <div class="set-title">Appearance</div>
          {@render toggle("Dark mode", "Apply the recipe's dark theme to this service (synced with Ferdium).", "isDarkModeEnabled", settingsSvc.isDarkModeEnabled === true)}
          {#if settingsSvc.isDarkModeEnabled}
            <div class="setrow">
              <div class="num-row">
                <label>Brightness<input class="num" type="number" value={settingsSvc.darkReaderBrightness ?? 100} onchange={(e) => saveNum("darkReaderBrightness", e.currentTarget.value)} /></label>
                <label>Contrast<input class="num" type="number" value={settingsSvc.darkReaderContrast ?? 90} onchange={(e) => saveNum("darkReaderContrast", e.currentTarget.value)} /></label>
                <label>Sepia<input class="num" type="number" value={settingsSvc.darkReaderSepia ?? 10} onchange={(e) => saveNum("darkReaderSepia", e.currentTarget.value)} /></label>
              </div>
              <p class="desc">Dark Reader fine-tuning (applies to this service's dark mode).</p>
            </div>
          {/if}
          {@render toggle("Use favicon as icon", "Use the site's favicon instead of the recipe icon.", "useFavicon", settingsSvc.useFavicon === true)}
          {@render toggle("Progress bar", "Show a loading progress bar for this service.", "isProgressbarEnabled", settingsSvc.isProgressbarEnabled === true)}
          <div class="setrow">
            <label class="block">
              Custom user agent
              <input value={settingsSvc.userAgentPref ?? ""} placeholder="empty = app default" onchange={(e) => saveText("userAgentPref", e.currentTarget.value, true)} />
            </label>
            <p class="desc">Per-service browser identity, overrides the global one. Reloads the service.</p>
          </div>

          <div class="set-title">Proxy</div>
          {@render toggle("Use a proxy", "Route this service through an HTTP/HTTPS proxy (synced with Ferdium).", "isProxyFeatureEnabled", settingsSvc.isProxyFeatureEnabled === true)}
          {#if settingsSvc.isProxyFeatureEnabled}
            <div class="setrow">
              <div class="proxy-grid">
                <input placeholder="Host" value={settingsSvc.proxyHost ?? ""} onchange={(e) => saveText("proxyHost", e.currentTarget.value)} />
                <input placeholder="Port" value={String(settingsSvc.proxyPort ?? "")} onchange={(e) => saveText("proxyPort", e.currentTarget.value)} />
                <input placeholder="Username (optional)" value={settingsSvc.proxyUser ?? ""} onchange={(e) => saveText("proxyUser", e.currentTarget.value)} />
                <input placeholder="Password (optional)" type="password" value={settingsSvc.proxyPassword ?? ""} onchange={(e) => saveText("proxyPassword", e.currentTarget.value)} />
              </div>
            </div>
          {/if}

          {#if error}<p class="error">{error}</p>{/if}
          <button class="danger" onclick={() => settingsSvc && handleDelete(settingsSvc)}>
            Delete this service
          </button>
        </div>
      {:else if view === "add"}
        <div class="panel">
          <div class="panel-head">
            <h2>Add a service</h2>
            <button class="link" onclick={backToService}>✕ close</button>
          </div>
          <p class="notice">⚠️ Passkey / biometric sign-in (Touch ID, security keys) isn't supported in the embedded webview. On a service's login screen, choose "try another way" and use a password + authenticator code (TOTP) or a phone prompt instead.</p>
          <label class="block">
            Name (optional)
            <input bind:value={newServiceName} placeholder="leave empty = recipe name" />
          </label>
          <input
            class="filter"
            bind:value={recipeQuery}
            placeholder="Filter among {allRecipes.length} services…"
          />
          {#if error}<p class="error">{error}</p>{/if}
          {#if recipesLoading}
            <p class="sub">Loading catalog…</p>
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
                <p class="sub">No service matches.</p>
              {/each}
            </div>
          {/if}
        </div>
      {:else if view === "workspaces"}
        <div class="panel">
          <div class="panel-head">
            <h2>Workspaces</h2>
            <button class="link" onclick={backToService}>✕ close</button>
          </div>

          <div class="searchrow">
            <input bind:value={newWorkspaceName} placeholder="New workspace name" />
            <button class="primary" onclick={handleCreateWorkspace}>Create</button>
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
                <button class="link" onclick={() => handleDeleteWorkspace(ws)}>delete</button>
              </div>
              <div class="set-title">Services in this workspace</div>
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
            <p class="sub">No workspace yet. Create one above.</p>
          {/each}
        </div>
      {:else if view === "appSettings"}
        <div class="panel">
          <div class="panel-head">
            <h2>Settings</h2>
            <button class="link" onclick={backToService}>✕ close</button>
          </div>

          <div class="tabs">
            {#each [["general", "General"], ["services", "Services"], ["appearance", "Appearance"], ["privacy", "Privacy"], ["advanced", "Advanced"], ["updates", "Updates"]] as [id, label] (id)}
              <button
                class="tab"
                class:on={settingsTab === id}
                onclick={() => (settingsTab = id as Tab)}>{label}</button>
            {/each}
          </div>

          {#if settingsTab === "general"}
            {@render appToggle("Launch at login", `Start Tauridium automatically ${loginText}.`, "autostart", appSettings.autostart)}
            {@render appToggle("Start in background", `Launch with the window hidden — Tauridium stays in the ${trayWord}.`, "startMinimized", appSettings.startMinimized)}
            {@render appToggle("Close button hides to tray", `The window's close button hides Tauridium to the ${trayWord} instead of quitting it.`, "closeToSystemTray", appSettings.closeToSystemTray)}
          {:else if settingsTab === "services"}
            {@render appToggle("Show disabled services", "Keep disabled services visible (dimmed) in the sidebar instead of hiding them.", "showDisabledServices", appSettings.showDisabledServices)}
            {@render appToggle("Show service names", "Show the name next to each service icon in the sidebar.", "showServiceName", appSettings.showServiceName)}
            {@render appToggle("Unread badge on muted services", "Still show the unread count on services that are muted.", "showMessageBadgeWhenMuted", appSettings.showMessageBadgeWhenMuted)}
            <div class="setrow">
              <label class="row-toggle">
                <span>Hibernate inactive services</span>
                <select
                  class="select"
                  bind:value={appSettings.hibernationTimer}
                  onchange={() => saveAppSetting("hibernationTimer", appSettings.hibernationTimer)}
                >
                  <option value={0}>Off</option>
                  <option value={30}>After 30s</option>
                  <option value={60}>After 1 min</option>
                  <option value={300}>After 5 min</option>
                </select>
              </label>
              <p class="desc">Unload inactive services (per-service "Allow hibernation" must be on) to save memory. A hibernated service stops reporting unread until you reopen it.</p>
            </div>
            {@render appToggle("Preload services at startup", "Load your services in the background right after launch so switching to them is instant (uses more memory).", "preloadServices", appSettings.preloadServices)}
          {:else if settingsTab === "appearance"}
            <div class="setrow">
              <label class="row-toggle">
                <span>Theme</span>
                <select
                  class="select"
                  bind:value={appSettings.theme}
                  onchange={() => saveAppSetting("theme", appSettings.theme)}
                >
                  <option value="system">System</option>
                  <option value="dark">Dark</option>
                  <option value="light">Light</option>
                </select>
              </label>
              <p class="desc">System follows your macOS appearance; Dark or Light force one.</p>
            </div>
            <div class="setrow">
              <label class="row-toggle">
                <span>Accent color</span>
                <span class="swatches">
                  {#each ["#ffc131", "#4f46e5", "#2563eb", "#0891b2", "#16a34a", "#d97706", "#dc2626", "#db2777", "#7c3aed"] as c (c)}
                    <button
                      class="swatch"
                      class:on={appSettings.accentColor === c}
                      style="background:{c}"
                      aria-label={c}
                      onclick={() => saveAppSetting("accentColor", c)}
                    ></button>
                  {/each}
                </span>
              </label>
              <p class="desc">Highlight color for the active service, buttons and selected workspace.</p>
            </div>

            <div class="set-title">Sidebar</div>
            <div class="setrow">
              <label class="row-toggle">
                <span>Sidebar width</span>
                <select
                  class="select"
                  bind:value={appSettings.sidebarWidth}
                  onchange={() => saveAppSetting("sidebarWidth", appSettings.sidebarWidth)}
                >
                  <option value={200}>Compact</option>
                  <option value={240}>Normal</option>
                  <option value={300}>Wide</option>
                </select>
              </label>
              <p class="desc">Width of the services sidebar.</p>
            </div>
            <div class="setrow">
              <label class="row-toggle">
                <span>Icon size</span>
                <select
                  class="select"
                  bind:value={appSettings.iconSize}
                  onchange={() => saveAppSetting("iconSize", appSettings.iconSize)}
                >
                  <option value={18}>Very small</option>
                  <option value={21}>Small</option>
                  <option value={24}>Normal</option>
                  <option value={28}>Large</option>
                  <option value={34}>Very large</option>
                </select>
              </label>
              <p class="desc">Size of the service icons in the sidebar.</p>
            </div>
            <div class="setrow">
              <label class="row-toggle">
                <span>Services location</span>
                <select
                  class="select"
                  bind:value={appSettings.sidebarServicesLocation}
                  onchange={() => saveAppSetting("sidebarServicesLocation", appSettings.sidebarServicesLocation)}
                >
                  <option value="top">Top</option>
                  <option value="center">Center</option>
                  <option value="bottom">Bottom</option>
                </select>
              </label>
              <p class="desc">Vertical position of the service list in the sidebar.</p>
            </div>
            {@render appToggle("Grayscale service icons", "Show icons in grayscale; full color on hover and when active.", "grayscaleServices", appSettings.grayscaleServices)}
            {#if appSettings.grayscaleServices}
              <div class="setrow">
                <label class="row-toggle">
                  <span>Grayscale dim level</span>
                  <input
                    class="range"
                    type="range"
                    min="0"
                    max="100"
                    value={appSettings.grayscaleDim}
                    onchange={(e) => saveAppSetting("grayscaleDim", Number(e.currentTarget.value))}
                  />
                </label>
                <p class="desc">How much inactive grayscale icons are dimmed.</p>
              </div>
            {/if}
          {:else if settingsTab === "privacy"}
            {@render appToggle("Private notifications", "Hide the sender and message content in system notifications (shows just 'New message').", "privateNotifications", appSettings.privateNotifications)}
          {:else if settingsTab === "advanced"}
            <div class="setrow">
              <label class="block">
                Custom user agent
                <input
                  value={appSettings.userAgentPref}
                  placeholder="empty = default (modern Safari)"
                  onchange={(e) => saveAppSetting("userAgentPref", e.currentTarget.value)}
                />
              </label>
              <p class="desc">Override the browser identity sent to services. Applies to newly opened services (restart to apply everywhere).</p>
            </div>
            <div class="set-title">Server</div>
            <code class="recipe">{server}</code>
            <p class="sub">Signed in as {me.email}. Sign out to change server.</p>
          {:else if settingsTab === "updates"}
            <div class="set-title">Version</div>
            <code class="recipe">Tauridium v{appVer}</code>
            {#if updateInfo}
              <p class="desc">A new version is available: <strong>v{updateInfo.version}</strong>.</p>
              <button class="primary" disabled={updInstalling} onclick={doInstall}>
                {updInstalling ? "Downloading & installing…" : `Update to v${updateInfo.version} & restart`}
              </button>
            {:else}
              <button class="primary" disabled={updChecking} onclick={() => checkUpdates(false)}>
                {updChecking ? "Checking…" : "Check for updates"}
              </button>
            {/if}
            {#if updStatus}<p class="desc">{updStatus}</p>{/if}
            <p class="sub">Updates are downloaded from GitHub Releases and verified with a signature.</p>
          {/if}

          {#if error}<p class="error">{error}</p>{/if}
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
      class:asleep={hibernated.has(s.id)}
      onclick={() => selectService(s)}
    >
      {#if failedIcons.has(s.id)}
        <span class="dot">{s.name.slice(0, 1)}</span>
      {:else}
        <img class="svc-icon" src={iconSrc(s)} alt="" onerror={() => markIconFailed(s.id)} />
      {/if}
      {#if appSettings.showServiceName}
        <span class="srow-name">{s.name}</span>
      {/if}
      {#if hibernated.has(s.id)}<span class="zzz" title="Hibernated">💤</span>{/if}
      {#if (unreadMap[s.id] ?? 0) > 0 && (s.isMuted !== true || appSettings.showMessageBadgeWhenMuted)}
        <span class="ubadge" class:muted={s.isMuted === true}>
          {unreadMap[s.id] > 99 ? "99+" : unreadMap[s.id]}
        </span>
      {/if}
    </button>
    <button class="cog" title="Settings" onclick={() => openServiceSettings(s)}>⚙</button>
  </div>
{/snippet}

{#snippet toggle(label: string, desc: string, key: keyof Service, checked: boolean)}
  <div class="setrow">
    <label class="row-toggle">
      <input
        type="checkbox"
        {checked}
        onchange={(e) => saveSetting(key, e.currentTarget.checked)}
      />
      <span>{label}</span>
    </label>
    <p class="desc">{desc}</p>
  </div>
{/snippet}

{#snippet appToggle(label: string, desc: string, key: keyof AppSettings, checked: boolean)}
  <div class="setrow">
    <label class="row-toggle">
      <input
        type="checkbox"
        {checked}
        onchange={(e) => saveAppSetting(key, e.currentTarget.checked)}
      />
      <span>{label}</span>
    </label>
    <p class="desc">{desc}</p>
  </div>
{/snippet}

<style>
  :global(:root) {
    --bg: #1f2230; --sidebar: #1b1d28; --card: #282b3a; --panel: #232633;
    --input: #1f2230; --border: #2f3445; --border2: #3a3f55;
    --text: #e8e8ef; --text2: #d6d9e6; --muted: #9aa0b5; --muted2: #6b7193;
    --hover: #262a3a; --accent: #ffc131; --accent-fg: #1f2230; --accent-soft: #b9b2ff; --link: #7a82a8;
  }
  :global(body.light) {
    --bg: #f3f4f8; --sidebar: #e9ebf1; --card: #ffffff; --panel: #ffffff;
    --input: #ffffff; --border: #d6dae6; --border2: #c8cddc;
    --text: #1c2030; --text2: #2a2f40; --muted: #5b6280; --muted2: #818aa6;
    --hover: #e4e7f0; --accent-soft: #5b52d6; --link: #6d75a0;
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
    color: var(--accent-fg); font-weight: 700; cursor: pointer;
  }
  .primary:disabled { opacity: 0.6; cursor: default; }
  .gear { align-self: flex-start; background: none; border: none; color: var(--muted); cursor: pointer; font-size: 12px; padding: 0; }
  .error { color: #ff8a8a; font-size: 13px; margin: 4px 0; }

  .shell { display: grid; grid-template-columns: var(--sidebar-w, 240px) 1fr; height: 100vh; }
  .sidebar {
    background: var(--sidebar); padding: 12px; overflow: hidden;
    display: flex; flex-direction: column; gap: 12px;
  }
  .svcarea { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow-y: auto; }
  :global(body[data-svcloc="center"]) .svcarea { justify-content: center; }
  :global(body[data-svcloc="bottom"]) .svcarea { justify-content: flex-end; }
  .account { display: flex; justify-content: space-between; align-items: center; font-size: 13px; }
  .link { background: none; border: none; color: var(--link); cursor: pointer; font-size: 12px; text-decoration: underline; }
  .add {
    background: var(--hover); border: 1px dashed var(--border2); color: var(--accent-soft);
    border-radius: 8px; padding: 8px; cursor: pointer; font-size: 13px;
  }
  .add:hover { filter: brightness(1.1); }
  .wspills { display: flex; flex-wrap: wrap; gap: 5px; }
  .pill {
    background: var(--hover); border: none; color: var(--muted);
    border-radius: 999px; padding: 3px 10px; cursor: pointer; font-size: 12px;
  }
  .pill.on { background: var(--accent); color: var(--accent-fg); }
  .pill.mng { background: transparent; border: 1px dashed var(--border2); color: var(--muted); font-size: 15px; line-height: 1; padding: 2px 9px; }
  .svclist { display: flex; flex-direction: column; gap: 2px; }

  .srow-wrap { display: flex; align-items: center; }
  .srow {
    display: flex; align-items: center; gap: 9px; flex: 1; min-width: 0;
    padding: 7px 8px; border: none; border-radius: 8px; background: none;
    color: var(--text2); cursor: pointer; text-align: left; font-size: 14px;
  }
  .srow:hover { background: var(--hover); }
  .srow.active { background: var(--accent); color: var(--accent-fg); }
  .srow.disabled { opacity: 0.45; }
  .srow.asleep .svc-icon, .srow.asleep .dot { filter: grayscale(1); opacity: 0.5; }
  .zzz { margin-left: auto; font-size: 12px; opacity: 0.8; }
  .svc-icon, .srow .dot { width: var(--icon-size, 22px); height: var(--icon-size, 22px); border-radius: 5px; object-fit: cover; flex: none; }
  .srow .dot { display: grid; place-items: center; background: var(--border2); font-size: 12px; font-weight: 700; }
  :global(body.grayscale) .svc-icon { filter: grayscale(1); opacity: var(--gray-op, 0.6); transition: filter 0.15s, opacity 0.15s; }
  :global(body.grayscale) .srow:hover .svc-icon,
  :global(body.grayscale) .srow.active .svc-icon { filter: none; opacity: 1; }
  .srow-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ubadge {
    margin-left: auto; background: #e23b3b; color: #fff; font-size: 11px; font-weight: 700;
    min-width: 18px; height: 18px; padding: 0 5px; border-radius: 9px; flex: none;
    display: inline-flex; align-items: center; justify-content: center;
  }
  .ubadge.muted { background: var(--muted2); }
  .cog { background: none; border: none; color: var(--muted2); cursor: pointer; font-size: 21px; line-height: 1; opacity: 0; padding: 2px 4px; }
  .srow-wrap:hover .cog { opacity: 1; }
  .cog:hover { color: var(--accent-soft); }
  .appcog {
    background: var(--hover); border: 1px solid var(--border);
    color: var(--text2); border-radius: 8px; padding: 9px; cursor: pointer; font-size: 13px;
    display: inline-flex; align-items: center; justify-content: center; gap: 7px;
  }
  .appcog .ic { font-size: 19px; line-height: 1; }
  .upddot { width: 8px; height: 8px; border-radius: 999px; background: #22c55e; display: inline-block; margin-left: 2px; }
  .appcog:hover { filter: brightness(1.1); }
  .count { font-size: 11px; color: var(--muted2); }
  .ver { font-weight: 700; color: var(--muted); }

  .stage { display: grid; place-items: center; overflow: auto; }
  .placeholder { text-align: center; color: var(--muted); }
  .panel {
    width: min(560px, 90%); align-self: start; margin: 40px auto;
    background: var(--panel); border: 1px solid var(--border); border-radius: 14px; padding: 22px;
    display: flex; flex-direction: column; gap: 14px;
  }
  .panel-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .head-actions { display: inline-flex; align-items: center; gap: 12px; }
  .primary.sm { padding: 6px 12px; font-size: 13px; }
  .panel-head h2 { margin: 0; font-size: 18px; }
  .recipe { color: var(--accent-soft); font-size: 12px; }
  .tabs { display: flex; gap: 4px; flex-wrap: wrap; border-bottom: 1px solid var(--border); padding-bottom: 8px; }
  .tab { background: none; border: none; color: var(--muted); cursor: pointer; font-size: 13px; padding: 5px 10px; border-radius: 8px; }
  .tab.on { background: var(--hover); color: var(--text); }
  .set-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted2); }
  .row-toggle { flex-direction: row; align-items: center; gap: 10px; color: var(--text2); font-size: 14px; cursor: pointer; }
  .row-toggle input { width: auto; }
  .setrow { display: flex; flex-direction: column; gap: 3px; }
  .desc { margin: 0 0 0 26px; color: var(--muted); font-size: 12px; line-height: 1.35; }
  .notice {
    margin: 0; padding: 9px 11px; border-radius: 8px; font-size: 12px; line-height: 1.4;
    background: rgba(217, 119, 6, 0.12); border: 1px solid rgba(217, 119, 6, 0.35); color: var(--text2);
  }
  .select {
    margin-left: auto; padding: 6px 9px; border-radius: 8px;
    border: 1px solid var(--border2); background: var(--input); color: var(--text); font-size: 13px;
  }
  .range { margin-left: auto; width: 130px; accent-color: var(--accent); }
  .swatches { margin-left: auto; display: inline-flex; gap: 7px; }
  .swatch {
    width: 22px; height: 22px; border-radius: 999px; border: none; padding: 0; cursor: pointer;
  }
  .swatch.on { outline: 2px solid var(--text); outline-offset: 2px; }
  .searchrow { display: flex; gap: 8px; }
  .searchrow input { flex: 1; }
  .wsedit {
    display: flex; flex-direction: column; gap: 8px; padding: 12px;
    border: 1px solid var(--border); border-radius: 10px; background: var(--input);
  }
  .wsedit-head { display: flex; gap: 10px; align-items: center; }
  .wsname { flex: 1; }
  .wsservices { display: flex; flex-direction: column; gap: 4px; max-height: 30vh; overflow-y: auto; }
  .block { gap: 6px; }
  .num-row { display: flex; gap: 12px; }
  .num-row label { flex: 1; flex-direction: column; gap: 4px; font-size: 12px; color: var(--muted); }
  .num { padding: 6px 8px; border-radius: 8px; border: 1px solid var(--border2); background: var(--input); color: var(--text); font-size: 13px; }
  .proxy-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
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
