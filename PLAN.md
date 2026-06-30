# pakeFerdium — Plan de conception

> Objectif : un **client Ferdium alternatif, léger** (Tauri v2, Rust + webview natif)
> en remplacement du client Electron — **qui reste connecté au serveur Ferdium**
> (api.ferdium.org ou self-hosted) : même compte, mêmes services/workspaces synchronisés.
> Cible principale : **macOS** (darwin).

## 1. Principe : on NE réinvente pas Ferdium, on en fait un client plus léger

- On garde **le serveur Ferdium** (la source de vérité : compte, services, workspaces, recipes).
- On remplace **uniquement le client Electron** par un client **Tauri** (~10 Mo vs ~200 Mo).
- Réutilisation maximale (licences OK) :
  - **ferdium-app** = **Apache-2.0** → on *porte* sa couche API serveur + ses modèles
    (service, recipe, workspace, calcul d'URL) de TS-Electron vers notre client TS. Attribution requise.
  - **ferdium-recipes** = **MIT** (par recipe) → on réutilise le catalogue (URL, user-agent, icône)
    et les `webview.js` (détection non-lus / notifs) via un **shim `Ferdium`** compatible.
- Ce qu'on NE garde pas : le runtime Electron, le serveur *local interne* embarqué (on parle
  directement au serveur distant en JWT), les dépendances lourdes (MobX/React si on veut + léger).

## 2. API serveur Ferdium (surface vérifiée)

Base : `https://<serveur>/v1/`. Auth : `POST /v1/auth/login` → **JWT**, renvoyé ensuite en
en-tête `Authorization: Bearer <jwt>` (middleware `auth:jwt`).

| Méthode | Route | Usage |
|---|---|---|
| POST | `/v1/auth/login` | login (email/password) → JWT |
| GET/PUT | `/v1/me` | compte utilisateur |
| GET | `/v1/me/services` | **liste des services de l'utilisateur** |
| POST / PUT | `/v1/service[/:id]` | créer / modifier un service |
| GET | `/v1/icon/:id` | icône d'un service |
| GET | `/v1/workspace` | **liste des workspaces** |
| POST / PUT | `/v1/workspace[/:id]` | créer / modifier un workspace |
| GET | `/v1/recipes/download/:recipe` | télécharger un recipe (paquet) |
| GET | `/v1/recipes/search` · `/recipes/popular` | rechercher / populaires |

> NB archi : le client Electron passe par un mini-serveur node *local* (en-tête
> `X-Ferdium-Local-Token`) qui proxifie le serveur distant. **Nous, on court-circuite** ce
> détour et on parle directement au serveur distant en JWT — plus simple, plus léger.

## 3. Réalité technique Tauri v2 (vérifiée)

| Brique | État | Implication |
|---|---|---|
| Multi-webview dans 1 fenêtre (`Window::add_child`) | ✅ flag `unstable`, bugs connus (#10011) | Création séquentielle / lazy |
| **Isolation session par service** | ✅ **VALIDÉ (Phase 0)** via `data_store_identifier` (PAS `data_directory`) | 1 UUID stable par service |
| Notifications natives | ✅ plugin `notification` ↔ API Web `Notification` | shim qui forwarde depuis le recipe |
| Badge dock / unread unifié | ⚠️ pas natif | custom (agrégation + badge macOS) |
| Recipe `webview.js` (nodeIntegration Electron) | ⚠️ pas de Node dans webview Tauri | shim `Ferdium` + adaptation (cas par cas) |

## 4. Architecture cible

```
┌──────────────────────────── Fenêtre Tauri (1) ─────────────────────────┐
│  Shell (UI : sidebar workspaces + services, état non-lus)              │
│  ┌──────┐  ┌───────────────────────────────────────────────────────┐   │
│  │ side │  │  Webview du service ACTIF (recipe rendu)              │   │
│  │ bar  │  │  - data_store_identifier = UUID(serviceId) (isolé)    │   │
│  │      │  │  - user-agent du recipe                               │   │
│  │      │  │  - shim Ferdium + webview.js (badges/notifs → IPC)    │   │
│  └──────┘  └───────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────┘
   Core Rust : fenêtre/webviews, sessions, notifs natives, badge dock, cache disque
   Couche sync (TS, portée de ferdium-app) : login JWT, /me/services, /workspace,
                                             recipes/download, calcul d'URL par service
   ▲ synchronise avec ▼
   ╔═══════════════════ Serveur Ferdium (api.ferdium.org ou self-hosted) ═══════════════╗
```

## 5. Stack

- **Backend** : Rust + **Tauri v2** (`features = ["unstable"]`), plugins `notification`, `http`.
- **Shell UI + couche sync** : **TypeScript + Vite** (framework léger : Svelte, ou vanilla).
  Porte les modules pertinents de ferdium-app (API serveur, modèles, URL building).
- **Persistance** : token (keychain via plugin), cache services/workspaces/recipes sur disque
  (résilience si serveur indispo — cf. issue ferdium #1838).
- **Sessions** : `data_store_identifier` = UUID dérivé du `serviceId`.

## 6. Plan par phases

### Phase 0 — Spike de dé-risquage ✅ FAIT (GO, 2026-06-30)
Multi-webview + isolation `data_store_identifier` + persistance : **validés sur macOS**.
Apprentissage clé : `data_directory` ignoré par WKWebView ; utiliser `data_store_identifier`.

### Phase 1 — Tranche verticale « connexion serveur » (prochaine)
But : **prouver la connexion à TON compte Ferdium**.
1. Écran de login (serveur URL + email/mot de passe) → `POST /v1/auth/login` → JWT stocké.
2. `GET /v1/me`, `GET /v1/me/services`, `GET /v1/workspace`.
3. Afficher la **vraie liste** de tes services/workspaces dans la sidebar (sans rendu webview encore).
Livrable : l'app se logge et liste tes services synchronisés. Le « pipe » serveur est prouvé.

### Phase 2 — Rendu des services ✅ FAIT
- Chaque service rendu dans une webview enfant isolée (`data_store_identifier` = UUID du serviceId),
  overlay à droite de la sidebar, lazy-load + switch + repositionnement au resize.
- URL résolue depuis le recipe (`config.serviceURL`/`hasCustomUrl`/`{teamId}`), recipes lus depuis
  `raw.githubusercontent.com/ferdium/ferdium-recipes` (cache disque).
- UA natif (pas de spoof Chrome). Session pakeFerdium persistée (session.json).
- **Patch wry** (`vendor/wry`, `[patch.crates-io]`) : `window.ipc` rendu mutable + shim IPC
  Electron injecté (no-op) pour tous les services → fait marcher Synology Chat & co.
  (cf. §7 et mémoire). Validé sur le compte réel : WhatsApp, Telegram, Discord, ChatGPT, Synology Chat.

### Phase 3 — Notifications & badges
- Shim `Ferdium` (`setBadge`, `injectCSS`, `setDialogTitle`, `safeParseInt`, `loop`) + `webview.js`.
- Notifs natives (plugin) + badge dock macOS = somme des non-lus.

### Phase 4 — Parité & confort
- Édition de services/workspaces (PUT/POST) répercutée serveur, dark mode, raccourcis,
  démarrage au login, résilience hors-ligne.

## 7. Points durs identifiés (honnêteté)

| Point dur | Approche |
|---|---|
| `webview.js` des recipes utilisent Node (Electron) | shim `Ferdium` ; adapter au cas par cas ; commencer par tes services |
| Calcul de l'URL finale par service (recipe + overrides) | porter la logique de ferdium-app (Apache-2.0) |
| Multi-webview `unstable` (white screens, races) | création séquentielle + lazy-load |
| Auth/keychain, refresh token | plugin stronghold/keychain ; re-login si expiré |

## 8. Sources

- Pake — https://github.com/tw93/pake
- Ferdium app (Apache-2.0) — https://github.com/ferdium/ferdium-app
- Ferdium server (API) — https://github.com/ferdium/ferdium-server · routes `/v1/*`
- Ferdium recipes (MIT) — https://github.com/ferdium/ferdium-recipes · docs/integration.md
- Auth client — https://deepwiki.com/ferdium/ferdium-app/7.2-authentication
- Tauri multi-webview — issues #2975 / #10011 ; isolation #11491 ; `data_store_identifier` (wry 0.55)
