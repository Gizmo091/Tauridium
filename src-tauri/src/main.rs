#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// pakeFerdium — client Ferdium léger (Tauri v2).
//
// Phase 1 : connexion au serveur Ferdium (login JWT, services, workspaces).
// Phase 2 : rendu de chaque service dans une webview enfant ISOLÉE, posée en
//           overlay sur la zone à droite de la sidebar. Isolation via
//           `data_store_identifier` = les 16 octets de l'UUID du service
//           (data_directory est ignoré par WKWebView sur macOS — cf. Phase 0).

use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::WebviewBuilder;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, RunEvent, State, Url, WebviewUrl, WindowEvent,
    Wry,
};
use tauri_plugin_notification::{NotificationExt, PermissionState};

// User-agent pour les appels API serveur Ferdium / récupération des recipes.
const API_UA: &str = concat!("pakeFerdium/", env!("CARGO_PKG_VERSION"));
// Largeur (logique) de la sidebar — doit coïncider avec le CSS du shell.
const SIDEBAR_W: f64 = 240.0;
// UA Safari moderne AVEC le token `Version/` : WhatsApp exige Safari >= 15, or la webview
// WKWebView native n'expose pas toujours ce token (-> "navigateur non supporté"). On reste
// du Safari (pas Chrome) pour ne pas casser les services qui dépendent du chemin Safari
// (ex. Synology Chat, cassé par un UA Chrome). L'override par recipe viendra plus tard.
const SERVICE_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15";

#[derive(Default)]
struct AppState {
    server: Mutex<Option<String>>,
    token: Mutex<Option<String>>,
    created: Mutex<HashSet<String>>, // serviceId des webviews déjà créées
    active: Mutex<Option<String>>,   // serviceId actuellement affiché
    unread: Mutex<HashMap<String, i64>>, // non-lus par service (pour le badge dock)
    flags: Mutex<HashMap<String, ServiceFlags>>, // réglages par service (notif/mute/badge)
}

#[derive(Clone, Copy)]
struct ServiceFlags {
    notif: bool,
    muted: bool,
    badge: bool,
}

impl Default for ServiceFlags {
    fn default() -> Self {
        // Par défaut : notifications + badge activés, non muté (comme Ferdium).
        ServiceFlags {
            notif: true,
            muted: false,
            badge: true,
        }
    }
}

// --- Auth ---------------------------------------------------------------

// Le client Ferdium transmet base64(sha256(password)) (cf. ferdium-app UserApi.ts).
fn ferdium_password_hash(password: &str) -> String {
    let digest = Sha256::digest(password.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn normalize_server(server: &str) -> String {
    server.trim().trim_end_matches('/').to_string()
}

async fn api_get(base: &str, token: &str, path: &str) -> Result<Value, String> {
    let res = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .bearer_auth(token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .map_err(|e| format!("Requête {path} échouée : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("{path} : HTTP {}", res.status()));
    }
    res.json()
        .await
        .map_err(|e| format!("Réponse {path} illisible : {e}"))
}

// Persistance de session : {serveur, token} dans app_data_dir/session.json (perms 600).
// Durcissement Keychain macOS prévu pour le build signé (Phase 4).
fn save_session(app: &AppHandle, server: &str, token: &str) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    let data = serde_json::json!({ "server": server, "token": token }).to_string();
    if std::fs::write(&path, data).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

fn clear_session(app: &AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_file(dir.join("session.json"));
    }
}

#[tauri::command]
async fn login(
    app: AppHandle,
    state: State<'_, AppState>,
    server: String,
    email: String,
    password: String,
) -> Result<Value, String> {
    let base = normalize_server(&server);
    let password_hash = ferdium_password_hash(&password);

    let res = reqwest::Client::new()
        .post(format!("{base}/v1/auth/login"))
        .basic_auth(&email, Some(&password_hash))
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .map_err(|e| format!("Connexion au serveur impossible : {e}"))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Identifiants refusés (HTTP {status}). {body}"));
    }

    let body: Value = res
        .json()
        .await
        .map_err(|e| format!("Réponse de login illisible : {e}"))?;
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .ok_or("Réponse de login sans token")?
        .to_string();

    {
        *state.server.lock().unwrap() = Some(base.clone());
        *state.token.lock().unwrap() = Some(token.clone());
    }
    save_session(&app, &base, &token);
    api_get(&base, &token, "/v1/me").await
}

// Restaure une session enregistrée au démarrage : valide le token via /v1/me.
#[tauri::command]
async fn restore_session(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let path = dir.join("session.json");
    let text = std::fs::read_to_string(&path).map_err(|_| "Aucune session".to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let server = v
        .get("server")
        .and_then(Value::as_str)
        .ok_or("Session invalide")?
        .to_string();
    let token = v
        .get("token")
        .and_then(Value::as_str)
        .ok_or("Session invalide")?
        .to_string();

    match api_get(&server, &token, "/v1/me").await {
        Ok(me) => {
            *state.server.lock().unwrap() = Some(server);
            *state.token.lock().unwrap() = Some(token);
            Ok(me)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(format!("Session expirée : {e}"))
        }
    }
}

fn current(state: &State<'_, AppState>) -> Result<(String, String), String> {
    let base = state.server.lock().unwrap().clone();
    let token = state.token.lock().unwrap().clone();
    match (base, token) {
        (Some(b), Some(t)) => Ok((b, t)),
        _ => Err("Non connecté".into()),
    }
}

#[tauri::command]
async fn get_services(state: State<'_, AppState>) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    api_get(&base, &token, "/v1/me/services").await
}

#[tauri::command]
async fn get_workspaces(state: State<'_, AppState>) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    api_get(&base, &token, "/v1/workspace").await
}

// --- Recipes & résolution d'URL ----------------------------------------

// Récupère le package.json du recipe (cache disque), depuis le repo ferdium-recipes.
async fn recipe_config(app_data: &Path, recipe_id: &str) -> Result<Value, String> {
    if recipe_id.is_empty()
        || !recipe_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("Identifiant de recipe invalide : {recipe_id}"));
    }

    let dir = app_data.join("recipes");
    let _ = std::fs::create_dir_all(&dir);
    let cache = dir.join(format!("{recipe_id}.json"));

    if let Ok(text) = std::fs::read_to_string(&cache) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            return Ok(v);
        }
    }

    let url = format!(
        "https://raw.githubusercontent.com/ferdium/ferdium-recipes/main/recipes/{recipe_id}/package.json"
    );
    let res = reqwest::Client::new()
        .get(&url)
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .map_err(|e| format!("Téléchargement du recipe {recipe_id} échoué : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Recipe {recipe_id} introuvable (HTTP {})", res.status()));
    }
    let text = res
        .text()
        .await
        .map_err(|e| format!("Recipe {recipe_id} illisible : {e}"))?;
    let _ = std::fs::write(&cache, &text);
    serde_json::from_str(&text).map_err(|e| format!("package.json {recipe_id} invalide : {e}"))
}

fn ensure_scheme(u: &str) -> String {
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("https://{u}")
    }
}

// Réplique le getter `url` du modèle Service de ferdium-app.
fn resolve_url(cfg: &Value, custom_url: Option<&str>, team: Option<&str>) -> Result<String, String> {
    let config = cfg.get("config").ok_or("Recipe sans bloc config")?;
    let service_url = config.get("serviceURL").and_then(Value::as_str).unwrap_or("");
    let has_custom = config
        .get("hasCustomUrl")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_team = config
        .get("hasTeamId")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if has_custom {
        if let Some(u) = custom_url.filter(|u| !u.is_empty()) {
            return Ok(ensure_scheme(u));
        }
    }
    if has_team {
        if let Some(t) = team.filter(|t| !t.is_empty()) {
            return Ok(service_url.replace("{teamId}", t));
        }
    }
    if service_url.is_empty() {
        return Err("Recipe sans serviceURL".into());
    }
    Ok(service_url.to_string())
}

// --- Runtime des recipes (webview.js -> non-lus) ------------------------

// Shim minimal de l'API `Ferdium` exposée aux webview.js des recipes. Exécute le
// code de scraping DOM du recipe ; `setBadge` alimente window.__pakeUnread (lu par le
// poller). injectCSS/notifs viendront plus tard ; tout est encapsulé dans un try/catch
// pour qu'un recipe incompatible ne casse jamais la page.
const RECIPE_PREAMBLE: &str = r#"(function(){
  try {
    var module = { exports: {} };
    var exports = module.exports;
    var __dirname = '';
    function require(m){ if(m==='path') return { join: function(){ return Array.prototype.slice.call(arguments).join('/'); } }; return {}; }
    var Ferdium = {
      setBadge: function(direct, indirect){ var d=parseInt(direct,10)||0, i=parseInt(indirect,10)||0; window.__pakeUnread = Math.max(0, d+i); },
      safeParseInt: function(v){ var n=parseInt(v,10); return isNaN(n)?0:n; },
      injectCSS: function(){}, injectJSUnsafe: function(){},
      setDialogTitle: function(){}, handleDarkMode: function(){},
      loop: function(cb){ try{cb();}catch(e){} setInterval(function(){ try{cb();}catch(e){} }, 1000); }
    };
    window.Ferdium = Ferdium;
"#;
const RECIPE_SUFFIX: &str = r#"
    if (typeof module.exports === 'function') module.exports(Ferdium, {});
  } catch(e){ console.warn('[pakeFerdium] recipe runtime error', e); }
})();"#;

// Récupère (et cache) le webview.js d'un recipe ; None s'il n'en a pas.
async fn recipe_webview_js(app_data: &Path, recipe_id: &str) -> Option<String> {
    if recipe_id.is_empty()
        || !recipe_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let dir = app_data.join("recipes");
    let _ = std::fs::create_dir_all(&dir);
    let cache = dir.join(format!("{recipe_id}.webview.js"));
    if let Ok(s) = std::fs::read_to_string(&cache) {
        return Some(s);
    }
    let url = format!(
        "https://raw.githubusercontent.com/ferdium/ferdium-recipes/main/recipes/{recipe_id}/webview.js"
    );
    let res = reqwest::Client::new()
        .get(&url)
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let text = res.text().await.ok()?;
    let _ = std::fs::write(&cache, &text);
    Some(text)
}

// --- Webviews de services ----------------------------------------------

// Les 16 octets de l'UUID du service -> identifiant de data store WKWebView.
fn uuid_to_bytes(s: &str) -> Option<[u8; 16]> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

// Rectangle (logique) de la zone service = tout à droite de la sidebar.
fn service_rect(
    win: &tauri::window::Window<Wry>,
) -> Result<(LogicalPosition<f64>, LogicalSize<f64>), String> {
    let phys = win.inner_size().map_err(|e| e.to_string())?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let w = phys.width as f64 / scale;
    let h = phys.height as f64 / scale;
    Ok((
        LogicalPosition::new(SIDEBAR_W, 0.0),
        LogicalSize::new((w - SIDEBAR_W).max(0.0), h),
    ))
}

// Certains services (ex. Synology Chat) lisent le global `ipc` (= window.ipc) et
// appellent l'API IPC d'Electron (ipc.on / ipc.sendToHost). Or `window.ipc` est défini
// par wry pour SON propre IPC ({postMessage}) — sans .on/.sendToHost → ces services
// crashent. wry posait window.ipc gelé/non-configurable : on a patché wry
// (vendor/wry) pour le rendre mutable, et on l'AUGMENTE ici (ajout des méthodes
// Electron en no-op, sans toucher postMessage). Le pont complet (sendToHost routé vers
// le natif pour badges/notifs + recipe webview.js) viendra en Phase 3.
const IPC_SHIM_JS: &str = r#"(function(){
  window.__PAKE_SHIM__ = (window.__PAKE_SHIM__ || 0) + 1;
  window.__pakeUnread = window.__pakeUnread || 0;
  // NB : Telegram Web A est une app Tauri qui appelle l'IPC bas niveau (fetch ipc:// +
  // postMessage) ; impossible à neutraliser proprement depuis du JS injecté. Ses erreurs
  // console sont du bruit non bloquant (Telegram fonctionne). Badge Telegram non géré.
  // Intercepte l'API web Notification (WKWebView ne la gère pas vraiment) : on empile
  // les notifs des services, drainées par le poller Rust -> notif système native.
  (function(){
    window.__pakeNotifQueue = window.__pakeNotifQueue || [];
    function N(title, options){
      options = options || {};
      try { window.__pakeNotifQueue.push({
        title: String(title == null ? '' : title),
        body: String(options.body == null ? '' : options.body)
      }); } catch(e){}
      this.title = title; this.body = options.body;
      this.onclick = this.onclose = this.onerror = this.onshow = null;
    }
    N.prototype.close = function(){};
    N.prototype.addEventListener = function(t, cb){ this['on' + t] = cb; };
    N.prototype.removeEventListener = function(){};
    N.prototype.dispatchEvent = function(){ return true; };
    N.permission = 'granted';
    N.requestPermission = function(cb){ if (typeof cb === 'function') cb('granted'); return Promise.resolve('granted'); };
    try { window.Notification = N; } catch(e){}
  })();
  var noop = function(){};
  // Capte les non-lus que les services Electron-aware émettent via sendToHost.
  function captureUnread(channel){
    try {
      if (channel === 'updateUnread' || channel === 'message-counts' || channel === 'updateBadge') {
        var v = arguments[1];
        var n = (typeof v === 'number') ? v
              : (v && typeof v.count === 'number') ? v.count
              : parseInt(v, 10);
        if (!isNaN(n)) window.__pakeUnread = Math.max(0, n);
      }
    } catch(e){}
  }
  var extra = {
    on: noop, once: noop, off: noop, addListener: noop,
    removeListener: noop, removeAllListeners: noop,
    send: noop, sendToHost: captureUnread,
    sendSync: function(){ return null; },
    invoke: function(){ return Promise.resolve(); }
  };
  function augment(v){
    if (!v || typeof v.on === 'function') return v;
    // 1) tente de muter l'objet en place
    try { for (var k in extra) if (typeof v[k] !== 'function') v[k] = extra[k]; } catch(e){}
    if (typeof v.on === 'function') return v;
    // 2) objet verrouillé : on reconstruit en préservant postMessage (IPC de Tauri)
    var out = {};
    try { for (var k in v) out[k] = v[k]; } catch(e){}
    try { if (v.postMessage) out.postMessage = v.postMessage.bind(v); } catch(e){}
    for (var k in extra) if (typeof out[k] !== 'function') out[k] = extra[k];
    return out;
  }
  var real = augment(window.ipc);
  try {
    // intercepte l'affectation de Tauri : on augmente pile quand il pose window.ipc
    Object.defineProperty(window, 'ipc', {
      configurable: true,
      get: function(){ return real; },
      set: function(v){ real = augment(v); }
    });
  } catch(e){
    // repli : polling
    var n = 0, iv = setInterval(function(){
      window.ipc = augment(window.ipc);
      if (typeof (window.ipc||{}).on === 'function' || ++n > 400) clearInterval(iv);
    }, 25);
  }
})();"#;

#[tauri::command]
async fn show_service(
    app: AppHandle,
    state: State<'_, AppState>,
    service_id: String,
    recipe_id: String,
    custom_url: Option<String>,
    team: Option<String>,
) -> Result<(), String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let cfg = recipe_config(&app_data, &recipe_id).await?;
    let url_str = resolve_url(&cfg, custom_url.as_deref(), team.as_deref())?;
    let url = Url::parse(&url_str).map_err(|e| format!("URL invalide « {url_str} » : {e}"))?;

    let win = app
        .get_window("main")
        .ok_or("Fenêtre principale introuvable")?;
    let label = format!("svc-{service_id}");
    let (pos, size) = service_rect(&win)?;

    let exists = state.created.lock().unwrap().contains(&service_id);
    if exists {
        if let Some(wv) = app.get_webview(&label) {
            let _ = wv.set_position(pos);
            let _ = wv.set_size(size);
        }
    } else {
        let store = uuid_to_bytes(&service_id).ok_or("serviceId n'est pas un UUID")?;
        // Runtime du recipe (scraping DOM des non-lus -> __pakeUnread) — best effort.
        let runtime = recipe_webview_js(&app_data, &recipe_id)
            .await
            .map(|js| format!("{RECIPE_PREAMBLE}{js}{RECIPE_SUFFIX}"));
        // UA Safari moderne (cf. SERVICE_UA) : satisfait WhatsApp (Safari >= 15) sans
        // casser Synology (reste du Safari, pas du Chrome).
        //
        // Shim IPC injecté pour TOUS les services (comme Ferdium expose ipcRenderer à
        // toutes ses webviews) : ceux qui en dépendent (Synology Chat…) ne crashent plus,
        // les autres l'ignorent.
        let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
            .data_store_identifier(store)
            .user_agent(SERVICE_UA)
            .initialization_script(IPC_SHIM_JS);
        if let Some(rt) = runtime {
            builder = builder.initialization_script(rt);
        }
        win.add_child(builder, pos, size)
            .map_err(|e| format!("Création de la webview du service échouée : {e}"))?;
        state.created.lock().unwrap().insert(service_id.clone());
    }

    // Affiche le service demandé, masque les autres.
    let created: Vec<String> = state.created.lock().unwrap().iter().cloned().collect();
    for sid in created {
        if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
            if sid == service_id {
                let _ = wv.show();
            } else {
                let _ = wv.hide();
            }
        }
    }
    *state.active.lock().unwrap() = Some(service_id);
    Ok(())
}

// Ouvre les devtools sur la webview du service actif (debug uniquement) — diagnostic.
#[tauri::command]
fn inspect_service(app: AppHandle, state: State<'_, AppState>) {
    #[cfg(debug_assertions)]
    {
        let active = state.active.lock().unwrap().clone();
        if let Some(sid) = active {
            if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
                wv.open_devtools();
            }
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, state);
    }
}

// Masque toutes les webviews de services (pour afficher un panneau plein écran du shell).
#[tauri::command]
fn hide_all_services(app: AppHandle, state: State<'_, AppState>) {
    let created: Vec<String> = state.created.lock().unwrap().iter().cloned().collect();
    for sid in created {
        if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
            let _ = wv.hide();
        }
    }
    *state.active.lock().unwrap() = None;
}

#[tauri::command]
fn close_services(app: AppHandle, state: State<'_, AppState>) {
    let created: Vec<String> = state.created.lock().unwrap().drain().collect();
    for sid in created {
        if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
            let _ = wv.close();
        }
    }
    *state.active.lock().unwrap() = None;
    state.unread.lock().unwrap().clear();
    if let Some(win) = app.get_window("main") {
        let _ = win.set_badge_count(None);
    }
}

// Enregistre les réglages d'un service (notif/mute/badge) respectés par le poller.
#[tauri::command]
fn set_service_flags(
    state: State<'_, AppState>,
    service_id: String,
    notif: bool,
    muted: bool,
    badge: bool,
) {
    state
        .flags
        .lock()
        .unwrap()
        .insert(service_id, ServiceFlags { notif, muted, badge });
}

// Modifie un service (réglages) -> PUT /v1/service/:id. `patch` doit inclure `name`.
#[tauri::command]
async fn update_service(
    state: State<'_, AppState>,
    service_id: String,
    patch: Value,
) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .put(format!("{base}/v1/service/{service_id}"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .json(&patch)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Modification du service : HTTP {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

// Crée un service -> POST /v1/service.
#[tauri::command]
async fn create_service(
    state: State<'_, AppState>,
    name: String,
    recipe_id: String,
) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .post(format!("{base}/v1/service"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .json(&serde_json::json!({ "name": name, "recipeId": recipe_id }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Création du service : HTTP {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

// Supprime un service -> DELETE /v1/service/:id (+ ferme la webview).
#[tauri::command]
async fn delete_service(
    app: AppHandle,
    state: State<'_, AppState>,
    service_id: String,
) -> Result<(), String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .delete(format!("{base}/v1/service/{service_id}"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Suppression du service : HTTP {}", res.status()));
    }
    if let Some(wv) = app.get_webview(&format!("svc-{service_id}")) {
        let _ = wv.close();
    }
    state.created.lock().unwrap().remove(&service_id);
    state.unread.lock().unwrap().remove(&service_id);
    state.flags.lock().unwrap().remove(&service_id);
    Ok(())
}

// Catalogue complet de recipes -> GET /v1/recipes. Le filtre se fait côté frontend.
// (Le serveur officiel ne sert pas /recipes/search — il renvoie [].)
#[tauri::command]
async fn list_recipes(state: State<'_, AppState>) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .get(format!("{base}/v1/recipes"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Catalogue de recipes : HTTP {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn logout(app: AppHandle, state: State<'_, AppState>) {
    *state.server.lock().unwrap() = None;
    *state.token.lock().unwrap() = None;
    clear_session(&app);
}

// Repositionne la webview active sur la zone service (appelé au resize).
fn reposition_active(app: &AppHandle) {
    let active = app.state::<AppState>().active.lock().unwrap().clone();
    let Some(sid) = active else { return };
    let Some(win) = app.get_window("main") else { return };
    if let Ok((pos, size)) = service_rect(&win) {
        if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
            let _ = wv.set_position(pos);
            let _ = wv.set_size(size);
        }
    }
}

// Poller du badge dock : lit window.__pakeUnread de chaque webview de service,
// agrège les non-lus et met à jour le badge dock macOS (set_badge_count).
fn start_badge_poller(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let services: Vec<String> = app
            .state::<AppState>()
            .created
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        for sid in services {
            let Some(wv) = app.get_webview(&format!("svc-{sid}")) else {
                continue;
            };
            let app2 = app.clone();
            let _ = wv.eval_with_callback(
                "(function(){return {u: Math.max(0, parseInt(window.__pakeUnread)||0), n: (window.__pakeNotifQueue||[]).splice(0)};})()",
                move |res| {
                    let v: serde_json::Value =
                        serde_json::from_str(&res).unwrap_or(serde_json::Value::Null);
                    let unread = v.get("u").and_then(|x| x.as_i64()).unwrap_or(0);
                    let flags = app2
                        .state::<AppState>()
                        .flags
                        .lock()
                        .unwrap()
                        .get(&sid)
                        .copied()
                        .unwrap_or_default();

                    // Notifications : on respecte mute / notifications désactivées.
                    if flags.notif && !flags.muted {
                        if let Some(arr) = v.get("n").and_then(|x| x.as_array()) {
                            for notif in arr {
                                let title =
                                    notif.get("title").and_then(|x| x.as_str()).unwrap_or("").trim();
                                let body = notif.get("body").and_then(|x| x.as_str()).unwrap_or("");
                                if title.is_empty() && body.is_empty() {
                                    continue;
                                }
                                let _ = app2
                                    .notification()
                                    .builder()
                                    .title(if title.is_empty() { "Message" } else { title })
                                    .body(body)
                                    .show();
                            }
                        }
                    }

                    // Badge : contribution nulle si badge désactivé ou service muté.
                    let contribution = if flags.badge && !flags.muted { unread } else { 0 };
                    let total = {
                        let st = app2.state::<AppState>();
                        let mut m = st.unread.lock().unwrap();
                        m.insert(sid.clone(), contribution);
                        m.values().sum::<i64>()
                    };
                    if let Some(win) = app2.get_window("main") {
                        let _ = win.set_badge_count(if total > 0 { Some(total) } else { None });
                    }
                },
            );
        }
    });
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle_main(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            if let Some(win) = app.get_window("main") {
                win.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) => reposition_active(&handle),
                    WindowEvent::CloseRequested { api, .. } => {
                        // close-to-tray : on cache la fenêtre au lieu de quitter l'app.
                        api.prevent_close();
                        if let Some(w) = handle.get_window("main") {
                            let _ = w.hide();
                        }
                    }
                    _ => {}
                });
            }

            // Icône menubar (tray) : afficher / quitter ; clic gauche = toggle fenêtre.
            let show = MenuItem::with_id(app, "show", "Afficher Tauridium", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Tauridium")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            // Demande l'autorisation de notifier au lancement.
            // NB : no-op sur macOS desktop (l'OS gère l'autorisation lui-même) ; réel
            // sur mobile / Windows / build .app signée.
            if let Ok(state) = app.notification().permission_state() {
                if state != PermissionState::Granted {
                    let _ = app.notification().request_permission();
                }
            }
            start_badge_poller(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            login,
            restore_session,
            get_services,
            get_workspaces,
            show_service,
            inspect_service,
            hide_all_services,
            close_services,
            logout,
            set_service_flags,
            update_service,
            create_service,
            delete_service,
            list_recipes
        ])
        .build(tauri::generate_context!())
        .expect("erreur au lancement de l'application tauri")
        .run(|app, event| {
            // Clic sur l'icône du dock (macOS) -> réafficher la fenêtre.
            if let RunEvent::Reopen { .. } = event {
                show_main(app);
            }
        });
}
