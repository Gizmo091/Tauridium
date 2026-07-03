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
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::WebviewBuilder;
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, WebviewUrl, WindowEvent,
    Wry,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::{NotificationExt, PermissionState};

// User-agent pour les appels API serveur Ferdium / récupération des recipes.
const API_UA: &str = concat!("Tauridium/", env!("CARGO_PKG_VERSION"));
// Largeur (logique) de la sidebar — doit coïncider avec le CSS du shell.
const SIDEBAR_W: f64 = 240.0;
// UA Safari moderne AVEC le token `Version/` : WhatsApp exige Safari >= 15, or la webview
// WKWebView native n'expose pas toujours ce token (-> "navigateur non supporté"). On reste
// du Safari (pas Chrome) pour ne pas casser les services qui dépendent du chemin Safari
// (ex. Synology Chat, cassé par un UA Chrome). L'override par recipe viendra plus tard.
const SERVICE_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15";

// UA « Chrome sans numéro de version » pour les hôtes Google sensibles (login, Gmail,
// Google Chat) : contourne le « navigateur non supporté » de Google (repris de ferx).
const GOOGLE_CHROMELESS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome Safari/537.36";

// Compat Google (spoof userAgentData / window.chrome / plugins / vendor…) injectée sur les
// services Google non sensibles. Défensif (try/catch partout). Repris de ferx.
const GOOGLE_AUTH_COMPAT_JS: &str = r#"(function() {
  document.addEventListener('securitypolicyviolation', function(e) {
    if (e.blockedURI && (e.blockedURI.indexOf('ipc:') !== -1 || e.blockedURI.indexOf('tauri:') !== -1)) { e.stopImmediatePropagation(); }
  }, true);
  try { Object.defineProperty(navigator, 'vendor', { get: function() { return 'Google Inc.'; }, configurable: true }); } catch(_) {}
  try { Object.defineProperty(navigator, 'webdriver', { get: function() { return false; }, configurable: true }); } catch(_) {}
  try { Object.defineProperty(navigator, 'pdfViewerEnabled', { get: function() { return true; }, configurable: true }); } catch(_) {}
  var pluginNames = ['PDF Viewer','Chrome PDF Viewer','Chromium PDF Viewer','Microsoft Edge PDF Viewer','WebKit built-in PDF'];
  var fakePlugins = { length: pluginNames.length, item: function(i) { return this[i] || null; }, namedItem: function(n) { for (var i = 0; i < this.length; i++) { if (this[i] && this[i].name === n) return this[i]; } return null; }, refresh: function() {} };
  for (var i = 0; i < pluginNames.length; i++) fakePlugins[i] = Object.freeze({ name: pluginNames[i], filename: 'internal-pdf-viewer', description: 'Portable Document Format', length: 1 });
  try { Object.defineProperty(navigator, 'plugins', { get: function() { return fakePlugins; }, configurable: true }); } catch(_) {}
  if (!window.chrome) {
    try { Object.defineProperty(window, 'chrome', { value: { app: { isInstalled: false }, runtime: {}, csi: function(){return {};}, loadTimes: function(){return {};} }, writable: true, configurable: true }); } catch(_) {}
  }
  if (!navigator.userAgentData) {
    var isMac = !(navigator.platform && navigator.platform.startsWith('Win'));
    var brands = Object.freeze([ Object.freeze({ brand: 'Google Chrome', version: '135' }), Object.freeze({ brand: 'Not-A.Brand', version: '8' }), Object.freeze({ brand: 'Chromium', version: '135' }) ]);
    try { Object.defineProperty(navigator, 'userAgentData', { value: Object.freeze({ brands: brands, mobile: false, platform: isMac ? 'macOS' : 'Windows', getHighEntropyValues: function() { return Promise.resolve({ brands: brands, mobile: false, platform: isMac ? 'macOS' : 'Windows', platformVersion: isMac ? '15.0.0' : '10.0.0', architecture: isMac ? 'arm' : 'x86', model: '', uaFullVersion: '135.0.0.0', fullVersionList: [{ brand: 'Google Chrome', version: '135.0.0.0' }, { brand: 'Chromium', version: '135.0.0.0' }] }); }, toJSON: function() { return { brands: brands, mobile: false, platform: isMac ? 'macOS' : 'Windows' }; } }), configurable: true, enumerable: true }); } catch(_) {}
  }
})();"#;

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

// Hôtes Google sensibles (login/Gmail/Chat) -> UA chromeless.
fn is_google_auth_host(host: &str) -> bool {
    [
        "gmail.com",
        "googlemail.com",
        "mail.google.com",
        "chat.google.com",
        "accounts.google.com",
    ]
    .iter()
    .any(|d| host_matches(host, d))
}

// Services Google génériques -> injection du script de compat.
fn is_google_host(host: &str) -> bool {
    ["google.com", "gmail.com", "youtube.com", "googlevideo.com"]
        .iter()
        .any(|d| host_matches(host, d))
}

#[derive(Default)]
struct AppState {
    server: Mutex<Option<String>>,
    token: Mutex<Option<String>>,
    created: Mutex<HashSet<String>>, // serviceId des webviews déjà créées
    active: Mutex<Option<String>>,   // serviceId actuellement affiché
    unread: Mutex<HashMap<String, i64>>, // non-lus par service (pour le badge dock)
    flags: Mutex<HashMap<String, ServiceFlags>>, // réglages par service (notif/mute/badge)
    settings: Mutex<Value>,          // cache des réglages app (lu par le poller, etc.)
    sidebar_w: Mutex<f64>,           // largeur de la sidebar (init en setup, def. 240)
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
        return Err(format!(
            "Recipe {recipe_id} introuvable (HTTP {})",
            res.status()
        ));
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
fn resolve_url(
    cfg: &Value,
    custom_url: Option<&str>,
    team: Option<&str>,
) -> Result<String, String> {
    let config = cfg.get("config").ok_or("Recipe sans bloc config")?;
    let service_url = config
        .get("serviceURL")
        .and_then(Value::as_str)
        .unwrap_or("");
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
    sidebar_w: f64,
) -> Result<(LogicalPosition<f64>, LogicalSize<f64>), String> {
    let phys = win.inner_size().map_err(|e| e.to_string())?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let w = phys.width as f64 / scale;
    let h = phys.height as f64 / scale;
    Ok((
        LogicalPosition::new(sidebar_w, 0.0),
        LogicalSize::new((w - sidebar_w).max(0.0), h),
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
// Crée (si absente) la webview d'un service à la position/taille données. Les fetchs de
// recette (config + webview.js) ne sont faits QU'ICI (création) — plus à chaque bascule.
#[allow(clippy::too_many_arguments)]
async fn create_service_webview(
    state: &State<'_, AppState>,
    win: &tauri::window::Window<Wry>,
    app_data: &Path,
    service_id: &str,
    recipe_id: &str,
    custom_url: Option<&str>,
    team: Option<&str>,
    user_agent_pref: Option<&str>,
    pos: LogicalPosition<f64>,
    size: LogicalSize<f64>,
) -> Result<(), String> {
    let cfg = recipe_config(app_data, recipe_id).await?;
    let url_str = resolve_url(&cfg, custom_url, team)?;
    let url = Url::parse(&url_str).map_err(|e| format!("URL invalide « {url_str} » : {e}"))?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    // Runtime du recipe (scraping DOM des non-lus -> __pakeUnread) — best effort.
    let runtime = recipe_webview_js(app_data, recipe_id)
        .await
        .map(|js| format!("{RECIPE_PREAMBLE}{js}{RECIPE_SUFFIX}"));
    let label = format!("svc-{service_id}");

    // UA : override par service, sinon global, sinon chromeless Google, sinon SERVICE_UA.
    let ua = {
        let per_service = user_agent_pref.map(str::trim).filter(|s| !s.is_empty());
        if let Some(p) = per_service {
            p.to_string()
        } else {
            let global = state
                .settings
                .lock()
                .unwrap()
                .get("userAgentPref")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if !global.is_empty() {
                global
            } else if is_google_auth_host(&host) {
                GOOGLE_CHROMELESS_UA.to_string()
            } else {
                SERVICE_UA.to_string()
            }
        }
    };

    // Shim IPC injecté pour TOUS les services (Synology Chat… en dépendent).
    let mut builder = WebviewBuilder::new(label, WebviewUrl::External(url))
        .user_agent(&ua)
        .initialization_script(IPC_SHIM_JS);
    // Isolation du stockage par service : macOS -> data_store_identifier (data_directory
    // ignoré) ; Windows/Linux -> data_directory dédié (sinon sessions partagées).
    #[cfg(target_os = "macos")]
    {
        let store = uuid_to_bytes(service_id).ok_or("serviceId n'est pas un UUID")?;
        builder = builder.data_store_identifier(store);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let dir = app_data.join("sessions").join(service_id);
        let _ = std::fs::create_dir_all(&dir);
        builder = builder.data_directory(dir);
    }
    // Compat Google (userAgentData / window.chrome…) sur les services Google génériques.
    if is_google_host(&host) && !is_google_auth_host(&host) {
        builder = builder.initialization_script(GOOGLE_AUTH_COMPAT_JS);
    }
    if let Some(rt) = runtime {
        builder = builder.initialization_script(rt);
    }
    win.add_child(builder, pos, size)
        .map_err(|e| format!("Création de la webview du service échouée : {e}"))?;
    state.created.lock().unwrap().insert(service_id.to_string());
    Ok(())
}

#[tauri::command]
async fn show_service(
    app: AppHandle,
    state: State<'_, AppState>,
    service_id: String,
    recipe_id: String,
    custom_url: Option<String>,
    team: Option<String>,
    user_agent_pref: Option<String>,
) -> Result<(), String> {
    let win = app
        .get_window("main")
        .ok_or("Fenêtre principale introuvable")?;
    let label = format!("svc-{service_id}");
    let sw = *state.sidebar_w.lock().unwrap();
    let (pos, size) = service_rect(&win, sw)?;

    let exists = state.created.lock().unwrap().contains(&service_id);
    if exists {
        // Déjà chargé (ou préchargé) : simple repositionnement, aucun fetch réseau.
        if let Some(wv) = app.get_webview(&label) {
            let _ = wv.set_position(pos);
            let _ = wv.set_size(size);
        }
    } else {
        let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        create_service_webview(
            &state,
            &win,
            &app_data,
            &service_id,
            &recipe_id,
            custom_url.as_deref(),
            team.as_deref(),
            user_agent_pref.as_deref(),
            pos,
            size,
        )
        .await?;
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

// Précharge un service EN ARRIÈRE-PLAN : crée sa webview hors-écran (elle charge la page)
// sans devenir active. Le passage ultérieur à ce service est alors quasi instantané.
#[tauri::command]
async fn preload_service(
    app: AppHandle,
    state: State<'_, AppState>,
    service_id: String,
    recipe_id: String,
    custom_url: Option<String>,
    team: Option<String>,
    user_agent_pref: Option<String>,
) -> Result<(), String> {
    if state.created.lock().unwrap().contains(&service_id) {
        return Ok(());
    }
    let win = app
        .get_window("main")
        .ok_or("Fenêtre principale introuvable")?;
    let sw = *state.sidebar_w.lock().unwrap();
    let (_, size) = service_rect(&win, sw)?;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    // Hors-écran : la webview charge la page sans jamais recouvrir le service actif.
    let offscreen = LogicalPosition::new(-30000.0, 0.0);
    create_service_webview(
        &state,
        &win,
        &app_data,
        &service_id,
        &recipe_id,
        custom_url.as_deref(),
        team.as_deref(),
        user_agent_pref.as_deref(),
        offscreen,
        size,
    )
    .await?;
    if let Some(wv) = app.get_webview(&format!("svc-{service_id}")) {
        let _ = wv.hide();
    }
    Ok(())
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

// Ferme la webview d'UN service (pour la recréer avec de nouveaux paramètres).
#[tauri::command]
fn close_service(app: AppHandle, state: State<'_, AppState>, service_id: String) {
    if let Some(wv) = app.get_webview(&format!("svc-{service_id}")) {
        let _ = wv.close();
    }
    state.created.lock().unwrap().remove(&service_id);
    state.unread.lock().unwrap().remove(&service_id);
    if state.active.lock().unwrap().as_deref() == Some(service_id.as_str()) {
        *state.active.lock().unwrap() = None;
    }
}

// Change la largeur de la sidebar et repositionne la webview du service actif.
#[tauri::command]
fn set_sidebar_width(app: AppHandle, state: State<'_, AppState>, width: f64) {
    *state.sidebar_w.lock().unwrap() = width.clamp(160.0, 420.0);
    reposition_active(&app);
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
    state.flags.lock().unwrap().insert(
        service_id,
        ServiceFlags {
            notif,
            muted,
            badge,
        },
    );
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

// --- Workspaces -----------------------------------------------------------

#[tauri::command]
async fn create_workspace(state: State<'_, AppState>, name: String) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .post(format!("{base}/v1/workspace"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Création du workspace : HTTP {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_workspace(
    state: State<'_, AppState>,
    workspace_id: String,
    name: String,
    services: Vec<String>,
) -> Result<Value, String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .put(format!("{base}/v1/workspace/{workspace_id}"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .json(&serde_json::json!({ "name": name, "services": services }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Modification du workspace : HTTP {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_workspace(state: State<'_, AppState>, workspace_id: String) -> Result<(), String> {
    let (base, token) = current(&state)?;
    let res = reqwest::Client::new()
        .delete(format!("{base}/v1/workspace/{workspace_id}"))
        .bearer_auth(&token)
        .header(reqwest::header::USER_AGENT, API_UA)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Suppression du workspace : HTTP {}", res.status()));
    }
    Ok(())
}

#[tauri::command]
fn logout(app: AppHandle, state: State<'_, AppState>) {
    *state.server.lock().unwrap() = None;
    *state.token.lock().unwrap() = None;
    clear_session(&app);
}

// Repositionne la webview active sur la zone service (appelé au resize).
fn reposition_active(app: &AppHandle) {
    let st = app.state::<AppState>();
    let active = st.active.lock().unwrap().clone();
    let Some(sid) = active else { return };
    let sw = *st.sidebar_w.lock().unwrap();
    let Some(win) = app.get_window("main") else {
        return;
    };
    if let Ok((pos, size)) = service_rect(&win, sw) {
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
                    let st = app2.state::<AppState>();
                    let flags = st.flags.lock().unwrap().get(&sid).copied().unwrap_or_default();

                    // Notifications : on respecte mute / notifications désactivées.
                    if flags.notif && !flags.muted {
                        let private = st
                            .settings
                            .lock()
                            .unwrap()
                            .get("privateNotifications")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        if let Some(arr) = v.get("n").and_then(|x| x.as_array()) {
                            for notif in arr {
                                let title =
                                    notif.get("title").and_then(|x| x.as_str()).unwrap_or("").trim();
                                let body = notif.get("body").and_then(|x| x.as_str()).unwrap_or("");
                                if title.is_empty() && body.is_empty() {
                                    continue;
                                }
                                let b = app2.notification().builder();
                                let _ = if private {
                                    b.title("New message").show()
                                } else {
                                    b.title(if title.is_empty() { "Message" } else { title })
                                        .body(body)
                                        .show()
                                };
                            }
                        }
                    }

                    // Stocke le brut par service (pour la sidebar), calcule le total dock
                    // (flags appliqués) et émet la carte des non-lus vers le shell.
                    let (map, total) = {
                        let mut m = st.unread.lock().unwrap();
                        m.insert(sid.clone(), unread);
                        let f = st.flags.lock().unwrap();
                        let total: i64 = m
                            .iter()
                            .map(|(id, &u)| {
                                let fl = f.get(id).copied().unwrap_or_default();
                                if fl.badge && !fl.muted {
                                    u
                                } else {
                                    0
                                }
                            })
                            .sum();
                        (m.clone(), total)
                    };
                    let _ = app2.emit("unread", &map);
                    if let Some(win) = app2.get_window("main") {
                        let _ = win.set_badge_count(if total > 0 { Some(total) } else { None });
                    }
                },
            );
        }
    });
}

// --- Réglages app (app_settings.json) -----------------------------------

fn app_settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("app_settings.json"))
}

fn read_app_settings_value(app: &AppHandle) -> Value {
    let mut v = serde_json::json!({
        "autostart": false,
        "startMinimized": false,
        "theme": "system",
        "accentColor": "#ffc131",
        "closeToSystemTray": true,
        "privateNotifications": false,
        "showDisabledServices": true,
        "showServiceName": true,
        "showMessageBadgeWhenMuted": true,
        "userAgentPref": "",
        "sidebarWidth": 240,
        "iconSize": 24,
        "grayscaleServices": false,
        "grayscaleDim": 50,
        "sidebarServicesLocation": "top",
        "hibernationTimer": 0,
        "preloadServices": true
    });
    if let Some(p) = app_settings_path(app) {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Ok(stored) = serde_json::from_str::<Value>(&text) {
                if let (Some(base), Some(obj)) = (v.as_object_mut(), stored.as_object()) {
                    for (k, val) in obj {
                        base.insert(k.clone(), val.clone());
                    }
                }
            }
        }
    }
    v
}

#[tauri::command]
fn get_app_settings(app: AppHandle) -> Value {
    let mut v = read_app_settings_value(&app);
    // Reflète l'état réel de l'autostart (géré par le plugin).
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        if let Some(o) = v.as_object_mut() {
            o.insert("autostart".into(), Value::Bool(enabled));
        }
    }
    v
}

#[tauri::command]
fn set_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: Value,
) -> Result<Value, String> {
    // Effet de bord : autostart via le plugin.
    if let Some(enabled) = patch.get("autostart").and_then(Value::as_bool) {
        let res = if enabled {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        res.map_err(|e| format!("Autostart : {e}"))?;
    }
    // Fusionne + persiste.
    let mut v = read_app_settings_value(&app);
    if let (Some(base), Some(obj)) = (v.as_object_mut(), patch.as_object()) {
        for (k, val) in obj {
            base.insert(k.clone(), val.clone());
        }
    }
    if let Some(p) = app_settings_path(&app) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&p, v.to_string());
    }
    *state.settings.lock().unwrap() = v.clone();
    Ok(v)
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

// Ouvre/ferme les devtools sur la webview du service actif (debug), puis repose le layout.
fn toggle_devtools(app: &AppHandle) {
    #[cfg(debug_assertions)]
    {
        let active = app.state::<AppState>().active.lock().unwrap().clone();
        if let Some(sid) = active {
            if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
                if wv.is_devtools_open() {
                    wv.close_devtools();
                } else {
                    wv.open_devtools();
                }
            }
        }
        reposition_active(app);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = app;
    }
}

// Recharge la webview du service actif (comme « Reload service » de Ferdium).
fn reload_active_service(app: &AppHandle) {
    let active = app.state::<AppState>().active.lock().unwrap().clone();
    if let Some(sid) = active {
        if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
            let _ = wv.reload();
        }
    }
}

// Recharge le shell (l'UI de l'app) — comme « Reload Ferdium ». On masque d'abord les
// services (le shell se réaffiche puis re-sélectionne le service actif au montage).
fn reload_app(app: &AppHandle) {
    let created: Vec<String> = app
        .state::<AppState>()
        .created
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    for sid in created {
        if let Some(wv) = app.get_webview(&format!("svc-{sid}")) {
            let _ = wv.hide();
        }
    }
    if let Some(wv) = app.get_webview("main") {
        let _ = wv.reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn password_hash_is_base64_of_sha256() {
        // sha256("password") encodé en base64 (cf. UserApi.ts de ferdium-app).
        assert_eq!(
            ferdium_password_hash("password"),
            "XohImNooBHFR0OVvjcYpJ3NgPQ1qq73WKhHvch0VQtg="
        );
        // 32 octets -> 44 caractères base64, quel que soit l'input.
        assert_eq!(ferdium_password_hash("").len(), 44);
        assert_eq!(ferdium_password_hash("é&@ 123").len(), 44);
    }

    #[test]
    fn normalize_server_trims_and_strips_trailing_slashes() {
        assert_eq!(
            normalize_server("  https://api.ferdium.org/  "),
            "https://api.ferdium.org"
        );
        assert_eq!(normalize_server("https://x.y"), "https://x.y");
        assert_eq!(normalize_server("https://x.y///"), "https://x.y");
    }

    #[test]
    fn ensure_scheme_prepends_https_when_missing() {
        assert_eq!(ensure_scheme("example.com"), "https://example.com");
        assert_eq!(ensure_scheme("https://example.com"), "https://example.com");
        assert_eq!(ensure_scheme("http://example.com"), "http://example.com");
    }

    #[test]
    fn resolve_url_uses_plain_service_url() {
        let cfg = json!({ "config": { "serviceURL": "https://web.whatsapp.com" } });
        assert_eq!(
            resolve_url(&cfg, None, None).unwrap(),
            "https://web.whatsapp.com"
        );
    }

    #[test]
    fn resolve_url_honours_custom_url_only_when_allowed() {
        let cfg = json!({ "config": { "serviceURL": "https://default", "hasCustomUrl": true } });
        assert_eq!(
            resolve_url(&cfg, Some("chat.example.fr"), None).unwrap(),
            "https://chat.example.fr"
        );
        // URL custom vide -> retombe sur serviceURL.
        assert_eq!(
            resolve_url(&cfg, Some(""), None).unwrap(),
            "https://default"
        );
        // Recipe sans hasCustomUrl -> l'URL custom est ignorée.
        let cfg2 = json!({ "config": { "serviceURL": "https://default" } });
        assert_eq!(
            resolve_url(&cfg2, Some("chat.example.fr"), None).unwrap(),
            "https://default"
        );
    }

    #[test]
    fn resolve_url_substitutes_team_id() {
        let cfg =
            json!({ "config": { "serviceURL": "https://{teamId}.slack.com", "hasTeamId": true } });
        assert_eq!(
            resolve_url(&cfg, None, Some("acme")).unwrap(),
            "https://acme.slack.com"
        );
    }

    #[test]
    fn resolve_url_errors_without_service_url() {
        assert!(resolve_url(&json!({ "config": {} }), None, None).is_err());
        assert!(resolve_url(&json!({}), None, None).is_err());
    }

    #[test]
    fn uuid_to_bytes_parses_valid_and_rejects_bad() {
        let b = uuid_to_bytes("00112233-4455-6677-8899-aabbccddeeff").unwrap();
        assert_eq!(b[0], 0x00);
        assert_eq!(b[1], 0x11);
        assert_eq!(b[15], 0xff);
        // 32 hex sans tirets accepté aussi.
        assert!(uuid_to_bytes("00112233445566778899aabbccddeeff").is_some());
        // mauvaise longueur / hex invalide.
        assert!(uuid_to_bytes("abc").is_none());
        assert!(uuid_to_bytes("zz112233-4455-6677-8899-aabbccddeeff").is_none());
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // Cache des réglages app en mémoire (lu par le poller, la fermeture, etc.).
            *app.state::<AppState>().settings.lock().unwrap() =
                read_app_settings_value(&app.handle());
            {
                let st = app.state::<AppState>();
                let w = st
                    .settings
                    .lock()
                    .unwrap()
                    .get("sidebarWidth")
                    .and_then(Value::as_f64)
                    .unwrap_or(SIDEBAR_W);
                *st.sidebar_w.lock().unwrap() = w;
            }

            let handle = app.handle().clone();
            if let Some(win) = app.get_window("main") {
                win.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) => reposition_active(&handle),
                    // Au retour de focus (ex. après fermeture des devtools), on repose le
                    // layout : la webview du service peut avoir été redimensionnée et
                    // recouvert la sidebar.
                    WindowEvent::Focused(true) => reposition_active(&handle),
                    WindowEvent::CloseRequested { api, .. } => {
                        // « close to tray » : on cache au lieu de quitter (sinon on quitte).
                        let close_to_tray = handle
                            .state::<AppState>()
                            .settings
                            .lock()
                            .unwrap()
                            .get("closeToSystemTray")
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        if close_to_tray {
                            api.prevent_close();
                            if let Some(w) = handle.get_window("main") {
                                let _ = w.hide();
                            }
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

            // Menu natif macOS : App / Edit / View (Toggle Developer Tools).
            {
                let app_sub = Submenu::with_items(
                    app,
                    "Tauridium",
                    true,
                    &[
                        &PredefinedMenuItem::about(app, None, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::hide(app, None)?,
                        &PredefinedMenuItem::hide_others(app, None)?,
                        &PredefinedMenuItem::show_all(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::quit(app, None)?,
                    ],
                )?;
                let edit = Submenu::with_items(
                    app,
                    "Edit",
                    true,
                    &[
                        &PredefinedMenuItem::undo(app, None)?,
                        &PredefinedMenuItem::redo(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::cut(app, None)?,
                        &PredefinedMenuItem::copy(app, None)?,
                        &PredefinedMenuItem::paste(app, None)?,
                        &PredefinedMenuItem::select_all(app, None)?,
                    ],
                )?;
                let reload_svc = MenuItem::with_id(
                    app,
                    "reload-service",
                    "Reload Service",
                    true,
                    Some("CmdOrCtrl+R"),
                )?;
                let reload_app_item = MenuItem::with_id(
                    app,
                    "reload-app",
                    "Reload Tauridium",
                    true,
                    Some("CmdOrCtrl+Shift+R"),
                )?;
                let devtools = MenuItem::with_id(
                    app,
                    "toggle-devtools",
                    "Toggle Developer Tools",
                    true,
                    Some("CmdOrCtrl+Alt+I"),
                )?;
                let view = Submenu::with_items(
                    app,
                    "View",
                    true,
                    &[
                        &reload_svc,
                        &reload_app_item,
                        &PredefinedMenuItem::separator(app)?,
                        &devtools,
                    ],
                )?;
                let menu = Menu::with_items(app, &[&app_sub, &edit, &view])?;
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle-devtools" => toggle_devtools(app),
                    "reload-service" => reload_active_service(app),
                    "reload-app" => reload_app(app),
                    _ => {}
                });
            }

            // Demande l'autorisation de notifier au lancement.
            // NB : no-op sur macOS desktop (l'OS gère l'autorisation lui-même) ; réel
            // sur mobile / Windows / build .app signée.
            if let Ok(state) = app.notification().permission_state() {
                if state != PermissionState::Granted {
                    let _ = app.notification().request_permission();
                }
            }
            // « Démarrer en arrière-plan » : on cache la fenêtre au lancement.
            if read_app_settings_value(&app.handle())
                .get("startMinimized")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                if let Some(w) = app.get_window("main") {
                    let _ = w.hide();
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
            preload_service,
            hide_all_services,
            close_service,
            set_sidebar_width,
            close_services,
            logout,
            set_service_flags,
            update_service,
            create_service,
            delete_service,
            list_recipes,
            create_workspace,
            update_workspace,
            delete_workspace,
            get_app_settings,
            set_app_settings
        ])
        .build(tauri::generate_context!())
        .expect("erreur au lancement de l'application tauri")
        .run(|_app, _event| {
            // Clic sur l'icône du dock (macOS) -> réafficher la fenêtre.
            // RunEvent::Reopen n'existe que sur macOS -> gate pour compiler ailleurs.
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = _event {
                show_main(_app);
            }
        });
}
